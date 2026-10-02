//! Independent device commands. Background jobs own inputs/proxies; only tick mutates Engine.
use crate::{Engine, Phase, nano_reference_ready, may_reuse_home, scan, timestamp};
use crate::devices::camera::{CameraAdapter, CameraProperties};
use crate::devices::turntable::{NanoAdapter, NanoConnectionState};
use crate::devices::xray::MoxtekAdapter;
use crate::operations::Completion;
use serde_json::{json,Value};
use std::sync::{mpsc,Arc};
use std::sync::atomic::{AtomicBool,Ordering};
use std::path::PathBuf;
use std::time::{Duration,Instant};

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestConnections {
    pub nano:Option<Arc<dyn Fn()->Result<Arc<NanoAdapter>,String>+Send+Sync>>,
    pub camera:Option<Arc<dyn Fn()->Result<CameraAdapter,String>+Send+Sync>>,
    pub xray:Option<Arc<dyn Fn()->Result<MoxtekAdapter,String>+Send+Sync>>,
}

impl Engine {
    pub(crate) fn enqueue_task_completed(&mut self)->Result<(),&'static str>{
        let plan=self.task_registry.prepare_completed(&self.parameters).map_err(|_|"TASK_COMPLETION_PREPARATION_FAILED")?;
        self.enqueue_device("task_storage","complete",move|cancel|{plan.execute(&cancel)?;Ok(Box::new(move|engine|{engine.task_registry.commit(plan)?;engine.phase=Phase::Completed;engine.scan_stage="completed";engine.scan_prompt=None;engine.last_error=None;engine.metadata_completed=true;engine.metadata_manifest=true;engine.metadata_resume=false;Ok(())}))})
    }
    pub(crate) fn enqueue_stop_for_shutdown(&mut self)->Result<(),&'static str>{self.enqueue_stop(true,false)}
    pub(crate) fn sync_camera_health(&mut self) {
        let Some(camera)=self.camera.as_ref()else{return;};
        let health=camera.health();
        let unknown_properties=health.connected&&health.properties.values().any(|property|!property.confirmed||property.actual.is_none()||property.error.is_some());
        if unknown_properties {self.preflight=false;self.capture_profile=None;}
        let lost=self.cached_camera_health.as_ref().is_some_and(|previous|previous.connected)&&!health.connected;
        let unconfirmed=health.isolated&&!health.cleanup_confirmed&&health.last_error.is_some();
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
            if event.generation!=self.operations.generation || self.operations.history.iter().any(|op|op.operation_id==event.id&&op.cancel_requested) {
                self.operations.complete(&event.id,"cancelled",None);
                continue;
            }
            // Remove the in-flight reservation before the owner applies a result.
            self.operations.complete(&event.id,"completed",None);
            let applied=result.and_then(|completion|completion(self));
            if let Err(error)=applied {
                let cleanup_unconfirmed=error.contains("cleanup unconfirmed") || error.contains("OFF unconfirmed");
                self.operations.complete(&event.id,if cleanup_unconfirmed {"cleanup_unconfirmed"} else {"failed"},Some(error.clone()));
                self.last_error=Some(error.clone());
                self.preflight=false;
                if cleanup_unconfirmed {self.fault_latched=true;self.phase=Phase::Fault;if self.manual_lock.as_deref()!=Some("Abandon waiting for confirmed cleanup"){self.manual_lock=Some(error.clone());}}
                self.log("ERR","devices",&error);
                let device=self.operations.history.iter().find(|op|op.operation_id==event.id).map(|op|op.device.clone());
                if device.as_deref().is_some_and(|device|matches!(device,"camera"|"turntable"|"xray")) && !self.operations.device_active("safety") {
                    self.fault_latched=true;
                    let _=self.enqueue_stop(true,false);
                    self.phase=Phase::Fault;
                }
            }
            self.sequence+=1;
        }
    }
    pub(crate) fn device_controls_snapshot(&self)->Value {
        let health=self.nano.as_ref().map(|n|n.health());
        let status=health.as_ref().and_then(|h|h.status.as_ref());
        json!({
            "camera":self.camera.as_ref().map(|c|c.control_snapshot()).or_else(||self.cached_camera_health.clone()).unwrap_or_default(),
            "turntable":{
                "connected":health.as_ref().is_some_and(|h|h.state==NanoConnectionState::Connected),
                "identity":health.as_ref().and_then(|h|h.identity.clone()),"status":status,
                "confirmedAngleDeg":health.as_ref().and_then(crate::confirmed_nano_angle),
                "benchMode":self.bench_mode,"hardwareCertified":false,
                "reason":if self.busy(){Some("Device controls occupied by task or cleanup")}else{self.manual_lock.as_deref()}
            },
            "taskLock":self.task_registry.snapshot(),
            "presets":self.preset_store.snapshot(),"cameraDraft":self.camera_draft,
            "exportJson":self.preset_export_json,"presetComparison":self.preset_comparison
        })
    }
    pub(crate) fn device_command(&mut self,command:&str,payload:Value)->Option<Result<(),&'static str>> {
        if !matches!(command,"connect"|"disconnect"|"stop"|"preflight"|"home"|"retry_device"|"restore_previous"|"start_scan"|"start_reconstruction"|"abandon_scan"|"preset_list"|"preset_save"|"preset_delete"|"preset_load"|"preset_compare"|"preset_import"|"preset_export"|"camera_pipe_descriptor"|"camera_test_capture"|"camera_connect"|"camera_disconnect"|"camera_status"|"camera_identity"|"camera_read_properties"|"camera_apply_properties"|"camera_capture"|"camera_live_view_start"|"camera_live_view_stop"|"camera_autofocus"|"camera_focus"|"turntable_connect"|"turntable_disconnect"|"turntable_status"|"turntable_identity"|"turntable_home"|"turntable_relative"|"turntable_to_angle"|"turntable_bench_mode"|"xray_connect"|"xray_disconnect"|"xray_status"|"xray_identity"|"xray_toggle"|"xray_off"|"send_voltage"|"send_current"|"usb_auto_shut_down_toggle"|"set_usb_shutdown_delay") {return None;}
        Some(self.execute_device_command(command,payload))
    }
    fn execute_device_command(&mut self,command:&str,payload:Value)->Result<(),&'static str> {
        match command {
            "camera_status"|"camera_identity"|"turntable_status"|"turntable_identity"|"xray_status"|"xray_identity"=>Ok(()),
            "camera_pipe_descriptor"=>{if self.camera.as_ref().and_then(|c|c.pipe_descriptor()).is_none(){return Err("LV_NOT_ACTIVE");}Ok(())},
            "stop"|"xray_off"=>self.enqueue_stop(command=="stop",false),
            "disconnect"=>self.enqueue_stop(true,true),
            "abandon_scan"=>{
                if !self.task_registry.locked(){return Err("NO_ACTIVE_TASK");}
                if !self.task_registry.can_abandon(){return Err("TASK_REGISTRY_INVALID");}
                self.enqueue_stop(true,false)?;
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
                    Ok(Box::new(move |engine|{engine.nano=Some(nano);engine.connected=true;engine.home_performed_this_process=false;engine.invalidate(Phase::Idle);engine.last_error=None;Ok(())}))
                })
            }
            "turntable_disconnect"=>{
                self.connection_control_allowed()?;
                let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
                self.enqueue_device("turntable","disconnect",move |_|{
                    nano.stop().map_err(|e|e.to_string())?;
                    nano.set_xray_warning(false).map_err(|e|e.to_string())?;
                    Ok(Box::new(|engine|{engine.nano=None;engine.connected=false;engine.homed=false;engine.home_performed_this_process=false;engine.preflight=false;Ok(())}))
                })
            }
            "camera_connect"=>self.enqueue_camera_connect(payload),
            "camera_disconnect"=>{
                self.connection_control_allowed()?;
                let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
                self.enqueue_device("camera","disconnect",move |_|{let health=camera.disconnect_confirmed().map_err(|error|format!("Camera cleanup unconfirmed: {error}"))?;if !health.cleanup_confirmed{return Err("Camera cleanup unconfirmed; isolated proxy retained".into());}Ok(Box::new(move |engine|{engine.cached_camera_health=Some(health);engine.camera=None;engine.preflight=false;Ok(())}))})
            }
            "camera_read_properties"|"camera_apply_properties"|"camera_capture"|"camera_test_capture"|"camera_live_view_start"|"camera_live_view_stop"|"camera_autofocus"|"camera_focus"=>self.enqueue_camera_control(command,payload),
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
                } else {let ua=payload.get("ua").and_then(Value::as_f64).ok_or("INVALID_CURRENT")?;(if ua>0.0{self.set_kv.min(12000.0/ua)}else{self.set_kv},ua)};
                self.validate_setpoint(kv,ua)?;
                let clamped=if voltage {ua!=self.set_ua}else{kv!=self.set_kv};
                let mut xray=self.xray.as_ref().cloned().ok_or("XRAY_NOT_CONNECTED")?;
                self.enqueue_device("xray",command,move |_|{let health=xray.set_parameters(kv,ua).map_err(|e|e.to_string())?;Ok(Box::new(move |engine|{engine.set_kv=health.set_voltage_kv.ok_or("Missing verified voltage")?;engine.set_ua=health.set_current_ua.ok_or("Missing verified current")?;if voltage{engine.set_kv_confirmed=true;engine.set_ua_confirmed|=clamped;}else{engine.set_ua_confirmed=true;engine.set_kv_confirmed|=clamped;}engine.cached_xray_health=Some(health);engine.preflight=false;Ok(())}))})
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
        self.ever_xray=true;self.bench_mode=false;
        #[cfg(test)]
        if let Some(factory)=self.test_connections.xray.as_ref(){self.xray=Some(factory().map_err(|_|"XRAY_TEST_FACTORY_FAILED")?);}
        let mut xray=self.xray.get_or_insert_with(MoxtekAdapter::new).clone();
        self.enqueue_device("xray","connect",move |cancel|{
            #[cfg(not(test))]
            let health=xray.discover_and_connect().map_err(|e|e.to_string())?;
            #[cfg(test)]
            let health=if xray.health().connected{xray.refresh_off_status().map_err(|e|e.to_string())?}else{return Err("No scripted Moxtek connection factory configured".into());};
            if cancel.load(Ordering::SeqCst){let _=xray.force_off();return Err("Moxtek connection cancelled".into());}
            Ok(Box::new(move |engine|{engine.cached_xray_health=Some(health);engine.beam_on=false;engine.usb_auto_shut_down=true;engine.usb_auto_shut_down_confirmed=false;engine.set_kv_confirmed=false;engine.set_ua_confirmed=false;engine.preflight=false;engine.last_error=None;Ok(())}))
        })
    }
    fn enqueue_camera_connect(&mut self,payload:Value)->Result<(),&'static str> {
        self.connection_control_allowed()?;
        #[cfg(not(test))]
        let camera=match self.camera.as_ref(){Some(camera)=>camera.clone(),None=>CameraAdapter::discover().map_err(|_|"CAMERA_BACKEND_UNAVAILABLE")?};
        #[cfg(test)]
        let camera=match self.camera.as_ref(){Some(camera)=>camera.clone(),None=>self.test_connections.camera.as_ref().ok_or("CAMERA_TEST_FACTORY_REQUIRED")?().map_err(|_|"CAMERA_BACKEND_UNAVAILABLE")?};
        // Retain the proxy immediately so global Stop can invalidate a stuck connect.
        self.camera=Some(camera.clone());
        let selected=payload.get("deviceId").and_then(Value::as_str).map(str::to_owned);
        self.enqueue_device("camera","connect",move |cancel|{
            let list=camera.discover_list().map_err(|e|e.to_string())?;
            let Some(selected)=selected else {return Ok(Box::new(move |engine|{engine.cached_camera_health=Some(camera.health());engine.preflight=false;if list.len()!=1{return Err("Select exactly one camera deviceId; connection is not implicit".into());}Ok(())}));};
            camera.connect_selected(&selected).map_err(|e|e.to_string())?;
            if cancel.load(Ordering::SeqCst){camera.request_cancel();return Err("Camera connection cancelled".into());}
            Ok(Box::new(move |engine|{engine.cached_camera_health=Some(camera.health());engine.preflight=false;engine.last_error=None;Ok(())}))
        })
    }
    fn enqueue_camera_control(&mut self,command:&str,payload:Value)->Result<(),&'static str> {
        if command=="camera_live_view_stop" && self.camera.is_none(){return Ok(());}
        // Stop Live View remains available during task occupation and cleanup.
        if command=="camera_read_properties"{self.connection_control_allowed()?;}
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
        if matches!(command,"camera_capture"|"camera_test_capture") && health.live_view.active {return Err("LIVE_VIEW_CAPTURE_FORBIDDEN");}
        let properties:Option<CameraProperties>=if command=="camera_apply_properties"{Some(serde_json::from_value(payload.get("properties").cloned().ok_or("INVALID_PARAMETERS")?).map_err(|_|"INVALID_PARAMETERS")?)}else{None};
        if let Some(properties)=properties.as_ref(){camera.validate_properties(properties).map_err(|_|"UNSUPPORTED_CAMERA_PROPERTIES")?;self.preflight=false;}
        let capture_dir=if matches!(command,"camera_capture"|"camera_test_capture") {
            match payload.get("directory").and_then(Value::as_str){Some(path)if !path.trim().is_empty()=>PathBuf::from(path),_=>configured_capture_directory()?}
        }else{PathBuf::new()};
        let mut capture_roots=self.task_registry.capture_roots();
        if !self.parameters.save_path.trim().is_empty()&&!self.parameters.task_id.trim().is_empty(){capture_roots.push(PathBuf::from(self.parameters.save_path.trim()).join(self.parameters.task_id.trim()));}
        if matches!(command,"camera_capture"|"camera_test_capture"){
            if !capture_dir.is_absolute(){return Err("INVALID_CAPTURE_DIRECTORY");}
            let known_root=PathBuf::from(self.parameters.save_path.trim()).join(self.parameters.task_id.trim());
            if (!self.parameters.save_path.trim().is_empty()&&!self.parameters.task_id.trim().is_empty()&&crate::task_registry::lexical_path(&capture_dir).starts_with(&crate::task_registry::lexical_path(&known_root)))||self.task_registry.capture_directory_conflicts(&capture_dir){return Err("CAPTURE_DIRECTORY_IS_SCAN_TASK");}
        }
        let tag=payload.get("presetId").and_then(Value::as_str).map(|id|self.preset_store.get(id).ok_or("PRESET_NOT_FOUND")).transpose()?.map(|preset|preset.tag());
        let kind=command.to_owned();let operation=format!("{}-capture-{}",self.engine_session_id,timestamp().replace([':', '.'],"-"));
        self.enqueue_device("camera",command,move |cancel|{
            match kind.as_str(){
                "camera_read_properties"=>{camera.read_properties().map_err(|e|e.to_string())?;}
                "camera_apply_properties"=>{
                    let result=camera.apply_properties(properties.as_ref().expect("validated properties")).map_err(|e|e.to_string())?;
                    if !result.complete{return Err(format!("Camera parameter application partial: {:?}",result.failed));}
                }
                "camera_capture"|"camera_test_capture"=>{
                    reject_capture_metadata_ancestors(&capture_dir)?;
                    for root in &capture_roots{if let(Ok(directory),Ok(root))=(std::fs::canonicalize(&capture_dir),std::fs::canonicalize(root)){if directory.starts_with(root){return Err("Independent capture aliases a retained scan directory".into());}}}
                    std::fs::create_dir_all(&capture_dir).map_err(|e|e.to_string())?;
                    camera.capture_to_path(&capture_dir.join(format!("{operation}.nef")),&operation,&cancel,None).map_err(|e|e.to_string())?;
                }
                "camera_live_view_start"=>{camera.start_live_view().map_err(|e|e.to_string())?;}
                "camera_live_view_stop"=>{camera.stop_live_view().map_err(|e|e.to_string())?;}
                "camera_autofocus"=>camera.autofocus().map_err(|e|e.to_string())?,
                "camera_focus"=>{let step=i32::try_from(payload.get("step").and_then(Value::as_i64).ok_or("Missing bounded focus step")?).map_err(|_|"Invalid focus step")?;camera.focus_step(step).map_err(|e|e.to_string())?;}
                _=>return Err("Unsupported camera command".into()),
            }
            if cancel.load(Ordering::SeqCst){return Err("Camera operation cancelled".into());}
            Ok(Box::new(move |engine|{
                let health=camera.health();
                if kind=="camera_apply_properties" {
                    if let Some(actual)=health.properties.get("shutter").and_then(|p|p.actual.as_ref()) {
                        let nikon=health.identity.as_ref().is_some_and(|i|i.model.eq_ignore_ascii_case("D7100")||i.model.eq_ignore_ascii_case("Nikon D7100"));
                        engine.parameters.exposure_ms=crate::devices::camera::shutter_ms(actual,nikon).ok_or("Actual shutter is not a supported timed exposure")?;
                    }
                    engine.preset_tag=tag;engine.preflight=false;engine.capture_profile=None;
                }
                engine.cached_camera_health=Some(health);engine.last_error=None;Ok(())
            }))
        })
    }
    fn enqueue_home(&mut self,recovery:bool)->Result<(),&'static str> {
        if recovery {
            if self.busy(){return Err("DEVICE_CONTROL_LOCKED");}
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
            Ok(Box::new(move |engine|{engine.homed=true;engine.home_performed_this_process=true;engine.beam_on=false;engine.phase=if engine.preflight{Phase::Ready}else{Phase::Idle};engine.last_error=None;Ok(())}))
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
        self.require_connected()?;
        if self.task_registry.locked()&&!self.task_registry.matches(&self.parameters){return Err("UNFINISHED_TASK_LOCKED");}
        if self.busy(){return Err("SCAN_ACTIVE");}
        self.parameters.validate()?;
        if !(1..=600).contains(&self.max_xray_sec){return Err("INVALID_PARAMETERS");}
        let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
        let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
        let mut xray=self.xray.as_ref().cloned().ok_or("XRAY_NOT_CONNECTED")?;
        let recovery=self.task_registry.matches(&self.parameters);
        if recovery && (payload.get("restore").and_then(Value::as_bool)!=Some(true)||payload.get("confirmUsbAutoShutdownRelease").and_then(Value::as_bool)!=Some(true)){return Err("RESTORE_PREFLIGHT_CONFIRMATION_REQUIRED");}
        if !recovery {
            if !self.usb_auto_shut_down_confirmed || self.usb_auto_shut_down{return Err("USB_AUTO_SHUTDOWN_RELEASE_REQUIRED");}
            if !self.set_kv_confirmed || !self.set_ua_confirmed{return Err("XRAY_SETPOINT_CONFIRMATION_REQUIRED");}
        }
        let params=self.parameters.clone();let kv=self.set_kv;let ua=self.set_ua;let reused=self.home_performed_this_process;let preset=self.preset_tag.clone();
        self.preflight=false;
        self.enqueue_device("system","preflight",move |cancel|{
            let (kv,ua,frozen_max,frozen_profile)=if recovery {let(kv,ua,max,profile)=read_frozen_recovery(&params)?;(kv,ua,Some(max),Some(profile))}else{(kv,ua,None,None)};
            if scan::manifest_file_exists(&params) && !scan::resume_manifest_exists(&params){return Err("Scan output exists; use a new task".into());}
            if camera.health().live_view.active{camera.stop_live_view().map_err(|e|e.to_string())?;}
            camera.configure_host_capture(params.exposure_ms).map_err(|e|e.to_string())?;
            let readback=camera.profile_readback().map_err(|e|e.to_string())?;
            let identity=camera.health().identity.ok_or("Camera identity not confirmed")?;
            let mut profile=crate::capture_profile::CaptureProfile::from_readback(identity,readback,preset)?;
            if let Some(frozen)=frozen_profile.as_ref(){frozen.matches(&profile)?;profile.preset=frozen.preset.clone();}
            if profile.exposure_ms()!=Some(params.exposure_ms){return Err("Camera shutter differs from the shared Exposure target".into());}
            xray.force_off().map_err(|e|format!("OFF unconfirmed during preflight: {e}"))?;
            if cancel.load(Ordering::SeqCst){return Err("Preflight cancelled before setpoint/USB writes".into());}
            if recovery {let usb=xray.set_usb_auto_shutdown(false).map_err(|e|e.to_string())?;if usb.usb_auto_shutdown!=Some(false){return Err("USB auto-shutdown release lacks current-session confirmation".into());}}
            let health=xray.set_parameters(kv,ua).map_err(|e|e.to_string())?;
            if health.locked!=Some(false)||!health.beam_off_confirmed||health.temperature_c.is_none_or(|c|!c.is_finite()||c>65.0){return Err("Moxtek health not safe for preflight".into());}
            let status=nano.status().map_err(|e|e.to_string())?;let reusable=may_reuse_home(reused,&status);
            if status.state=="FAULT"{nano.clear_fault_cancelled(cancel.clone()).map_err(|e|e.to_string())?;}
            if !reusable{nano.rearm_cancelled(cancel.clone()).map_err(|e|e.to_string())?;}
            if cancel.load(Ordering::SeqCst){return Err("Preflight cancelled".into());}
            let restoring=scan::resume_manifest_exists(&params);
            Ok(Box::new(move |engine|{engine.capture_profile=Some(profile);engine.cached_camera_health=Some(camera.health());engine.set_kv=health.set_voltage_kv.ok_or("Verified voltage missing")?;engine.set_ua=health.set_current_ua.ok_or("Verified current missing")?;engine.cached_xray_health=Some(health);if recovery{engine.set_kv_confirmed=true;engine.set_ua_confirmed=true;engine.usb_auto_shut_down=false;engine.usb_auto_shut_down_confirmed=true;if let Some(max)=frozen_max{engine.max_xray_sec=max;}}engine.fault_latched=false;engine.preflight=true;engine.homed=reusable;engine.phase=if reusable{Phase::Ready}else{Phase::ReadyForHome};engine.scan_geometry=None;engine.scan_references=scan::ReferenceFrames::default();engine.metadata_resume=restoring;engine.scan_prompt=if restoring{None}else{Some("geometry")};engine.scan_stage=if restoring{"idle"}else{"geometry"};engine.last_error=None;Ok(())}))
        })
    }
    fn enqueue_restore(&mut self)->Result<(),&'static str> {
        if self.busy(){return Err("SCAN_ACTIVE");}
        if !self.preflight{return Err("PREFLIGHT_REQUIRED");}if !self.homed{return Err("HOME_REQUIRED");}
        let params=self.parameters.clone();let kv=self.set_kv;let ua=self.set_ua;
        let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
        let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
        let preset=self.preset_tag.clone();
        self.enqueue_device("system","restore",move |cancel|{
            if !nano_reference_ready(&nano.status().map_err(|e|e.to_string())?){return Err("HOME_REQUIRED".into());}
            let identity=camera.health().identity.ok_or("Camera identity missing")?;
            let actual=crate::capture_profile::CaptureProfile::from_readback(identity,camera.profile_readback().map_err(|e|e.to_string())?,preset)?;
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
        self.enqueue_device("system","reconstruction_prepare",move |cancel|{
            if !scan::completed_manifest_exists(&parameters){return Err("RECONSTRUCTION_LOCKED".into());}
            let scan_dir=PathBuf::from(parameters.save_path.trim()).join(parameters.task_id.trim());
            let cached=reconstruction::cached_result(&scan_dir,method)?;
            if cancel.load(Ordering::SeqCst){return Err("Reconstruction preparation cancelled".into());}
            let handle=if cached.is_none(){Some(ReconstructionHandle::start(ReconstructionRequest{scan_dir,method})?)}else{None};
            Ok(Box::new(move |engine|{engine.reconstruction_method=Some(method);if let Some(result)=cached{engine.reconstruction_status="completed";engine.reconstruction_percent=100;engine.reconstruction_message="重构已缓存".into();engine.reconstruction_cache_path=Some(result.cache_path.display().to_string());}else{engine.reconstruction=handle;engine.reconstruction_status="running";engine.reconstruction_percent=0;engine.reconstruction_message="准备重构".into();engine.reconstruction_cache_path=None;}Ok(())}))
        })
    }
    fn enqueue_start_scan(&mut self,payload:Value)->Result<(),&'static str> {
        self.require_connected()?;
        if self.busy(){return Err("SCAN_ACTIVE");}
        if !self.preflight{return Err("PREFLIGHT_REQUIRED");}if !self.homed{return Err("HOME_REQUIRED");}
        if self.scan_prompt.is_some()||self.scan_geometry.is_none(){return Err("SCAN_GEOMETRY_REQUIRED");}
        let nano=self.nano.as_ref().cloned().ok_or("NANO_NOT_CONNECTED")?;
        let camera=self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
        let expected=self.capture_profile.clone().ok_or("CAPTURE_PROFILE_REQUIRED")?;
        let preset=self.preset_tag.clone();
        let plan=self.task_registry.prepare_register(&self.parameters).map_err(|_|"TASK_REGISTRY_SAVE_FAILED")?;
        self.enqueue_device("system","start_scan",move |cancel|{
            let status=nano.status().map_err(|e|e.to_string())?;
            if !nano_reference_ready(&status){return Err("HOME_REQUIRED".into());}
            if camera.health().live_view.active{camera.stop_live_view().map_err(|e|e.to_string())?;}
            let identity=camera.health().identity.ok_or("Camera identity missing")?;
            let actual=crate::capture_profile::CaptureProfile::from_readback(identity,camera.profile_readback().map_err(|e|e.to_string())?,preset)?;
            expected.matches(&actual)?;
            if cancel.load(Ordering::SeqCst){return Err("Scan start cancelled".into());}
            plan.execute(&cancel)?;
            Ok(Box::new(move |engine|{engine.task_registry.commit(plan)?;engine.capture_profile=Some(actual);engine.command_core("start_scan",payload).map_err(str::to_owned)}))
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
        if all {self.operations.cancel();}else{self.operations.cancel_device("xray");}
        if all {self.scan_stop_requested=true;}
        self.preflight=false;if all {self.homed=false;self.home_performed_this_process=false;self.phase=Phase::Stopping;}
        // Admit all three safety actions before awaiting any cleanup.
        let xray_receive=self.xray.as_ref().map(|x|x.request_off());
        let nano_receive=if all {self.nano.as_ref().map(|nano|{let nano=nano.clone();let(tx,rx)=mpsc::channel();std::thread::spawn(move||{let result=nano.stop();let _=tx.send(result.map_err(|e|e.to_string()));});rx})}else{None};
        let warning_nano=self.nano.as_ref().cloned();
        let camera=if all {self.camera.as_ref().cloned()}else{None};
        if let Some(camera)=camera.as_ref(){camera.request_cancel();}
        if let Some(scan)=self.real_scan.as_ref(){scan.cancel_only();}
        let mut xray=self.xray.as_ref().cloned();
        self.enqueue_device("safety","stop",move |_|{
            let deadline=Instant::now()+Duration::from_secs(6);
            let mut errors=vec![];
            let off_confirmed=if let Some(rx)=xray_receive {match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())){Ok(Ok(health))=>health.beam_off_confirmed,Ok(Err(error))=>{errors.push(format!("OFF unconfirmed: {error}"));false},Err(_)=>{errors.push("OFF unconfirmed: worker timeout".into());false}}}else{false};
            if let Some(nano)=warning_nano{if let Err(error)=nano.set_xray_warning(!off_confirmed){errors.push(format!("Nano warning state unconfirmed: {error}"));}}
            if let Some(rx)=nano_receive{match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())){Ok(Ok(()))=>{},Ok(Err(error))=>errors.push(error),Err(_)=>errors.push("Nano cleanup unconfirmed".into())}}
            if let Some(camera)=camera.as_ref(){while !camera.health().cleanup_confirmed && Instant::now()<deadline{std::thread::sleep(Duration::from_millis(20));}if !camera.health().cleanup_confirmed{errors.push("Camera cleanup unconfirmed; session isolated".into());}}
            if disconnect && errors.is_empty(){if let Some(xray)=xray.as_mut(){xray.disconnect().map_err(|e|format!("OFF unconfirmed: {e}"))?;}}
            Ok(Box::new(move |engine|{
                if !errors.is_empty(){engine.fault_latched=true;engine.phase=Phase::Fault;return Err(errors.join("; "));}
                if all {engine.cached_camera_health=engine.camera.as_ref().map(|camera|camera.health());}
                engine.cached_xray_health=engine.xray.as_ref().map(|xray|xray.health());
                if disconnect {engine.nano=None;engine.camera=None;engine.xray=None;engine.connected=false;engine.set_kv_confirmed=false;engine.set_ua_confirmed=false;engine.usb_auto_shut_down_confirmed=false;}
                engine.beam_on=false;
                if all && engine.real_scan.is_none(){engine.phase=if engine.fault_latched{Phase::Fault}else{Phase::Stopped};}
                if all && engine.manual_lock.as_deref()==Some("Abandon waiting for confirmed cleanup") {
                    if engine.real_scan.is_some(){return Err("Scan cleanup unconfirmed; abandon not committed".into());}
                    let plan=engine.task_registry.prepare_abandon()?;
                    engine.enqueue_device("task_storage","abandon",move|cancel|{plan.execute(&cancel)?;Ok(Box::new(move|engine|{engine.task_registry.commit(plan)?;engine.manual_lock=None;Ok(())}))}).map_err(str::to_owned)?;
                }
                else if all {engine.manual_lock=None;}
                engine.last_error=None;Ok(())
            }))
        })
    }
}

