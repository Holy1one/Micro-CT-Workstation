//! Independent device commands. Background jobs own inputs/proxies; only tick mutates Engine.
use crate::{Engine, Phase, nano_reference_ready, may_reuse_home, scan};
use crate::devices::camera::{CameraAdapter, CameraProperties};
use crate::devices::turntable::{NanoAdapter, NanoConnectionState};
use crate::devices::xray::MoxtekAdapter;
use crate::operations::Completion;
use serde_json::{json,Value};
use serde::Serialize;
use std::sync::{mpsc,Arc};
use std::sync::atomic::{AtomicBool,Ordering};
use std::path::PathBuf;
use std::time::{Duration,Instant};

#[derive(Clone,Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct RegisteredCapture {
    pub capture_id:String,pub session_id:String,pub operation_id:String,
    pub path:PathBuf,pub bytes:u64,pub sha256:String,pub format:String,pub exposure_ended:bool,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestConnections {
    pub nano:Option<Arc<dyn Fn()->Result<Arc<NanoAdapter>,String>+Send+Sync>>,
    pub camera:Option<Arc<dyn Fn()->Result<CameraAdapter,String>+Send+Sync>>,
    pub xray:Option<Arc<dyn Fn()->Result<MoxtekAdapter,String>+Send+Sync>>,
}

