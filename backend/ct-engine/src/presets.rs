//! Versioned, multi-preset camera-only storage. All imported entries are validated
//! before one atomic write. Device application is a separate explicit command;
//! this module never opens a camera or stores scan, radiation or motion settings.

use crate::capture_profile::{validate_parameter_keys, PresetTag};
use crate::devices::camera::CameraProperties;
use crate::task_registry::atomic_store;
use crate::timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::atomic::AtomicBool;

const MAX_PRESETS: usize = 128;
pub const MAX_PRESET_BYTES: usize = 1_048_576;
static PRESET_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub models: Vec<String>,
    pub parameters: CameraProperties,
    pub source: String,
    pub saved_at: String,
}
impl CameraPreset {
    pub fn tag(&self) -> PresetTag { PresetTag { id:self.id.clone(),name:self.name.clone(),source:self.source.clone() } }
    pub fn validate_model(&self, model: &str) -> Result<(), String> {
        if self.models.iter().any(|candidate| candidate.eq_ignore_ascii_case(model)) { Ok(()) }
        else { Err(format!("Preset {} does not apply to {model}",self.name)) }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresetFile { pub schema_version:u32, pub presets:Vec<CameraPreset> }

pub struct PresetStore { path:PathBuf, file:PresetFile, error:Option<String> }
pub struct PresetWritePlan{path:PathBuf,expected:PresetFile,next:PresetFile}
pub struct PresetWriteSeed{path:PathBuf,expected:PresetFile}
impl PresetWriteSeed{
    pub fn prepare_save(self,name:String,description:String,model:String,confirmed:CameraProperties,source:String)->Result<PresetWritePlan,String>{
        let id=format!("preset-{}-{}",chrono::Utc::now().format("%Y%m%dT%H%M%S%3f"),PRESET_SEQUENCE.fetch_add(1,Ordering::Relaxed));let mut next=self.expected.clone();
        next.presets.push(CameraPreset{id,name,description,models:vec![model],parameters:confirmed,source,saved_at:timestamp()});PresetStore::validate_file(&next)?;
        Ok(PresetWritePlan{path:self.path,expected:self.expected,next})
    }
}
impl PresetWritePlan{pub fn execute(&self,cancel:&AtomicBool)->Result<(),String>{if self.path.exists(){let current:PresetFile=serde_json::from_slice(&fs::read(&self.path).map_err(|error|error.to_string())?).map_err(|error|error.to_string())?;if serde_json::to_vec(&current).map_err(|error|error.to_string())?!=serde_json::to_vec(&self.expected).map_err(|error|error.to_string())?{return Err("Stale preset persistence plan".into());}}crate::task_registry::atomic_store_cancelled(&self.path,&serde_json::to_vec_pretty(&self.next).map_err(|error|error.to_string())?,cancel)}}
impl PresetStore {
    pub fn write_seed(&self)->Result<PresetWriteSeed,String>{if let Some(error)=self.error.as_ref(){return Err(error.clone());}Ok(PresetWriteSeed{path:self.path.clone(),expected:self.file.clone()})}
    pub fn prepare_save(&self,name:String,description:String,model:String,confirmed:CameraProperties,source:String)->Result<PresetWritePlan,String>{
        if let Some(error)=self.error.as_ref(){return Err(error.clone());}
        let id=format!("preset-{}-{}",chrono::Utc::now().format("%Y%m%dT%H%M%S%3f"),PRESET_SEQUENCE.fetch_add(1,Ordering::Relaxed));
        let mut next=self.file.clone();next.presets.push(CameraPreset{id,name,description,models:vec![model],parameters:confirmed,source,saved_at:timestamp()});Self::validate_file(&next)?;
        Ok(PresetWritePlan{path:self.path.clone(),expected:self.file.clone(),next})
    }
    pub fn prepare_delete(&self,id:&str)->Result<PresetWritePlan,String>{
        if let Some(error)=self.error.as_ref(){return Err(error.clone());}
        let mut next=self.file.clone();let before=next.presets.len();next.presets.retain(|preset|preset.id!=id);if before==next.presets.len(){return Err("PRESET_NOT_FOUND".into());}Self::validate_file(&next)?;Ok(PresetWritePlan{path:self.path.clone(),expected:self.file.clone(),next})
    }
    pub fn prepare_import(&self,text:&str,mut validate:impl FnMut(&CameraPreset)->Result<(),String>)->Result<PresetWritePlan,String>{if let Some(error)=self.error.as_ref(){return Err(error.clone());}let next=Self::parse_import(text)?;for preset in &next.presets{validate(preset)?;}Ok(PresetWritePlan{path:self.path.clone(),expected:self.file.clone(),next})}
    pub fn commit(&mut self,plan:PresetWritePlan)->Result<(),String>{if serde_json::to_vec(&self.file).map_err(|error|error.to_string())?!=serde_json::to_vec(&plan.expected).map_err(|error|error.to_string())?{return Err("Stale preset persistence completion".into());}self.file=plan.next;self.error=None;Ok(())}
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut store = Self { path,file:PresetFile{schema_version:1,presets:Vec::new()},error:None };
        if !store.path.exists() { return store; }
        match fs::read(&store.path).map_err(|error| error.to_string()).and_then(|bytes| {
            let text=String::from_utf8(bytes).map_err(|error| error.to_string())?; Self::parse_import(&text)
        }) {
            Ok(file)=>store.file=file,
            Err(error)=>store.error=Some(format!("Camera presets cannot be verified: {error}")),
        }
        store
    }
    pub fn snapshot(&self) -> Value { json!({"schemaVersion":1,"presets":self.file.presets,"error":self.error}) }
    pub fn list(&self) -> Vec<CameraPreset> { self.file.presets.clone() }
    pub fn get(&self,id:&str) -> Option<CameraPreset> { self.file.presets.iter().find(|preset|preset.id==id).cloned() }
    pub fn error(&self) -> Option<&str> { self.error.as_deref() }
    pub fn save(&mut self,name:String,description:String,model:String,confirmed:CameraProperties,source:String) -> Result<CameraPreset,String> {
        if let Some(error)=&self.error { return Err(error.clone()); }
        let id=format!("preset-{}-{}",chrono::Utc::now().format("%Y%m%dT%H%M%S%3f"),PRESET_SEQUENCE.fetch_add(1,Ordering::Relaxed));
        let preset=CameraPreset{id,name,description,models:vec![model],parameters:confirmed,source,saved_at:timestamp()};
        let mut file=self.file.clone(); file.presets.push(preset.clone()); Self::validate_file(&file)?;
        self.persist(file)?; Ok(preset)
    }
    pub fn delete(&mut self,id:&str) -> Result<(),String> {
        if let Some(error)=&self.error { return Err(error.clone()); }
        let mut file=self.file.clone(); let before=file.presets.len(); file.presets.retain(|preset|preset.id!=id);
        if before==file.presets.len() {return Err("PRESET_NOT_FOUND".into());}
        self.persist(file)
    }
    pub fn parse_import(text:&str) -> Result<PresetFile,String> {
        if text.len()>MAX_PRESET_BYTES {return Err("Preset file exceeds 1 MiB".into());}
        let file:PresetFile=serde_json::from_str(text).map_err(|error|error.to_string())?;
        Self::validate_file(&file)?; Ok(file)
    }
    /// The caller's validator checks the active model and every described
    /// capability. No entry is silently skipped or persisted on partial failure.
    pub fn import_json(&mut self,text:&str,mut validate:impl FnMut(&CameraPreset)->Result<(),String>) -> Result<(),String> {
        if let Some(error)=&self.error {return Err(error.clone());}
        let file=Self::parse_import(text)?;
        for preset in &file.presets {validate(preset)?;}
        self.persist(file)
    }
    pub fn export_json(&self) -> Result<String,String> {
        if let Some(error)=&self.error {return Err(error.clone());}
        Self::validate_file(&self.file)?;
        serde_json::to_string_pretty(&self.file).map_err(|error|error.to_string())
    }
    fn validate_file(file:&PresetFile) -> Result<(),String> {
        if file.schema_version!=1 || file.presets.len()>MAX_PRESETS {return Err("Unsupported preset schema or too many presets".into());}
        let mut ids=BTreeSet::new();
        for preset in &file.presets {
            if !valid_text(&preset.id,96,false) || !preset.id.bytes().all(|byte|byte.is_ascii_alphanumeric()||matches!(byte,b'-'|b'_'))
                || !ids.insert(&preset.id) || !valid_text(&preset.name,80,false)
                || !valid_text(&preset.description,1024,true) || preset.models.is_empty() || preset.models.len()>16
                || preset.models.iter().any(|model|!valid_text(model,128,false))
                || !valid_text(&preset.source,256,false) || !valid_text(&preset.saved_at,64,false) {
                return Err("Invalid preset identity, text or model list".into());
            }
            validate_parameter_keys(&preset.parameters)?;
        }
        Ok(())
    }
    fn persist(&mut self,file:PresetFile) -> Result<(),String> {
        Self::validate_file(&file)?;
        atomic_store(&self.path,&serde_json::to_vec_pretty(&file).map_err(|error|error.to_string())?)?;
        self.file=file; Ok(())
    }
}
fn valid_text(value:&str,limit:usize,allow_empty:bool)->bool {
    (allow_empty||!value.trim().is_empty()) && value.len()<=limit && !value.chars().any(|character| character.is_control() && character!='\n')
}

pub fn compare_parameters(preset:&CameraProperties,actual:&CameraProperties)->Value {
    Value::Array(preset.iter().map(|(key,target)|json!({"key":key,"preset":target,"actual":actual.get(key),"matches":actual.get(key)==Some(target)})).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::camera::CameraValue;
    fn store(topic:&str)->PresetStore {
        let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/camera")
            .join(format!("{}-presets-{topic}",chrono::Local::now().format("%Y%m%d-%H%M%S"))).join("presets.json");
        PresetStore::load(path)
    }
    #[test]
    fn multiple_named_presets_roundtrip_native_values_without_scan_fields() {
        let mut store=store("roundtrip"); let values=[("shutter".into(),CameraValue::Integer(0x00010005))].into_iter().collect();
        let first=store.save("NEF 200ms".into(),"".into(),"D7100".into(),values,"operator-confirmed".into()).unwrap();
        store.save("Other".into(),"description".into(),"D7100".into(),first.parameters.clone(),"operator-confirmed".into()).unwrap();
        let exported=store.export_json().unwrap(); let parsed=PresetStore::parse_import(&exported).unwrap(); assert_eq!(parsed.presets.len(),2);
        assert_eq!(parsed.presets[0].parameters["shutter"],CameraValue::Integer(0x00010005));
        assert!(!exported.contains("savePath")); assert!(!exported.contains("focusPosition"));
    }
    #[test]
    fn invalid_or_incompatible_import_is_atomic_and_does_not_skip_fields() {
        let mut store=store("import"); let values=[("iso".into(),CameraValue::Integer(100))].into_iter().collect();
        store.save("Existing".into(),"".into(),"D7100".into(),values,"confirmed".into()).unwrap();
        let before=store.export_json().unwrap(); assert!(store.import_json(&before,|_|Err("unsupported capability".into())).is_err()); assert_eq!(store.export_json().unwrap(),before);
        let mut data:Value=serde_json::from_str(&before).unwrap(); data["presets"][0]["parameters"]["savePath"]=json!("outside");
        assert!(PresetStore::parse_import(&data.to_string()).is_err()); data["schemaVersion"]=json!(99); assert!(PresetStore::parse_import(&data.to_string()).is_err());
    }
}
