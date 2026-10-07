//! Authoritative maintenance barrier. Distribution consumers use this report,
//! then EOF and owned-engine exit; process exit alone never proves cleanup.
use crate::{Engine,Phase,timestamp};
use serde::Serialize;
use serde_json::Value;
use std::sync::mpsc;
use std::time::{Duration,Instant};

#[derive(Clone,Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct SoftwareUpdatePreparation {
    pub session_id:String,pub operation_id:String,pub status:String,pub ready:bool,pub maintenance_locked:bool,
    pub created_at:String,pub completed_at:Option<String>,
    pub turntable_stop:String,pub xray_off:String,pub camera_cleanup:String,
    pub no_active_transactions:bool,pub error:Option<String>,
    #[serde(skip)] cancel_requested:bool,
}
impl Engine {
    pub(crate) fn prepare_software_update(&mut self)->Result<(),&'static str>{
        if self.maintenance_lock{return Err("SOFTWARE_UPDATE_MAINTENANCE_LOCKED");}
        if self.task_registry.locked(){return Err("UNFINISHED_TASK_LOCKED");}
        if self.busy()||self.operations.is_active(){return Err("SOFTWARE_UPDATE_BUSY");}
        if self.manual_lock.is_some(){return Err("DEVICE_CLEANUP_UNCONFIRMED");}
        if self.camera.as_ref().is_some_and(|camera|camera.health().busy)||self.nano.as_ref().and_then(|nano|nano.health().status).is_some_and(|status|!matches!(status.state.as_str(),"IDLE"|"LOCKED"|"FAULT")){return Err("SOFTWARE_UPDATE_BUSY");}
        if self.camera.as_ref().is_some_and(|camera|camera.health().live_view.active){return Err("SOFTWARE_UPDATE_LIVE_VIEW_ACTIVE");}
        self.maintenance_lock=true;self.preflight=false;self.homed=false;
        let session=self.engine_session_id.clone();let created=timestamp();
        self.software_update_preparation=Some(SoftwareUpdatePreparation{session_id:session.clone(),operation_id:String::new(),status:"preparing".into(),ready:false,maintenance_locked:true,created_at:created.clone(),completed_at:None,turntable_stop:"unknown".into(),xray_off:"unknown".into(),camera_cleanup:"unknown".into(),no_active_transactions:false,error:None,cancel_requested:false});
        let nano=self.nano.as_ref().cloned();let camera=self.camera.as_ref().cloned();
        let xray_receive=self.xray.as_ref().map(|xray|xray.request_off());
        let nano_receive=nano.as_ref().map(|nano|{let nano=nano.clone();let(tx,rx)=mpsc::channel();std::thread::spawn(move||{let _=tx.send(nano.stop().map_err(|e|e.to_string()));});rx});
        let cached_camera=self.cached_camera_health.clone();let cached_xray=self.cached_xray_health.clone();
        let owned_nano=self.ever_nano||nano.is_some();let owned_xray=self.ever_xray||self.xray.is_some()||cached_xray.is_some();let owned_camera=self.ever_camera||camera.is_some()||cached_camera.is_some();
        let queued=self.enqueue_device("software_update","prepare_software_update",move |_|{
            let deadline=Instant::now()+Duration::from_secs(20);let mut errors=Vec::new();
            let xray_off=if let Some(rx)=xray_receive{match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())){Ok(Ok(health)) if health.beam_off_confirmed=>"confirmed",Ok(Err(error))=>{errors.push(format!("OFF unconfirmed: {error}"));"unknown"},_=>{errors.push("OFF unconfirmed".into());"unknown"}}}else if !owned_xray{"never-owned"}else if cached_xray.as_ref().is_some_and(|h|!h.connected&&h.beam_off_confirmed){"confirmed"}else{errors.push("Previously owned X-ray has no confirmed OFF/closed owner".into());"unknown"};
            let turntable_stop=if let Some(rx)=nano_receive{match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())){Ok(Ok(()))=>"confirmed",_=>{errors.push("Nano cleanup unconfirmed: fresh STOP was not confirmed".into());"unknown"}}}else if !owned_nano{"never-owned"}else{errors.push("Previously owned Nano has no fresh STOP owner".into());"unknown"};
            if let Some(nano)=nano.as_ref(){if let Err(error)=nano.set_xray_warning(xray_off=="unknown"){errors.push(format!("Nano warning cleanup unconfirmed: {error}"));}}
            let (camera_cleanup,health)=if let Some(camera)=camera{match camera.disconnect_confirmed(){Ok(health) if health.cleanup_confirmed=>("confirmed",Some(health)),Ok(health)=>{errors.push("Camera cleanup unconfirmed".into());("unknown",Some(health))},Err(error)=>{errors.push(format!("Camera cleanup unconfirmed: {error}"));("unknown",Some(camera.health()))}}}else if !owned_camera{("never-owned",None)}else if cached_camera.as_ref().is_some_and(|h|!h.connected&&h.cleanup_confirmed){("confirmed",cached_camera)}else{errors.push("Previously owned camera lacks confirmed closed-session cleanup".into());("unknown",cached_camera)};
            Ok(Box::new(move |engine|{
                let no_active=engine.real_scan.is_none()&&engine.reconstruction.is_none()&&!engine.operations.is_active()&&!engine.task_registry.locked()&&!matches!(engine.phase,Phase::Running|Phase::Paused|Phase::Finishing|Phase::Stopping);
                let report=engine.software_update_preparation.as_mut().ok_or("Maintenance report missing")?;
                if report.session_id!=session||report.created_at!=created||report.status!="preparing"{return Err("Stale maintenance report refused".into());}
                report.turntable_stop=turntable_stop.into();report.xray_off=xray_off.into();report.camera_cleanup=camera_cleanup.into();report.no_active_transactions=no_active;report.completed_at=Some(timestamp());
                if !no_active{errors.push("Transactions or operations remain active".into());}
                report.error=(!errors.is_empty()).then(||errors.join("; "));
                report.ready=errors.is_empty()&&!report.cancel_requested;
                report.status=if !errors.is_empty(){"blocked"}else if report.cancel_requested{"cancelled"}else{"ready"}.into();
                if errors.is_empty()&&report.cancel_requested{engine.maintenance_lock=false;report.maintenance_locked=false;}
                if camera_cleanup=="confirmed"{engine.camera=None;}if let Some(health)=health{engine.cached_camera_health=Some(health);}
                engine.cached_xray_health=engine.xray.as_ref().map(|x|x.health()).or_else(||engine.cached_xray_health.clone());
                if xray_off=="confirmed"{engine.beam_on=false;}
                engine.log(if errors.is_empty(){"PASS"}else{"ERR"},"system",if errors.is_empty(){"Software update device cleanup confirmed; maintenance ownership retained until shutdown or explicit cancel"}else{"Software update blocked: device cleanup remains unconfirmed"});
                Ok(())
            }))
        });
        match queued{Ok(())=>{self.software_update_preparation.as_mut().unwrap().operation_id=self.operations.last_id.clone().unwrap();Ok(())},Err(error)=>{self.software_update_preparation.as_mut().unwrap().status="blocked".into();self.software_update_preparation.as_mut().unwrap().error=Some(error.into());Err(error)}}
    }
    pub(crate) fn cancel_software_update_prepare(&mut self,payload:&Value)->Result<(),&'static str>{
        let report=self.software_update_preparation.as_mut().ok_or("SOFTWARE_UPDATE_NOT_PREPARED")?;
        if payload.get("operationId").and_then(Value::as_str)!=Some(report.operation_id.as_str())||report.session_id!=self.engine_session_id{return Err("SOFTWARE_UPDATE_OPERATION_MISMATCH");}
        if report.status=="preparing"{report.cancel_requested=true;return Ok(());}
        if report.status=="ready"||report.status=="cancelled"{report.ready=false;report.status="cancelled".into();report.completed_at=Some(timestamp());report.maintenance_locked=false;self.maintenance_lock=false;return Ok(());}
        Err("DEVICE_CLEANUP_UNCONFIRMED")
    }
    pub(crate) fn sync_software_update_preparation(&mut self){
        let Some(report)=self.software_update_preparation.as_mut()else{return;};
        if report.status!="preparing"{return;}
        if let Some(op)=self.operations.history.iter().find(|op|op.operation_id==report.operation_id){
            if matches!(op.status.as_str(),"failed"|"cleanup_unconfirmed"|"cancelled"){
                report.ready=false;report.status="blocked".into();report.completed_at=Some(timestamp());report.error=Some(op.error.clone().unwrap_or_else(||"Preparation safety evidence invalidated by Stop/cancel".into()));
            }
        }
    }
    pub(crate) fn invalidate_update_report_for_stop(&mut self){
        if let Some(report)=self.software_update_preparation.as_mut(){if self.maintenance_lock&&report.status=="ready"{report.ready=false;report.status="blocked".into();report.error=Some("New safety operation invalidated the previous update report".into());}}
    }
}

