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

#[derive(Clone)]
pub struct TaskRegistry {
    path: PathBuf,
    file: RegisterFile,
    error: Option<String>,
    identity_verified: bool,
    pending_registration: Option<RegisterFile>,
}

/// Versioned registry status for a safely finished card acquisition whose data is
/// still on the camera card. The record stays retained (its directory cannot be
/// reused) while no scan hardware remains occupied, and it is never treated as a
/// completed or reconstructable project until the USB import is verified.
pub const TASK_STATUS_ACQUIRED_PENDING_IMPORT: &str = "acquired_pending_import";

pub struct TaskWritePlan {path:PathBuf,expected:RegisterFile,next:RegisterFile,register:Option<Parameters>,completed:Option<Parameters>,unpersisted:Option<RegisterFile>,deleted:Vec<Parameters>}impl TaskWritePlan {
    pub fn deleted_count(&self)->usize{self.deleted.len()}
    pub fn active_deleted(&self)->bool{self.expected.active.is_some()&&self.next.active.is_none()}
    pub fn execute(&self,cancel:&AtomicBool)->Result<(),String>{
        if cancel.load(Ordering::SeqCst){return Err("Task persistence cancelled".into());}
        for parameters in &self.deleted {
            if !directory_confirmed_deleted(parameters)?{return Err("SCAN_OUTPUT_UNVERIFIED: task directory reappeared during reconciliation".into());}
        }
        if let Some(parameters)=self.register.as_ref(){
            if self.expected.retained.iter().any(|record|same_location(&record.parameters,parameters))&&!self.deleted.iter().any(|old|same_location(old,parameters)){return Err("SCAN_OUTPUT_RETAINED".into());}
            let root=acquisition_root(parameters);let same=self.expected.active.as_ref().is_some_and(|record|record.parameters==*parameters);
            if !same&&root.exists()&&fs::read_dir(root).map_err(|error|error.to_string())?.next().is_some(){return Err("SCAN_OUTPUT_EXISTS".into());}
        }
        if let Some(parameters)=self.completed.as_ref(){validate_acquisition_record(parameters,true)?;}
        if self.path.exists(){let current:RegisterFile=serde_json::from_reader(File::open(&self.path).map_err(|error|error.to_string())?).map_err(|error|error.to_string())?;
            let bytes=serde_json::to_vec(&current).map_err(|error|error.to_string())?;
            let expected=bytes==serde_json::to_vec(&self.expected).map_err(|error|error.to_string())?;
            let unpersisted=self.unpersisted.as_ref().is_some_and(|old|serde_json::to_vec(old).is_ok_and(|old|old==bytes));
            if !expected&&!unpersisted{return Err("Stale task persistence plan cannot overwrite a newer registration".into());}}
        atomic_store_cancelled(&self.path,&serde_json::to_vec_pretty(&self.next).map_err(|error|error.to_string())?,cancel)
    }
}