fn read_frozen_recovery(parameters:&crate::Parameters)->Result<(f64,f64,u32,crate::capture_profile::CaptureProfile),String> {
    let root=PathBuf::from(parameters.save_path.trim()).join(parameters.task_id.trim());
    let path=if root.join("manifest.json").is_file(){root.join("manifest.json")}else{root.join("preparation-checkpoint.json")};
    if std::fs::metadata(&path).map_err(|e|e.to_string())?.len()>8*1024*1024{return Err("Retained acquisition metadata exceeds limit".into());}
    let data:Value=serde_json::from_reader(std::fs::File::open(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if data["schemaVersion"]!=3 || data["scanId"].as_str()!=Some(parameters.task_id.as_str()) || data["projectionCount"].as_u64()!=Some(u64::from(parameters.projection_count)) || data["exposureMs"].as_f64()!=Some(parameters.exposure_ms){return Err("Retained task lacks matching schema 3 configuration; legacy tasks cannot resume with this backend".into());}
    let profile:crate::capture_profile::CaptureProfile=serde_json::from_value(data["captureProfile"].clone()).map_err(|e|e.to_string())?;profile.validate()?;
    let kv=data["voltageKv"].as_f64().ok_or("Frozen voltage missing")?;let ua=data["currentUa"].as_f64().ok_or("Frozen current missing")?;
    let max=u32::try_from(data["maxXraySec"].as_u64().ok_or("Frozen continuous X-ray limit missing")?).map_err(|_|"Invalid frozen continuous X-ray limit")?;
    if !(1..=600).contains(&max){return Err("Frozen continuous X-ray limit outside bounds".into());}
    Ok((kv,ua,max,profile))
}
fn reject_capture_metadata_ancestors(directory:&std::path::Path)->Result<(),String>{
    let mut ancestor=directory.to_path_buf();while !ancestor.exists(){if !ancestor.pop(){return Err("Capture directory has no verifiable ancestor".into());}}
    let mut ancestor=std::fs::canonicalize(ancestor).map_err(|error|error.to_string())?;
    loop{if ancestor.join("manifest.json").exists()||ancestor.join("preparation-checkpoint.json").exists(){return Err("Independent capture directory lies below retained scan metadata".into());}if !ancestor.pop(){break;}}
    Ok(())
}

#[cfg(windows)]
fn configured_capture_directory()->Result<PathBuf,&'static str> {
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath,FOLDERID_Pictures,KF_FLAG_DEFAULT};
    use windows::Win32::System::Com::CoTaskMemFree;
    unsafe {
        let pointer=SHGetKnownFolderPath(&FOLDERID_Pictures,KF_FLAG_DEFAULT,None).map_err(|_|"PICTURES_DIRECTORY_UNAVAILABLE")?;
        let path=pointer.to_string().map_err(|_|"PICTURES_DIRECTORY_UNAVAILABLE");
        CoTaskMemFree(Some(pointer.0.cast()));
        Ok(PathBuf::from(path?).join("MicroCT/CameraCaptures"))
    }
}
#[cfg(not(windows))]
fn configured_capture_directory()->Result<PathBuf,&'static str>{Err("CAMERA_CAPTURE_DIRECTORY_REQUIRED")}
