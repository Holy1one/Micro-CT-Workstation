//! Engine-owned persistent acquisition lock. Inputs are validated task parameters;
//! outputs are retained task records and an explicit manual-control gate. A corrupt
//! register or acquisition record stays locked; no image or manifest is rewritten.

use crate::{timestamp, Parameters};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool,Ordering};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRecord {
    pub parameters: Parameters,
    pub status: String,
    pub registered_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegisterFile {
    schema_version: u32,
    active: Option<TaskRecord>,
    retained: Vec<TaskRecord>,
}

pub struct TaskRegistry {
    path: PathBuf,
    file: RegisterFile,
    error: Option<String>,
    identity_verified: bool,
}

pub struct TaskWritePlan {path:PathBuf,expected:RegisterFile,next:RegisterFile,register:Option<Parameters>,completed:Option<Parameters>}
impl TaskWritePlan {
    pub fn execute(&self,cancel:&AtomicBool)->Result<(),String>{
        if cancel.load(Ordering::SeqCst){return Err("Task persistence cancelled".into());}
        if let Some(parameters)=self.register.as_ref(){
            if self.expected.retained.iter().any(|record|same_location(&record.parameters,parameters)){return Err("SCAN_OUTPUT_RETAINED".into());}
            let root=acquisition_root(parameters);let same=self.expected.active.as_ref().is_some_and(|record|record.parameters==*parameters);
            if !same&&root.exists()&&fs::read_dir(root).map_err(|error|error.to_string())?.next().is_some(){return Err("SCAN_OUTPUT_EXISTS".into());}
        }
        if let Some(parameters)=self.completed.as_ref(){validate_acquisition_record(parameters,true)?;}
        if self.path.exists(){let current:RegisterFile=serde_json::from_reader(File::open(&self.path).map_err(|error|error.to_string())?).map_err(|error|error.to_string())?;
            if serde_json::to_vec(&current).map_err(|error|error.to_string())?!=serde_json::to_vec(&self.expected).map_err(|error|error.to_string())?{return Err("Stale task persistence plan cannot overwrite a newer registration".into());}}
        atomic_store_cancelled(&self.path,&serde_json::to_vec_pretty(&self.next).map_err(|error|error.to_string())?,cancel)
    }
}

impl TaskRegistry {
    pub fn capture_directory_conflicts(&self,directory:&Path)->bool{self.file.active.iter().chain(self.file.retained.iter()).any(|record|lexical_path(directory).starts_with(&lexical_path(&acquisition_root(&record.parameters))))}
    pub fn capture_roots(&self)->Vec<PathBuf>{self.file.active.iter().chain(self.file.retained.iter()).map(|record|acquisition_root(&record.parameters)).collect()}
    pub fn prepare_register(&mut self,parameters:&Parameters)->Result<TaskWritePlan,String>{
        parameters.validate().map_err(str::to_owned)?;
        if let Some(error)=self.error.as_ref(){return Err(error.clone());}
        if self.file.active.is_some()&&!self.matches(parameters){return Err("UNFINISHED_TASK_LOCKED".into());}
        let expected=self.file.clone();let mut next=expected.clone();
        if !self.matches(parameters){let now=timestamp();next.active=Some(TaskRecord{parameters:parameters.clone(),status:"unfinished".into(),registered_at:now.clone(),updated_at:now});}
        // Reserve the sole in-memory lock before a background write can start.
        self.file=next.clone();
        Ok(TaskWritePlan{path:self.path.clone(),expected,next,register:Some(parameters.clone()),completed:None})
    }
    pub fn prepare_completed(&self,parameters:&Parameters)->Result<TaskWritePlan,String>{
        if !self.matches(parameters){return Err("Task completion identity mismatch".into());}
        self.prepare_terminal("completed",Some(parameters.clone()))
    }
    pub fn prepare_abandon(&self)->Result<TaskWritePlan,String>{
        if !self.can_abandon(){return Err("Task identity is unverified".into());}
        self.prepare_terminal("abandoned",None)
    }
    fn prepare_terminal(&self,status:&str,completed:Option<Parameters>)->Result<TaskWritePlan,String>{
        let expected=self.file.clone();let mut next=expected.clone();let mut record=next.active.take().ok_or("NO_ACTIVE_TASK")?;record.status=status.into();record.updated_at=timestamp();next.retained.push(record);
        Ok(TaskWritePlan{path:self.path.clone(),expected,next,register:None,completed})
    }
    pub fn commit(&mut self,plan:TaskWritePlan)->Result<(),String>{
        let expected=if plan.register.is_some(){&plan.next}else{&plan.expected};
        if serde_json::to_vec(&self.file).map_err(|error|error.to_string())?!=serde_json::to_vec(expected).map_err(|error|error.to_string())?{return Err("Stale task persistence completion".into());}
        self.file=plan.next;self.error=None;Ok(())
    }
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut registry = Self { path, file: RegisterFile { schema_version: 1, active: None, retained: Vec::new() }, error: None, identity_verified: true };
        if !registry.path.exists() { return registry; }
        match File::open(&registry.path).map_err(|error| error.to_string())
            .and_then(|file| serde_json::from_reader::<_, RegisterFile>(file).map_err(|error| error.to_string())) {
            Ok(file) if file.schema_version == 1 => {
                registry.file = file;
                if let Err(error) = registry.validate_structure() { registry.identity_verified=false;registry.error=Some(error); }
                else if let Some(active)=&registry.file.active {
                    if let Err(error)=validate_acquisition_record(&active.parameters,false) {registry.error=Some(error);}
                }
            }
            Ok(_) => {registry.identity_verified=false;registry.error = Some("Unsupported active-task registry schema".into());},
            Err(error) => {registry.identity_verified=false;registry.error = Some(format!("Active-task registry cannot be verified: {error}"));},
        }
        registry
    }

    fn validate_structure(&self) -> Result<(), String> {
        for record in self.file.active.iter().chain(self.file.retained.iter()) {
            record.parameters.validate().map_err(str::to_owned)?;
            if !matches!(record.status.as_str(), "unfinished" | "completed" | "abandoned")
                || record.registered_at.is_empty() || record.updated_at.is_empty() {
                return Err("Invalid persistent task record".into());
            }
        }
        if self.file.retained.iter().any(|record|record.status=="unfinished") {return Err("Retained register contains an unclosed task".into());}
        if let Some(active) = &self.file.active {
            if active.status != "unfinished" { return Err("Active task has an invalid terminal state".into()); }
        }
        Ok(())
    }

    pub fn locked(&self) -> bool { self.error.is_some() || self.file.active.is_some() }
    pub fn can_abandon(&self) -> bool {self.identity_verified && self.file.active.is_some()}
    pub fn error(&self) -> Option<&str> { self.error.as_deref() }
    pub fn matches(&self, parameters: &Parameters) -> bool {
        self.error.is_none() && self.file.active.as_ref().is_some_and(|record| record.parameters == *parameters)
    }
    pub fn restore_parameters(&self) -> Option<Parameters> { self.file.active.as_ref().map(|record| record.parameters.clone()) }
    pub fn check_manual(&self) -> Result<(), String> {
        if let Some(error) = &self.error { return Err(error.clone()); }
        if self.file.active.is_some() { return Err("UNFINISHED_TASK_LOCKED: resume or explicitly abandon the retained task".into()); }
        Ok(())
    }
    pub fn snapshot(&self) -> Value {
        json!({"locked":self.locked(),"reason":self.error.clone().or_else(|| self.file.active.as_ref().map(|_| "Unfinished scan: resume or abandon before manual parameters, focus or motion".into())),
            "taskId":self.file.active.as_ref().map(|record| &record.parameters.task_id),
            "savePath":self.file.active.as_ref().map(|record| &record.parameters.save_path),
            "status":if self.error.is_some(){"invalid"}else if self.file.active.is_some(){"unfinished"}else{"unlocked"},
            "retained":self.file.retained})
    }

    pub fn check_target_available(&self, parameters: &Parameters) -> Result<(), String> {
        parameters.validate().map_err(str::to_owned)?;
        if let Some(error) = &self.error { return Err(error.clone()); }
        if self.file.active.is_some() && !self.matches(parameters) {
            return Err("UNFINISHED_TASK_LOCKED: changing task ID or directory cannot release the lock".into());
        }
        if self.file.retained.iter().any(|record| same_location(&record.parameters, parameters)) {
            return Err("SCAN_OUTPUT_RETAINED: use a new task directory".into());
        }
        let root = acquisition_root(parameters);
        if !self.matches(parameters) && root.exists() && fs::read_dir(&root).map_err(|error| error.to_string())?.next().is_some() {
            return Err("SCAN_OUTPUT_EXISTS: retained data cannot be overwritten".into());
        }
        Ok(())
    }

    /// Persist BEFORE creating an acquisition checkpoint or starting a worker.
    /// A crash between registration and file creation is deliberately locked.
    pub fn register(&mut self, parameters: &Parameters) -> Result<(), String> {
        self.check_target_available(parameters)?;
        if self.matches(parameters) { return Ok(()); }
        let now = timestamp();
        let record = TaskRecord { parameters: parameters.clone(), status: "unfinished".into(), registered_at: now.clone(), updated_at: now };
        let mut next = self.file.clone();
        next.active = Some(record);
        self.persist(next)
    }

    /// The caller must have independently confirmed capture and device cleanup.
    pub fn mark_completed(&mut self, parameters: &Parameters) -> Result<(), String> {
        if !self.matches(parameters) { return Err("Completion does not match the registered active task".into()); }
        validate_acquisition_record(parameters, true)?;
        self.mark_terminal("completed")
    }

    /// Call only after worker cleanup and required beam-OFF confirmation. Files
    /// remain byte-for-byte unchanged and this location cannot be used again.
    pub fn abandon(&mut self) -> Result<(), String> {
        // A verified task identity can be explicitly abandoned after domain
        // cleanup even when its manifest is missing/corrupt. An untrusted
        // register cannot identify what would be abandoned and stays closed.
        if !self.identity_verified {return Err(self.error.clone().unwrap_or("Task identity is unverified".into()));}
        if self.file.active.is_none() { return Err("NO_ACTIVE_TASK".into()); }
        self.mark_terminal("abandoned")
    }

    fn mark_terminal(&mut self, status: &str) -> Result<(), String> {
        let mut next = self.file.clone();
        let mut record = next.active.take().ok_or("NO_ACTIVE_TASK")?;
        record.status = status.into();
        record.updated_at = timestamp();
        next.retained.push(record);
        self.persist(next)
    }

    fn persist(&mut self, next: RegisterFile) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&next).map_err(|error| error.to_string())?;
        atomic_store(&self.path, &bytes)?;
        self.file = next;
        self.error=None;
        Ok(())
    }
}