impl TaskRegistry {
    pub fn capture_directory_conflicts(&self,directory:&Path)->bool{self.file.active.iter().chain(self.file.retained.iter()).any(|record|lexical_path(directory).starts_with(&lexical_path(&acquisition_root(&record.parameters))))}
    pub fn capture_roots(&self)->Vec<PathBuf>{self.file.active.iter().chain(self.file.retained.iter()).map(|record|acquisition_root(&record.parameters)).collect()}
    #[cfg(test)]
    pub fn retained_location(&self,parameters:&Parameters)->bool {
        self.check_retained_location(parameters).unwrap_or(true)
    }
    pub fn check_retained_location(&self,parameters:&Parameters)->Result<bool,String>{
        if !self.file.retained.iter().any(|record|same_location(&record.parameters,parameters)){return Ok(false);}
        directory_confirmed_deleted(parameters).map(|deleted|!deleted)
    }
    pub fn prepare_register(&mut self,parameters:&Parameters)->Result<TaskWritePlan,String>{
        self.check_target_available(parameters)?;
        let expected=self.file.clone();let mut next=expected.clone();
        let deleted=expected.retained.iter().filter(|record|same_location(&record.parameters,parameters)).map(|record|record.parameters.clone()).collect::<Vec<_>>();
        next.retained.retain(|record|!same_location(&record.parameters,parameters));
        if !self.matches(parameters){let now=timestamp();next.active=Some(TaskRecord{parameters:parameters.clone(),status:"unfinished".into(),registered_at:now.clone(),updated_at:now});self.pending_registration=Some(expected.clone());}
        // Reserve the sole in-memory lock before a background write can start.
        self.file=next.clone();
        Ok(TaskWritePlan{path:self.path.clone(),expected,next,register:Some(parameters.clone()),completed:None,unpersisted:None,deleted})
    }
    pub fn prepare_completed(&self,parameters:&Parameters)->Result<TaskWritePlan,String>{
        if !self.matches(parameters){return Err("Task completion identity mismatch".into());}
        self.prepare_terminal("completed",Some(parameters.clone()))
    }
    /// Only a failed new start whose durable registration never happened can
    /// release its reservation. A persisted or externally changed record stays.
    pub fn discard_unpersisted_registration(&mut self,parameters:&Parameters)->Result<bool,String>{
        let Some(previous)=self.pending_registration.as_ref()else{return Ok(false);};
        if !self.matches(parameters){return Ok(false);}
        let unchanged=match File::open(&self.path){
            Ok(file)=>{
                let current:RegisterFile=serde_json::from_reader(file).map_err(|error|error.to_string())?;
                serde_json::to_vec(&current).map_err(|error|error.to_string())?==serde_json::to_vec(previous).map_err(|error|error.to_string())?
            }
            Err(error)if error.kind()==std::io::ErrorKind::NotFound=>previous.active.is_none()&&previous.retained.is_empty(),
            Err(error)=>return Err(error.to_string()),
        };
        if !unchanged{return Ok(false);}
        self.file=self.pending_registration.take().unwrap();Ok(true)
    }
    pub fn prepare_abandon(&self)->Result<TaskWritePlan,String>{
        if !self.can_abandon(){return Err("Task identity is unverified".into());}
        self.prepare_terminal("abandoned",None)
    }
    /// Safe finishing of a card acquisition whose data is still on the camera card.
    /// The task leaves the active slot (so scan hardware is no longer occupied and
    /// manual controls unlock) but stays retained with its directory reserved, and
    /// it is never reconstructable until the import is verified.
    pub fn mark_acquired_pending_import(&mut self, parameters: &Parameters) -> Result<(), String> {
        if !self.matches(parameters) {
            return Err("Card acquisition completion does not match the registered active task".into());
        }
        validate_acquired_card_record(parameters)?;
        self.mark_terminal(TASK_STATUS_ACQUIRED_PENDING_IMPORT)
    }
    /// The authorized USB import verified every card object: the retained task is
    /// sealed as completed. The manifest itself must already prove the verified
    /// state, so a caller cannot promote a pending import.
    pub fn mark_import_verified(&mut self, parameters: &Parameters) -> Result<(), String> {
        validate_verified_card_record(parameters)?;
        let mut next = self.file.clone();
        let record = next
            .retained
            .iter_mut()
            .find(|record| {
                record.status == TASK_STATUS_ACQUIRED_PENDING_IMPORT
                    && same_location(&record.parameters, parameters)
            })
            .ok_or("No retained acquisition is waiting for its import")?;
        record.status = "completed".into();
        record.updated_at = timestamp();
        self.persist(next)
    }
    /// Retained tasks whose card data still has to be imported, in registry order.
    pub fn pending_import_tasks(&self) -> Vec<&TaskRecord> {
        self.file
            .retained
            .iter()
            .filter(|record| record.status == TASK_STATUS_ACQUIRED_PENDING_IMPORT)
            .collect()
    }
    fn prepare_terminal(&self,status:&str,completed:Option<Parameters>)->Result<TaskWritePlan,String>{
        let expected=self.file.clone();let mut next=expected.clone();let mut record=next.active.take().ok_or("NO_ACTIVE_TASK")?;record.status=status.into();record.updated_at=timestamp();next.retained.push(record);
        // Stop may arrive before a reserved registration's background write.
        // Only its exact frozen predecessor is an allowed alternative, never
        // an arbitrary registry or a newer task written by another owner.
        let unpersisted=if status=="abandoned"{self.pending_registration.clone()}else{None};
        Ok(TaskWritePlan{path:self.path.clone(),expected,next,register:None,completed,unpersisted,deleted:vec![]})
    }
    pub fn needs_directory_reconciliation(&self)->bool {
        self.identity_verified&&(self.file.active.is_some()||!self.file.retained.is_empty())
    }
    /// Runs on a background copy, never while the owner has acquisition work.
    /// Missing metadata inside an existing directory is not proof of deletion.
    pub fn prepare_deleted_directories(&self)->Option<TaskWritePlan> {
        if !self.needs_directory_reconciliation(){return None;}
        let expected=self.file.clone();let mut next=expected.clone();let mut deleted=vec![];
        next.retained.retain(|record|{
            if directory_confirmed_deleted(&record.parameters)==Ok(true){deleted.push(record.parameters.clone());false}else{true}
        });
        if let Some(record)=next.active.as_ref(){
            if directory_confirmed_deleted(&record.parameters)==Ok(true){deleted.push(record.parameters.clone());next.active=None;}
        }
        if deleted.is_empty(){return None;}
        Some(TaskWritePlan{path:self.path.clone(),expected,next,register:None,completed:None,unpersisted:self.pending_registration.clone(),deleted})
    }
    pub fn commit(&mut self,plan:TaskWritePlan)->Result<(),String>{
        let expected=if plan.register.is_some(){&plan.next}else{&plan.expected};
        if serde_json::to_vec(&self.file).map_err(|error|error.to_string())?!=serde_json::to_vec(expected).map_err(|error|error.to_string())?{return Err("Stale task persistence completion".into());}
        let preserve_error=!plan.deleted.is_empty()&&plan.next.active.is_some();
        self.file=plan.next;if !preserve_error{self.error=None;}self.pending_registration=None;Ok(())
    }
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut registry = Self { path, file: RegisterFile { schema_version: 1, active: None, retained: Vec::new() }, error: None, identity_verified: true, pending_registration: None };
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
            if !matches!(record.status.as_str(), "unfinished" | "completed" | "abandoned" | TASK_STATUS_ACQUIRED_PENDING_IMPORT)
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
        if self.check_retained_location(parameters)? {
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
        let plan=self.prepare_register(parameters)?;
        plan.execute(&AtomicBool::new(false))?;self.commit(plan)
    }

