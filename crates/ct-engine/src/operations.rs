//! Background I/O jobs return mutations to the sole domain owner, never another Engine.
use crate::{timestamp, Engine};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::sync::atomic::{AtomicBool, Ordering};

pub type Completion = Box<dyn FnOnce(&mut Engine) -> Result<(), String> + Send>;
type JobResult = Result<Completion, String>;
pub struct Event { pub id: String, pub generation: u64, pub result: Option<JobResult> }
#[derive(Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Operation {
    pub operation_id: String, pub device_session_id: String, pub device: String,
    pub kind: String, pub stage: String, pub cancel_requested: bool, pub status: String,
    pub error: Option<String>, pub started_at: String, pub updated_at: String,
}
pub struct Operations {
    pub history: Vec<Operation>, pub generation: u64, pub last_id: Option<String>,
    counter: u64, sender: mpsc::Sender<Event>, receiver: mpsc::Receiver<Event>,
    active: HashMap<String, Arc<AtomicBool>>,
}
impl Default for Operations {
    fn default() -> Self { let (sender,receiver)=mpsc::channel(); Self { history:vec![], generation:0,last_id:None,counter:0,sender,receiver,active:HashMap::new() } }
}
impl Operations {
    pub fn is_active(&self) -> bool { !self.active.is_empty() }
    pub fn blocks_controls(&self)->bool{self.history.iter().any(|operation|self.active.contains_key(&operation.operation_id)&&operation.kind!="metadata_probe")}
    pub fn device_active(&self, device:&str) -> bool { self.history.iter().any(|op| op.device==device && self.active.contains_key(&op.operation_id)) }
    pub fn spawn(&mut self, session:&str, device:&str, kind:&str, job:impl FnOnce(Arc<AtomicBool>)->JobResult+Send+'static) -> Result<String,&'static str> {
        if device!="safety" && (self.active.len()>=16 || self.device_active(device)) { return Err("DEVICE_OPERATION_ACTIVE"); }
        self.counter+=1;
        let id=format!("{session}-op-{}",self.counter);
        let now=timestamp();
        self.history.push(Operation { operation_id:id.clone(),device_session_id:session.into(),device:device.into(),kind:kind.into(),stage:"queued".into(),cancel_requested:false,status:"queued".into(),error:None,started_at:now.clone(),updated_at:now });
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
    pub fn cancel(&mut self) {
        self.generation+=1;
        for (id, token) in &self.active {
            token.store(true,Ordering::SeqCst);
            if let Some(op)=self.history.iter_mut().find(|op| &op.operation_id==id) { op.cancel_requested=true;op.status="cancelling".into();op.stage="cleanup_requested".into();op.updated_at=timestamp(); }
        }
    }
    pub fn cancel_device(&mut self,device:&str) {
        for op in self.history.iter_mut().filter(|op|op.device==device) {
            if let Some(token)=self.active.get(&op.operation_id){token.store(true,Ordering::SeqCst);op.cancel_requested=true;op.status="cancelling".into();op.stage="cleanup_requested".into();op.updated_at=timestamp();}
        }
    }
    pub fn drain(&mut self)->Vec<Event> { self.receiver.try_iter().collect() }
    pub fn complete(&mut self,id:&str,status:&str,error:Option<String>) {
        self.active.remove(id);
        if let Some(op)=self.history.iter_mut().find(|op|op.operation_id==id) { op.status=status.into();op.stage=if status=="completed" {"confirmed"} else {status}.into();op.error=error;op.updated_at=timestamp(); }
    }
    pub fn started(&mut self,id:&str) { if let Some(op)=self.history.iter_mut().find(|op|op.operation_id==id && !op.cancel_requested) {op.status="running".into();op.stage="device_io".into();op.updated_at=timestamp();} }
}