#[cfg(test)]
mod tests {
    use super::*;use serde_json::json;
    fn pump(engine:&mut Engine){let deadline=Instant::now()+Duration::from_secs(5);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();std::thread::sleep(Duration::from_millis(5));}assert!(!engine.operations.is_active());}
    #[test]
    fn never_owned_report_keeps_unknown_display_and_maintenance_until_exact_cancel(){
        let mut engine=Engine::new(false);engine.prepare_software_update().unwrap();let id=engine.operations.last_id.clone().unwrap();
        assert_eq!(engine.command("set_capture_root",json!({"savePath":"E:/unsafe"})),Err("SOFTWARE_UPDATE_MAINTENANCE_LOCKED"));pump(&mut engine);
        let report=engine.software_update_preparation.as_ref().unwrap();assert!(report.ready&&report.maintenance_locked&&report.no_active_transactions);assert_eq!(report.session_id,engine.engine_session_id);assert_eq!(report.operation_id,id);assert_eq!(report.xray_off,"never-owned");assert_eq!(report.turntable_stop,"never-owned");assert_eq!(report.camera_cleanup,"never-owned");assert_eq!(engine.snapshot()["workstation"]["xray"]["beamState"],"unknown");
        assert_eq!(engine.cancel_software_update_prepare(&json!({"operationId":"stale"})),Err("SOFTWARE_UPDATE_OPERATION_MISMATCH"));assert!(engine.maintenance_lock);engine.cancel_software_update_prepare(&json!({"operationId":id})).unwrap();assert!(!engine.maintenance_lock);assert!(!engine.software_update_preparation.as_ref().unwrap().ready);
    }
    #[test]
    fn owned_devices_need_fresh_cleanup_and_ordinary_stop_cannot_clear_maintenance(){
        let mut engine=Engine::new(false);engine.nano=Some(crate::devices::turntable::test_homed_nano());engine.camera=Some(crate::devices::camera::scripted_camera(Default::default()));engine.xray=Some(crate::devices::xray::test_worker(20.0,100.0).0);
        engine.prepare_software_update().unwrap();pump(&mut engine);let report=engine.software_update_preparation.as_ref().unwrap();assert!(report.ready);assert_eq!(report.xray_off,"confirmed");assert_eq!(report.turntable_stop,"confirmed");assert_eq!(report.camera_cleanup,"confirmed");assert!(engine.camera.is_none());
        engine.command("stop",json!({})).unwrap();pump(&mut engine);assert!(engine.maintenance_lock);assert!(!engine.software_update_preparation.as_ref().unwrap().ready);assert_eq!(engine.command("camera_live_view_start",json!({})),Err("SOFTWARE_UPDATE_MAINTENANCE_LOCKED"));engine.shutdown_with_timeout(Duration::from_secs(3)).unwrap();assert!(engine.maintenance_lock);
    }
    #[test]
    fn update_refuses_scan_busy_live_view_and_persistent_task_without_auto_abandon(){
        for phase in [Phase::Running,Phase::Paused,Phase::Finishing,Phase::Stopping]{let mut engine=Engine::new(false);engine.phase=phase;assert_eq!(engine.prepare_software_update(),Err("SOFTWARE_UPDATE_BUSY"));assert!(!engine.maintenance_lock);}
        let mut engine=Engine::new(false);let camera=crate::devices::camera::scripted_camera(Default::default());camera.start_live_view().unwrap();engine.camera=Some(camera.clone());assert_eq!(engine.prepare_software_update(),Err("SOFTWARE_UPDATE_LIVE_VIEW_ACTIVE"));assert!(camera.health().live_view.active);camera.stop_live_view().unwrap();
        let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine").join(format!("{}-update-task-lock",chrono::Utc::now().timestamp_nanos_opt().unwrap()));engine.parameters=crate::Parameters{task_id:"retained".into(),save_path:root.to_string_lossy().into(),projection_count:3,angle_step_deg:120.0,exposure_ms:200.0};engine.task_registry.register(&engine.parameters).unwrap();assert_eq!(engine.prepare_software_update(),Err("UNFINISHED_TASK_LOCKED"));assert!(engine.task_registry.locked());assert!(!engine.maintenance_lock);
    }
    #[test]
    fn unknown_or_failed_cleanup_stays_blocked_and_cannot_be_cancelled_into_safety(){
        let mut engine=Engine::new(false);engine.ever_xray=true;engine.ever_nano=true;engine.cached_camera_health=Some(crate::devices::camera::CameraHealth{cleanup_confirmed:false,..Default::default()});engine.prepare_software_update().unwrap();let id=engine.operations.last_id.clone().unwrap();pump(&mut engine);let report=engine.software_update_preparation.as_ref().unwrap();assert!(!report.ready);assert_eq!(report.status,"blocked");assert_eq!(report.xray_off,"unknown");assert!(engine.maintenance_lock);assert_eq!(engine.cancel_software_update_prepare(&json!({"operationId":id})),Err("DEVICE_CLEANUP_UNCONFIRMED"));
    }
    #[test]
    fn cancel_while_preparing_waits_for_real_cleanup_before_releasing_maintenance(){
        let mut engine=Engine::new(false);engine.prepare_software_update().unwrap();let id=engine.operations.last_id.clone().unwrap();engine.cancel_software_update_prepare(&json!({"operationId":id})).unwrap();assert!(engine.maintenance_lock);pump(&mut engine);assert_eq!(engine.software_update_preparation.as_ref().unwrap().status,"cancelled");assert!(!engine.maintenance_lock);assert!(!engine.software_update_preparation.as_ref().unwrap().ready);
    }
}