    /// The caller must have independently confirmed capture and device cleanup.
    pub fn mark_completed(&mut self, parameters: &Parameters) -> Result<(), String> {
        if !self.matches(parameters) { return Err("Completion does not match the registered active task".into()); }
        validate_acquisition_record(parameters, true)?;
        self.mark_terminal("completed")
    }

    /// Call only after worker cleanup and required beam-OFF confirmation. Files
    /// remain byte-for-byte unchanged; reuse remains blocked while they exist.
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

fn directory_confirmed_deleted(parameters:&Parameters)->Result<bool,String> {
    // Path::exists() also returns false for access errors and disconnected
    // drives. Require the configured parent to be accessible, and distinguish
    // a genuinely absent entry from inaccessible/dangling symlink evidence.
    parameters.validate().map_err(str::to_owned)?;
    let parent=Path::new(parameters.save_path.trim());
    if !parent.is_absolute(){return Err("SCAN_OUTPUT_UNVERIFIED: capture root is not absolute".into());}
    let mut entries=fs::read_dir(parent).map_err(|error|format!("SCAN_OUTPUT_UNVERIFIED: capture root cannot be read: {error}"))?;
    if let Some(entry)=entries.next(){entry.map_err(|error|format!("SCAN_OUTPUT_UNVERIFIED: capture root cannot be enumerated: {error}"))?;}
    match fs::symlink_metadata(acquisition_root(parameters)) {
        Ok(_)=>Ok(false),
        Err(error)if error.kind()==std::io::ErrorKind::NotFound=>Ok(true),
        Err(error)=>Err(format!("SCAN_OUTPUT_UNVERIFIED: task directory cannot be checked: {error}")),
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
    let data = registered_record_json(parameters)?;
    if data["schemaVersion"].as_u64()==Some(3) || data["schemaVersion"].as_u64()==Some(4) {
        let profile:crate::capture_profile::CaptureProfile=serde_json::from_value(data["captureProfile"].clone()).map_err(|error|format!("Registered capture profile is invalid: {error}"))?;
        profile.validate()?;
        if profile.exposure_ms()!=Some(parameters.exposure_ms)
            || !data["maxXraySec"].as_u64().is_some_and(|maximum|(1..=u64::from(crate::scan::SCAN_MAX_XRAY_SEC)).contains(&maximum))
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

/// Loads the registered manifest/checkpoint and checks that it belongs to this
/// task. Schema 4 is accepted here so a card acquisition can be registered; the
/// status-specific rules live in the callers below.
fn registered_record_json(parameters: &Parameters) -> Result<Value, String> {
    let root = acquisition_root(parameters);
    let manifest = root.join("manifest.json");
    let checkpoint = root.join("preparation-checkpoint.json");
    let path = if manifest.exists() { manifest } else if checkpoint.exists() { checkpoint }
        else { return Err("Registered acquisition evidence is missing; manual control remains locked".into()); };
    let data: Value = serde_json::from_reader(File::open(path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("Registered acquisition evidence is corrupt: {error}"))?;
    if !matches!(data["schemaVersion"].as_u64(), Some(2 | 3 | 4))
        || data["scanId"].as_str() != Some(parameters.task_id.as_str())
        || data["projectionCount"].as_u64() != Some(u64::from(parameters.projection_count))
        || data["exposureMs"].as_f64() != Some(parameters.exposure_ms) {
        return Err("Registered acquisition evidence does not match the active task".into());
    }
    Ok(data)
}

/// Safe finishing evidence for a card task whose import is still open: schema 4,
/// card destination and card profile, `acquired`, an open data state, one record
/// per projection with real card provenance, and no local frames yet.
fn validate_acquired_card_record(parameters: &Parameters) -> Result<(), String> {
    let data = registered_record_json(parameters)?;
    if data["schemaVersion"].as_u64() != Some(4) {
        return Err("A card acquisition must be registered as schema 4".into());
    }
    let destination = serde_json::from_value::<crate::capture_profile::CaptureDestination>(
        data["captureDestination"].clone(),
    )
    .map_err(|_| "Registered card acquisition has no card destination")?;
    if destination != crate::capture_profile::CaptureDestination::CameraCard {
        return Err("Registered card acquisition is not a card destination".into());
    }
    let profile = serde_json::from_value::<crate::capture_profile::CaptureProfile>(
        data["captureProfile"].clone(),
    )
    .map_err(|error| format!("Registered capture profile is invalid: {error}"))?;
    if !profile.is_card_destination() {
        return Err("Registered card acquisition lacks the frozen card profile".into());
    }
    if data["acquisitionStatus"].as_str() != Some(crate::scan::ACQUISITION_ACQUIRED) {
        return Err("Card acquisition is not safely finished".into());
    }
    if !matches!(
        data["dataStatus"].as_str(),
        Some(crate::scan::DATA_PENDING_IMPORT) | Some(crate::scan::DATA_IMPORTING)
    ) {
        return Err("Card acquisition data is not open for import".into());
    }
    if data["frames"].as_array().is_some_and(|frames| !frames.is_empty()) {
        return Err("A card acquisition waiting for its import cannot hold local frames".into());
    }
    let records: Vec<crate::scan::CardCaptureRecord> =
        serde_json::from_value(data["captureRecords"].clone())
            .map_err(|error| format!("Registered card records are invalid: {error}"))?;
    if records.iter().filter(|record| record.kind == "projection").count()
        != parameters.projection_count as usize
    {
        return Err("Registered card records do not cover every projection".into());
    }
    if records.iter().any(|record| {
        !crate::scan::card_data_status_valid(&record.state)
            || record.storage_id == 0
            || record.handle == 0
            || record.source_file_name.trim().is_empty()
            || record.camera_session_id.trim().is_empty()
            || record.operation_id.trim().is_empty()
            || record.imported_file.is_some()
    }) {
        return Err("Registered card records lack their acquisition provenance".into());
    }
    Ok(())
}

/// Verified-import evidence: the schema-4 manifest must already prove the sealed
/// import before a retained task may become completed.
fn validate_verified_card_record(parameters: &Parameters) -> Result<(), String> {
    let data = registered_record_json(parameters)?;
    if data["schemaVersion"].as_u64() != Some(4)
        || data["captureDestination"].as_str() != Some("cameraCard")
        || data["acquisitionStatus"].as_str() != Some(crate::scan::ACQUISITION_ACQUIRED)
        || data["dataStatus"].as_str() != Some(crate::scan::DATA_VERIFIED)
        || data["completed"].as_bool() != Some(true)
        || data["stage"].as_str() != Some("completed")
        || data["frames"].as_array().map(Vec::len) != Some(parameters.projection_count as usize)
        || data["references"]["preDark"].as_array().map(Vec::len) != Some(10)
        || data["references"]["preFlat"].as_array().map(Vec::len) != Some(10)
    {
        return Err("Import verification is missing sealed, committed card evidence".into());
    }
    let profile = serde_json::from_value::<crate::capture_profile::CaptureProfile>(
        data["captureProfile"].clone(),
    )
    .map_err(|error| format!("Registered capture profile is invalid: {error}"))?;
    if !profile.is_card_destination() {
        return Err("Verified card acquisition lacks the frozen card profile".into());
    }
    let records: Vec<crate::scan::CardCaptureRecord> =
        serde_json::from_value(data["captureRecords"].clone())
            .map_err(|error| format!("Registered card records are invalid: {error}"))?;
    if records.iter().filter(|record| record.kind == "projection").count()
        != parameters.projection_count as usize
        || !records.iter().all(crate::scan::card_record_is_reconstructable)
    {
        return Err("Verified card records are not all reconstructable".into());
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine")
            .join(format!("{}-task-register-{topic}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        fs::create_dir_all(&root).unwrap(); root
    }
    fn parameters(root: &Path) -> Parameters { Parameters { task_id:"scan-a".into(),save_path:root.to_string_lossy().into(),projection_count:2,angle_step_deg:180.0,exposure_ms:200.0 } }
    fn checkpoint(parameters: &Parameters) {
        let root = acquisition_root(parameters); fs::create_dir_all(&root).unwrap();
        fs::write(root.join("preparation-checkpoint.json"), serde_json::to_vec(&json!({"schemaVersion":3,"scanId":parameters.task_id,"projectionCount":2,"exposureMs":200.0,"stage":"geometry","captureProfile":crate::capture_profile::test_profile(),"maxXraySec":300,"voltageKv":20.0,"currentUa":100.0})).unwrap()).unwrap();
    }
    fn delete_owned_task(parameters:&Parameters){
        let directory=acquisition_root(parameters);let parent=Path::new(&parameters.save_path).canonicalize().unwrap();
        assert!(directory.canonicalize().unwrap().starts_with(parent));fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn deleted_retained_directory_reuses_same_id_and_persists_new_ownership(){
        let root=root("deleted-retained");let path=root.join("active-task.json");let parameters=parameters(&root);
        let mut registry=TaskRegistry::load(&path);registry.register(&parameters).unwrap();checkpoint(&parameters);registry.abandon().unwrap();
        assert!(registry.retained_location(&parameters));delete_owned_task(&parameters);
        assert!(!registry.retained_location(&parameters));registry.register(&parameters).unwrap();checkpoint(&parameters);
        let restored=TaskRegistry::load(&path);assert!(restored.matches(&parameters));assert_eq!(restored.snapshot()["retained"],json!([]));
    }
    #[test]
    fn deleted_active_directory_reconciliation_unlocks_across_restart(){
        let root=root("deleted-active");let path=root.join("active-task.json");let parameters=parameters(&root);
        let mut registry=TaskRegistry::load(&path);registry.register(&parameters).unwrap();checkpoint(&parameters);delete_owned_task(&parameters);
        let mut restored=TaskRegistry::load(&path);assert!(restored.locked());
        let plan=restored.prepare_deleted_directories().unwrap();assert!(plan.active_deleted());plan.execute(&AtomicBool::new(false)).unwrap();restored.commit(plan).unwrap();
        assert!(!restored.locked());assert!(!TaskRegistry::load(&path).locked());
    }
    #[test]
    fn unreadable_parent_and_existing_directory_without_metadata_keep_ownership(){
        let root=root("unavailable-parent");let path=root.join("active-task.json");let photo=root.join("photos");fs::create_dir(&photo).unwrap();let parameters=parameters(&photo);
        let mut registry=TaskRegistry::load(&path);registry.register(&parameters).unwrap();checkpoint(&parameters);registry.abandon().unwrap();
        fs::remove_file(acquisition_root(&parameters).join("preparation-checkpoint.json")).unwrap();
        assert!(registry.prepare_deleted_directories().is_none());assert!(registry.retained_location(&parameters));
        fs::rename(&photo,root.join("disconnected-photo-root")).unwrap();
        let before=fs::read(&path).unwrap();assert!(registry.prepare_deleted_directories().is_none());assert!(registry.retained_location(&parameters));assert!(registry.prepare_register(&parameters).is_err());assert_eq!(fs::read(&path).unwrap(),before);
    }
    #[test]
    fn directory_reappearing_before_reconciliation_cannot_release_ownership(){
        let root=root("reappeared");let path=root.join("active-task.json");let parameters=parameters(&root);
        let mut registry=TaskRegistry::load(&path);registry.register(&parameters).unwrap();checkpoint(&parameters);registry.abandon().unwrap();delete_owned_task(&parameters);
        let plan=registry.prepare_deleted_directories().unwrap();let before=fs::read(&path).unwrap();fs::create_dir(acquisition_root(&parameters)).unwrap();let original=acquisition_root(&parameters).join("original.nef");fs::write(&original,b"returned original").unwrap();
        assert!(plan.execute(&AtomicBool::new(false)).unwrap_err().contains("reappeared"));assert_eq!(fs::read(&path).unwrap(),before);assert_eq!(fs::read(original).unwrap(),b"returned original");
    }
    #[test]
    fn stale_reconciliation_cannot_overwrite_a_new_registry(){
        let root=root("stale-reconciliation");let path=root.join("active-task.json");let parameters=parameters(&root);
        let mut registry=TaskRegistry::load(&path);registry.register(&parameters).unwrap();checkpoint(&parameters);registry.abandon().unwrap();delete_owned_task(&parameters);let plan=registry.prepare_deleted_directories().unwrap();
        let mut newer=TaskRegistry::load(&path);let mut new_parameters=parameters.clone();new_parameters.task_id="new-owner".into();newer.register(&new_parameters).unwrap();let before=fs::read(&path).unwrap();
        assert!(plan.execute(&AtomicBool::new(false)).is_err());assert_eq!(fs::read(path).unwrap(),before);
    }
    #[test]
    fn failed_start_releases_only_an_unpersisted_reservation(){
        let root=root("reservation-conflict");let path=root.join("active-task.json");let parameters=parameters(&root);
        let mut registry=TaskRegistry::load(&path);let plan=registry.prepare_register(&parameters).unwrap();fs::create_dir(acquisition_root(&parameters)).unwrap();let original=acquisition_root(&parameters).join("original.nef");fs::write(&original,b"external data").unwrap();
        assert!(plan.execute(&AtomicBool::new(false)).unwrap_err().contains("SCAN_OUTPUT_EXISTS"));assert!(registry.discard_unpersisted_registration(&parameters).unwrap());assert!(!registry.locked());assert!(!path.exists());assert_eq!(fs::read(&original).unwrap(),b"external data");
        fs::remove_file(original).unwrap();let plan=registry.prepare_register(&parameters).unwrap();plan.execute(&AtomicBool::new(false)).unwrap();
        assert!(!registry.discard_unpersisted_registration(&parameters).unwrap());assert!(registry.locked());
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

    fn card_record_json(index: u32, kind: &str, verified: bool) -> Value {
        let mut record = json!({
            "index": index, "kind": kind, "angleDeg": -(index as f64), "exposureMs": 200.0,
            "state": if verified { crate::scan::DATA_VERIFIED } else { crate::scan::DATA_PENDING_IMPORT },
            "cameraSessionId": "session-card", "operationId": format!("op-{index}"),
            "storageId": 65537u32, "storageChanged": false, "handle": 256u32 + index,
            "parent": 1u32, "sourceFileName": format!("DSC_{index:04}.NEF"),
            "format": 12288u32, "size": 40_000_000u64, "captureDate": "20261010T120000"
        });
        if verified {
            record["importedFile"] = json!({"index": index, "angleDeg": -(index as f64), "exposureMs": 200.0,
                "fileName": format!("frame-{index:04}.nef"), "path": format!("/data/frame-{index:04}.nef"),
                "bytes": 40_000_000u64, "sha256": "aa".repeat(32)});
        }
        record
    }

    /// Writes a schema-4 card manifest: `verified` selects a sealed import with
    /// local frames, otherwise a safely finished acquisition whose data is open.
    fn card_manifest(parameters: &Parameters, data_status: &str, verified: bool) {
        let root = acquisition_root(parameters);
        fs::create_dir_all(&root).unwrap();
        let frame = |index: u32| json!({"index": index, "angleDeg": -(index as f64), "exposureMs": 200.0,
            "fileName": format!("frame-{index:04}.nef"), "path": format!("/data/frame-{index:04}.nef"),
            "bytes": 40_000_000u64, "sha256": "aa".repeat(32)});
        let reference = |index: u32| json!({"index": index, "exposureMs": 200.0,
            "fileName": format!("ref-{index:04}.nef"), "path": format!("/data/ref-{index:04}.nef"),
            "bytes": 40_000_000u64, "sha256": "bb".repeat(32)});
        let records: Vec<Value> = (1..=parameters.projection_count)
            .map(|index| card_record_json(index, "projection", verified))
            .chain((1..=2u32).map(|index| card_record_json(index, "preDark", verified)))
            .collect();
        let manifest = json!({
            "schemaVersion": 4, "scanId": parameters.task_id,
            "projectionCount": parameters.projection_count, "exposureMs": parameters.exposure_ms,
            "captureProfile": crate::capture_profile::test_card_profile("single"),
            "maxXraySec": 1000, "voltageKv": 20.0, "currentUa": 100.0,
            "captureDestination": "cameraCard", "acquisitionStatus": crate::scan::ACQUISITION_ACQUIRED,
            "dataStatus": data_status,
            "stage": if verified { "completed" } else { "projections" },
            "completed": verified,
            "geometry": {"sodMm":40.0,"objectToDetectorMm":25.0,"detectorWidthMm":40.0,
                "centerOffsetXMm":0.0,"centerOffsetYMm":0.0,"rotationDirection":"counterclockwise",
                "mirrorX":false,"measurement":"measured"},
            "references": {"preDark": (1..=10u32).map(reference).collect::<Vec<_>>(),
                "preFlat": (1..=10u32).map(reference).collect::<Vec<_>>(), "postFlat": [], "postDark": []},
            "frames": if verified { (1..=parameters.projection_count).map(frame).collect::<Vec<_>>() } else { Vec::new() },
            "captureRecords": records
        });
        fs::write(root.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn acquired_pending_import_unlocks_the_scan_and_reserves_the_directory_across_restart() {
        let root = root("card-pending-import"); let path = root.join("active-task.json"); let parameters = parameters(&root);
        let mut registry = TaskRegistry::load(&path); registry.register(&parameters).unwrap();
        card_manifest(&parameters, crate::scan::DATA_PENDING_IMPORT, false);
        registry.mark_acquired_pending_import(&parameters).unwrap();
        // Safe finishing released the scan occupancy and the manual locks ...
        assert!(!registry.locked() && registry.error().is_none());
        assert!(registry.check_manual().is_ok());
        assert_eq!(registry.snapshot()["status"], "unlocked");
        assert_eq!(registry.pending_import_tasks().len(), 1);
        // ... but the directory stays reserved and the task is not completed.
        assert!(registry.retained_location(&parameters));
        assert!(registry.check_target_available(&parameters).unwrap_err().contains("SCAN_OUTPUT_RETAINED"));
        assert!(registry.prepare_register(&parameters).is_err());
        // A restart keeps the pending-import state, the reservation and no lock.
        let restored = TaskRegistry::load(&path);
        assert!(restored.error().is_none() && !restored.locked());
        assert_eq!(restored.pending_import_tasks().len(), 1);
        assert!(restored.retained_location(&parameters));
        assert_eq!(restored.snapshot()["retained"][0]["status"], TASK_STATUS_ACQUIRED_PENDING_IMPORT);
    }

    #[test]
    fn only_a_sealed_verified_import_completes_a_pending_import_task() {
        let root = root("card-verified-import"); let path = root.join("active-task.json"); let parameters = parameters(&root);
        let mut registry = TaskRegistry::load(&path); registry.register(&parameters).unwrap();
        card_manifest(&parameters, crate::scan::DATA_PENDING_IMPORT, false);
        registry.mark_acquired_pending_import(&parameters).unwrap();
        // The import is still open, so it cannot seal the task.
        assert!(registry.mark_import_verified(&parameters).is_err());
        assert_eq!(registry.pending_import_tasks().len(), 1);
        // A "verified" status without the local file identity is refused.
        card_manifest(&parameters, crate::scan::DATA_VERIFIED, false);
        assert!(registry.mark_import_verified(&parameters).is_err());
        // Only the sealed, fully reconstructable import completes the record.
        card_manifest(&parameters, crate::scan::DATA_VERIFIED, true);
        registry.mark_import_verified(&parameters).unwrap();
        assert!(registry.pending_import_tasks().is_empty());
        assert_eq!(registry.snapshot()["retained"][0]["status"], "completed");
        assert!(registry.retained_location(&parameters));
        let restored = TaskRegistry::load(&path);
        assert_eq!(restored.snapshot()["retained"][0]["status"], "completed");
        assert!(!restored.locked());
    }

    #[test]
    fn pending_import_requires_card_evidence_with_provenance() {
        let root = root("card-evidence"); let path = root.join("active-task.json"); let parameters = parameters(&root);
        let mut registry = TaskRegistry::load(&path); registry.register(&parameters).unwrap();
        // A RAM schema-3 checkpoint is not card evidence.
        checkpoint(&parameters);
        assert!(registry.mark_acquired_pending_import(&parameters).is_err());
        // A card acquisition that already holds local frames is not waiting for import.
        card_manifest(&parameters, crate::scan::DATA_PENDING_IMPORT, true);
        assert!(registry.mark_acquired_pending_import(&parameters).is_err());
        // A record without its card provenance is refused.
        card_manifest(&parameters, crate::scan::DATA_PENDING_IMPORT, false);
        let manifest_path = acquisition_root(&parameters).join("manifest.json");
        let mut data: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        data["captureRecords"][0]["sourceFileName"] = json!("");
        fs::write(&manifest_path, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(registry.mark_acquired_pending_import(&parameters).is_err());
        // A record count that does not cover every projection is refused.
        card_manifest(&parameters, crate::scan::DATA_PENDING_IMPORT, false);
        let mut data: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        // Index 0 is the first projection record; dropping it leaves one projection.
        data["captureRecords"].as_array_mut().unwrap().remove(0);
        fs::write(&manifest_path, serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(registry.mark_acquired_pending_import(&parameters).is_err());
        // The complete card evidence is accepted.
        card_manifest(&parameters, crate::scan::DATA_PENDING_IMPORT, false);
        registry.mark_acquired_pending_import(&parameters).unwrap();
        assert_eq!(registry.pending_import_tasks().len(), 1);
        assert!(!registry.locked());
    }
}
