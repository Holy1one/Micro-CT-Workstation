//! Background I/O jobs return mutations to the sole domain owner, never another Engine.
use crate::{timestamp, Engine};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

pub type Completion = Box<dyn FnOnce(&mut Engine) -> Result<(), String> + Send>;
type JobResult = Result<Completion, String>;
pub struct Event { pub id: String, pub generation: u64, pub result: Option<JobResult> }
#[derive(Clone, Default, PartialEq, Serialize)]
#[serde(rename_all="camelCase")]
pub struct OperationProgress {
    pub processed_bytes: u64, pub total_bytes: u64,
    pub verified_files: usize, pub total_files: usize,
    pub percent: u8, pub estimated_remaining_sec: Option<u64>,
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn only_completed_owner_receipt_reaches_one_hundred_and_late_progress_is_ignored() {
        for terminal in ["completed","rejected","cancelled"] {
            let mut operations=Operations::default();
            let id=operations.spawn("test","task_project","open_project",|_|Err("fixture worker result".into())).unwrap();
            let shared=Arc::new(Mutex::new(OperationProgress {percent:100,total_bytes:100,processed_bytes:100,estimated_remaining_sec:Some(0),..Default::default()}));
            operations.track_progress(&id,shared.clone());let _=operations.drain();
            assert_eq!(operations.history.last().unwrap().progress.as_ref().unwrap().percent,99);
            if terminal=="cancelled" {
                operations.cancel();shared.lock().unwrap().processed_bytes=999;let _=operations.drain();
                assert_eq!(operations.history.last().unwrap().progress.as_ref().unwrap().processed_bytes,100);
            }
            operations.complete(&id,terminal,None);
            shared.lock().unwrap().percent=50;let _=operations.drain();
            let receipt=operations.history.last().unwrap();let progress=receipt.progress.as_ref().unwrap();
            assert_eq!(receipt.status,terminal);
            assert_eq!(progress.percent,if terminal=="completed"{100}else{99});
            assert_eq!(progress.estimated_remaining_sec,if terminal=="completed"{Some(0)}else{None});
        }
    }
}
pub type SharedProgress = Arc<Mutex<OperationProgress>>;
#[derive(Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Operation {
    pub operation_id: String, pub device_session_id: String, pub device: String,
    pub kind: String, pub stage: String, pub cancel_requested: bool, pub status: String,
    pub error: Option<String>, pub started_at: String, pub updated_at: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub progress: Option<OperationProgress>,
}
pub struct Operations {
    pub history: Vec<Operation>, pub generation: u64, pub last_id: Option<String>,
    counter: u64, sender: mpsc::Sender<Event>, receiver: mpsc::Receiver<Event>,
    active: HashMap<String, Arc<AtomicBool>>,
    progress: HashMap<String, SharedProgress>,
}
impl Default for Operations {
    fn default() -> Self { let (sender,receiver)=mpsc::channel(); Self { history:vec![], generation:0,last_id:None,counter:0,sender,receiver,active:HashMap::new(),progress:HashMap::new() } }
}
impl Operations {
    pub(crate) fn next_id(&self,session:&str)->String{format!("{session}-op-{}",self.counter+1)}
    pub fn is_active(&self) -> bool { !self.active.is_empty() }
    pub fn blocks_controls(&self)->bool{self.history.iter().any(|operation|self.active.contains_key(&operation.operation_id)&&operation.kind!="metadata_probe")}
    pub fn device_active(&self, device:&str) -> bool { self.history.iter().any(|op| op.device==device && self.active.contains_key(&op.operation_id)) }
    pub fn spawn(&mut self, session:&str, device:&str, kind:&str, job:impl FnOnce(Arc<AtomicBool>)->JobResult+Send+'static) -> Result<String,&'static str> {
        if device!="safety" && (self.active.len()>=16 || self.device_active(device)) { return Err("DEVICE_OPERATION_ACTIVE"); }
        self.counter+=1;
        let id=format!("{session}-op-{}",self.counter);
        let now=timestamp();
        self.history.push(Operation { operation_id:id.clone(),device_session_id:session.into(),device:device.into(),kind:kind.into(),stage:"queued".into(),cancel_requested:false,status:"queued".into(),error:None,started_at:now.clone(),updated_at:now,progress:None });
        if self.history.len()>128 { if let Some(index)=self.history.iter().position(|op| !self.active.contains_key(&op.operation_id)) { self.history.remove(index); } }
        let cancel=Arc::new(AtomicBool::new(false));
        self.active.insert(id.clone(),cancel.clone());
        self.last_id=Some(id.clone());
        let sender=self.sender.clone(); let operation_id=id.clone(); let generation=self.generation;
        std::thread::spawn(move || {
            let _=sender.send(Event{id:operation_id.clone(),generation,result:None});
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| if cancel.load(Ordering::SeqCst){Err("Device operation cancelled before I/O".into())}else{job(cancel)})).unwrap_or_else(|_|Err("Device operation worker panicked; cleanup required".into()));
            let _=sender.send(Event{id:operation_id,generation,result:Some(result)});
        });
        Ok(id)
    }
    /// Continue the same receipt after an owner-only admission stage. No
    /// completed snapshot escapes between validation and persistence/start.
    pub(crate) fn continue_job(&mut self,id:&str,job:impl FnOnce(Arc<AtomicBool>)->JobResult+Send+'static)->Result<(),&'static str>{
        if self.active.contains_key(id){return Err("DEVICE_OPERATION_ACTIVE");}
        let operation=self.history.iter_mut().find(|operation|operation.operation_id==id).ok_or("OPERATION_RECEIPT_MISSING")?;
        if operation.status!="completed"||operation.cancel_requested{return Err("OPERATION_CONTINUATION_CANCELLED");}
        operation.status="queued".into();operation.stage="registry_persistence".into();operation.updated_at=timestamp();
        let cancel=Arc::new(AtomicBool::new(false));self.active.insert(id.into(),cancel.clone());self.last_id=Some(id.into());
        let sender=self.sender.clone();let operation_id=id.to_owned();let generation=self.generation;
        std::thread::spawn(move||{
            let _=sender.send(Event{id:operation_id.clone(),generation,result:None});
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||if cancel.load(Ordering::SeqCst){Err("Device operation cancelled before continuation I/O".into())}else{job(cancel)})).unwrap_or_else(|_|Err("Device operation continuation panicked; cleanup required".into()));
            let _=sender.send(Event{id:operation_id,generation,result:Some(result)});
        });Ok(())
    }
    #[cfg(test)]
    pub fn cancel(&mut self) {
        self.cancel_jobs(false);
    }
    /// Terminal storage writes have already passed device cleanup. Let their
    /// atomic write and owner commit finish together so Stop cannot orphan a
    /// durable register change. They never start device I/O or a new scan.
    pub fn cancel_for_stop(&mut self) {
        self.cancel_jobs(true);
    }
    pub fn terminal_task_write(&self,id:&str)->bool {
        self.history.iter().any(|op|op.operation_id==id&&op.device=="task_storage"
            && matches!(op.kind.as_str(),"complete"|"abandon"|"reconcile_deleted")&&!op.cancel_requested)
    }
    fn cancel_jobs(&mut self, preserve_terminal:bool) {
        self.generation+=1;
        for (id, token) in &self.active {
            if preserve_terminal && self.terminal_task_write(id) {continue;}
            token.store(true,Ordering::SeqCst);
            if let Some(op)=self.history.iter_mut().find(|op| &op.operation_id==id) { op.cancel_requested=true;op.status="cancelling".into();op.stage="cleanup_requested".into();op.updated_at=timestamp(); }
        }
    }
    pub fn cancel_device(&mut self,device:&str) {
        for op in self.history.iter_mut().filter(|op|op.device==device) {
            if let Some(token)=self.active.get(&op.operation_id){token.store(true,Ordering::SeqCst);op.cancel_requested=true;op.status="cancelling".into();op.stage="cleanup_requested".into();op.updated_at=timestamp();}
        }
    }
    pub(crate) fn track_progress(&mut self,id:&str,progress:SharedProgress) {
        if self.active.contains_key(id) { self.progress.insert(id.into(),progress); }
    }
    pub fn drain(&mut self)->Vec<Event> {
        for op in self.history.iter_mut().filter(|op| !op.cancel_requested) {
            if let Some(shared)=self.progress.get(&op.operation_id) {
                if let Ok(progress)=shared.lock() {
                    let mut progress=progress.clone();
                    // Only owner commit may report 100%, never a worker timer.
                    progress.percent=progress.percent.min(99);
                    if op.progress.as_ref()!=Some(&progress) {op.progress=Some(progress);op.updated_at=timestamp();}
                }
            }
        }
        self.receiver.try_iter().collect()
    }
    pub fn complete(&mut self,id:&str,status:&str,error:Option<String>) {
        self.active.remove(id);
        self.progress.remove(id);
        if let Some(op)=self.history.iter_mut().find(|op|op.operation_id==id) { op.status=status.into();op.stage=if status=="completed" {"confirmed"} else {status}.into();op.error=error;op.updated_at=timestamp(); }
        if let Some(progress)=self.history.iter_mut().find(|op|op.operation_id==id).and_then(|op|op.progress.as_mut()) {
            if status=="completed" {progress.percent=100;progress.estimated_remaining_sec=Some(0);}
            else {progress.percent=progress.percent.min(99);progress.estimated_remaining_sec=None;}
        }
    }
    pub fn started(&mut self,id:&str) { if let Some(op)=self.history.iter_mut().find(|op|op.operation_id==id && !op.cancel_requested) {op.status="running".into();op.stage="device_io".into();op.updated_at=timestamp();} }
}