impl Engine {
    pub(crate) fn camera_scan_readiness(&self)->Value {
        let health=self.camera.as_ref().map(|c|c.health()).or_else(||self.cached_camera_health.clone()).unwrap_or_default();
        let issues=crate::capture_profile::scan_camera_issues(&health);
        json!({"ready":issues.is_empty(),"issues":issues})
    }
    pub(crate) fn set_capture_root(&mut self,payload:&Value)->Result<(),&'static str>{
        self.manual_control_allowed()?;
        let path=payload.get("savePath").and_then(Value::as_str).ok_or("INVALID_PARAMETERS")?.trim();
        if path.is_empty()||!PathBuf::from(path).is_absolute()||path.contains('\0'){return Err("INVALID_CAPTURE_DIRECTORY");}
        reject_capture_metadata_ancestors(&PathBuf::from(path)).map_err(|_|"CAPTURE_DIRECTORY_IS_SCAN_TASK")?;
        self.parameters.save_path=path.into();self.preflight=false;self.capture_profile=None;self.scan_geometry=None;self.scan_prompt=None;
        self.schedule_metadata_probe();Ok(())
    }
    pub(crate) fn prepare_scan(&mut self,payload:&Value)->Result<(),&'static str>{
        self.manual_control_allowed()?;
        #[derive(serde::Deserialize)]
        #[serde(rename_all="camelCase",deny_unknown_fields)]
        struct Preparation { #[serde(rename="type")] _command:Option<String>,task_id:String,projection_count:u32,
            #[serde(default)] max_xray_sec:Option<u32>,geometry:scan::ScanGeometry }
        let input:Preparation=serde_json::from_value(payload.clone()).map_err(|_|"INVALID_SCAN_PREPARATION")?;
        input.geometry.validate()?;
        if input.geometry.rotation_direction!="counterclockwise"||input.geometry.measurement!="measured"{return Err("INVALID_SCAN_GEOMETRY");}
        // An explicit limit for this task is accepted as given. Without one the
        // domain applies the destination-specific default when it freezes the
        // profile: 1000 s for a camera-card acquisition, 600 s for host RAM.
        let max_xray_sec=match input.max_xray_sec{
            Some(value)=>{if !crate::scan::scan_xray_seconds_valid(value){return Err("INVALID_PARAMETERS");}value}
            None=>self.max_xray_sec,
        };
        let exposure=if self.preview{self.parameters.exposure_ms}else{
            let health=self.camera.as_ref().ok_or("CAMERA_NOT_CONNECTED")?.health();
            crate::devices::camera::shutter_property_ms(health.properties.get("shutter").ok_or("CAMERA_SHUTTER_UNCONFIRMED")?).map_err(|_|"CAMERA_SHUTTER_UNCONFIRMED")?
        };
        let parameters=crate::Parameters{task_id:input.task_id.trim().into(),save_path:self.parameters.save_path.clone(),projection_count:input.projection_count,angle_step_deg:360.0/f64::from(input.projection_count),exposure_ms:exposure};
        parameters.validate()?;
        if !PathBuf::from(parameters.save_path.trim()).is_absolute(){return Err("INVALID_CAPTURE_DIRECTORY");}
        let root=PathBuf::from(parameters.save_path.trim()).join(parameters.task_id.trim());
        reject_capture_metadata_ancestors(&root).map_err(|_|"SCAN_OUTPUT_EXISTS_USE_NEW_TASK")?;
        if root.exists()&&std::fs::read_dir(&root).map_err(|_|"SCAN_OUTPUT_UNVERIFIED")?.next().is_some(){return Err("SCAN_OUTPUT_EXISTS_USE_NEW_TASK");}
        let independent=PathBuf::from(parameters.save_path.trim()).join("CameraCaptures");
        let actual_root=verified_candidate_path(&root).map_err(|_|"SCAN_OUTPUT_UNVERIFIED")?;
        let actual_independent=verified_candidate_path(&independent).map_err(|_|"CAPTURE_DIRECTORY_UNVERIFIED")?;
        if actual_root.components().any(|part|part.as_os_str().to_string_lossy().eq_ignore_ascii_case("CameraCaptures"))||crate::task_registry::lexical_path(&actual_root).starts_with(&crate::task_registry::lexical_path(&actual_independent)){return Err("RESERVED_TASK_DIRECTORY");}
        self.parameters=parameters;self.max_xray_sec=max_xray_sec;self.max_xray_sec_explicit=input.max_xray_sec.is_some();self.current=0;self.preflight=false;self.capture_profile=None;self.homed=false;self.phase=Phase::Idle;
        self.scan_geometry=Some(input.geometry);self.scan_references=scan::ReferenceFrames::default();self.scan_prompt=None;self.scan_stage="idle";self.restored_scan=None;self.restored_frames.clear();self.real_frames.clear();
        self.schedule_metadata_probe();Ok(())
    }
    pub(crate) fn enqueue_task_completed(&mut self)->Result<(),&'static str>{
        let plan=self.task_registry.prepare_completed(&self.parameters).map_err(|_|"TASK_COMPLETION_PREPARATION_FAILED")?;
        self.enqueue_device("task_storage","complete",move|cancel|{plan.execute(&cancel)?;Ok(Box::new(move|engine|{engine.task_registry.commit(plan)?;engine.phase=if engine.task_exit_pending{Phase::Stopping}else{Phase::Completed};engine.scan_stage=if engine.task_exit_pending{"stopping"}else{"completed"};engine.scan_prompt=None;engine.last_error=None;engine.metadata_completed=true;engine.metadata_manifest=true;engine.metadata_resume=false;Ok(())}))})
    }
    /// Safe finishing of a card acquisition. The record leaves the active slot, so
    /// the scan hardware is released and manual controls unlock, but it stays
    /// retained with its directory reserved and is never completed or
    /// reconstructable until the authorized USB import is verified. The registry
    /// itself re-validates the persisted schema-4 manifest (`acquired` plus an open
    /// data state with real card provenance), so this cannot promote an unproven
    /// acquisition.
    pub(crate) fn enqueue_acquired_pending_import(&mut self)->Result<(),&'static str>{
        let parameters=self.parameters.clone();
        self.enqueue_device("task_storage","card_pending_import",move|_cancel|{
            Ok(Box::new(move|engine|{
                engine.task_registry.mark_acquired_pending_import(&parameters)?;
                engine.phase=if engine.task_exit_pending{Phase::Stopping}else{Phase::Stopped};
                engine.scan_stage="pending_import";
                engine.scan_prompt=None;
                engine.last_error=None;
                engine.metadata_manifest=true;
                engine.metadata_resume=false;
                engine.metadata_completed=false;
                // The import state belongs to the newest pending-import task. The
                // registry just re-validated one card record per projection, so the
                // acquired projection count is proven evidence, not a plan value.
                let reporter=crate::devices::camera::card_import::CardImportReporter::new();
                let projections=parameters.projection_count;
                reporter.publish(|value|value.acquired_projections=projections);
                engine.card_import=Some(reporter);
                engine.log("PASS","system","Card acquisition safely finished; the task is retained as acquired with import required. The NEFs stay on the camera card until the authorized USB import is verified, and reconstruction stays locked.");
                Ok(())
            }))
        })
    }
    /// The retained pending-import task an import request names. An explicit
    /// `taskId`/`savePath` pair must match a retained record exactly; otherwise
    /// the active task is used when it is itself pending, a single pending task is
    /// accepted, and anything ambiguous is refused.
    fn pending_import_parameters(&self,payload:&Value)->Result<crate::Parameters,&'static str>{
        let pending:Vec<crate::Parameters>=self.task_registry.pending_import_tasks().iter().map(|record|record.parameters.clone()).collect();
        if pending.is_empty(){return Err("NO_PENDING_IMPORT_TASK");}
        match (payload.get("taskId").and_then(Value::as_str).map(str::trim).filter(|value|!value.is_empty()),
               payload.get("savePath").and_then(Value::as_str).map(str::trim).filter(|value|!value.is_empty())) {
            (Some(task_id),Some(save_path))=>pending.iter()
                .find(|parameters|parameters.task_id==task_id&&parameters.save_path==save_path)
                .cloned()
                .ok_or("PENDING_IMPORT_TASK_NOT_FOUND"),
            (None,None)=>{
                if pending.iter().any(|parameters|*parameters==self.parameters){Ok(self.parameters.clone())}
                else if pending.len()==1{Ok(pending[0].clone())}
                else{Err("PENDING_IMPORT_TASK_AMBIGUOUS")}
            }
            _=>Err("INVALID_PARAMETERS"),
        }
    }
    /// The authorized USB import entry point. It holds the camera transport
    /// occupation through the ordinary camera device operation, so an independent
    /// capture, Live View, a parameter change and a new scan cannot contend for the
    /// single COM owner while it runs. Nothing is started for a task the registry
    /// does not already retain as `acquired_pending_import`.
    pub(crate) fn enqueue_card_import(&mut self,payload:&Value)->Result<(),&'static str>{
        if self.preview{return Err("PRODUCTION_LOCKED");}
        if self.maintenance_lock{return Err("SOFTWARE_UPDATE_MAINTENANCE_LOCKED");}
        if self.real_scan.is_some()||self.busy(){return Err("DEVICE_CONTROL_LOCKED");}
        if self.operations.device_active("camera"){return Err("DEVICE_OPERATION_ACTIVE");}
        if self.card_import.as_ref().is_some_and(|reporter|reporter.progress.lock().map(|value|value.status==crate::devices::camera::card_import::IMPORT_STATUS_RUNNING).unwrap_or(false)){return Err("CARD_IMPORT_ACTIVE");}
        let parameters=self.pending_import_parameters(payload)?;
        let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
        let health=camera.health();
        if !health.connected||health.isolated||!health.cleanup_confirmed{return Err("CAMERA_NOT_CONNECTED");}
        if health.live_view.active{return Err("CAMERA_LIVE_VIEW_ACTIVE");}
        let reporter=crate::devices::camera::card_import::CardImportReporter::new();
        reporter.publish(|value|{value.status=crate::devices::camera::card_import::IMPORT_STATUS_RUNNING;value.stage="admission";});
        let camera_for_job=camera.clone();
        let parameters_for_job=parameters.clone();
        let reporter_for_job=reporter.clone();
        let operation_progress=reporter.operation.clone();
        self.enqueue_device("camera","card_import",move|cancel|{
            match crate::devices::camera::card_import::run_card_import(&camera_for_job,&parameters_for_job,&cancel,&reporter_for_job){
                Ok(summary)=>{
                    let parameters=parameters_for_job.clone();
                    Ok(Box::new(move|engine|engine.complete_card_import(&parameters,summary)))
                }
                Err(failure)=>{
                    let cancelled=matches!(failure,crate::devices::camera::card_import::CardImportFailure::Cancelled);
                    let message=failure.message();
                    reporter_for_job.publish(|value|{
                        value.status=if cancelled{crate::devices::camera::card_import::IMPORT_STATUS_CANCELLED}else{crate::devices::camera::card_import::IMPORT_STATUS_FAILED};
                        value.stage=if cancelled{"cancelled"}else{"failed"};
                        if !cancelled{value.error=Some(message.clone());}
                    });
                    Err(message)
                }
            }
        })?;
        let operation_id=self.operations.last_id.clone().unwrap_or_default();
        reporter.publish(|value|value.operation_id=operation_id.clone());
        self.operations.track_progress(&operation_id,operation_progress);
        self.card_import=Some(reporter);
        self.log("INFO","camera",&format!("Authorized USB import started for task {} on the same camera owner (operation {}); reconstruction stays locked until every record is verified and sealed",parameters.task_id,operation_id));
        Ok(())
    }
    /// Cancels a running import without touching the verified files, the
    /// acquisition journal or the retained pending-import state. The camera session
    /// is invalidated exactly like an operator Stop so an in-flight transfer is
    /// bounded; the task stays re-importable.
    pub(crate) fn cancel_card_import(&mut self)->Result<(),&'static str>{
        let running=self.card_import.as_ref().is_some_and(|reporter|reporter.progress.lock().map(|value|value.status==crate::devices::camera::card_import::IMPORT_STATUS_RUNNING).unwrap_or(false));
        if !running{return Err("CARD_IMPORT_NOT_RUNNING");}
        self.operations.cancel_device("camera");
        if let Some(camera)=self.camera.as_ref(){camera.request_cancel();}
        self.log("ACTION","camera","Card import cancellation requested; every verified file, the acquisition journal and the pending-import state are retained");
        Ok(())
    }
    /// Owner-side completion of a sealed import. The manifest already proves the
    /// verified state, so the registry promotes the retained record and only then
    /// does the current task become reconstructable.
    fn complete_card_import(&mut self,parameters:&crate::Parameters,summary:crate::devices::camera::card_import::CardImportSummary)->Result<(),String>{
        let registered=self.task_registry.mark_import_verified(parameters);
        if let Err(error)=registered{
            if let Some(reporter)=self.card_import.as_ref(){
                reporter.publish(|value|{value.status=crate::devices::camera::card_import::IMPORT_STATUS_FAILED;value.stage="failed";value.error=Some(error.clone());});
            }
            return Err(format!("The sealed card import could not be registered as completed: {error}"));
        }
        let current=self.parameters==*parameters;
        if current{
            // No fabricated completion: the engine's own reconstruction gate must
            // accept the durable manifest before this task is reported complete.
            let verified=crate::scan::completed_manifest_exists(&self.parameters);
            self.metadata_manifest=true;
            self.metadata_resume=false;
            self.metadata_completed=verified;
            if !verified{
                self.fault_latched=true;
                self.preflight=false;
                self.phase=Phase::Fault;
                self.scan_stage="fault";
                let error="The sealed card import is not accepted by the completed-manifest gate; reconstruction stays locked".to_owned();
                self.last_error=Some(error.clone());
                self.log("ERR","camera",&error);
                return Err(error);
            }
            self.phase=if self.task_exit_pending{Phase::Stopping}else{Phase::Completed};
            self.scan_stage=if self.task_exit_pending{"stopping"}else{"completed"};
            self.scan_prompt=None;
            self.last_error=None;
        }
        self.log("PASS","camera",&format!("Card import verified and sealed: {}/{} records with local file identities and {} projection(s); the retained task is registered as completed and reconstruction is unlocked",summary.verified,summary.records,summary.projections));
        Ok(())
    }
    pub(crate) fn enqueue_stop_for_shutdown(&mut self)->Result<(),&'static str>{self.enqueue_stop(true,false)}
    pub(crate) fn sync_camera_health(&mut self) {
        let Some(camera)=self.camera.as_ref()else{return;};
        let health=camera.health();
        self.sync_camera_readback(health.clone());
        self.reconcile_camera_cleanup_lock(&health);
    }
    /// A cleanup that only confirms after the Stop path's own wait has expired
    /// must release the camera-cleanup lock. Without this the operator would have
    /// to restart the workstation even though the termination evidence did
    /// arrive; the shared `fault_latched` still requires a successful preflight,
    /// and other locks (Abandon, maintenance, busy) are untouched.
    fn reconcile_camera_cleanup_lock(&mut self,health:&crate::devices::camera::CameraHealth) {
        if !health.cleanup_confirmed { return; }
        if !self.manual_lock.as_deref().is_some_and(|reason|reason.contains("Camera cleanup unconfirmed")) { return; }
        self.manual_lock=None;
        self.log("PASS","camera","Late camera cleanup confirmation arrived; the camera cleanup lock is released. The shared fault latch still requires a successful preflight");
    }
    fn sync_camera_readback(&mut self,health:crate::devices::camera::CameraHealth){
        let measured_changed=self.cached_camera_health.as_ref().is_some_and(|previous|previous.connected&&health.connected&&(
            previous.properties.len()!=health.properties.len()||health.properties.iter().any(|(key,property)|previous.properties.get(key).is_none_or(|old|old.actual!=property.actual||old.code!=property.code||old.data_type!=property.data_type))
        ));
        // A new camera session (reconnect or in-process restart) invalidates
        // every session-bound proof even when identity and values are identical.
        let session_changed=self.cached_camera_health.as_ref().is_some_and(|previous|previous.connected&&health.connected&&previous.session_id!=health.session_id);
        let unknown_properties=health.connected&&health.properties.values().any(|property|property.error.is_some()||(property.supported&&(!property.confirmed||property.actual.is_none())));
        let capability_evidence_changed=self.cached_camera_health.as_ref().is_some_and(|previous|previous.connected&&health.connected&&(
            previous.identity!=health.identity
            ||previous.device_info.as_ref().map(|info|(&info.dataset_sha256,&info.effective_properties))!=health.device_info.as_ref().map(|info|(&info.dataset_sha256,&info.effective_properties))
            ||previous.capabilities.properties!=health.capabilities.properties||previous.capabilities.host_capture!=health.capabilities.host_capture
            ||previous.capabilities.scan_nef!=health.capabilities.scan_nef||previous.capabilities.live_view!=health.capabilities.live_view
            ||previous.capabilities.autofocus!=health.capabilities.autofocus||previous.capabilities.focus_step!=health.capabilities.focus_step
            ||health.properties.iter().any(|(key,property)|previous.properties.get(key).is_none_or(|old|old.supported!=property.supported||old.native_writable!=property.native_writable||old.range!=property.range||old.supported_values!=property.supported_values))
        ));
        if unknown_properties||measured_changed||capability_evidence_changed||session_changed {self.preflight=false;self.capture_profile=None;}
        // The active disconnect owns both its transient cancellation state and
        // its bounded cleanup result. Starting a second Stop here would cancel
        // that result before its unconfirmed-cleanup evidence reaches the owner.
        let expected_disconnect=self.operations.device_active("software_update")||self.operations.history.iter().any(|operation|operation.device=="camera"&&operation.kind=="disconnect"&&matches!(operation.status.as_str(),"queued"|"running")&&!operation.cancel_requested);
        let lost=self.cached_camera_health.as_ref().is_some_and(|previous|previous.connected)&&!health.connected&&!expected_disconnect;
        let unconfirmed=health.isolated&&!health.cleanup_confirmed&&health.last_error.is_some()&&!expected_disconnect;
        self.cached_camera_health=Some(health);
        if (lost||unconfirmed) && !self.operations.device_active("safety") && !self.fault_latched {
            self.fault_latched=true;self.preflight=false;let _=self.enqueue_stop(true,false);self.phase=Phase::Fault;
        }
    }
    pub(crate) fn manual_control_allowed(&self)->Result<(),&'static str> {
        self.connection_control_allowed()?;
        self.task_registry.check_manual().map_err(|_|"UNFINISHED_TASK_LOCKED")?;
        Ok(())
    }
    fn connection_control_allowed(&self)->Result<(),&'static str>{
        if self.maintenance_lock{return Err("SOFTWARE_UPDATE_MAINTENANCE_LOCKED");}
        if self.busy(){return Err("DEVICE_CONTROL_LOCKED");}
        if self.manual_lock.is_some(){return Err("DEVICE_CLEANUP_UNCONFIRMED");}
        Ok(())
    }
    pub(crate) fn enqueue_device(&mut self,device:&str,kind:&str,job:impl FnOnce(Arc<AtomicBool>)->Result<Completion,String>+Send+'static)->Result<(),&'static str> {
        let device_session = match device {
            "camera" => self.camera.as_ref().and_then(|c|c.health().session_id).unwrap_or_else(||self.engine_session_id.clone()),
            _=>self.engine_session_id.clone(),
        };
        self.operations.spawn(&device_session,device,kind,job)?;
        Ok(())
    }
    pub(crate) fn enqueue_checkpoint(&mut self)->Result<(),&'static str>{
        let plan=self.task_registry.prepare_register(&self.parameters).map_err(|_|"TASK_REGISTRY_SAVE_FAILED")?;
        let parameters=self.parameters.clone();let kv=self.set_kv;let ua=self.set_ua;let max=self.max_xray_sec;let profile=self.capture_profile.clone().ok_or("CAPTURE_PROFILE_REQUIRED")?;
        self.enqueue_device("task_storage","checkpoint",move|cancel|{plan.execute(&cancel)?;if cancel.load(Ordering::SeqCst){return Err("Checkpoint cancelled".into());}scan::persist_preparation_checkpoint(&parameters,kv,ua,max,&profile)?;Ok(Box::new(move|engine|{engine.task_registry.commit(plan)?;engine.invalidate(Phase::Stopped);engine.scan_stage="stopped";engine.metadata_manifest=false;engine.metadata_resume=false;Ok(())}))})
    }
    pub(crate) fn schedule_metadata_probe(&mut self){
        self.metadata_generation+=1;self.metadata_manifest=false;self.metadata_resume=false;self.metadata_completed=false;
        if self.preview||self.parameters.validate().is_err(){return;}
        let parameters=self.parameters.clone();let generation=self.metadata_generation;
        let _=self.enqueue_device("task_metadata","metadata_probe",move|cancel|{
            let manifest=scan::manifest_file_exists(&parameters);let resume=scan::resume_manifest_exists(&parameters);let completed=scan::completed_manifest_exists(&parameters);
            if cancel.load(Ordering::SeqCst){return Err("Metadata probe cancelled".into());}
            Ok(Box::new(move|engine|{if engine.metadata_generation==generation&&engine.parameters==parameters{engine.metadata_manifest=manifest;engine.metadata_resume=resume;engine.metadata_completed=completed;}Ok(())}))
        });
    }
    pub(crate) fn sync_operations(&mut self) {
        for event in self.operations.drain() {
            let Some(result)=event.result else {self.operations.started(&event.id);continue;};
            if (event.generation!=self.operations.generation && !self.operations.terminal_task_write(&event.id)) || self.operations.history.iter().any(|op|op.operation_id==event.id&&op.cancel_requested) {
                // Cancellation invalidates successful mutations, not evidence
                // that OFF/cleanup remains unconfirmed. Never execute a stale
                // completion; retain its explicit failure and the safety lock.
                if let Err(error)=result {
                    if cleanup_unconfirmed_error(&error) {
                        self.record_operation_error(&event.id,error);
                        self.sequence+=1;
                        continue;
                    }
                }
                self.operations.complete(&event.id,"cancelled",None);
                continue;
            }
            // Remove the in-flight reservation before the owner applies a result.
            self.operations.complete(&event.id,"completed",None);
            let applied=result.and_then(|completion|completion(self));
            if let Err(error)=applied {
                self.record_operation_error(&event.id,error);
            }
            self.sequence+=1;
        }
    }
    fn record_operation_error(&mut self,id:&str,error:String) {
        // Reading an invalid project must not alter the selected dataset,
        // invalidate device evidence or trigger hardware cleanup.
        if self.operations.history.iter().any(|op|op.operation_id==id&&op.device=="task_project") {
            self.operations.complete(id,"rejected",Some(error.clone()));
            self.log("ERR","system",&error);
            return;
        }
        let cleanup_unconfirmed=cleanup_unconfirmed_error(&error);
        // Discovery can fail before any serial port is opened (missing USB,
        // descriptor failure or ambiguous identities). Only a never-owned
        // source qualifies: a former connection or any port evidence must keep
        // the ordinary fail-closed Stop/OFF and warning path.
        let unopened_xray_discovery=!cleanup_unconfirmed && !self.ever_xray && error.starts_with("discovery error:")
            && self.operations.history.iter().any(|op|op.operation_id==id&&op.device=="xray"&&op.kind=="connect")
            && self.xray.as_ref().is_none_or(|xray|{let health=xray.health();health.port.is_none()&&!health.connected&&!health.beam_on&&health.last_error.is_none()})
            && self.cached_xray_health.as_ref().is_none_or(|health|health.port.is_none()&&!health.connected&&!health.beam_on&&health.last_error.is_none());
        // A partial apply is the device refusing requested *values* while the
        // session stays healthy and every rejected key has an actual/unknown
        // readback. That is operator-correctable, so it must not latch the device
        // fault or run Stop: hardware showed a single out-of-range Live View
        // control (or camera property) tearing the whole camera session down and
        // making the wrong value unrecoverable without reconnecting. Preflight is
        // still invalidated below, so no scan can proceed on an unverified profile.
        let value_rejected=!cleanup_unconfirmed&&device_value_rejection_error(&error);
        let storage_rejected=!cleanup_unconfirmed&&self.real_scan.is_none()
            &&["SCAN_OUTPUT_RETAINED","SCAN_OUTPUT_EXISTS","SCAN_OUTPUT_UNVERIFIED"].iter().any(|code|error.starts_with(code))
            &&self.operations.history.iter().any(|op|op.operation_id==id&&op.device=="system"&&op.kind=="start_scan");
        if storage_rejected {
            if let Ok(true)=self.task_registry.discard_unpersisted_registration(&self.parameters){self.log("INFO","system","Rejected scan start released its unpersisted reservation; no acquisition was started.");}
        }
        self.operations.complete(id,if cleanup_unconfirmed {"cleanup_unconfirmed"} else if value_rejected || unopened_xray_discovery || storage_rejected {"rejected"} else {"failed"},Some(error.clone()));
        self.last_error=Some(error.clone());self.preflight=false;
        if cleanup_unconfirmed {self.fault_latched=true;self.phase=Phase::Fault;if self.manual_lock.as_deref()!=Some("Abandon waiting for confirmed cleanup"){self.manual_lock=Some(error.clone());}}
        self.log("ERR","devices",&error);
        if unopened_xray_discovery {
            self.xray=None;self.cached_xray_health=None;
            self.log("INFO","xray","Moxtek was not detected; no source session was opened. Check USB detection in Windows and retry.");
        }
        let device=self.operations.history.iter().find(|op|op.operation_id==id).map(|op|op.device.clone());
        if device.as_deref()==Some("task_storage")&&self.task_exit_pending {
            self.task_stop_cleanup_confirmed=false;self.fault_latched=true;self.phase=Phase::Fault;
        }
        if device.as_deref().is_some_and(|device|matches!(device,"camera"|"turntable"|"xray")) && !self.operations.device_active("safety") && !value_rejected && !unopened_xray_discovery {
            self.fault_latched=true;let _=self.enqueue_stop(true,false);self.phase=Phase::Fault;
        }
    }
    pub(crate) fn device_controls_snapshot(&self)->Value {
        let health=self.nano.as_ref().map(|n|n.health());
        let status=health.as_ref().and_then(|h|h.status.as_ref());
        let mut camera=serde_json::to_value(self.camera.as_ref().map(|c|c.control_snapshot()).or_else(||self.cached_camera_health.clone()).unwrap_or_default()).unwrap_or_else(|_|json!({}));
        camera["lastCapturedImage"]=self.captures.back().map(|capture|serde_json::to_value(capture).unwrap_or(Value::Null)).unwrap_or(Value::Null);
        json!({
            "camera":camera,
            "turntable":{
                "connected":health.as_ref().is_some_and(|h|h.state==NanoConnectionState::Connected),
                "identity":health.as_ref().and_then(|h|h.identity.clone()),"status":status,
                "confirmedAngleDeg":health.as_ref().and_then(crate::confirmed_nano_angle),
                "benchMode":self.bench_mode,"hardwareCertified":false,
                "reason":if self.maintenance_lock{Some("Software update maintenance ownership")}else if self.busy(){Some("Device controls occupied by task or cleanup")}else{self.manual_lock.as_deref()}
            },
            "taskLock":self.task_registry.snapshot(),
            "presets":self.preset_store.snapshot(),"cameraDraft":self.camera_draft,
            "exportJson":self.preset_export_json,"presetComparison":self.preset_comparison
        })
    }
    pub(crate) fn device_command(&mut self,command:&str,payload:Value)->Option<Result<(),&'static str>> {
        if !matches!(command,"connect"|"disconnect"|"stop"|"preflight"|"home"|"retry_device"|"restore_previous"|"start_scan"|"start_reconstruction"|"abandon_scan"|"preset_list"|"preset_save"|"preset_delete"|"preset_load"|"preset_compare"|"preset_import"|"preset_export"|"camera_pipe_descriptor"|"camera_test_capture"|"camera_connect"|"camera_disconnect"|"camera_status"|"camera_identity"|"camera_read_properties"|"camera_apply_properties"|"camera_capture"|"camera_live_view_start"|"camera_live_view_stop"|"camera_live_view_read"|"camera_live_view_configure"|"camera_autofocus"|"camera_focus"|"camera_card_import"|"camera_card_import_cancel"|"start_card_import"|"cancel_card_import"|"turntable_connect"|"turntable_disconnect"|"turntable_status"|"turntable_identity"|"turntable_home"|"turntable_relative"|"turntable_to_angle"|"turntable_bench_mode"|"xray_connect"|"xray_disconnect"|"xray_status"|"xray_identity"|"xray_toggle"|"xray_off"|"send_voltage"|"send_current"|"usb_auto_shut_down_toggle"|"set_usb_shutdown_delay") {return None;}
        Some(self.execute_device_command(command,payload))
    }
    fn execute_device_command(&mut self,command:&str,payload:Value)->Result<(),&'static str> {
        match command {
            "camera_status"|"camera_identity"|"turntable_identity"|"xray_status"|"xray_identity"=>Ok(()),
            "turntable_status"=>{
                let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
                // HOME/MOVE already publish their evidence from the same owner.
                // A queued STATUS must not time out behind them and fault a motion.
                if self.real_scan.is_some()||self.operations.device_active("turntable")||self.operations.device_active("safety"){return Ok(());}
                self.enqueue_device("turntable","status",move |cancel|{
                    let status=nano.status().map_err(|error|error.to_string())?;
                    if cancel.load(Ordering::SeqCst){return Err("Turntable status request cancelled".into());}
                    Ok(Box::new(move |engine|{if status.state=="FAULT"{engine.fault_latched=true;engine.preflight=false;engine.homed=false;engine.phase=Phase::Fault;return Err("Nano STATUS reports FAULT".into());}engine.last_error=None;Ok(())}))
                })
            }
            "camera_pipe_descriptor"=>{if self.camera.as_ref().and_then(|c|c.pipe_descriptor()).is_none(){return Err("LV_NOT_ACTIVE");}Ok(())},
            "stop"=>{
                // Explicit Stop ends the acquisition. Shutdown, disconnect and
                // fault cleanup keep their resumable registration.
                if !self.maintenance_lock && (self.task_registry.locked() || self.real_scan.is_some()
                    || matches!(self.phase,Phase::Running|Phase::Paused|Phase::Finishing)
                    || self.operations.history.iter().any(|op|op.kind=="start_scan"&&matches!(op.status.as_str(),"queued"|"running"))) {
                    self.task_exit_pending=true;
                    self.manual_lock=Some("Abandon waiting for confirmed cleanup".into());
                }
                self.enqueue_stop(true,false)
            }
            "xray_off"=>self.enqueue_stop(false,false),
            "disconnect"=>self.enqueue_stop(true,true),
            "abandon_scan"=>{
                if !self.task_registry.locked(){return Err("NO_ACTIVE_TASK");}
                if !self.task_registry.can_abandon(){return Err("TASK_REGISTRY_INVALID");}
                self.enqueue_stop(true,false)?;
                self.task_exit_pending=true;
                self.manual_lock=Some("Abandon waiting for confirmed cleanup".into());
                Ok(())
            }
            "preset_list"=>Ok(()),
            "preset_save"|"preset_delete"|"preset_load"|"preset_compare"|"preset_import"|"preset_export"=>self.preset_command(command,payload),
            "turntable_bench_mode"=>{
                self.manual_control_allowed()?;
                let enabled=payload.get("enabled").and_then(Value::as_bool).ok_or("INVALID_PARAMETERS")?;
                if enabled {
                    if self.ever_xray || self.xray.is_some() {return Err("BENCH_CANNOT_OVERRIDE_XRAY_SESSION");}
                    if payload.get("physicallyIsolated").and_then(Value::as_bool)!=Some(true) {return Err("PHYSICAL_ISOLATION_CONFIRMATION_REQUIRED");}
                }
                self.bench_mode=enabled;
                self.preflight=false;
                Ok(())
            }
            "connect"|"turntable_connect"=>{
                if command=="connect" && payload.get("adapter").and_then(Value::as_str)!=Some("real_hardware") {return Err(if payload["adapter"]=="developer_preview"{"PRODUCTION_LOCKED"}else{"INVALID_ADAPTER"});}
                self.connection_control_allowed()?;
                if self.nano.is_some() {return Err("ALREADY_CONNECTED");}
                #[cfg(test)]
                let factory=self.test_connections.nano.clone();
                self.ever_nano=true;
                self.enqueue_device("turntable","connect",move |cancel|{
                    #[cfg(not(test))]
                    let mut nano=NanoAdapter::new();
                    #[cfg(not(test))]
                    nano.discover_and_connect().map_err(|e|e.to_string())?;
                    #[cfg(not(test))]
                    let nano=Arc::new(nano);
                    #[cfg(test)]
                    let nano=factory.ok_or("No scripted Nano connection factory configured")?()?;
                    nano.status().map_err(|e|e.to_string())?;
                    if cancel.load(Ordering::SeqCst) {let _=nano.stop();return Err("Turntable connection cancelled".into());}
                    Ok(Box::new(move |engine|{engine.ever_nano=true;engine.nano=Some(nano);engine.connected=true;engine.reference_verified_this_process=false;engine.invalidate(Phase::Idle);engine.last_error=None;Ok(())}))
                })
            }
            "turntable_disconnect"=>{
                self.connection_control_allowed()?;
                let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
                self.enqueue_device("turntable","disconnect",move |_|{
                    nano.stop().map_err(|e|e.to_string())?;
                    nano.set_xray_warning(false).map_err(|e|e.to_string())?;
                    Ok(Box::new(|engine|{engine.nano=None;engine.connected=false;engine.homed=false;engine.reference_verified_this_process=false;engine.preflight=false;Ok(())}))
                })
            }
            "camera_connect"=>self.enqueue_camera_connect(payload),
            "camera_disconnect"=>{
                self.connection_control_allowed()?;
                let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
                self.enqueue_device("camera","disconnect",move |_|{let health=camera.disconnect_confirmed().map_err(|error|format!("Camera cleanup unconfirmed: {error}"))?;if !health.cleanup_confirmed{return Err("Camera cleanup unconfirmed; isolated proxy retained".into());}Ok(Box::new(move |engine|{engine.cached_camera_health=Some(health);engine.camera=None;engine.preflight=false;Ok(())}))})
            }
            "camera_read_properties"|"camera_apply_properties"|"camera_capture"|"camera_test_capture"|"camera_live_view_start"|"camera_live_view_stop"|"camera_live_view_read"|"camera_live_view_configure"|"camera_autofocus"|"camera_focus"=>self.enqueue_camera_control(command,payload),
            "camera_card_import"|"start_card_import"=>self.enqueue_card_import(&payload),
            "camera_card_import_cancel"|"cancel_card_import"=>self.cancel_card_import(),
            "home"|"turntable_home"=>self.enqueue_home(command=="home"),
            "turntable_relative"|"turntable_to_angle"=>self.enqueue_motion(command,payload),
            "retry_device"=>{
                match payload.get("device").and_then(Value::as_str) {
                    Some("camera")=>self.enqueue_camera_connect(payload),
                    Some("xray")=>self.enqueue_xray_connect(),
                    Some("turntable")=>{
                        if let Some(nano)=self.nano.as_ref().cloned() {
                            self.connection_control_allowed()?;
                            self.enqueue_device("turntable","status",move |_|{nano.ping().map_err(|e|e.to_string())?;nano.status().map_err(|e|e.to_string())?;Ok(Box::new(|engine|{engine.last_error=None;Ok(())}))})
                        } else {self.execute_device_command("turntable_connect",json!({}))}
                    }
                    _=>Err("INVALID_DEVICE"),
                }
            }
            "xray_connect"=>self.enqueue_xray_connect(),
            "xray_disconnect"=>{
                self.connection_control_allowed()?;
                let mut xray=self.xray.as_ref().cloned().ok_or("XRAY_NOT_CONNECTED")?;
                self.enqueue_device("xray","disconnect",move |_|{xray.disconnect().map_err(|e|format!("OFF unconfirmed: {e}"))?;Ok(Box::new(|engine|{engine.cached_xray_health=engine.xray.as_ref().map(|x|x.health());engine.xray=None;engine.beam_on=false;engine.preflight=false;engine.set_kv_confirmed=false;engine.set_ua_confirmed=false;Ok(())}))})
            }
            "xray_toggle"=>{
                if self.beam_on || self.xray.as_ref().is_some_and(|x|x.health().beam_on) {return self.enqueue_stop(false,false);}
                self.manual_control_allowed()?;
                if self.bench_mode {return Err("BENCH_MODE_XRAY_FORBIDDEN");}
                if self.fault_latched {return Err("FAULT_RECOVERY_REQUIRED");}
                if self.xray.is_none(){return Err("XRAY_NOT_CONNECTED");}
                if !self.set_kv_confirmed || !self.set_ua_confirmed {return Err("XRAY_SETPOINT_CONFIRMATION_REQUIRED");}
                let mut xray=self.xray.as_ref().cloned().ok_or("XRAY_NOT_CONNECTED")?;
                let max=self.max_xray_sec;
                self.enqueue_device("xray","beam_on",move |cancel|{let health=xray.beam_on_shared(cancel,Instant::now()+Duration::from_secs(u64::from(max))).map_err(|e|e.to_string())?;Ok(Box::new(move |engine|{engine.cached_xray_health=Some(health);engine.beam_on=true;Ok(())}))})
            }
            "send_voltage"|"send_current"=>{
                self.manual_control_allowed()?;
                if self.beam_on {return Err("XRAY_BEAM_ACTIVE");}
                let voltage=command=="send_voltage";
                let (kv,ua)=if voltage {
                    let kv=payload.get("kv").and_then(Value::as_f64).ok_or("INVALID_VOLTAGE")?;
                    (kv,self.set_ua.min(12000.0/kv))
                } else {
                    let ua=payload.get("ua").and_then(Value::as_f64).ok_or("INVALID_CURRENT")?;
                    let derived=if ua>0.0{self.set_kv.min(12000.0/ua)}else{self.set_kv};
                    // A verified voltage readback can land a few counts below the
                    // nominal bound (hardware reported 3.99971 kV for a 4.000 kV
                    // request). Reusing that readback as this command's voltage must
                    // not reject the operator's own confirmed minimum, and it must
                    // never raise the voltage above the confirmed setpoint.
                    let derived=crate::devices::xray::nominal_voltage(derived);
                    (derived,ua)
                };
                self.validate_setpoint(kv,ua)?;
                let clamped=if voltage {ua!=self.set_ua}else{kv!=self.set_kv};
                let requested=if voltage {kv}else{ua};
                let command_name=if voltage {"SEND V"}else{"SEND I"};
                self.log("ACTION","xray",&format!("{command_name} requested {requested:.3} {} · waiting for device confirmation",if voltage{"kV"}else{"µA"}));
                let mut xray=self.xray.as_ref().cloned().ok_or("XRAY_NOT_CONNECTED")?;
                self.enqueue_device("xray",command,move |_|{
                    let health=xray.set_parameters(kv,ua).map_err(|e|e.to_string())?;
                    Ok(Box::new(move |engine|{
                        let accepted_kv=health.set_voltage_kv.ok_or("Missing verified voltage")?;
                        let accepted_ua=health.set_current_ua.ok_or("Missing verified current")?;
                        if !health.beam_off_confirmed {return Err("OFF unconfirmed after setpoint write".into());}
                        engine.set_kv=accepted_kv;engine.set_ua=accepted_ua;
                        if voltage{engine.set_kv_confirmed=true;engine.set_ua_confirmed|=clamped;}else{engine.set_ua_confirmed=true;engine.set_kv_confirmed|=clamped;}
                        engine.cached_xray_health=Some(health);engine.preflight=false;
                        if clamped {engine.log("WARN","xray",&format!("{command_name} adjusted the other channel to respect the 12 W limit"));}
                        engine.log("PASS","xray",&format!("{command_name} applied and verified OFF · {accepted_kv:.3} kV / {accepted_ua:.3} µA"));
                        Ok(())
                    }))
                })
            }
            "usb_auto_shut_down_toggle"|"set_usb_shutdown_delay"=>{
                self.manual_control_allowed()?;
                let mut xray=self.xray.as_ref().cloned().ok_or("XRAY_NOT_CONNECTED")?;
                let enabled=!self.usb_auto_shut_down;
                let delay=if command=="set_usb_shutdown_delay"{Some(u16::try_from(payload.get("delay").and_then(Value::as_u64).ok_or("INVALID_PARAMETERS")?).map_err(|_|"INVALID_PARAMETERS")?)}else{None};
                if delay==Some(0){return Err("INVALID_PARAMETERS");}
                self.enqueue_device("xray",command,move |_|{
                    let health=if let Some(delay)=delay{xray.set_usb_shutdown_delay(delay)}else{if enabled && xray.health().beam_on{xray.force_off().map_err(|e|e.to_string())?;}xray.set_usb_auto_shutdown(enabled)}.map_err(|e:crate::devices::xray::XrayError|e.to_string())?;
                    Ok(Box::new(move |engine|{engine.cached_xray_health=Some(health);if delay.is_none(){engine.usb_auto_shut_down=enabled;engine.usb_auto_shut_down_confirmed=true;}engine.preflight=false;Ok(())}))
                })
            }
            "preflight"=>self.enqueue_preflight(payload),
            "restore_previous"=>self.enqueue_restore(),
            "start_scan"=>self.enqueue_start_scan(payload),
            "start_reconstruction"=>self.enqueue_reconstruction(payload),
            _=>Err("UNKNOWN_COMMAND"),
        }
    }
    fn enqueue_xray_connect(&mut self)->Result<(),&'static str> {
        self.connection_control_allowed()?;
        if self.xray.as_ref().is_some_and(|x|x.health().connected){return Err("XRAY_ALREADY_CONNECTED");}
        self.bench_mode=false;
        #[cfg(test)]
        if let Some(factory)=self.test_connections.xray.as_ref(){self.xray=Some(factory().map_err(|_|"XRAY_TEST_FACTORY_FAILED")?);}
        let mut xray=self.xray.get_or_insert_with(MoxtekAdapter::new).clone();
        self.enqueue_device("xray","connect",move |cancel|{
            #[cfg(not(test))]
            let health=xray.discover_and_connect().map_err(|e|e.to_string())?;
            #[cfg(test)]
            let health=if xray.health().connected{xray.refresh_off_status().map_err(|e|e.to_string())?}else{return Err("No scripted Moxtek connection factory configured".into());};
            if cancel.load(Ordering::SeqCst){let _=xray.force_off();return Err("Moxtek connection cancelled".into());}
            Ok(Box::new(move |engine|{engine.ever_xray=true;engine.cached_xray_health=Some(health);engine.beam_on=false;engine.usb_auto_shut_down=true;engine.usb_auto_shut_down_confirmed=false;engine.set_kv_confirmed=false;engine.set_ua_confirmed=false;engine.preflight=false;engine.last_error=None;Ok(())}))
        })
    }
    fn camera_unconfirmed_cleanup(&self)->bool {
        self.camera.as_ref().is_some_and(|camera|{let health=camera.health();health.isolated&&!health.cleanup_confirmed})
    }
    /// Only the camera-cleanup lock may be released by an explicit operator
    /// acknowledgement, and only while the camera really is an unconfirmed
    /// isolated session. `Stop` may join several reasons into one lock string, so
    /// the camera-cleanup marker is matched anywhere in it; the Abandon lock does
    /// not contain that marker and the shared fault latch is never touched.
    fn camera_cleanup_retirement_allowed(&self)->bool {
        self.manual_lock.as_deref().is_some_and(|reason|reason.contains("Camera cleanup unconfirmed"))
            &&self.camera_unconfirmed_cleanup()
    }
    fn enqueue_camera_connect(&mut self,payload:Value)->Result<(),&'static str> {
        // One judgement for "the operator selected a device": a blank or
        // whitespace-only id is treated as *not provided* everywhere below. A gate
        // that trims while the kind/worker decision used `is_some()` let
        // `deviceId:""` bypass the repeated-connect gate and still attempt an
        // explicit connect, reproducing the fault + isolation it prevents.
        let selected=payload.get("deviceId").and_then(Value::as_str).map(str::trim).filter(|id|!id.is_empty()).map(str::to_owned);
        let acknowledged=payload.get("acknowledgeUnconfirmedCleanup").and_then(Value::as_bool)==Some(true);
        match self.connection_control_allowed() {
            Ok(())=>{}
            Err("DEVICE_CLEANUP_UNCONFIRMED") if acknowledged&&selected.is_some()&&self.camera_cleanup_retirement_allowed()=>{
                // Explicit operator confirmation. The isolated proxy's transport is
                // already closed and it can never confirm cleanup, so it must be
                // retired instead of reused: keeping it would reject every request
                // and turn the recovery attempt into a new fault. The unconfirmed
                // session stays recorded in the operation history, the WARN log and
                // the shared fault latch; the snapshot shows the newly selected
                // session once it connects, and the global fault latch is
                // deliberately NOT cleared here because it is shared with the other
                // devices and with the Abandon lock.
                self.camera=None;
                self.manual_lock=None;
                self.preflight=false;
                self.log("WARN","camera","Operator explicitly acknowledged an unconfirmed camera cleanup; the retired session stays isolated and unknown, and a newly selected camera session may be opened");
            }
            Err(other)=>return Err(other),
        }
        // Locks and cleanup verdicts take precedence (checked above); beyond those,
        // selecting a session while a healthy one is already connected is an
        // operator state error, not a device fault. Reject it before enqueueing so
        // a repeated connect cannot latch the global fault, isolate the working
        // session or release the camera — all three were observed on hardware.
        if selected.is_some() {
            if let Some(camera)=self.camera.as_ref() {
                let health=camera.health();
                if health.connected && !health.isolated {
                    return Err("CAMERA_ALREADY_CONNECTED");
                }
            }
        }
        #[cfg(not(test))]
        let camera=match self.camera.as_ref(){Some(camera)=>camera.clone(),None=>CameraAdapter::discover().map_err(|_|"CAMERA_BACKEND_UNAVAILABLE")?};
        #[cfg(test)]
        let camera=match self.camera.as_ref(){Some(camera)=>camera.clone(),None=>self.test_connections.camera.as_ref().ok_or("CAMERA_TEST_FACTORY_REQUIRED")?().map_err(|_|"CAMERA_BACKEND_UNAVAILABLE")?};
        // Retain the proxy immediately so global Stop can invalidate a stuck connect.
        self.ever_camera=true;self.camera=Some(camera.clone());
        // A discovery-only request never opens a session, so its receipt must not
        // claim a connection: the operation kind is what a polling frontend sees.
        let kind=if selected.is_some(){"connect"}else{"camera_discover"};
        self.enqueue_device("camera",kind,move |cancel|{
            let list=camera.discover_list().map_err(|e|e.to_string())?;
            let Some(selected)=selected else {return Ok(Box::new(move |engine|{
                let connected=camera.health().connected;
                engine.cached_camera_health=Some(camera.health());engine.preflight=false;
                // How many devices discovery found is an operator instruction, not
                // a device fault: escalating it would latch the shared fault and
                // issue a global Stop for a plain "nothing selected yet" outcome.
                // The discovered list in the snapshot is the machine-readable
                // signal; the log carries the wording.
                engine.log("INFO","camera",&if connected{
                    "Camera discovery refreshed for the selected session".to_owned()
                }else if list.is_empty(){
                    "Camera discovery found no device; nothing was selected and no session was opened".to_owned()
                }else if list.len()==1{
                    "Camera discovery found one device; select it explicitly to open a session".to_owned()
                }else{
                    format!("Camera discovery found {} devices; select exactly one deviceId to open a session",list.len())
                });
                Ok(())
            }));};
            camera.connect_selected(&selected).map_err(|e|e.to_string())?;
            if cancel.load(Ordering::SeqCst){camera.request_cancel();return Err("Camera connection cancelled".into());}
            Ok(Box::new(move |engine|{engine.cached_camera_health=Some(camera.health());engine.preflight=false;engine.last_error=None;Ok(())}))
        })
    }
    fn enqueue_camera_control(&mut self,command:&str,payload:Value)->Result<(),&'static str> {
        if command=="camera_live_view_stop" && self.camera.is_none(){return Ok(());}
        // Stop Live View remains available during task occupation and cleanup.
        if matches!(command,"camera_read_properties"|"camera_live_view_read"){self.connection_control_allowed()?;}
        else if command!="camera_live_view_stop"{self.manual_control_allowed()?;}
        let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
        if command=="camera_live_view_stop" && self.operations.device_active("camera") {
            self.operations.cancel_device("camera");camera.request_cancel();
            return self.enqueue_device("camera_cleanup","live_view_stop",move |_|{
                let deadline=Instant::now()+Duration::from_secs(6);
                while !camera.health().cleanup_confirmed&&Instant::now()<deadline{std::thread::sleep(Duration::from_millis(20));}
                if !camera.health().cleanup_confirmed{return Err("Live View cleanup unconfirmed; old start session isolated".into());}
                Ok(Box::new(move |engine|{engine.cached_camera_health=Some(camera.health());engine.preflight=false;Ok(())}))
            });
        }
        let health=camera.health();
        if command=="camera_live_view_stop"&&!health.connected&&health.cleanup_confirmed{return Ok(());}
        if !health.connected || health.isolated{return Err("CAMERA_NOT_READY");}
        let focus_step=if command=="camera_focus"{
            let value=payload.get("step").and_then(Value::as_i64).ok_or("INVALID_CAMERA_FOCUS_STEP")?;
            Some(crate::devices::camera::validate_focus_step(value).map_err(|_|"INVALID_CAMERA_FOCUS_STEP")?)
        }else{None};
        let live_session=if matches!(command,"camera_live_view_read"|"camera_live_view_configure"){
            let session=payload.get("sessionId").and_then(Value::as_str).filter(|s|s.len()==32&&s.bytes().all(|b|b.is_ascii_hexdigit())).ok_or("INVALID_LIVE_VIEW_SESSION")?;
            if !health.live_view.active||health.live_view.session_id.as_deref()!=Some(session){return Err("STALE_LIVE_VIEW_SESSION");}Some(session.to_owned())
        }else{None};
        let live_controls:Option<CameraProperties>=if command=="camera_live_view_configure"{
            let controls:CameraProperties=serde_json::from_value(payload.get("controls").cloned().ok_or("INVALID_LIVE_VIEW_CONTROLS")?).map_err(|_|"INVALID_LIVE_VIEW_CONTROLS")?;
            if controls.is_empty()||controls.len()>2||controls.iter().any(|(key,value)|!matches!(key.as_str(),"cameraZoom"|"focusMode")||!matches!(value.integer(),Some(0..=255))){return Err("INVALID_LIVE_VIEW_CONTROLS");}Some(controls)
        }else{None};
        if matches!(command,"camera_capture"|"camera_test_capture") && health.live_view.active {return Err("LIVE_VIEW_CAPTURE_FORBIDDEN");}
        let properties:Option<CameraProperties>=if command=="camera_apply_properties"{Some(serde_json::from_value(payload.get("properties").cloned().ok_or("INVALID_PARAMETERS")?).map_err(|_|"INVALID_PARAMETERS")?)}else{None};
        if let Some(properties)=properties.as_ref(){camera.property_rejection(properties).map_err(|rejection|rejection.code())?;self.preflight=false;}
        let capture_dir=if matches!(command,"camera_capture"|"camera_test_capture") {
            let expected=PathBuf::from(self.parameters.save_path.trim()).join("CameraCaptures");
            if command=="camera_capture" {
                if self.parameters.save_path.trim().is_empty(){return Err("CAMERA_CAPTURE_ROOT_REQUIRED");}
                if payload.get("directory").and_then(Value::as_str).is_some_and(|path|crate::task_registry::lexical_path(&PathBuf::from(path))!=crate::task_registry::lexical_path(&expected)){return Err("CAMERA_CAPTURE_DIRECTORY_REQUIRES_ROOT");}
                expected
            }else{match payload.get("directory").and_then(Value::as_str){Some(path)if !path.trim().is_empty()=>PathBuf::from(path),_=>{if self.parameters.save_path.trim().is_empty(){return Err("CAMERA_CAPTURE_ROOT_REQUIRED");}expected}}}
        }else{PathBuf::new()};
        let mut capture_roots=self.task_registry.capture_roots();
        if !self.parameters.save_path.trim().is_empty()&&!self.parameters.task_id.trim().is_empty(){capture_roots.push(PathBuf::from(self.parameters.save_path.trim()).join(self.parameters.task_id.trim()));}
        if matches!(command,"camera_capture"|"camera_test_capture"){
            if !capture_dir.is_absolute(){return Err("INVALID_CAPTURE_DIRECTORY");}
            let known_root=PathBuf::from(self.parameters.save_path.trim()).join(self.parameters.task_id.trim());
            if (!self.parameters.save_path.trim().is_empty()&&!self.parameters.task_id.trim().is_empty()&&crate::task_registry::lexical_path(&capture_dir).starts_with(&crate::task_registry::lexical_path(&known_root)))||self.task_registry.capture_directory_conflicts(&capture_dir){return Err("CAPTURE_DIRECTORY_IS_SCAN_TASK");}
        }
        let tag=payload.get("presetId").and_then(Value::as_str).map(|id|self.preset_store.get(id).ok_or("PRESET_NOT_FOUND")).transpose()?.map(|preset|preset.tag());
        // Request validation above stays authoritative (missing/invalid parameters
        // and capture roots keep their documented codes). Only now, with a well
        // formed request, are the published readiness/capability flags enforced so
        // a request the snapshot already marks unqualified becomes a clean
        // rejection instead of a worker-level failure that latches a fault and
        // tears down the whole camera session. Only the exact precondition the
        // worker would apply is hoisted, so no new capability is granted.
        if matches!(command,"camera_capture"|"camera_test_capture")&&crate::devices::camera::capture_readiness(&health).is_err(){return Err("CAMERA_CAPTURE_UNQUALIFIED");}
        if command=="camera_focus"&&(!health.capabilities.focus_step||!health.live_view.active){return Err("CAMERA_FOCUS_UNQUALIFIED");}
        if command=="camera_autofocus"&&!health.capabilities.autofocus{return Err("CAMERA_AUTOFOCUS_UNQUALIFIED");}
        let kind=command.to_owned();let operation=self.operations.next_id(health.session_id.as_deref().unwrap_or(&self.engine_session_id));
        self.enqueue_device("camera",command,move |cancel|{
            let mut captured=None;
            match kind.as_str(){
                "camera_read_properties"=>{camera.read_properties().map_err(|e|e.to_string())?;}
                "camera_live_view_read"=>{camera.read_live_view_controls(live_session.as_deref().expect("validated LV session")).map_err(|e|e.to_string())?;}
                "camera_live_view_configure"=>{let result=camera.configure_live_view(live_session.as_deref().expect("validated LV session"),live_controls.as_ref().expect("validated LV controls")).map_err(|e|e.to_string())?;if !result.complete{return Err(format!("Live View control application incomplete: {:?}",result.failed));}}
                "camera_apply_properties"=>{
                    let result=camera.apply_properties(properties.as_ref().expect("validated properties")).map_err(|e|e.to_string())?;
                    if !result.complete{return Err(format!("Camera parameter application partial: {:?}",result.failed));}
                }
                "camera_capture"|"camera_test_capture"=>{
                    reject_capture_metadata_ancestors(&capture_dir)?;
                    for root in &capture_roots{if let(Ok(directory),Ok(root))=(std::fs::canonicalize(&capture_dir),std::fs::canonicalize(root)){if directory.starts_with(root){return Err("Independent capture aliases a retained scan directory".into());}}}
                    std::fs::create_dir_all(&capture_dir).map_err(|e|e.to_string())?;
                    captured=Some(camera.capture_independent_to_path(&capture_dir.join(format!("{operation}.nef")),&operation,&cancel).map_err(|e|e.to_string())?);
                }
                "camera_live_view_start"=>{camera.start_live_view().map_err(|e|e.to_string())?;}
                "camera_live_view_stop"=>{camera.stop_live_view().map_err(|e|e.to_string())?;}
                "camera_autofocus"=>camera.autofocus().map_err(|e|e.to_string())?,
                "camera_focus"=>camera.focus_step(focus_step.expect("validated focus step")).map_err(|e|e.to_string())?,
                _=>return Err("Unsupported camera command".into()),
            }
            if cancel.load(Ordering::SeqCst){return Err("Camera operation cancelled".into());}
            Ok(Box::new(move |engine|{
                let health=camera.health();
                if let Some(image)=captured {
                    if !image.exposure_ended || image.size==0 || image.sha256.len()!=64 || image.operation_id!=operation || Some(image.session_id.as_str())!=health.session_id.as_deref(){return Err("Independent capture completion evidence mismatch".into());}
                    engine.captures.push_back(RegisteredCapture{capture_id:operation.clone(),session_id:image.session_id,operation_id:image.operation_id,path:image.path,bytes:image.size,sha256:image.sha256,format:image.format,exposure_ended:image.exposure_ended});
                    if engine.captures.len()>128{engine.captures.pop_front();}
                    engine.log("PASS","camera","Single host NEF output saved and registered with complete file/hash evidence; physical exposure count is not established");
                }
                if kind=="camera_apply_properties" {
                    if properties.as_ref().is_some_and(|properties|properties.contains_key("shutter")) {
                        let shutter=health.properties.get("shutter").ok_or("Applied shutter lacks a described readback")?;
                        engine.parameters.exposure_ms=crate::devices::camera::shutter_property_ms(shutter).map_err(|error|error.to_string())?;
                    }
                    engine.preset_tag=tag;engine.preflight=false;engine.capture_profile=None;
                }
                engine.cached_camera_health=Some(health);engine.last_error=None;Ok(())
            }))
        })
    }
    fn enqueue_home(&mut self,recovery:bool)->Result<(),&'static str> {
        if recovery {
            self.connection_control_allowed()?;
            if !self.preflight{return Err("PREFLIGHT_REQUIRED");}
            if self.task_registry.locked()&&!self.task_registry.matches(&self.parameters){return Err("UNFINISHED_TASK_LOCKED");}
        }else{self.manual_control_allowed()?;}
        let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
        let xray=self.xray.as_ref().cloned();
        if xray.is_none() && (!self.bench_mode || self.ever_xray){return Err("HOME_REQUIRES_XRAY_OFF_OR_ISOLATED_BENCH");}
        self.enqueue_device("turntable","home",move |cancel|{
            if let Some(mut xray)=xray{xray.force_off().map_err(|e|format!("OFF unconfirmed before HOME: {e}"))?;}
            if cancel.load(Ordering::SeqCst){return Err("HOME cancelled before motion".into());}
            let status=nano.status().map_err(|e|e.to_string())?;
            if status.state=="FAULT"{nano.clear_fault_cancelled(cancel.clone()).map_err(|e|e.to_string())?;}
            nano.rearm_cancelled(cancel.clone()).map_err(|e|e.to_string())?;
            let status=nano.home_cancelled(cancel.clone()).map_err(|e|e.to_string())?;
            if cancel.load(Ordering::SeqCst){let _=nano.stop();return Err("HOME cancelled".into());}
            if !nano_reference_ready(&status){return Err("HOME completed without valid idle reference".into());}
            Ok(Box::new(move |engine|{engine.homed=true;engine.reference_verified_this_process=true;engine.beam_on=false;engine.phase=if engine.preflight{Phase::Ready}else{Phase::Idle};engine.last_error=None;Ok(())}))
        })
    }
    fn enqueue_motion(&mut self,command:&str,payload:Value)->Result<(),&'static str> {
        self.manual_control_allowed()?;
        let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
        let angle=payload.get("angleDeg").and_then(Value::as_f64).ok_or("INVALID_PARAMETERS")?;
        if !angle.is_finite() || angle<0.0 || angle>360.0 || (angle*1000.0-(angle*1000.0).round()).abs()>1e-6{return Err("INVALID_MOTION_ANGLE");}
        let relative=command=="turntable_relative";
        if relative && angle<=0.0 || !relative && angle>=360.0{return Err("INVALID_MOTION_ANGLE");}
        let mdeg=(angle*1000.0).round()as i32;
        self.enqueue_device("turntable",command,move |cancel|{
            let status=nano.status().map_err(|e|e.to_string())?;
            let plan=if relative{crate::devices::turntable::plan_relative(&status,mdeg)}else{crate::devices::turntable::plan_to_phase(&status,mdeg)}.map_err(|e|e.to_string())?;
            if cancel.load(Ordering::SeqCst){return Err("Motion cancelled before command".into());}
            if !plan.is_noop(){
                let ticket=nano.execute_plan_cancelled(&plan,cancel.clone()).map_err(|e|e.to_string())?;
                let held=nano.status().map_err(|e|e.to_string())?;
                crate::devices::turntable::verify_plan_capture_hold(&plan,&ticket,&held).map_err(|e|e.to_string())?;
                if cancel.load(Ordering::SeqCst){let _=nano.stop();return Err("Motion cancelled in capture hold".into());}
                nano.capture_done_cancelled(ticket.command_id,cancel.clone()).map_err(|e|e.to_string())?;
                let idle=nano.status().map_err(|e|e.to_string())?;
                crate::devices::turntable::verify_plan_released(&plan,&idle).map_err(|e|e.to_string())?;
            }
            Ok(Box::new(|engine|{engine.last_error=None;Ok(())}))
        })
    }
    fn enqueue_preflight(&mut self,payload:Value)->Result<(),&'static str> {
        self.connection_control_allowed()?;
        self.require_connected()?;
        if self.task_registry.locked()&&!self.task_registry.matches(&self.parameters){return Err("UNFINISHED_TASK_LOCKED");}
        if self.busy(){return Err("SCAN_ACTIVE");}
        self.parameters.validate()?;
        if !crate::scan::scan_xray_seconds_valid(self.max_xray_sec){return Err("INVALID_PARAMETERS");}
        let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
        let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
        let mut xray=self.xray.as_ref().cloned().ok_or("XRAY_NOT_CONNECTED")?;
        let recovery=self.task_registry.matches(&self.parameters);
        if !recovery&&self.scan_geometry.is_none(){return Err("SCAN_PREPARATION_REQUIRED");}
        if recovery && (payload.get("restore").and_then(Value::as_bool)!=Some(true)||payload.get("confirmUsbAutoShutdownRelease").and_then(Value::as_bool)!=Some(true)){return Err("RESTORE_PREFLIGHT_CONFIRMATION_REQUIRED");}
        if !recovery {
            if !self.usb_auto_shut_down_confirmed || self.usb_auto_shut_down{return Err("USB_AUTO_SHUTDOWN_RELEASE_REQUIRED");}
            if !self.set_kv_confirmed || !self.set_ua_confirmed{return Err("XRAY_SETPOINT_CONFIRMATION_REQUIRED");}
        }
        let unchanged=recovery&&payload.get("confirmTurntableUnchanged").and_then(Value::as_bool)==Some(true);
        let confirmed_home_zero=unchanged&&payload.get("confirmReferenceAtHome").and_then(Value::as_bool)==Some(true);
        let params=self.parameters.clone();let kv=self.set_kv;let ua=self.set_ua;let reused=self.reference_verified_this_process;let preset=self.preset_tag.clone();
        self.preflight=false;
        self.enqueue_device("system","preflight",move |cancel|{
            let (kv,ua,frozen_max,frozen_profile)=if recovery {let(kv,ua,max,profile)=read_frozen_recovery(&params)?;(kv,ua,Some(max),Some(profile))}else{(kv,ua,None,None)};
            if scan::manifest_file_exists(&params) && !scan::resume_manifest_exists(&params){return Err("Scan output exists; use a new task".into());}
            if camera.health().live_view.active{return Err("LIVE_VIEW_CAPTURE_FORBIDDEN: explicitly stop Live View".into());}
            camera.read_properties().map_err(|e|e.to_string())?;
            let mut profile=freeze_capture_profile(&camera,&nano,preset)?;
            if let Some(frozen)=frozen_profile.as_ref(){frozen.matches(&profile)?;profile.preset=frozen.preset.clone();}
            if profile.exposure_ms()!=Some(params.exposure_ms){return Err("Camera shutter differs from the shared Exposure target".into());}
            xray.force_off().map_err(|e|format!("OFF unconfirmed during preflight: {e}"))?;
            if cancel.load(Ordering::SeqCst){return Err("Preflight cancelled before setpoint/USB writes".into());}
            if recovery {let usb=xray.set_usb_auto_shutdown(false).map_err(|e|e.to_string())?;if usb.usb_auto_shutdown!=Some(false){return Err("USB auto-shutdown release lacks current-session confirmation".into());}}
            let health=xray.set_parameters(kv,ua).map_err(|e|e.to_string())?;
            if health.locked!=Some(false)||!health.beam_off_confirmed||health.temperature_c.is_none_or(|c|!c.is_finite()||c>65.0){return Err("Moxtek health not safe for preflight".into());}
            let status=nano.status().map_err(|e|e.to_string())?;
            let mut reusable=may_reuse_home(reused,unchanged,&status);
            if unchanged && !reusable {
                // Verify retained acquisition before trusting a stopped controller counter.
                let checkpoint=scan::load_resume(&params,kv,ua,&profile)?;
                let at_confirmed_origin=confirmed_home_zero&&status.hall_active&&status.position_pulses==0;
                if status.state=="FAULT" || (!at_confirmed_origin&&!scan::recovery_position_matches(&checkpoint,&params,status.position_pulses,status.pulses_per_rev)) {
                    return Err("REFERENCE_RECOVERY_POSITION_UNVERIFIED: current position does not match the checkpoint; do not continue without re-establishing the reference".into());
                }
                nano.set_xray_warning(false).map_err(|e|e.to_string())?;
                nano.rearm_cancelled(cancel.clone()).map_err(|e|e.to_string())?;
                let restored=nano.restore_reference_cancelled(status.position_pulses,cancel.clone()).map_err(|e|e.to_string())?;
                reusable=nano_reference_ready(&restored);
            }
            if status.state=="FAULT"{nano.clear_fault_cancelled(cancel.clone()).map_err(|e|e.to_string())?;}
            if !reusable{nano.rearm_cancelled(cancel.clone()).map_err(|e|e.to_string())?;}
            if cancel.load(Ordering::SeqCst){return Err("Preflight cancelled".into());}
            let restoring=scan::resume_manifest_exists(&params);
            Ok(Box::new(move |engine|{engine.capture_profile=Some(profile.clone());engine.cached_camera_health=Some(camera.health());engine.set_kv=health.set_voltage_kv.ok_or("Verified voltage missing")?;engine.set_ua=health.set_current_ua.ok_or("Verified current missing")?;engine.cached_xray_health=Some(health);if recovery{engine.set_kv_confirmed=true;engine.set_ua_confirmed=true;engine.usb_auto_shut_down=false;engine.usb_auto_shut_down_confirmed=true;if let Some(max)=frozen_max{engine.max_xray_sec=max;}}else if let Some(limit)=card_default_max_xray_sec(&profile,engine.max_xray_sec_explicit){
                // A new camera-card acquisition defaults to the unified 1000 s
                // continuous limit. An explicit value for this task is kept, and a
                // host-RAM task keeps the 600 s default.
                engine.max_xray_sec=limit;
                engine.log("INFO","system",&format!("Camera-card acquisition defaults to the unified {limit} s continuous X-ray limit; set an explicit limit before Preflight to change it"));
            }engine.fault_latched=false;engine.preflight=true;engine.homed=reusable;engine.reference_verified_this_process|=reusable;engine.phase=if reusable{Phase::Ready}else{Phase::ReadyForHome};engine.scan_references=scan::ReferenceFrames::default();engine.metadata_resume=restoring;engine.scan_prompt=if restoring||engine.scan_geometry.is_some(){None}else{Some("geometry")};engine.scan_stage=if engine.scan_prompt.is_some(){"geometry"}else{"idle"};engine.last_error=None;if unchanged&&reusable{engine.log("PASS","nano","Operator-confirmed unchanged turntable reference verified; no HOME or movement performed");}Ok(())}))
        })
    }
    fn enqueue_restore(&mut self)->Result<(),&'static str> {
        self.connection_control_allowed()?;
        if self.task_registry.check_retained_location(&self.parameters).map_err(|error|{self.last_error=Some(error);"SCAN_OUTPUT_UNVERIFIED"})?{return Err("SCAN_OUTPUT_RETAINED");}
        if self.busy(){return Err("SCAN_ACTIVE");}
        if !self.preflight{return Err("PREFLIGHT_REQUIRED");}if !self.homed{return Err("HOME_REQUIRED");}
        let params=self.parameters.clone();let kv=self.set_kv;let ua=self.set_ua;
        let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
        let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
        let preset=self.preset_tag.clone();
        self.enqueue_device("system","restore",move |cancel|{
            if !nano_reference_ready(&nano.status().map_err(|e|e.to_string())?){return Err("HOME_REQUIRED".into());}
            if camera.health().live_view.active{return Err("LIVE_VIEW_CAPTURE_FORBIDDEN".into());}
            camera.read_properties().map_err(|e|e.to_string())?;
            let actual=freeze_capture_profile(&camera,&nano,preset)?;
            let restored=scan::load_resume(&params,kv,ua,&actual)?;
            if cancel.load(Ordering::SeqCst){return Err("Restore cancelled".into());}
            Ok(Box::new(move |engine|{engine.max_xray_sec=restored.max_xray_sec;engine.capture_profile=Some(actual);engine.current=restored.frames.len()as u32;engine.real_frames=restored.frames.iter().map(|f|serde_json::to_value(f).unwrap_or(Value::Null)).collect();engine.scan_geometry=Some(restored.geometry.clone());engine.scan_references=restored.references.clone();engine.restored_frames=restored.frames.clone();engine.restored_scan=Some(restored);engine.scan_prompt=None;engine.scan_stage="idle";engine.phase=Phase::Ready;Ok(())}))
        })
    }
    fn enqueue_reconstruction(&mut self,payload:Value)->Result<(),&'static str> {
        use crate::reconstruction::{self,ReconstructionMethod,ReconstructionHandle,ReconstructionRequest};
        if self.busy()||self.task_registry.locked(){return Err("RECONSTRUCTION_LOCKED");}
        let method=match payload.get("method").and_then(Value::as_str){Some("fdk")=>ReconstructionMethod::Fdk,Some("sirt")=>ReconstructionMethod::Sirt,Some("cgls")=>ReconstructionMethod::Cgls,_=>return Err("INVALID_RECONSTRUCTION_METHOD")};
        if !reconstruction::available_methods().iter().any(|entry|entry.method==method&&entry.enabled) || self.parameters.projection_count<3 || method==ReconstructionMethod::Sirt&&!reconstruction::sirt_feasible(self.parameters.projection_count){return Err("RECONSTRUCTION_METHOD_UNAVAILABLE");}
        let parameters=self.parameters.clone();
        let options=payload.get("options").map(|value|serde_json::from_value::<reconstruction::spectral::Options>(value.clone()))
            .transpose().map_err(|_|"INVALID_RECONSTRUCTION_OPTIONS")?;
        if let Some(options)=&options {
            options.validate().map_err(|_|"INVALID_RECONSTRUCTION_OPTIONS")?;
            if method!=ReconstructionMethod::Fdk {return Err("INVALID_RECONSTRUCTION_OPTIONS");}
        }
        self.enqueue_device("system","reconstruction_prepare",move |cancel|{
            if !scan::completed_manifest_exists(&parameters){return Err("RECONSTRUCTION_LOCKED".into());}
            let scan_dir=PathBuf::from(parameters.save_path.trim()).join(parameters.task_id.trim());
            let cached=match &options {Some(options)=>reconstruction::spectral::cached_result(&scan_dir,method,options)?,
                None=>reconstruction::cached_result(&scan_dir,method)?};
            if cancel.load(Ordering::SeqCst){return Err("Reconstruction preparation cancelled".into());}
            let handle=if cached.is_none(){Some(match options {Some(options)=>ReconstructionHandle::start_with_options(ReconstructionRequest{scan_dir,method},options)?,
                None=>ReconstructionHandle::start(ReconstructionRequest{scan_dir,method})?})}else{None};
            Ok(Box::new(move |engine|{engine.reconstruction_event_sequence=0;engine.reconstruction_method=Some(method);engine.log("INFO","reconstruction","Reconstruction requested from sealed acquisition");if let Some(result)=cached{engine.reconstruction_status="completed";engine.reconstruction_percent=100;engine.reconstruction_message="重构已缓存".into();engine.reconstruction_cache_path=Some(result.cache_path.display().to_string());engine.log("PASS","reconstruction","Verified reconstruction cache loaded");}else{engine.reconstruction=handle;engine.reconstruction_status="running";engine.reconstruction_percent=0;engine.reconstruction_message="准备重构".into();engine.reconstruction_cache_path=None;}Ok(())}))
        })
    }
    fn enqueue_start_scan(&mut self,payload:Value)->Result<(),&'static str> {
        if self.task_registry.check_retained_location(&self.parameters).map_err(|error|{self.last_error=Some(error);"SCAN_OUTPUT_UNVERIFIED"})?{return Err("SCAN_OUTPUT_RETAINED");}
        self.require_connected()?;
        if self.busy(){return Err("SCAN_ACTIVE");}
        if !self.preflight{return Err("PREFLIGHT_REQUIRED");}if !self.homed{return Err("HOME_REQUIRED");}
        if self.scan_prompt.is_some()||self.scan_geometry.is_none(){return Err("SCAN_GEOMETRY_REQUIRED");}
        let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
        let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
        let expected=self.capture_profile.clone().ok_or("CAPTURE_PROFILE_REQUIRED")?;
        let preset=self.preset_tag.clone();
        let parameters=self.parameters.clone();
        let operation_id=self.operations.next_id(&self.engine_session_id);
        self.enqueue_device("system","start_scan",move |cancel|{
            let status=nano.status().map_err(|e|e.to_string())?;
            if !nano_reference_ready(&status){return Err("HOME_REQUIRED".into());}
            if camera.health().live_view.active{return Err("LIVE_VIEW_CAPTURE_FORBIDDEN: explicitly stop Live View".into());}
            camera.read_properties().map_err(|e|e.to_string())?;
            let actual=freeze_capture_profile(&camera,&nano,preset)?;
            expected.matches(&actual)?;
            if cancel.load(Ordering::SeqCst){return Err("Scan start cancelled".into());}
            Ok(Box::new(move |engine|{
                // Known admission rejection must leave an unstarted new task
                // editable. Existing retained locks remain owned by registry.
                if engine.parameters!=parameters||!engine.preflight||!engine.homed{return Err("START_ADMISSION_INVALIDATED".into());}
                let plan=engine.task_registry.prepare_register(&parameters)?;
                engine.operations.continue_job(&operation_id,move |cancel|{
                    plan.execute(&cancel)?;
                    if cancel.load(Ordering::SeqCst){return Err("Scan start cancelled during persistence".into());}
                    Ok(Box::new(move |engine|{engine.task_registry.commit(plan)?;engine.capture_profile=Some(actual);engine.command_core("start_scan",payload).map_err(str::to_owned)}))
                }).map_err(str::to_owned)
            }))
        })
    }
    fn preset_command(&mut self,command:&str,payload:Value)->Result<(),&'static str> {
        if command=="preset_export" {self.preset_export_json=Some(self.preset_store.export_json().map_err(|_|"PRESET_EXPORT_FAILED")?);return Ok(());}
        if command=="preset_compare"{self.connection_control_allowed()?;}else{self.manual_control_allowed()?;}
        let id=payload.get("id").and_then(Value::as_str);
        match command {
            "preset_delete"=>{let plan=self.preset_store.prepare_delete(id.ok_or("INVALID_PARAMETERS")?).map_err(|_|"PRESET_DELETE_FAILED")?;self.enqueue_preset_write(plan)},
            "preset_load"=>{
                let preset=self.preset_store.get(id.ok_or("INVALID_PARAMETERS")?).ok_or("PRESET_NOT_FOUND")?;
                let camera=self.camera.as_ref().ok_or("CAMERA_NOT_CONNECTED")?;
                let health=camera.health();let model=health.identity.as_ref().map(|i|i.model.as_str()).ok_or("CAMERA_NOT_CONNECTED")?;
                preset.validate_model(model).map_err(|_|"PRESET_MODEL_MISMATCH")?;
                camera.validate_properties(&preset.parameters).map_err(|_|"UNSUPPORTED_CAMERA_PROPERTIES")?;
                self.camera_draft=preset.parameters;Ok(())
            }
            "preset_import"=>{
                let camera=self.camera.as_ref().ok_or("CAMERA_NOT_CONNECTED")?;
                let model=camera.health().identity.map(|i|i.model).ok_or("CAMERA_NOT_CONNECTED")?;
                let text=payload.get("jsonText").and_then(Value::as_str).ok_or("INVALID_PARAMETERS")?;
                let plan=self.preset_store.prepare_import(text,|preset|{preset.validate_model(&model)?;camera.validate_properties(&preset.parameters).map_err(|e|e.to_string())}).map_err(|_|"PRESET_IMPORT_INVALID")?;self.enqueue_preset_write(plan)
            }
            "preset_save"|"preset_compare"=>{
                let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
                let preset=if command=="preset_compare"{Some(self.preset_store.get(id.ok_or("INVALID_PARAMETERS")?).ok_or("PRESET_NOT_FOUND")?)}else{None};
                let name=payload.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
                let description=payload.get("description").and_then(Value::as_str).unwrap_or("").to_owned();
                let source=payload.get("source").and_then(Value::as_str).unwrap_or("operator-confirmed").to_owned();
                let seed=if command=="preset_save"{Some(self.preset_store.write_seed().map_err(|_|"PRESET_STORAGE_INVALID")?)}else{None};
                self.enqueue_device("camera",command,move |cancel|{
                    let props=camera.read_properties().map_err(|e|e.to_string())?;
                    let actual:CameraProperties=props.into_iter().filter(|(key,property)|property.confirmed&&crate::capture_profile::PRESET_ALLOWED_KEYS.contains(&key.as_str())).filter_map(|(key,property)|property.actual.map(|value|(key,value))).collect();
                    let model=camera.health().identity.ok_or("Camera identity missing")?.model;
                    let plan=if let Some(seed)=seed{let plan=seed.prepare_save(name,description,model.clone(),actual.clone(),source)?;plan.execute(&cancel)?;Some(plan)}else{None};
                    Ok(Box::new(move |engine|{if let Some(preset)=preset{preset.validate_model(&model)?;engine.preset_comparison=crate::presets::compare_parameters(&preset.parameters,&actual);}if let Some(plan)=plan{engine.preset_store.commit(plan)?;}Ok(())}))
                })
            }
            _=>Err("UNKNOWN_COMMAND"),
        }
    }
    fn enqueue_preset_write(&mut self,plan:crate::presets::PresetWritePlan)->Result<(),&'static str>{self.enqueue_device("preset_storage","persist",move|cancel|{plan.execute(&cancel)?;Ok(Box::new(move|engine|engine.preset_store.commit(plan)))})}
    fn enqueue_stop(&mut self,all:bool,disconnect:bool)->Result<(),&'static str> {
        self.invalidate_update_report_for_stop();
        if all {self.operations.cancel_for_stop();}else{self.operations.cancel_device("xray");}
        if all {self.scan_stop_requested=true;self.task_stop_cleanup_confirmed=false;}
        self.preflight=false;if all {self.homed=false;self.reference_verified_this_process=false;self.phase=Phase::Stopping;}
        // Admit all three safety actions before awaiting any cleanup.
        let xray_receive=self.xray.as_ref().map(|x|x.request_off());
        let nano_receive=if all {self.nano.as_ref().map(|nano|{let nano=nano.clone();let(tx,rx)=mpsc::channel();std::thread::spawn(move||{let result=nano.stop();let _=tx.send(result.map_err(|e|e.to_string()));});rx})}else{None};
        let warning_nano=self.nano.as_ref().cloned();
        let camera=if all {self.camera.as_ref().cloned()}else{None};
        // End Live View while the camera session is still valid. Hardware shows
        // the end-Live-View transaction never completes once the session has been
        // invalidated by `request_cancel()`, while the ordinary Live View stop —
        // issued before any cancellation — confirms OFF in ~0.6 s on the same
        // device. Cancelling first is exactly what made Stop during Live View end
        // in `cleanup_unconfirmed`. Bounded so a wedged worker cannot delay the
        // safety Stop; any failure still leaves the fail-closed cleanup verdict.
        if let Some(camera)=camera.as_ref(){
            let health=camera.health();
            // No `busy` gate: a Stop that arrives while the worker is finishing
            // another operation is exactly the case that must still end Live View
            // on a valid session — skipping it here would fall back to the order
            // hardware proved fails (cancel first => end-LV never completes). The
            // call is bounded, so a busy worker can delay the safety Stop by at
            // most the budget below and the fail-closed cleanup path is unchanged.
            if health.connected && health.live_view.active {
                let _=camera.stop_live_view_with_timeout(Duration::from_secs(5));
            }
        }
        if let Some(camera)=camera.as_ref(){camera.request_cancel();}
        if let Some(scan)=self.real_scan.as_ref(){scan.cancel_only();}
        let mut xray=self.xray.as_ref().cloned();
        self.enqueue_device("safety","stop",move |_|{
            let deadline=Instant::now()+Duration::from_secs(6);
            let mut errors=vec![];
            let off_confirmed=if let Some(rx)=xray_receive {match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())){Ok(Ok(health))=>health.beam_off_confirmed,Ok(Err(error))=>{errors.push(format!("OFF unconfirmed: {error}"));false},Err(_)=>{errors.push("OFF unconfirmed: worker timeout".into());false}}}else{false};
            if let Some(rx)=nano_receive{match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())){Ok(Ok(()))=>{},Ok(Err(error))=>errors.push(error),Err(_)=>errors.push("Nano cleanup unconfirmed".into())}}
            // OFF and STOP were already admitted independently. The warning
            // command must follow STOP readback, including warning ON for UNKNOWN.
            if let Some(nano)=warning_nano{if let Err(error)=nano.set_xray_warning(!off_confirmed){errors.push(format!("Nano warning state unconfirmed: {error}"));}}
            if let Some(camera)=camera.as_ref(){while !camera.health().cleanup_confirmed && Instant::now()<deadline{std::thread::sleep(Duration::from_millis(20));}if !camera.health().cleanup_confirmed{errors.push("Camera cleanup unconfirmed; session isolated".into());}}
            if disconnect && errors.is_empty(){if let Some(xray)=xray.as_mut(){xray.disconnect().map_err(|e|format!("OFF unconfirmed: {e}"))?;}}
            // Failure evidence travels in the result envelope so cancellation
            // can preserve it without executing an old Engine mutation.
            if !errors.is_empty(){return Err(errors.join("; "));}
            Ok(Box::new(move |engine|{
                if all {engine.cached_camera_health=engine.camera.as_ref().map(|camera|camera.health());}
                engine.cached_xray_health=engine.xray.as_ref().map(|xray|xray.health());
                if disconnect {engine.nano=None;engine.camera=None;engine.xray=None;engine.connected=false;engine.set_kv_confirmed=false;engine.set_ua_confirmed=false;engine.usb_auto_shut_down_confirmed=false;}
                engine.beam_on=false;
                if all {engine.task_stop_cleanup_confirmed=true;}
                if all && engine.real_scan.is_none(){engine.phase=if engine.fault_latched{Phase::Fault}else if engine.task_exit_pending{Phase::Stopping}else{Phase::Stopped};}
                if all && !engine.task_exit_pending && !engine.maintenance_lock {engine.manual_lock=None;}
                engine.last_error=None;Ok(())
            }))
        })
    }

    /// Join the two independently completing paths: device Stop and scan-worker
    /// return. Keep the task lock until both and the durable terminal write finish.
    pub(crate) fn sync_task_exit(&mut self) {
        if !self.task_exit_pending || !self.task_stop_cleanup_confirmed
            || self.real_scan.is_some() || self.operations.blocks_controls() {return;}
        let health=self.xray.as_ref().map(|xray|xray.health()).or_else(||self.cached_xray_health.clone());
        let xray_unknown=(self.xray.is_some() || self.ever_xray || health.as_ref().is_some_and(|h|h.port.is_some()))
            && !health.as_ref().is_some_and(|h|h.beam_off_confirmed&&!h.beam_on);
        let camera_unknown=self.camera.as_ref().map(|camera|camera.health()).or_else(||self.cached_camera_health.clone())
            .is_some_and(|h|!h.cleanup_confirmed);
        let result=if xray_unknown {Err("OFF unconfirmed; task remains retained".into())}
            else if camera_unknown {Err("Camera cleanup unconfirmed; task remains retained".into())}
            else if !self.task_registry.locked() {self.finish_task_exit();Ok(())}
            else {self.task_registry.prepare_abandon().and_then(|plan|{
                self.enqueue_device("task_storage","abandon",move|cancel|{
                    plan.execute(&cancel)?;
                    Ok(Box::new(move|engine|{engine.task_registry.commit(plan)?;Ok(())}))
                }).map_err(str::to_owned)
            })};
        if let Err(error)=result {
            self.task_stop_cleanup_confirmed=false;
            self.fault_latched=true;self.phase=Phase::Fault;self.last_error=Some(error.clone());
            self.log("ERR","system",&format!("Task closure failed: {error}"));
        }
    }
    pub(crate) fn finish_task_exit(&mut self) {
        self.task_exit_pending=false;
        self.task_stop_cleanup_confirmed=false;
        self.manual_lock=None;
        self.invalidate(if self.fault_latched{Phase::Fault}else{Phase::Stopped});
        self.clear_task_selection();
        self.log("PASS","system","Scan ended and task closed; all acquired files retained. Prepare a new task and repeat Preflight and HOME.");
    }
    fn clear_task_selection(&mut self) {
        self.parameters.task_id.clear();self.current=0;self.real_frames.clear();
        self.scan_references=scan::ReferenceFrames::default();self.scan_geometry=None;self.capture_profile=None;
        self.scan_stage="stopped";self.metadata_generation+=1;
        self.metadata_manifest=false;self.metadata_resume=false;self.metadata_completed=false;
    }
    fn may_reconcile_deleted_tasks(&self)->bool {
        !self.preview&&!self.shutting_down&&!self.maintenance_lock&&self.real_scan.is_none()
            &&self.reconstruction.is_none()&&!self.operations.blocks_controls()
            &&!matches!(self.phase,Phase::Running|Phase::Paused|Phase::Finishing|Phase::Stopping)
            &&self.camera.as_ref().map(|camera|camera.health()).or_else(||self.cached_camera_health.clone()).is_none_or(|h|!h.busy&&h.cleanup_confirmed&&!h.live_view.active)
            &&((!self.ever_xray&&self.xray.is_none())||self.xray.as_ref().map(|x|x.health()).or_else(||self.cached_xray_health.clone()).is_some_and(|h|h.beam_off_confirmed&&!h.beam_on))
            &&self.nano.as_ref().is_none_or(|nano|nano.health().status.is_none_or(|status|!matches!(status.state.as_str(),"MOVING"|"HOMING"|"CAPTURING")))
    }
    /// Filesystem probing stays off the engine loop. Only confirmed deletion
    /// produces a receipt/write; ordinary probes neither occupy controls nor
    /// replace the latest user operation. Never revoke an active worker's lock.
    pub(crate) fn sync_deleted_task_directories(&mut self) {
        if self.shutting_down {self.task_reconcile_probe=None;return;}
        if let Some(receiver)=self.task_reconcile_probe.as_ref(){
            match receiver.try_recv(){
                Ok(plan)=>{
                    self.task_reconcile_probe=None;
                    if self.may_reconcile_deleted_tasks(){if let Some(plan)=plan{
                        let deleted=plan.deleted_count();let active_deleted=plan.active_deleted();
                        let _=self.enqueue_device("task_storage","reconcile_deleted",move|cancel|{
                            plan.execute(&cancel)?;
                            Ok(Box::new(move|engine|{
                                engine.task_registry.commit(plan)?;
                                if active_deleted {
                                    engine.clear_task_selection();
                                    engine.invalidate(if engine.fault_latched{Phase::Fault}else if engine.task_exit_pending{Phase::Stopping}else{Phase::Idle});
                                }
                                engine.log("INFO","system",&format!("Released {deleted} task location(s): acquisition directories were deleted; existing files and device safety state were not changed."));
                                Ok(())
                            }))
                        });
                    }}
                }
                Err(mpsc::TryRecvError::Disconnected)=>self.task_reconcile_probe=None,
                Err(mpsc::TryRecvError::Empty)=>return,
            }
        }
        if self.task_reconcile_probe.is_none()&&Instant::now()>=self.task_reconcile_due
            &&self.may_reconcile_deleted_tasks()&&self.task_registry.needs_directory_reconciliation(){
            let registry=self.task_registry.clone();let(sender,receiver)=mpsc::channel();
            self.task_reconcile_probe=Some(receiver);self.task_reconcile_due=Instant::now()+Duration::from_secs(1);
            std::thread::spawn(move||{let _=sender.send(registry.prepare_deleted_directories());});
        }
    }
}