fn same_location(first: &Parameters, second: &Parameters) -> bool {
    // Windows file names are case insensitive. An existing directory is also
    // canonicalized so aliases cannot escape the retained-location check.
    let normalize = |parameters: &Parameters| {
        let path = acquisition_root(parameters);
        fs::canonicalize(&path).unwrap_or(path).to_string_lossy().replace('\\', "/").to_lowercase()
    };
    normalize(first) == normalize(second)
}

fn acquisition_root(parameters: &Parameters) -> PathBuf { Path::new(parameters.save_path.trim()).join(parameters.task_id.trim()) }
pub(crate) fn lexical_path(path:&Path)->String{
    let path=if path.is_absolute(){path.to_path_buf()}else{std::env::current_dir().unwrap_or_default().join(path)};
    let mut normalized=PathBuf::new();for part in path.components(){match part{std::path::Component::ParentDir=>{normalized.pop();},std::path::Component::CurDir=>{},_=>normalized.push(part.as_os_str())}}
    format!("{}/",normalized.to_string_lossy().replace('\\',"/").trim_end_matches('/').to_lowercase())
}

fn validate_acquisition_record(parameters: &Parameters, completed: bool) -> Result<(), String> {
    let root = acquisition_root(parameters);
    let manifest = root.join("manifest.json");
    let checkpoint = root.join("preparation-checkpoint.json");
    let path = if manifest.exists() { manifest } else if checkpoint.exists() { checkpoint }
        else { return Err("Registered acquisition evidence is missing; manual control remains locked".into()); };
    let data: Value = serde_json::from_reader(File::open(path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("Registered acquisition evidence is corrupt: {error}"))?;
    if !matches!(data["schemaVersion"].as_u64(), Some(2 | 3))
        || data["scanId"].as_str() != Some(parameters.task_id.as_str())
        || data["projectionCount"].as_u64() != Some(u64::from(parameters.projection_count))
        || data["exposureMs"].as_f64() != Some(parameters.exposure_ms) {
        return Err("Registered acquisition evidence does not match the active task".into());
    }
    if data["schemaVersion"].as_u64()==Some(3) {
        let profile:crate::capture_profile::CaptureProfile=serde_json::from_value(data["captureProfile"].clone()).map_err(|error|format!("Registered capture profile is invalid: {error}"))?;
        profile.validate()?;
        if profile.exposure_ms()!=Some(parameters.exposure_ms)
            || !data["maxXraySec"].as_u64().is_some_and(|maximum|(1..=600).contains(&maximum))
            || data["voltageKv"].as_f64().is_none_or(|value|!value.is_finite())
            || data["currentUa"].as_f64().is_none_or(|value|!value.is_finite()) {
            return Err("Registered acquisition has incomplete frozen safety or capture parameters".into());
        }
    }
    if completed && (data["completed"].as_bool() != Some(true)
        || data["stage"].as_str() != Some("completed")
        || data["frames"].as_array().map(Vec::len) != Some(parameters.projection_count as usize)
        || data["references"]["preDark"].as_array().map(Vec::len) != Some(10)
        || data["references"]["preFlat"].as_array().map(Vec::len) != Some(10)) {
        return Err("Completion is missing sealed, committed acquisition evidence".into());
    }
    Ok(())
}

pub(crate) fn atomic_store(path: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_store_cancelled(path,bytes,&AtomicBool::new(false))
}
pub(crate) fn atomic_store_cancelled(path:&Path,bytes:&[u8],cancel:&AtomicBool)->Result<(),String>{
    if cancel.load(Ordering::SeqCst){return Err("Persistent write cancelled".into());}
    let parent = path.parent().ok_or("Storage path has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let suffix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), suffix));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|error| error.to_string())?;
        file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|error| error.to_string())?;
        drop(file);
        if cancel.load(Ordering::SeqCst){return Err("Persistent write cancelled before commit".into());}
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(topic: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine")
            .join(format!("{}-task-register-{topic}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        fs::create_dir_all(&root).unwrap(); root
    }
    fn parameters(root: &Path) -> Parameters { Parameters { task_id:"scan-a".into(),save_path:root.to_string_lossy().into(),projection_count:2,angle_step_deg:180.0,exposure_ms:200.0 } }
    fn checkpoint(parameters: &Parameters) {
        let root = acquisition_root(parameters); fs::create_dir_all(&root).unwrap();
        fs::write(root.join("preparation-checkpoint.json"), serde_json::to_vec(&json!({"schemaVersion":3,"scanId":parameters.task_id,"projectionCount":2,"exposureMs":200.0,"stage":"geometry","captureProfile":crate::capture_profile::test_profile(),"maxXraySec":300,"voltageKv":20.0,"currentUa":100.0})).unwrap()).unwrap();
    }
    #[test]
    fn unfinished_lock_survives_restart_and_task_id_change() {
        let root = root("restart"); let path = root.join("active-task.json"); let parameters = parameters(&root);
        let mut registry = TaskRegistry::load(&path); registry.register(&parameters).unwrap(); checkpoint(&parameters);
        let restored = TaskRegistry::load(&path); assert!(restored.locked()); assert!(restored.matches(&parameters));
        let mut changed = parameters.clone(); changed.task_id = "scan-b".into();
        assert!(restored.check_target_available(&changed).is_err()); assert!(restored.check_manual().is_err());
    }
    #[test]
    fn abandon_persists_unlock_keeps_files_and_prohibits_reuse() {
        let root = root("abandon"); let path = root.join("active-task.json"); let parameters = parameters(&root);
        let mut registry = TaskRegistry::load(&path); registry.register(&parameters).unwrap(); checkpoint(&parameters);
        let file = acquisition_root(&parameters).join("original.nef"); fs::write(&file,b"retained original").unwrap();
        registry.abandon().unwrap(); let restored = TaskRegistry::load(&path);
        assert!(!restored.locked()); assert_eq!(fs::read(file).unwrap(),b"retained original"); assert!(restored.check_target_available(&parameters).is_err());
        assert_eq!(restored.snapshot()["retained"][0]["status"],"abandoned");
    }
    #[test]
    fn corrupt_register_and_missing_or_corrupt_manifest_stay_locked() {
        let root = root("corrupt"); let path = root.join("active-task.json"); fs::write(&path,b"broken").unwrap();
        assert!(TaskRegistry::load(&path).locked()); fs::remove_file(&path).unwrap();
        let parameters = parameters(&root); let mut registry = TaskRegistry::load(&path); registry.register(&parameters).unwrap();
        assert!(TaskRegistry::load(&path).error().is_some()); checkpoint(&parameters);
        fs::write(acquisition_root(&parameters).join("manifest.json"),b"truncated").unwrap();
        assert!(TaskRegistry::load(&path).error().is_some());
    }
    #[test]
    fn verified_task_can_be_abandoned_after_missing_evidence_but_corrupt_identity_cannot() {
        let root=root("missing-evidence");let path=root.join("active-task.json");let parameters=parameters(&root);
        let mut registry=TaskRegistry::load(&path);registry.register(&parameters).unwrap();
        let mut restored=TaskRegistry::load(&path);assert!(restored.locked());assert!(restored.check_manual().is_err());
        restored.abandon().unwrap();assert!(!TaskRegistry::load(&path).locked());
        fs::write(&path,b"invalid identity").unwrap();assert!(TaskRegistry::load(&path).abandon().is_err());
    }

    #[test]
    fn cancelled_and_stale_plans_keep_the_reserved_lock_and_never_overwrite_newer_state(){
        let root=root("cancelled-plan");let path=root.join("active-task.json");let parameters=parameters(&root);let mut registry=TaskRegistry::load(&path);
        let old=registry.prepare_register(&parameters).unwrap();assert!(registry.locked());
        assert!(old.execute(&AtomicBool::new(true)).is_err());assert!(!path.exists());
        let abandon=registry.prepare_abandon().unwrap();abandon.execute(&AtomicBool::new(false)).unwrap();registry.commit(abandon).unwrap();
        assert!(!registry.locked());assert!(old.execute(&AtomicBool::new(false)).is_err());assert!(!TaskRegistry::load(&path).locked());
    }
}