fn cleanup_unconfirmed_error(error:&str)->bool {error.contains("cleanup unconfirmed")||error.contains("OFF unconfirmed")}

/// A device that refused requested *values* while the session stayed healthy. The
/// two product prefixes come from the camera job's partial-apply reports, and the
/// only value-rejection code with hardware evidence is `PTP 0x201c`
/// (InvalidDevicePropValue, observed for `cameraZoom`); every other PTP or
/// transport outcome stays a device failure so the change cannot silently turn a
/// real fault into a "correctable" rejection.
fn device_value_rejection_error(error:&str)->bool {
    let partial=error.starts_with("Live View control application incomplete: ")
        ||error.starts_with("Camera parameter application partial: ");
    partial
        &&error.contains("PTP response 0x201c")
        &&!["camera unavailable:","camera I/O failed:","camera timeout:","camera session isolated:","camera operation cancelled","camera busy:","camera capability unavailable:","camera safety check failed:"]
            .iter()
            .any(|marker|error.contains(marker))
}

/// Freezes the current camera and controller evidence as the profile of the
/// destination the camera itself confirmed. An unknown destination is refused
/// instead of being defaulted to either side, and a card profile additionally
/// requires the pinned card-capture settle firmware identity of the controller.
fn freeze_capture_profile(
    camera:&CameraAdapter,
    nano:&NanoAdapter,
    preset:Option<crate::capture_profile::PresetTag>,
)->Result<crate::capture_profile::CaptureProfile,String>{
    let health=camera.health();
    let destination=health.properties.get("recordingMedia").and_then(|property|property.actual.as_ref()).and_then(crate::devices::camera::CameraValue::integer);
    match destination {
        Some(0) if health.capabilities.card_capture => {
            // The card readback is the card counterpart of `profile_readback`; the
            // RAM readback requires the SDRAM destination and would refuse here.
            let readback=camera.card_profile_readback().map_err(|error|error.to_string())?;
            let identity=nano.health().identity.ok_or("Turntable identity is unknown; the card-capture settle firmware cannot be frozen")?;
            let policy=crate::scan::card_capture_policy(&identity,health.capabilities.card_overflow)?;
            crate::capture_profile::CaptureProfile::from_card_readback(&health,readback,preset,policy)
        }
        Some(0)=>Err("CAMERA_CARD_CAPTURE_UNSUPPORTED: the selected camera body does not advertise the standard card-capture operations".into()),
        Some(1)=>{
            let readback=camera.profile_readback().map_err(|error|error.to_string())?;
            crate::capture_profile::CaptureProfile::from_camera_readback(&health,readback,preset)
        }
        _=>Err("CAMERA_DESTINATION_UNKNOWN: explicitly Apply Host RAM or camera-card saving, verify the readback, then repeat Preflight".into()),
    }
}

/// The continuous X-ray limit a newly frozen acquisition defaults to.
///
/// A camera-card acquisition defaults to the unified 1000 s limit because its
/// per-projection overhead is lower; a host-RAM task keeps the historical 600 s
/// default. An operator value for this task is never replaced, and a recovered or
/// restored task keeps its own frozen value because this is only consulted when a
/// new profile is frozen.
fn card_default_max_xray_sec(
    profile:&crate::capture_profile::CaptureProfile,
    explicit:bool,
)->Option<u32>{
    (profile.is_card_destination()&&!explicit).then_some(crate::scan::SCAN_MAX_XRAY_SEC)
}

fn read_frozen_recovery(parameters:&crate::Parameters)->Result<(f64,f64,u32,crate::capture_profile::CaptureProfile),String> {
    let root=PathBuf::from(parameters.save_path.trim()).join(parameters.task_id.trim());
    let path=if root.join("manifest.json").is_file(){root.join("manifest.json")}else{root.join("preparation-checkpoint.json")};
    if std::fs::metadata(&path).map_err(|e|e.to_string())?.len()>8*1024*1024{return Err("Retained acquisition metadata exceeds limit".into());}
    let data:Value=serde_json::from_reader(std::fs::File::open(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if data["schemaVersion"]!=3 || data["scanId"].as_str()!=Some(parameters.task_id.as_str()) || data["projectionCount"].as_u64()!=Some(u64::from(parameters.projection_count)) || data["exposureMs"].as_f64()!=Some(parameters.exposure_ms){return Err("Retained task lacks matching schema 3 configuration; legacy tasks cannot resume with this backend".into());}
    let profile:crate::capture_profile::CaptureProfile=serde_json::from_value(data["captureProfile"].clone()).map_err(|e|e.to_string())?;profile.require_current_policy()?;
    let kv=data["voltageKv"].as_f64().ok_or("Frozen voltage missing")?;let ua=data["currentUa"].as_f64().ok_or("Frozen current missing")?;
    let max=u32::try_from(data["maxXraySec"].as_u64().ok_or("Frozen continuous X-ray limit missing")?).map_err(|_|"Invalid frozen continuous X-ray limit")?;
    if !crate::scan::scan_xray_seconds_valid(max){return Err("Frozen continuous X-ray limit outside bounds".into());}
    Ok((kv,ua,max,profile))
}
fn verified_candidate_path(directory:&std::path::Path)->Result<PathBuf,String>{
    let mut ancestor=directory.to_path_buf();let mut suffix=Vec::new();
    while !ancestor.exists(){suffix.push(ancestor.file_name().ok_or("Path ancestor is unverifiable")?.to_os_string());if !ancestor.pop(){return Err("Path ancestor missing".into());}}
    let mut actual=std::fs::canonicalize(ancestor).map_err(|e|e.to_string())?;for component in suffix.iter().rev(){actual.push(component);}Ok(actual)
}

fn reject_capture_metadata_ancestors(directory:&std::path::Path)->Result<(),String>{
    let mut ancestor=directory.to_path_buf();while !ancestor.exists(){if !ancestor.pop(){return Err("Capture directory has no verifiable ancestor".into());}}
    let mut ancestor=std::fs::canonicalize(ancestor).map_err(|error|error.to_string())?;
    loop{if ancestor.join("manifest.json").exists()||ancestor.join("preparation-checkpoint.json").exists(){return Err("Independent capture directory lies below retained scan metadata".into());}if !ancestor.pop(){break;}}
    Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::devices::camera::{scripted_camera,ScriptedCameraConfig};
    // The existing domain STOP cleanup budget is six seconds; allow its
    // terminal evidence under parallel test load without changing that budget.
    fn pump(engine:&mut Engine){let deadline=Instant::now()+Duration::from_secs(8);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}assert!(!engine.operations.is_active());}
    fn geometry()->Value{json!({"sodMm":100.0,"objectToDetectorMm":100.0,"detectorWidthMm":40.0,"centerOffsetXMm":1.0,"centerOffsetYMm":-1.0,"rotationDirection":"counterclockwise","mirrorX":true,"measurement":"measured"})}
    fn registered_stop_fixture(topic:&str)->(Engine,PathBuf) {
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine")
            .join(format!("{}-stop-{topic}",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        engine.parameters=crate::Parameters{task_id:"old-task".into(),save_path:root.to_string_lossy().into(),projection_count:3,angle_step_deg:120.0,exposure_ms:200.0};
        engine.task_registry.register(&engine.parameters).unwrap();
        std::fs::create_dir_all(root.join("old-task")).unwrap();
        std::fs::write(root.join("old-task/original.nef"),b"SCRIPTED TEST ORIGINAL").unwrap();
        (engine,root)
    }
    #[test]
    fn deleted_directory_releases_idle_ownership_but_not_an_active_phase(){
        let(mut engine,root)=registered_stop_fixture("deleted-idle");
        let target=root.join("old-task");assert!(target.canonicalize().unwrap().starts_with(root.canonicalize().unwrap()));std::fs::remove_dir_all(target).unwrap();
        engine.phase=Phase::Paused;engine.sync_deleted_task_directories();assert!(engine.task_reconcile_probe.is_none());assert!(engine.task_registry.locked());
        engine.phase=Phase::Stopped;let deadline=Instant::now()+Duration::from_secs(3);
        while engine.task_registry.locked()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}
        assert!(!engine.task_registry.locked());assert!(engine.parameters.task_id.is_empty());assert!(!engine.preflight&&!engine.homed);assert!(!engine.fault_latched);
        assert!(!crate::task_registry::TaskRegistry::load(root.join("engine-data/active-task.json")).locked());
    }
    #[test]
    fn file_conflict_after_reservation_is_rejected_without_a_phantom_lock(){
        let(mut engine,root)=registered_stop_fixture("start-file-race");engine.task_registry.abandon().unwrap();let old_registry=std::fs::read(root.join("engine-data/active-task.json")).unwrap();
        engine.parameters.task_id="new-start".into();let plan=engine.task_registry.prepare_register(&engine.parameters).unwrap();std::fs::create_dir(root.join("new-start")).unwrap();std::fs::write(root.join("new-start/original.nef"),b"external source").unwrap();
        engine.enqueue_device("system","start_scan",move|cancel|{plan.execute(&cancel)?;unreachable!()}).unwrap();let id=engine.operations.last_id.clone().unwrap();pump(&mut engine);
        assert_eq!(engine.operations.history.iter().find(|op|op.operation_id==id).unwrap().status,"rejected");assert!(!engine.task_registry.locked());assert!(engine.real_scan.is_none());assert!(!engine.fault_latched);assert_eq!(std::fs::read(root.join("engine-data/active-task.json")).unwrap(),old_registry);
        engine.shutdown_with_timeout(Duration::from_secs(3)).unwrap();
    }
    #[test]
    fn actual_start_can_reuse_a_deleted_retained_directory_without_waiting_for_poll(){
        let(mut engine,root)=registered_stop_fixture("reused-start");engine.task_registry.abandon().unwrap();let target=root.join("old-task");assert!(target.canonicalize().unwrap().starts_with(root.canonicalize().unwrap()));std::fs::remove_dir_all(target).unwrap();
        let(camera,_)=crate::devices::camera::scripted_camera_with_probe(Default::default());engine.capture_profile=Some(crate::capture_profile::CaptureProfile::from_camera_readback(&camera.health(),camera.profile_readback().unwrap(),None).unwrap());engine.camera=Some(camera);engine.nano=Some(crate::devices::turntable::test_homed_nano());let(mut xray,_)=crate::devices::xray::test_worker(20.0,100.0);xray.set_usb_auto_shutdown(false).unwrap();engine.xray=Some(xray);engine.set_kv=20.0;engine.set_ua=100.0;engine.usb_auto_shut_down=false;engine.usb_auto_shut_down_confirmed=true;
        engine.connected=true;engine.preflight=true;engine.homed=true;engine.scan_geometry=Some(serde_json::from_value(geometry()).unwrap());
        engine.device_command("start_scan",json!({})).unwrap().unwrap();pump(&mut engine);assert!(engine.real_scan.is_some());assert!(engine.task_registry.locked());assert_eq!(engine.task_registry.snapshot()["retained"],json!([]));
        engine.device_command("stop",json!({})).unwrap().unwrap();engine.shutdown_with_timeout(Duration::from_secs(5)).unwrap();
    }
    #[test]
    fn explicit_stop_closes_retained_task_preserves_files_and_admits_new_preparation() {
        let(mut engine,root)=registered_stop_fixture("new-preparation");
        let old=engine.parameters.clone();
        let camera=scripted_camera(ScriptedCameraConfig::default());engine.camera=Some(camera);
        assert_eq!(engine.device_command("stop",json!({})).unwrap(),Ok(()));
        assert!(engine.task_registry.locked());assert_eq!(engine.phase,Phase::Stopping);
        assert_eq!(engine.prepare_scan(&json!({"taskId":"new-task","projectionCount":3,"maxXraySec":120,"geometry":geometry()})),Err("DEVICE_CONTROL_LOCKED"));
        pump(&mut engine);engine.tick();
        assert!(!engine.task_registry.locked());assert!(!engine.task_exit_pending);assert_eq!(engine.phase,Phase::Stopped);
        assert_eq!(std::fs::read(root.join("old-task/original.nef")).unwrap(),b"SCRIPTED TEST ORIGINAL");
        let registry=engine.task_registry.snapshot();assert_eq!(registry["retained"][0]["status"],"abandoned");
        assert_eq!(registry["retained"][0]["parameters"]["taskId"],old.task_id);
        assert!(engine.parameters.task_id.is_empty());assert!(!engine.preflight&&!engine.homed);
        assert!(!crate::task_registry::TaskRegistry::load(root.join("engine-data/active-task.json")).locked());
        engine.camera=Some(scripted_camera(ScriptedCameraConfig::default()));
        assert_eq!(engine.prepare_scan(&json!({"taskId":"new-task","projectionCount":3,"maxXraySec":120,"geometry":geometry()})),Ok(()));
        assert_eq!(engine.parameters.task_id,"new-task");assert!(!engine.task_registry.locked());
        engine.parameters=old;
        assert_eq!(engine.device_command("restore_previous",json!({})).unwrap(),Err("SCAN_OUTPUT_RETAINED"));
        assert_eq!(engine.device_command("start_scan",json!({})).unwrap(),Err("SCAN_OUTPUT_RETAINED"));
    }
    #[test]
    fn stop_waits_for_terminal_write_and_repeat_stop_cannot_orphan_its_commit() {
        let(mut engine,root)=registered_stop_fixture("terminal-race");
        let plan=engine.task_registry.prepare_abandon().unwrap();
        engine.task_exit_pending=true;engine.task_stop_cleanup_confirmed=true;
        engine.phase=Phase::Stopping;engine.manual_lock=Some("Abandon waiting for confirmed cleanup".into());
        let(entered,ready)=mpsc::channel();let(release,wait)=mpsc::channel();
        engine.enqueue_device("task_storage","abandon",move|cancel|{
            entered.send(()).unwrap();wait.recv().unwrap();plan.execute(&cancel)?;
            Ok(Box::new(move|engine|engine.task_registry.commit(plan)))
        }).unwrap();
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let terminal_id=engine.operations.last_id.clone().unwrap();
        engine.device_command("stop",json!({})).unwrap().unwrap();pump_stop_only(&mut engine);
        assert!(engine.task_registry.locked());
        assert!(!engine.operations.history.iter().find(|op|op.operation_id==terminal_id).unwrap().cancel_requested);
        release.send(()).unwrap();pump(&mut engine);engine.tick();
        assert!(!engine.task_registry.locked());assert!(!engine.task_exit_pending);
        assert_eq!(engine.operations.history.iter().find(|op|op.operation_id==terminal_id).unwrap().status,"completed");
        assert_eq!(std::fs::read(root.join("old-task/original.nef")).unwrap(),b"SCRIPTED TEST ORIGINAL");
    }
    fn pump_stop_only(engine:&mut Engine) {
        let deadline=Instant::now()+Duration::from_secs(8);
        while engine.operations.device_active("safety")&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}
        assert!(!engine.operations.device_active("safety"));
    }
    #[test]
    fn stop_can_close_a_reserved_registration_before_its_write_without_losing_history() {
        let(mut engine,root)=registered_stop_fixture("reserved-start");
        let plan=engine.task_registry.prepare_abandon().unwrap();plan.execute(&AtomicBool::new(false)).unwrap();engine.task_registry.commit(plan).unwrap();
        engine.parameters.task_id="reserved-task".into();
        let _not_written=engine.task_registry.prepare_register(&engine.parameters).unwrap();
        engine.device_command("stop",json!({})).unwrap().unwrap();pump(&mut engine);engine.tick();
        assert!(!engine.task_registry.locked(),"{}",engine.snapshot());
        let retained=engine.task_registry.snapshot()["retained"].as_array().unwrap().clone();
        assert_eq!(retained.len(),2);assert_eq!(retained[1]["status"],"abandoned");
        assert_eq!(std::fs::read(root.join("old-task/original.nef")).unwrap(),b"SCRIPTED TEST ORIGINAL");
    }
    #[test]
    fn failed_terminal_write_keeps_task_locked_and_reports_fault() {
        let(mut engine,root)=registered_stop_fixture("storage-failure");
        // An unrelated on-disk registration must never be overwritten by Stop.
        let file=root.join("engine-data/active-task.json");
        let mut data:Value=serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        data["active"]["parameters"]["taskId"]=json!("another-owner-task");
        std::fs::write(&file,serde_json::to_vec(&data).unwrap()).unwrap();
        engine.device_command("stop",json!({})).unwrap().unwrap();pump(&mut engine);engine.tick();
        assert!(engine.task_registry.locked());assert!(engine.task_exit_pending);assert_eq!(engine.phase,Phase::Fault);
        assert!(engine.last_error.as_deref().unwrap().contains("Stale task persistence plan"));
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&file).unwrap()).unwrap(),data);
        assert_eq!(std::fs::read(root.join("old-task/original.nef")).unwrap(),b"SCRIPTED TEST ORIGINAL");
    }
    #[test]
    fn production_setpoint_command_logs_request_and_confirmed_pair_before_beam_on() {
        let mut engine=Engine::new(false);
        let(xray,_)=crate::devices::xray::test_worker(60.0,100.0);engine.xray=Some(xray);
        engine.set_kv=60.0;engine.set_ua=100.0;
        for(command,payload,label)in[("send_voltage",json!({"kv":70.0}),"SEND V"),("send_current",json!({"ua":200.0}),"SEND I")] {
            engine.device_command(command,payload).unwrap().unwrap();pump(&mut engine);
            let snapshot=engine.snapshot();let logs=snapshot["workstation"]["consoleLogs"].as_array().unwrap();
            assert!(logs.iter().any(|entry|entry["source"]=="xray"&&entry["level"]=="ACTION"&&entry["message"].as_str().unwrap().starts_with(&format!("{label} requested"))));
            assert!(logs.iter().any(|entry|entry["source"]=="xray"&&entry["level"]=="PASS"&&entry["message"].as_str().unwrap().starts_with(&format!("{label} applied and verified OFF"))));
            assert!(!engine.beam_on);assert!(engine.xray.as_ref().unwrap().health().beam_off_confirmed);
            // These are quantized device readbacks, not the nominal operating
            // point used by the adapter's existing strict 12 W admission gate.
            if command=="send_current" {
                assert!((engine.set_kv-60.0).abs()<0.3);assert!((engine.set_ua-200.0).abs()<5.0);
                assert!(logs.iter().any(|entry|entry["source"]=="xray"&&entry["level"]=="WARN"&&entry["message"].as_str().unwrap().contains("12 W limit")));
            }
        }
    }
    #[test]
    fn recovery_device_entry_points_preserve_maintenance_and_cleanup_locks(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-recovery-admission",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        engine.parameters=crate::Parameters{task_id:"retained".into(),save_path:root.to_string_lossy().into(),projection_count:2,angle_step_deg:180.0,exposure_ms:200.0};
        engine.preflight=true;
        engine.task_registry.register(&engine.parameters).unwrap();
        for maintenance in [true,false]{
            engine.maintenance_lock=maintenance;
            engine.manual_lock=if maintenance{None}else{Some("Cleanup unconfirmed".into())};
            let expected=if maintenance{"SOFTWARE_UPDATE_MAINTENANCE_LOCKED"}else{"DEVICE_CLEANUP_UNCONFIRMED"};
            for command in ["preflight","home","restore_previous"]{
                assert_eq!(engine.command(command,json!({})),Err(expected),"public {command}");
                assert_eq!(engine.device_command(command,json!({})).unwrap(),Err(expected),"domain {command}");
            }
            assert!(engine.preflight,"rejected preparation must not invalidate existing evidence");
            assert!(engine.operations.history.is_empty());
            assert!(engine.task_registry.locked());
        }
        engine.maintenance_lock=false;engine.manual_lock=None;
        assert_eq!(engine.enqueue_home(true),Err("NANO_NOT_CONNECTED"),"same-task recovery must still pass the persistent task lock");
    }
    #[test]
    fn invalid_focus_step_is_rejected_before_operation_without_fault_or_cleanup(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-focus-admission",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        for payload in [json!({}),json!({"step":0}),json!({"step":501}),json!({"step":-501}),json!({"step":1.5}),json!({"step":"1"}),json!({"step":2147483648i64}),json!({"step":-2147483649i64})]{
            assert_eq!(engine.device_command("camera_focus",payload).unwrap(),Err("INVALID_CAMERA_FOCUS_STEP"));
            engine.tick();assert!(engine.operations.history.is_empty());assert!(!engine.operations.is_active());assert!(!engine.fault_latched);assert!(engine.manual_lock.is_none());
            assert!(camera.health().connected&&camera.health().cleanup_confirmed);assert_eq!(probe.write_count(),0);assert_eq!(probe.live_view_stop_count(),0);
        }
    }
    #[test]
    fn capture_readiness_requires_the_runtime_diagnostic_evidence(){
        let camera=crate::devices::camera::scripted_camera(ScriptedCameraConfig::default());
        camera.disconnect_confirmed().unwrap();
        camera.connect_selected("fake-camera-1").unwrap();
        let ready=camera.health();
        assert!(crate::devices::camera::capture_readiness(&ready).is_ok(),"scripted session must be capture ready: {:?}",crate::devices::camera::capture_readiness(&ready).err());
        let mut missing=ready.clone();
        missing.runtime_properties.remove("infoDisplayErrorStatus");
        assert!(crate::devices::camera::capture_readiness(&missing).is_err(),"a missing runtime diagnostic must refuse capture before enqueue");
        let mut value=serde_json::to_value(&ready).unwrap();
        value["runtimeProperties"]["infoDisplayErrorStatus"]["confirmed"]=serde_json::json!(false);
        let unconfirmed:crate::devices::camera::CameraHealth=serde_json::from_value(value).unwrap();
        assert!(crate::devices::camera::capture_readiness(&unconfirmed).is_err(),"an unconfirmed runtime diagnostic must refuse capture before enqueue");
    }
    #[test]
    fn camera_parameter_rejection_vocabulary_is_stable(){
        use crate::devices::camera::PropertyRejection as R;
        let cases=[
            (R::SessionUnavailable,"CAMERA_PARAMETER_SESSION_UNAVAILABLE"),
            (R::Count,"INVALID_CAMERA_PARAMETER_COUNT"),
            (R::LiveViewForbidden,"LIVE_VIEW_APPLY_FORBIDDEN"),
            (R::Undescribed{key:"iso".into()},"CAMERA_PROPERTY_UNDESCRIBED"),
            (R::BracketingDescription,"CAMERA_PROPERTY_UNDESCRIBED"),
            (R::FocusModeLocked,"CAMERA_FOCUS_MODE_LOCKED"),
            (R::WriteOperationUnsupported,"CAMERA_PROPERTY_WRITE_UNSUPPORTED"),
            (R::UnsupportedValue{key:"iso".into()},"CAMERA_PROPERTY_UNSUPPORTED_VALUE"),
            (R::Dependency{key:"iso".into(),reason:"parent first".into()},"CAMERA_PROPERTY_DEPENDENCY_UNMET"),
        ];
        for (rejection,code) in &cases{assert_eq!(rejection.code(),*code,"{:?}",rejection);}
        // A frontend maps these strings to operator text, so the vocabulary must
        // stay unique and must never re-absorb the legacy collapsed code.
        let mut codes:Vec<&str>=cases.iter().map(|(rejection,_)|rejection.code()).collect();
        codes.sort_unstable();codes.dedup();
        assert_eq!(codes.len(),8,"the classified vocabulary must be unique: {codes:?}");
        assert!(!codes.contains(&"UNSUPPORTED_CAMERA_PROPERTIES"),"the apply path must not reuse the legacy collapsed code");
        // The operator-facing wording must stay the previously validated text.
        assert_eq!(crate::devices::camera::PropertyRejection::LiveViewForbidden.message(),"Live View开启期间禁止普通摄影参数Apply；请先明确停止预览");
        assert_eq!(crate::devices::camera::PropertyRejection::Count.message(),"camera Apply requires 1..64 described properties");
    }
    #[test]
    fn unready_capture_is_rejected_before_operation_without_fault(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-capture-admission",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        camera.disconnect_confirmed().unwrap();
        engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        engine.parameters.save_path=root.join("captures").to_string_lossy().into_owned();
        engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1"})).unwrap().unwrap();pump(&mut engine);
        // Documented request-validation precedence survives the readiness gate.
        let save_path=engine.parameters.save_path.clone();
        engine.parameters.save_path=String::new();
        assert_eq!(engine.device_command("camera_capture",json!({})).unwrap(),Err("CAMERA_CAPTURE_ROOT_REQUIRED"));
        engine.parameters.save_path=save_path;
        // An unready session is now a clean pre-enqueue refusal instead of a
        // worker failure that faults and tears the session down.
        probe.set_actual("driveMode",crate::devices::camera::CameraValue::Integer(2));
        engine.device_command("camera_read_properties",json!({})).unwrap().unwrap();pump(&mut engine);
        assert!(crate::devices::camera::capture_readiness(&camera.health()).is_err(),"driveMode=2 must make the session capture-unready");
        let before=engine.operations.history.len();
        assert_eq!(engine.device_command("camera_capture",json!({})).unwrap(),Err("CAMERA_CAPTURE_UNQUALIFIED"));
        engine.tick();
        assert_eq!(engine.operations.history.len(),before,"a rejected capture must not create an operation");
        assert!(!engine.fault_latched);assert!(engine.manual_lock.is_none());
        assert!(camera.health().connected&&camera.health().cleanup_confirmed);
    }
    #[test]
    fn stop_during_live_view_ends_live_view_before_cancelling(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-stop-during-lv",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        camera.disconnect_confirmed().unwrap();
        engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1"})).unwrap().unwrap();pump(&mut engine);
        engine.device_command("camera_live_view_start",json!({})).unwrap().unwrap();pump(&mut engine);
        assert!(camera.health().live_view.active,"Live View must be running before the stop");
        probe.reset_live_view_stop_tracking();
        engine.device_command("stop",json!({})).unwrap().unwrap();
        // Discriminating assertion: the pre-stop runs synchronously inside the stop
        // command, so Live View is already ended *before* the queued safety job is
        // pumped. In the old order nothing stops Live View until that job runs, so
        // this would be 0 and the following assertion would fail.
        assert_eq!(probe.live_view_stop_count(),1,"the stop must end Live View synchronously, before the safety job runs");
        assert!(!camera.health().live_view.active,"Live View must already be ended by the stop command itself");
        pump(&mut engine);
        // Ending Live View on the still-valid session is what lets the stop
        // confirm cleanup; cancelling first made the end-LV transaction hang. The
        // discriminating assertion is the synchronous one below: in the old order
        // nothing stops Live View until the queued safety job runs.
        assert_eq!(probe.live_view_stop_count(),1,"the stop must issue exactly one end-Live-View");
        assert!(!camera.health().live_view.active,"the stop must end Live View");
        assert!(camera.health().cleanup_confirmed,"the stop must confirm camera cleanup");
        assert!(!engine.fault_latched,"a clean stop must not latch a fault");
        assert!(engine.manual_lock.is_none(),"a clean stop must not leave a cleanup lock");
        // No acknowledgement/recovery step is needed to use the camera again.
        assert!(engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1"})).unwrap().is_ok());
        pump(&mut engine);
        assert!(camera.health().connected);
    }
    #[test]
    fn repeated_camera_connect_is_rejected_without_faulting_or_dropping_the_session(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-repeated-connect",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        let(camera,_)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        camera.disconnect_confirmed().unwrap();
        engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1"})).unwrap().unwrap();pump(&mut engine);
        assert!(camera.health().connected);
        // A repeated explicit connect is an operator state error: it must be rejected
        // before any device work instead of faulting the engine, isolating the
        // session or releasing the camera (hardware showed all three).
        assert_eq!(engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1"})).unwrap(),Err("CAMERA_ALREADY_CONNECTED"));
        assert!(!engine.operations.is_active(),"a rejected connect must not enqueue a device operation");
        pump(&mut engine);
        assert!(camera.health().connected,"the rejected connect must not drop the session");
        assert!(!camera.health().isolated,"the rejected connect must not isolate the session");
        assert!(!engine.fault_latched,"an operator state error must not latch a device fault");
        assert!(engine.manual_lock.is_none());
        // A blank id is not a selection: it must behave exactly like an omitted
        // deviceId (discovery-only refresh), never as a connect to "".
        assert_eq!(engine.device_command("camera_connect",json!({"deviceId":"   "})).unwrap().unwrap(),());
        pump(&mut engine);
        assert!(!engine.fault_latched,"a blank device id must not fault");
        assert!(camera.health().connected);
        // A discovery-only refresh stays allowed while a session is connected.
        assert!(engine.device_command("camera_connect",json!({})).unwrap().is_ok());
        pump(&mut engine);
        assert!(camera.health().connected);
    }
    #[test]
    fn device_value_rejection_does_not_latch_a_fault_but_a_transport_failure_does(){
        // Classifier contract first: a device that refused requested *values* is
        // recoverable; a transport/session failure is not.
        assert!(device_value_rejection_error("Camera parameter application partial: {\"iso\": \"camera command failed: PTP response 0x201c\"}"));
        assert!(device_value_rejection_error("Live View control application incomplete: {\"cameraZoom\": \"camera command failed: PTP response 0x201c\"}"));
        // Only the evidenced value-rejection code counts; other PTP/transport
        // outcomes must keep faulting.
        assert!(!device_value_rejection_error("Camera parameter application partial: {\"iso\": \"camera command failed: PTP response 0x2019\"}"));
        assert!(!device_value_rejection_error("Camera parameter application partial: {\"iso\": \"camera busy: queue full\"}"));
        assert!(!device_value_rejection_error("Camera parameter application partial: {\"iso\": \"camera capability unavailable: not writable\"}"));
        assert!(!device_value_rejection_error("Camera parameter application partial: {\"iso\": \"camera unavailable: transport lost\"}"));
        assert!(!device_value_rejection_error("camera I/O failed: broken pipe"));
        assert!(!device_value_rejection_error("Camera cleanup unconfirmed; session isolated"));
        // Then the engine path: a camera operation that fails on refused values must
        // not latch the fault or run Stop, while a transport failure still does.
        for (error,faults) in [
            ("Camera parameter application partial: {\"whiteBalance\": \"camera command failed: PTP response 0x201c\"}",false),
            ("Live View control application incomplete: {\"cameraZoom\": \"camera command failed: PTP response 0x201c\"}",false),
            ("camera I/O failed: broken pipe",true),
        ] {
            let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-value-rejection",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
            let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
            let(camera,_)=crate::devices::camera::scripted_camera_with_probe(Default::default());
            camera.disconnect_confirmed().unwrap();
            engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
            engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1"})).unwrap().unwrap();pump(&mut engine);
            engine.preflight=true;
            engine.device_command("camera_read_properties",json!({})).unwrap().unwrap();
            let id=engine.operations.last_id.clone().expect("queued camera operation");
            engine.record_operation_error(&id,error.to_string());
            let operation=engine.operations.history.iter().find(|operation|operation.operation_id==id).expect("operation receipt");
            assert_eq!(engine.fault_latched,faults,"{error}");
            assert_eq!(engine.operations.history.iter().any(|operation|operation.kind=="stop"),faults,"{error}");
            assert!(!engine.preflight,"a failed camera operation must invalidate preflight: {error}");
            assert_eq!(operation.status,if faults {"failed"} else {"rejected"},"{error}");
        }
    }
    #[test]
    fn discovery_device_count_is_an_operator_prompt_not_a_global_fault(){
        for (count,label) in [(0usize,"zero devices"),(2usize,"two devices")]{
            let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-discover-{count}",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
            let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
            let camera=crate::devices::camera::scripted_camera(ScriptedCameraConfig{discovered:count,..Default::default()});
            // The scripted device starts connected; discovery must not be what
            // changes connectivity either way.
            camera.disconnect_confirmed().unwrap();
            engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
            engine.device_command("camera_connect",json!({})).unwrap().unwrap();pump(&mut engine);
            let discover=engine.operations.history.iter().find(|operation|operation.kind=="camera_discover")
                .unwrap_or_else(||panic!("{label}: discovery must produce a camera_discover receipt"));
            assert_eq!(discover.status,"completed","{label}: discovery with {count} devices must complete");
            assert_eq!(discover.error,None,"{label}: the device count is not an operation error");
            assert_eq!(camera.health().discovered.len(),count,"{label}: the discovered list must be published as read; got {:?}",camera.health().discovered);
            assert!(!camera.health().connected,"{label}: enumeration must not select a device implicitly");
            assert!(!engine.fault_latched,"{label}: a device count is an operator prompt, not a device fault");
            assert!(engine.manual_lock.is_none(),"{label}: discovery must not create a cleanup lock");
            assert!(engine.operations.history.iter().all(|operation|operation.kind!="stop"),"{label}: discovery must not trigger a global Stop");
        }
    }
    #[test]
    fn acknowledged_camera_cleanup_retirement_is_scoped_to_the_camera_lock(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-cleanup-ack",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        let(camera,_probe)=crate::devices::camera::scripted_camera_with_probe(ScriptedCameraConfig{close_failure:true,..Default::default()});
        engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        // A real stop whose cleanup can never be confirmed produces the same
        // isolated/unconfirmed state observed on hardware.
        engine.device_command("stop",json!({})).unwrap().unwrap();pump(&mut engine);
        assert!(engine.manual_lock.as_deref().is_some_and(|reason|reason.starts_with("Camera cleanup unconfirmed")),"stop must latch the camera cleanup lock: {:?}",engine.manual_lock);
        assert!(engine.fault_latched);
        // 1. The Abandon lock is not a camera-cleanup lock and must never be released.
        let camera_lock=engine.manual_lock.clone();
        engine.manual_lock=Some("Abandon waiting for confirmed cleanup".into());
        assert_eq!(engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1","acknowledgeUnconfirmedCleanup":true})).unwrap(),Err("DEVICE_CLEANUP_UNCONFIRMED"));
        assert!(engine.fault_latched);
        // 2. Acknowledging the camera cleanup retires the unusable isolated proxy
        //    (its transport is closed and can never confirm cleanup) so a newly
        //    selected session can be opened, while the shared fault latch survives.
        engine.manual_lock=camera_lock;
        engine.test_connections.camera=Some(Arc::new(||{let camera=crate::devices::camera::scripted_camera(ScriptedCameraConfig::default());camera.disconnect_confirmed().map_err(|error|error.to_string())?;Ok(camera)}));
        assert!(engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1","acknowledgeUnconfirmedCleanup":true})).unwrap().is_ok());
        assert!(engine.manual_lock.is_none());
        assert!(engine.fault_latched,"the device-shared fault latch must survive an acknowledgement");
        pump(&mut engine);
        let connect=engine.operations.history.iter().find(|operation|operation.kind=="connect");
        assert!(connect.is_some_and(|operation|operation.status=="completed"),"the fresh adapter must complete a connect after the acknowledgement: {:?}",engine.operations.history.iter().map(|operation|(operation.kind.clone(),operation.status.clone(),operation.error.clone())).collect::<Vec<_>>());
        // 3. Without an explicit acknowledgement the lock still blocks everything.
        engine.manual_lock=Some("Camera cleanup unconfirmed; session isolated".into());
        assert_eq!(engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1"})).unwrap(),Err("DEVICE_CLEANUP_UNCONFIRMED"));
    }
    #[test]
    fn discovery_only_camera_connect_reports_a_discovery_receipt(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-camera-discover-kind",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));let(camera,_probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        // Real discovery never opens a session: start from a disconnected body so
        // this receipt test models the hardware flow (scripted cameras start connected).
        camera.disconnect_confirmed().unwrap();
        engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        engine.device_command("camera_connect",json!({})).unwrap().unwrap();pump(&mut engine);
        assert_eq!(engine.operations.history.iter().filter(|operation|operation.kind=="camera_discover").count(),1,"discovery-only receipt must not claim a connection");
        assert_eq!(engine.operations.history.iter().filter(|operation|operation.kind=="connect").count(),0);
        engine.device_command("camera_connect",json!({"deviceId":"fake-camera-1"})).unwrap().unwrap();pump(&mut engine);
        assert_eq!(engine.operations.history.iter().filter(|operation|operation.kind=="connect").count(),1,"an explicit deviceId produces a connection receipt");
        assert_eq!(engine.operations.history.iter().filter(|operation|operation.kind=="camera_discover").count(),1);
    }
    #[test]
    fn unqualified_focus_and_autofocus_are_rejected_before_operation_without_fault(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-focus-qualification",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        // Published readiness is authoritative: a well formed request that the
        // snapshot already marks unqualified must not reach the camera worker,
        // because a worker level failure latches a fault and tears the session down.
        assert!(!camera.health().live_view.active,"scripted camera must start outside Live View");
        for payload in [json!({"step":10}),json!({"step":-10})]{
            assert_eq!(engine.device_command("camera_focus",payload).unwrap(),Err("CAMERA_FOCUS_UNQUALIFIED"));
            engine.tick();assert!(engine.operations.history.is_empty());assert!(!engine.operations.is_active());assert!(!engine.fault_latched);assert!(engine.manual_lock.is_none());
            assert!(camera.health().connected&&camera.health().cleanup_confirmed);assert_eq!(probe.write_count(),0);
        }
        if !camera.health().capabilities.autofocus {
            assert_eq!(engine.device_command("camera_autofocus",json!({})).unwrap(),Err("CAMERA_AUTOFOCUS_UNQUALIFIED"));
            engine.tick();assert!(engine.operations.history.is_empty());assert!(!engine.fault_latched);assert!(engine.manual_lock.is_none());
            assert!(camera.health().connected&&camera.health().cleanup_confirmed);
        }
    }
    #[cfg(windows)]
    #[test]
    fn ordinary_apply_during_live_view_rejects_noop_empty_and_changed_without_fault_or_stop(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera").join(format!("{}-live-apply-admission",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        let session=camera.start_live_view().unwrap().live_view.session_id.unwrap();engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        let actual=serde_json::to_value(camera.health().properties["iso"].actual.clone().unwrap()).unwrap();
        for properties in [json!({"iso":actual}),json!({}),json!({"iso":800})]{
            assert_eq!(engine.device_command("camera_apply_properties",json!({"properties":properties})).unwrap(),Err("LIVE_VIEW_APPLY_FORBIDDEN"));
            engine.tick();assert!(engine.operations.history.is_empty());assert!(!engine.operations.is_active());assert!(!engine.fault_latched);assert!(engine.manual_lock.is_none());
            assert!(camera.health().connected&&camera.health().live_view.active);assert_eq!(probe.live_view_stop_count(),0);assert_eq!(probe.write_count(),0);
        }
        engine.device_command("preset_list",json!({})).unwrap().unwrap();assert!(engine.operations.history.is_empty());
        let model=camera.health().identity.unwrap().model;let parameters=crate::devices::camera::CameraProperties::from([("iso".into(),camera.health().properties["iso"].actual.clone().unwrap())]);
        let preset=engine.preset_store.save("LV read-only comparison".into(),String::new(),model,parameters,"offline regression".into()).unwrap();
        engine.device_command("preset_compare",json!({"id":preset.id})).unwrap().unwrap();let compare=engine.operations.last_id.clone().unwrap();pump(&mut engine);
        assert_eq!(engine.operations.history.iter().find(|operation|operation.operation_id==compare).unwrap().status,"completed");assert_eq!(engine.preset_comparison[0]["matches"],true);assert!(camera.health().live_view.active);
        engine.device_command("camera_live_view_read",json!({"sessionId":session})).unwrap().unwrap();pump(&mut engine);assert!(camera.health().live_view.active);
        engine.device_command("camera_live_view_configure",json!({"sessionId":session,"controls":{"cameraZoom":1}})).unwrap().unwrap();let configure=engine.operations.last_id.clone().unwrap();pump(&mut engine);
        assert_eq!(engine.operations.history.iter().find(|operation|operation.operation_id==configure).unwrap().status,"completed");
        assert!(!engine.fault_latched);assert!(camera.health().connected&&camera.health().live_view.active);assert_eq!(probe.live_view_stop_count(),0);assert_eq!(probe.write_count(),1);
        camera.stop_live_view().unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn live_control_domain_rejects_stale_or_invalid_payload_atomically(){
        let mut engine=Engine::new(false);let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        let session=camera.start_live_view().unwrap().live_view.session_id.unwrap();engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        for payload in [json!({"controls":{"cameraZoom":1}}),json!({"sessionId":"bad","controls":{"cameraZoom":1}}),json!({"sessionId":session,"controls":{}}),json!({"sessionId":session,"controls":{"cameraZoom":1,"iso":800}}),json!({"sessionId":session,"controls":{"cameraZoom":256}}),json!({"sessionId":session,"controls":{"cameraZoom":1.5}})]{
            assert!(engine.device_command("camera_live_view_configure",payload).unwrap().is_err());
            assert!(!engine.operations.is_active());assert_eq!(probe.write_count(),0);
        }
        camera.stop_live_view().unwrap();camera.start_live_view().unwrap();
        for command in ["camera_live_view_read","camera_live_view_configure"]{
            assert_eq!(engine.device_command(command,json!({"sessionId":session,"controls":{"cameraZoom":1}})).unwrap(),Err("STALE_LIVE_VIEW_SESSION"));
        }
        assert_eq!(probe.write_count(),0);camera.stop_live_view().unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn live_control_domain_preserves_scan_cleanup_maintenance_and_task_occupancy(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-lv-domain-admission",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());
        let session=camera.start_live_view().unwrap().live_view.session_id.unwrap();engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        let payload=json!({"sessionId":session,"controls":{"cameraZoom":1}});
        for phase in [Phase::Running,Phase::Paused,Phase::Finishing,Phase::Stopping]{
            engine.phase=phase;for command in ["camera_live_view_read","camera_live_view_configure"]{assert_eq!(engine.device_command(command,payload.clone()).unwrap(),Err("DEVICE_CONTROL_LOCKED"));}
        }
        engine.phase=Phase::Idle;engine.manual_lock=Some("cleanup unconfirmed".into());
        assert_eq!(engine.device_command("camera_live_view_configure",payload.clone()).unwrap(),Err("DEVICE_CLEANUP_UNCONFIRMED"));engine.manual_lock=None;
        engine.maintenance_lock=true;assert_eq!(engine.device_command("camera_live_view_read",payload.clone()).unwrap(),Err("SOFTWARE_UPDATE_MAINTENANCE_LOCKED"));engine.maintenance_lock=false;
        let(release,receive)=mpsc::channel();engine.enqueue_device("camera","held_live_configure",move |_|{receive.recv_timeout(Duration::from_secs(2)).unwrap();Ok(Box::new(|_|Ok(())))}).unwrap();
        for command in ["camera_live_view_read","camera_live_view_configure"]{assert_eq!(engine.device_command(command,payload.clone()).unwrap(),Err("DEVICE_CONTROL_LOCKED"));}
        release.send(()).unwrap();pump(&mut engine);
        engine.parameters=crate::Parameters{task_id:"retained-lv".into(),save_path:root.to_string_lossy().into(),projection_count:2,angle_step_deg:180.0,exposure_ms:200.0};engine.task_registry.register(&engine.parameters).unwrap();
        assert_eq!(engine.device_command("camera_live_view_configure",payload.clone()).unwrap(),Err("UNFINISHED_TASK_LOCKED"));
        engine.device_command("camera_live_view_read",payload).unwrap().unwrap();let read=engine.operations.last_id.clone().unwrap();pump(&mut engine);
        assert_eq!(engine.operations.history.iter().find(|op|op.operation_id==read).unwrap().status,"completed");
        assert!(engine.task_registry.locked());assert_eq!(probe.write_count(),0);camera.stop_live_view().unwrap();
    }
    #[test]
    fn preflight_and_start_fresh_read_without_camera_writes_or_implicit_live_view_stop(){
        let mut engine=Engine::new(false);let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());engine.camera=Some(camera.clone());engine.cached_camera_health=Some(camera.health());engine.nano=Some(crate::devices::turntable::test_homed_nano());engine.connected=true;engine.reference_verified_this_process=true;
        let(mut xray,_)=crate::devices::xray::test_worker(20.0,100.0);xray.set_usb_auto_shutdown(false).unwrap();engine.xray=Some(xray);engine.set_kv=20.0;engine.set_ua=100.0;engine.set_kv_confirmed=true;engine.set_ua_confirmed=true;engine.usb_auto_shut_down=false;engine.usb_auto_shut_down_confirmed=true;
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-read-only-preflight",chrono::Utc::now().timestamp_nanos_opt().unwrap()));engine.parameters.save_path=root.to_string_lossy().into();engine.prepare_scan(&json!({"taskId":"fixed","projectionCount":3,"maxXraySec":120,"geometry":geometry()})).unwrap();
        assert_eq!(engine.command("update_scan_setup",json!({"setup":{"exposureMs":125.0}})),Err("EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY"));assert_eq!(probe.write_count(),0);
        camera.start_live_view().unwrap();let prior_stops=probe.live_view_stop_count();engine.device_command("preflight",json!({})).unwrap().unwrap();pump(&mut engine);assert!(!engine.preflight);assert!(camera.health().live_view.active);assert_eq!(probe.live_view_stop_count(),prior_stops);assert_eq!(probe.write_count(),0);
        camera.stop_live_view().unwrap();engine.device_command("preflight",json!({})).unwrap().unwrap();pump(&mut engine);assert!(engine.preflight,"{}",engine.snapshot());assert!(engine.scan_geometry.is_some());assert!(engine.scan_prompt.is_none());assert!(engine.homed);assert_eq!(probe.write_count(),0);
        engine.device_command("start_scan",json!({})).unwrap().unwrap();let start_id=engine.operations.last_id.clone().unwrap();pump(&mut engine);assert_eq!(engine.operations.history.iter().filter(|op|op.kind=="start_scan").count(),1);assert_eq!(engine.operations.history.iter().find(|op|op.operation_id==start_id).unwrap().status,"completed");assert!(engine.real_scan.is_some(),"Start receipt completes only after persistence and actual scan launch");let deadline=Instant::now()+Duration::from_secs(2);while engine.scan_prompt.is_none()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}assert_eq!(engine.scan_prompt,Some("preReferences"));assert_eq!(probe.write_count(),0);
        engine.device_command("stop",json!({})).unwrap().unwrap();let deadline=Instant::now()+Duration::from_secs(5);while (engine.real_scan.is_some()||engine.operations.is_active())&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}assert!(engine.real_scan.is_none());assert_eq!(probe.write_count(),0);
    }
    #[test]
    fn rejected_start_drift_or_live_view_does_not_register_new_task_and_apply_remains_available(){
        use crate::devices::camera::CameraValue;
        for condition in ["iso","whiteBalance","liveView"]{
            let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-start-admission-{condition}",chrono::Utc::now().timestamp_nanos_opt().unwrap()));let storage=root.join("engine-data");
            let mut engine=Engine::new_with_storage(false,&storage);let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());engine.camera=Some(camera.clone());engine.cached_camera_health=Some(camera.health());engine.nano=Some(crate::devices::turntable::test_homed_nano());engine.connected=true;engine.reference_verified_this_process=true;
            let(mut xray,_)=crate::devices::xray::test_worker(20.0,100.0);xray.set_usb_auto_shutdown(false).unwrap();engine.xray=Some(xray);engine.set_kv=20.0;engine.set_ua=100.0;engine.set_kv_confirmed=true;engine.set_ua_confirmed=true;engine.usb_auto_shut_down=false;engine.usb_auto_shut_down_confirmed=true;engine.parameters.save_path=root.to_string_lossy().into();
            engine.prepare_scan(&json!({"taskId":"not-started","projectionCount":3,"maxXraySec":120,"geometry":geometry()})).unwrap();engine.device_command("preflight",json!({})).unwrap().unwrap();pump(&mut engine);assert!(engine.preflight&&engine.homed);assert!(!engine.task_registry.locked());
            let original=camera.health().properties.get(condition).and_then(|p|p.actual.clone());
            if condition=="liveView"{camera.start_live_view().unwrap();}else{probe.set_actual(condition,CameraValue::Integer(if condition=="iso"{400}else{2}));}
            engine.device_command("start_scan",json!({})).unwrap().unwrap();let start=engine.operations.last_id.clone().unwrap();pump(&mut engine);
            let failed=engine.operations.history.iter().find(|op|op.operation_id==start).unwrap();assert_eq!(failed.status,"failed","{condition}: {}",engine.snapshot());assert!(failed.error.is_some());assert!(!engine.task_registry.locked(),"fresh rejection cannot create a phantom task: {condition}");assert!(engine.real_scan.is_none());assert!(!root.join("not-started").exists());assert!(!storage.join("active-task.json").exists());assert_eq!(engine.current,0);assert_eq!(probe.write_count(),0);
            if condition=="liveView"{engine.device_command("camera_live_view_stop",json!({})).unwrap().unwrap();pump(&mut engine);}
            let properties=if condition=="liveView"{json!({"iso":200})}else{json!({condition:original.unwrap()})};
            engine.device_command("camera_apply_properties",json!({"properties":properties})).unwrap().unwrap();let apply=engine.operations.last_id.clone().unwrap();pump(&mut engine);assert_eq!(engine.operations.history.iter().find(|op|op.operation_id==apply).unwrap().status,"completed","operator can Apply after rejected {condition}");assert!(!engine.task_registry.locked());
            engine.shutdown_with_timeout(Duration::from_secs(3)).unwrap();
        }
    }
    #[test]
    fn rejected_start_preserves_an_existing_retained_task_lock(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-retained-start-admission",chrono::Utc::now().timestamp_nanos_opt().unwrap()));let storage=root.join("engine-data");let mut engine=Engine::new_with_storage(false,&storage);engine.parameters=crate::Parameters{task_id:"retained".into(),save_path:root.to_string_lossy().into(),projection_count:3,angle_step_deg:120.0,exposure_ms:200.0};engine.task_registry.register(&engine.parameters).unwrap();
        let(camera,probe)=crate::devices::camera::scripted_camera_with_probe(Default::default());engine.capture_profile=Some(crate::capture_profile::CaptureProfile::from_camera_readback(&camera.health(),camera.profile_readback().unwrap(),None).unwrap());engine.camera=Some(camera);engine.nano=Some(crate::devices::turntable::test_homed_nano());engine.connected=true;engine.preflight=true;engine.homed=true;engine.scan_geometry=Some(serde_json::from_value(geometry()).unwrap());let before=std::fs::read(storage.join("active-task.json")).unwrap();probe.set_actual("iso",crate::devices::camera::CameraValue::Integer(400));
        engine.device_command("start_scan",json!({})).unwrap().unwrap();pump(&mut engine);assert!(engine.task_registry.locked());assert!(engine.task_registry.matches(&engine.parameters));assert_eq!(std::fs::read(storage.join("active-task.json")).unwrap(),before);assert_eq!(engine.manual_control_allowed(),Err("UNFINISHED_TASK_LOCKED"));
    }
    #[test]
    fn atomic_preparation_uses_confirmed_native_shutter_and_has_no_task_lock_or_disk_capture(){
        let mut engine=Engine::new(false);engine.camera=Some(scripted_camera(Default::default()));
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-atomic-preparation",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        engine.set_capture_root(&json!({"savePath":root.to_string_lossy()})).unwrap();
        let before=engine.parameters.clone();let old_max=engine.max_xray_sec;
        let mut draft=json!({"taskId":"scan-valid","projectionCount":3,"maxXraySec":120,"geometry":geometry()});draft["geometry"]["sodMm"]=json!(0.0);
        assert_eq!(engine.prepare_scan(&draft),Err("INVALID_SCAN_GEOMETRY"));assert_eq!(engine.parameters,before);assert_eq!(engine.max_xray_sec,old_max);assert!(!engine.task_registry.locked());assert!(!root.exists());
        draft["geometry"]=geometry();engine.prepare_scan(&draft).unwrap();
        assert_eq!(engine.parameters.exposure_ms,200.0);assert_eq!(engine.parameters.angle_step_deg,120.0);assert_eq!(engine.max_xray_sec,120);assert!(engine.scan_geometry.as_ref().unwrap().mirror_x);assert!(engine.scan_prompt.is_none());assert!(!engine.task_registry.locked());assert!(!root.exists());
        let accepted=engine.parameters.clone();for id in ["CameraCaptures","cameracaptures","CON","alias.","unsafe:ads"]{draft["taskId"]=json!(id);assert!(engine.prepare_scan(&draft).is_err());assert_eq!(engine.parameters,accepted);}
    }
    #[test]
    fn independent_capture_registers_only_complete_image_in_shared_root_without_scan_progress(){
        let mut engine=Engine::new(false);engine.camera=Some(scripted_camera(Default::default()));
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-capture-register",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        engine.set_capture_root(&json!({"savePath":root.to_string_lossy()})).unwrap();
        engine.device_command("camera_capture",json!({})).unwrap().unwrap();pump(&mut engine);
        let capture=engine.captures.back().expect("complete image must register");assert!(capture.path.starts_with(root.join("CameraCaptures")));assert_eq!(std::fs::metadata(&capture.path).unwrap().len(),capture.bytes);assert!(capture.exposure_ended);assert_eq!(capture.sha256.len(),64);assert_eq!(engine.current,0);assert!(engine.real_frames.is_empty());assert!(!engine.task_registry.locked());
        let id=capture.capture_id.clone();let path=capture.path.clone();assert!(engine.command("camera_capture_descriptor",json!({"captureId":id})).is_ok());assert_eq!(engine.command("camera_capture_descriptor",json!({"captureId":path.to_string_lossy()})),Err("CAPTURE_NOT_REGISTERED"));assert!(engine.snapshot()["deviceControls"]["camera"]["lastCapturedImage"]["bytes"].is_u64());
        assert_eq!(engine.device_command("camera_capture",json!({"directory":root.join("arbitrary").to_string_lossy()})).unwrap(),Err("CAMERA_CAPTURE_DIRECTORY_REQUIRES_ROOT"));
        let camera=engine.camera.as_ref().unwrap().clone();camera.start_live_view().unwrap();let count=engine.captures.len();assert_eq!(engine.device_command("camera_capture",json!({})).unwrap(),Err("LIVE_VIEW_CAPTURE_FORBIDDEN"));assert!(camera.health().live_view.active);assert_eq!(engine.captures.len(),count);camera.stop_live_view().unwrap();
    }
    #[test]
    fn cancelled_transfer_does_not_register_or_commit_an_independent_image(){
        let mut engine=Engine::new(false);let(tx,rx)=mpsc::channel();engine.camera=Some(scripted_camera(ScriptedCameraConfig{capture_started_signal:Some(tx),transfer_delay:Duration::from_millis(200),..Default::default()}));
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-capture-cancel-register",chrono::Utc::now().timestamp_nanos_opt().unwrap()));engine.parameters.save_path=root.to_string_lossy().into();
        engine.device_command("camera_capture",json!({})).unwrap().unwrap();rx.recv_timeout(Duration::from_secs(2)).unwrap();engine.device_command("stop",json!({})).unwrap().unwrap();pump(&mut engine);assert!(engine.captures.is_empty());assert_eq!(engine.current,0);assert!(!engine.task_registry.locked());assert!(!std::fs::read_dir(root.join("CameraCaptures")).unwrap().any(|file|file.unwrap().path().extension().is_some_and(|ext|ext=="nef")));
    }
    #[test]
    fn iso_only_apply_does_not_decode_or_replace_the_shared_shutter_target(){
        let mut engine=Engine::new(false);engine.camera=Some(scripted_camera(ScriptedCameraConfig::default()));engine.parameters.exposure_ms=123.0;
        engine.device_command("camera_apply_properties",json!({"properties":{"iso":800}})).unwrap().unwrap();
        let expected=engine.operations.last_id.clone().unwrap();let deadline=Instant::now()+Duration::from_secs(3);
        while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}
        assert_eq!(engine.operations.history.iter().find(|operation|operation.operation_id==expected).unwrap().status,"completed");
        assert_eq!(engine.parameters.exposure_ms,123.0);assert!(!engine.fault_latched);assert!(engine.camera.as_ref().unwrap().health().connected);
    }
    #[test]
    fn source_unknown_warning_on_follows_delayed_nano_stop_confirmation(){
        let(nano,events)=crate::devices::turntable::test_delayed_stop_nano();let mut engine=Engine::new(false);engine.nano=Some(nano);engine.connected=true;
        engine.device_command("stop",json!({})).unwrap().unwrap();let expected=engine.operations.last_id.clone().unwrap();
        let deadline=Instant::now()+Duration::from_secs(3);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}
        assert_eq!(engine.operations.history.iter().find(|operation|operation.operation_id==expected).unwrap().status,"completed");
        assert!(!engine.fault_latched);assert_eq!(engine.snapshot()["workstation"]["xray"]["beamState"],"unknown");
        let events=events.lock().unwrap();let stopped=events.iter().position(|line|line.starts_with("RX STOPPED ")).unwrap();
        let warning=events.iter().position(|line|line.starts_with("TX XRAY_WARNING ")&&line.ends_with(" ON")).unwrap();
        assert!(stopped<warning,"warning ON must not race an admitted STOP: {events:?}");
        assert!(!events.iter().any(|line|line.starts_with("TX HOME ")||line.starts_with("TX MOVE_")));
    }
    #[test]
    fn unopened_moxtek_discovery_failure_does_not_stop_or_alarm(){
        let(nano,events)=crate::devices::turntable::test_delayed_stop_nano();
        let mut engine=Engine::new(false);engine.nano=Some(nano);engine.connected=true;
        engine.xray=Some(MoxtekAdapter::new());
        let error="discovery error: expected exactly one connected Moxtek VID=277B PID=07D1 SN=183616; found 0";
        engine.enqueue_device("xray","connect",move |_|Err(error.into())).unwrap();
        let id=engine.operations.last_id.clone().unwrap();pump(&mut engine);
        let operation=engine.operations.history.iter().find(|op|op.operation_id==id).unwrap();
        assert_eq!(operation.status,"rejected");assert_eq!(operation.error.as_deref(),Some(error));
        assert!(!engine.fault_latched);assert!(engine.manual_lock.is_none());assert!(!engine.ever_xray);
        assert!(engine.xray.is_none()&&engine.cached_xray_health.is_none());
        assert!(engine.operations.history.iter().all(|op|op.kind!="stop"));
        assert_eq!(engine.workstation_view()["xray"]["beamState"],"unknown");
        assert!(!engine.workstation_view()["xray"]["manualControlsEnabled"].as_bool().unwrap());
        assert!(engine.manual_control_allowed().is_ok());
        for _ in 0..200{engine.tick();}
        let events=events.lock().unwrap();
        assert!(!events.iter().any(|line|line.starts_with("TX STOP ")||line.starts_with("TX XRAY_WARNING ")||line.starts_with("TX HOME ")||line.starts_with("TX MOVE_")),"{events:?}");
        assert!(engine.logs.iter().any(|log|log["message"].as_str().is_some_and(|text|text.contains("found 0"))));
    }
    #[test]
    fn discovery_failure_cannot_erase_a_previously_owned_xray_session(){
        for remembered in [false,true] {
        let(nano,events)=crate::devices::turntable::test_delayed_stop_nano();
        let mut engine=Engine::new(false);engine.nano=Some(nano);engine.connected=true;engine.ever_xray=remembered;
        engine.cached_xray_health=Some(crate::devices::xray::XrayHealth{port:Some("COM-XRAY".into()),last_error:Some("Previous OFF was not confirmed".into()),..Default::default()});
        engine.xray=Some(MoxtekAdapter::new());
        engine.enqueue_device("xray","connect",move |_|Err("discovery error: found 0".into())).unwrap();pump(&mut engine);
        assert!(engine.fault_latched);assert!(engine.manual_lock.is_some());assert!(engine.ever_xray);
        assert_eq!(engine.workstation_view()["xray"]["beamState"],"unknown");
        assert!(engine.operations.history.iter().any(|op|op.kind=="stop"&&op.status=="cleanup_unconfirmed"));
        assert!(events.lock().unwrap().iter().any(|line|line.starts_with("TX XRAY_WARNING ")&&line.ends_with(" ON")));
        }
    }
    #[test]
    fn status_after_stop_reads_locked_reference_from_the_real_nano_worker(){
        let(nano,events)=crate::devices::turntable::test_delayed_stop_nano();let mut engine=Engine::new(false);engine.nano=Some(nano);engine.connected=true;
        engine.device_command("stop",json!({})).unwrap().unwrap();let deadline=Instant::now()+Duration::from_secs(3);
        while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}
        assert!(engine.nano.as_ref().unwrap().health().status.is_none());
        let before=events.lock().unwrap().iter().filter(|line|line.starts_with("TX STATUS ")).count();
        engine.device_command("turntable_status",json!({})).unwrap().unwrap();let expected=engine.operations.last_id.clone().unwrap();
        let deadline=Instant::now()+Duration::from_secs(3);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}
        assert_eq!(engine.operations.history.iter().find(|operation|operation.operation_id==expected).unwrap().status,"completed");
        let status=engine.nano.as_ref().unwrap().health().status.unwrap();assert_eq!(status.state,"LOCKED");assert!(!status.homed);assert!(!status.reference_valid);assert!(!status.rearmed);
        assert_eq!(events.lock().unwrap().iter().filter(|line|line.starts_with("TX STATUS ")).count(),before+1);
        assert_eq!(engine.snapshot()["deviceControls"]["turntable"]["status"]["state"],"LOCKED");
    }
    #[test]
    fn pending_unconfirmed_disconnect_keeps_its_cleanup_result_instead_of_cancelling_itself(){
        let mut engine=Engine::new(false);let camera=scripted_camera(ScriptedCameraConfig{close_failure:true,..Default::default()});
        engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        let(ready,receive)=mpsc::channel();let(release,resume)=mpsc::channel();
        engine.enqueue_device("camera","disconnect",move |_|{
            let error=camera.disconnect_confirmed().unwrap_err();ready.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(3)).unwrap();Err(format!("Camera cleanup unconfirmed: {error}"))
        }).unwrap();let expected=engine.operations.last_id.clone().unwrap();
        receive.recv_timeout(Duration::from_secs(2)).unwrap();engine.tick();
        assert!(!engine.camera.as_ref().unwrap().health().cleanup_confirmed);
        assert!(!engine.operations.history.iter().find(|op|op.operation_id==expected).unwrap().cancel_requested);
        assert!(!engine.operations.device_active("safety"));
        release.send(()).unwrap();let deadline=Instant::now()+Duration::from_secs(2);
        while engine.operations.history.iter().find(|op|op.operation_id==expected).unwrap().status!="cleanup_unconfirmed"&&Instant::now()<deadline{
            engine.tick();std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(engine.operations.history.iter().find(|op|op.operation_id==expected).unwrap().status,"cleanup_unconfirmed");
        assert!(engine.fault_latched);assert!(engine.manual_lock.is_some());assert!(engine.camera.is_some());
    }
    #[test]
    fn cancelled_late_cleanup_failure_preserves_fault_and_abandon_lock_without_applying_stale_success(){
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").join(format!("{}-late-cleanup-evidence",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        engine.parameters=crate::Parameters{task_id:"retained-cleanup".into(),save_path:root.to_string_lossy().into(),projection_count:2,angle_step_deg:180.0,exposure_ms:200.0};
        engine.task_registry.register(&engine.parameters).unwrap();engine.manual_lock=Some("Abandon waiting for confirmed cleanup".into());
        let(ready,receive)=mpsc::channel();let(release,resume)=mpsc::channel();
        engine.enqueue_device("safety","stop",move |_|{ready.send(()).unwrap();resume.recv_timeout(Duration::from_secs(2)).unwrap();Err("Camera cleanup unconfirmed; scripted late evidence".into())}).unwrap();
        let expected=engine.operations.last_id.clone().unwrap();receive.recv_timeout(Duration::from_secs(2)).unwrap();
        engine.operations.cancel();release.send(()).unwrap();let deadline=Instant::now()+Duration::from_secs(2);
        while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}
        let operation=engine.operations.history.iter().find(|op|op.operation_id==expected).unwrap();
        assert_eq!(operation.status,"cleanup_unconfirmed");assert!(operation.error.as_ref().unwrap().contains("late evidence"));
        assert!(engine.fault_latched);assert!(engine.task_registry.locked());assert_eq!(engine.manual_lock.as_deref(),Some("Abandon waiting for confirmed cleanup"));
    }
    #[test]
    fn camera_identity_capability_and_descriptor_changes_invalidate_preflight_with_unchanged_actuals(){
        let camera=scripted_camera(ScriptedCameraConfig::default());let original=camera.health();
        for change in ["identity","operations","effectiveProperties","captureCapability","nativeRange"] {
            let mut health=original.clone();match change {
                "identity"=>health.identity.as_mut().unwrap().serial=Some("new-device".into()),
                "operations"=>health.device_info.as_mut().unwrap().dataset_sha256="changed-device-info".into(),
                "effectiveProperties"=>health.device_info.as_mut().unwrap().effective_properties.push(0xd1a4),
                "captureCapability"=>health.capabilities.host_capture=false,
                _=>health.properties.get_mut("iso").unwrap().range=Some([100,6400,100]),
            }
            let mut engine=Engine::new(false);engine.cached_camera_health=Some(original.clone());engine.preflight=true;engine.capture_profile=Some(crate::capture_profile::test_profile());
            engine.sync_camera_readback(health);assert!(!engine.preflight,"{change}");assert!(engine.capture_profile.is_none());assert!(!engine.fault_latched);
        }
    }
    #[test]
    fn unadvertised_optional_camera_controls_do_not_invalidate_confirmed_preflight(){
        let camera=scripted_camera(ScriptedCameraConfig::default());let mut health=camera.health();
        let optional=health.properties.get_mut("hdrExposureDifference").unwrap();optional.supported=false;optional.readable=false;optional.actual=None;optional.confirmed=false;optional.error=None;
        let mut engine=Engine::new(false);engine.cached_camera_health=Some(health.clone());engine.preflight=true;
        engine.capture_profile=Some(crate::capture_profile::test_profile());engine.sync_camera_readback(health.clone());
        assert!(engine.preflight);assert!(engine.capture_profile.is_some());
        health.properties.get_mut("hdrExposureDifference").unwrap().error=Some("advertised descriptor failed".into());engine.sync_camera_readback(health);
        assert!(!engine.preflight);assert!(engine.capture_profile.is_none());
    }
    #[test]
    fn confirmed_explicit_camera_disconnect_does_not_cancel_its_pending_domain_operation(){
        let mut engine=Engine::new(false);let camera=scripted_camera(ScriptedCameraConfig::default());engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        let(ready,receive)=mpsc::channel();
        engine.enqueue_device("camera","disconnect",move |_|{let health=camera.disconnect_confirmed().map_err(|error|error.to_string())?;ready.send(()).unwrap();std::thread::sleep(Duration::from_millis(80));Ok(Box::new(move |engine|{engine.cached_camera_health=Some(health);engine.camera=None;Ok(())}))}).unwrap();
        let expected=engine.operations.last_id.clone().unwrap();receive.recv_timeout(Duration::from_secs(2)).unwrap();engine.tick();
        assert!(!engine.fault_latched);assert!(!engine.operations.history.iter().find(|operation|operation.operation_id==expected).unwrap().cancel_requested);
        let deadline=Instant::now()+Duration::from_secs(2);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}
        assert_eq!(engine.operations.history.iter().find(|operation|operation.operation_id==expected).unwrap().status,"completed");assert!(!engine.fault_latched);assert!(engine.camera.is_none());
    }
    #[test]
    fn unrequested_camera_retirement_still_triggers_domain_fault_and_cleanup(){
        let mut engine=Engine::new(false);let camera=scripted_camera(ScriptedCameraConfig::default());engine.cached_camera_health=Some(camera.health());engine.camera=Some(camera.clone());
        camera.disconnect_confirmed().unwrap();engine.tick();assert!(engine.fault_latched);assert!(engine.operations.history.iter().any(|operation|operation.device=="safety"));
    }
    #[test]
    fn readonly_physical_focus_change_invalidates_preflight_without_an_applied_target(){
        use crate::devices::camera::CameraValue;
        let camera=scripted_camera(ScriptedCameraConfig::default());let mut previous=camera.health();
        let focus=previous.properties.get_mut("focusMode").unwrap();focus.actual=Some(CameraValue::Integer(3));focus.target=None;focus.writable=false;focus.error=None;focus.confirmed=true;
        let mut actual=previous.clone();actual.properties.get_mut("focusMode").unwrap().actual=Some(CameraValue::Integer(0));
        let mut engine=Engine::new(false);engine.cached_camera_health=Some(previous);engine.preflight=true;engine.capture_profile=Some(crate::capture_profile::test_profile());
        engine.sync_camera_readback(actual);assert!(!engine.preflight);assert!(engine.capture_profile.is_none());
        let focus=&engine.cached_camera_health.as_ref().unwrap().properties["focusMode"];assert!(focus.target.is_none());assert_eq!(focus.actual,Some(CameraValue::Integer(0)));assert!(!engine.fault_latched);
    }
    #[test]
    fn card_safe_finishing_is_registered_as_acquired_pending_import_and_never_completed(){
        let(mut engine,_root)=registered_stop_fixture("card-pending-import");
        let parameters=engine.parameters.clone();
        assert_eq!(engine.snapshot()["cardAcquisition"]["destination"],"unknown");
        assert_eq!(engine.snapshot()["cardAcquisition"]["pendingImportTasks"],0);
        let(camera,_)=crate::devices::camera::scripted_camera_with_probe(ScriptedCameraConfig{card_capture:true,..Default::default()});
        let identity=crate::devices::turntable::NanoIdentity{
            device:crate::devices::turntable::EXPECTED_DEVICE.into(),
            version:crate::scan::CARD_FIRMWARE_VERSION.into(),protocol:2,
            build:crate::scan::CARD_FIRMWARE_BUILD.into(),buzzer_installed:true,
            capabilities:vec!["HEARTBEAT_ACK".into(),crate::scan::CARD_SETTLE_CAPABILITY.into()]};
        let policy=crate::scan::card_capture_policy(&identity,false).unwrap();
        let profile=crate::capture_profile::CaptureProfile::from_card_readback(&camera.health(),camera.card_profile_readback().unwrap(),None,policy).unwrap();
        // The default limit of a new card acquisition is the unified 1000 s; an
        // explicit value is kept and a host-RAM profile never takes the card default.
        assert_eq!(card_default_max_xray_sec(&profile,false),Some(crate::scan::SCAN_MAX_XRAY_SEC));
        assert_eq!(card_default_max_xray_sec(&profile,true),None);
        assert_eq!(card_default_max_xray_sec(&crate::capture_profile::test_profile(),false),None);
        let record=|index:u32,kind:&str|json!({"index":index,"kind":kind,"angleDeg":0.0,
            "exposureMs":parameters.exposure_ms,"state":crate::scan::DATA_PENDING_IMPORT,
            "cameraSessionId":"session-card","operationId":format!("op-{index}"),
            "storageId":65537u32,"storageChanged":false,"handle":256u32+index,"parent":1u32,
            "sourceFileName":format!("DSC_{index:04}.NEF"),"format":12288u32,
            "size":26733241u64,"captureDate":"20261011T120000"});
        let records:Vec<Value>=(1..=parameters.projection_count).map(|index|record(index,"projection"))
            .chain((1..=20u32).map(|index|record(index,"preDark"))).collect();
        let manifest=|acquisition:&str,records:Vec<Value>|json!({"schemaVersion":4,
            "scanId":parameters.task_id,"projectionCount":parameters.projection_count,
            "exposureMs":parameters.exposure_ms,"captureProfile":profile,
            "maxXraySec":crate::scan::SCAN_MAX_XRAY_SEC,"voltageKv":20.0,"currentUa":100.0,
            "captureDestination":"cameraCard","acquisitionStatus":acquisition,
            "dataStatus":crate::scan::DATA_PENDING_IMPORT,"stage":"finalizing","completed":false,
            "geometry":geometry(),"references":{"preDark":[],"preFlat":[],"postFlat":[],"postDark":[]},
            "frames":[],"captureRecords":records});
        let directory=PathBuf::from(parameters.save_path.trim()).join(parameters.task_id.trim());
        std::fs::create_dir_all(&directory).unwrap();
        // An acquisition that is not safely finished cannot be promoted, and the
        // task keeps its lock.
        std::fs::write(directory.join("manifest.json"),serde_json::to_vec(&manifest("running",records.clone())).unwrap()).unwrap();
        engine.enqueue_acquired_pending_import().unwrap();
        pump(&mut engine);
        assert!(engine.task_registry.locked());
        assert!(engine.task_registry.pending_import_tasks().is_empty());
        assert_ne!(engine.phase,Phase::Completed);
        assert!(engine.metadata_completed==false);
        // Safe finishing (worker joined, beam OFF confirmed, ledger persisted) is
        // registered as a retained task with import required: no hardware lock, no
        // completion and no reconstruction.
        std::fs::write(directory.join("manifest.json"),serde_json::to_vec(&manifest(crate::scan::ACQUISITION_ACQUIRED,records)).unwrap()).unwrap();
        engine.enqueue_acquired_pending_import().unwrap();
        pump(&mut engine);
        assert!(!engine.task_registry.locked(),"the scan occupancy is released");
        assert_eq!(engine.phase,Phase::Stopped);
        assert_eq!(engine.scan_stage,"pending_import");
        assert!(!engine.metadata_completed,"a pending import is never reconstructable");
        assert_eq!(engine.task_registry.pending_import_tasks().len(),1);
        assert!(engine.task_registry.check_retained_location(&parameters).unwrap());
        assert_eq!(engine.task_registry.snapshot()["retained"][0]["status"],crate::task_registry::TASK_STATUS_ACQUIRED_PENDING_IMPORT);
        let snapshot=engine.snapshot();
        assert_eq!(snapshot["cardAcquisition"]["pendingImportTasks"],1);
        assert_eq!(snapshot["cardAcquisition"]["currentTaskPendingImport"],true);
        assert_eq!(snapshot["cardAcquisition"]["importRequired"],false,"an import requirement needs the active card profile");
        assert_eq!(snapshot["workstation"]["reconstruction"]["unlocked"],false);
        assert_eq!(engine.task_registry.mark_import_verified(&parameters),Err("Import verification is missing sealed, committed card evidence".into()));
    }
    /// Hardware finding (2026-10-08, real Moxtek): after `send_voltage 4 kV` the
    /// verified readback is 3.99971 kV, and a following `send_current 10 uA`
    /// derived its voltage from that readback and was rejected as
    /// INVALID_XRAY_SETPOINT. The nominal minimum inside readback noise must stay
    /// usable, while a genuinely low verified setpoint must still be refused.
    #[test]
    fn a_verified_minimum_voltage_readback_still_admits_the_operators_current_command(){
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/xray").join(format!("{}-setpoint-readback-noise",chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        let(mut xray,_)=crate::devices::xray::test_worker(20.0,100.0);xray.set_usb_auto_shutdown(false).unwrap();
        engine.xray=Some(xray);engine.set_kv=3.999710083007813;engine.set_ua=9.9945068359375;
        engine.set_kv_confirmed=true;engine.set_ua_confirmed=false;
        engine.usb_auto_shut_down=false;engine.usb_auto_shut_down_confirmed=true;
        assert_eq!(engine.device_command("send_current",json!({"ua":10})).unwrap(),Ok(()));
        pump(&mut engine);
        assert!(engine.set_ua_confirmed,"the operator's 10 uA command must be admitted against the verified 4 kV readback");
        assert_eq!(engine.set_kv_confirmed,true);
        // A verified voltage genuinely below the minimum keeps its own bound and
        // must not be silently raised to the minimum.
        let mut low=Engine::new(false);
        let(mut worker,_)=crate::devices::xray::test_worker(20.0,100.0);worker.set_usb_auto_shutdown(false).unwrap();
        low.xray=Some(worker);low.set_kv=2.0;low.set_ua=10.0;low.set_kv_confirmed=true;low.set_ua_confirmed=false;
        assert_eq!(low.device_command("send_current",json!({"ua":10})).unwrap(),Err("INVALID_XRAY_SETPOINT"));
    }
}
