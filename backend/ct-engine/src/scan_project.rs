//! Read-only loading of sealed scan projects for offline reconstruction.
//! The operation commits only after the manifest and every input pass the
//! reconstruction validator. It never acquires a device or writes scan data.

use crate::{reconstruction, scan, Engine, Parameters};
use serde_json::Value;
use std::{fs, path::Path, sync::{Arc,Mutex,atomic::{AtomicBool, Ordering}}};

struct Project {
    parameters: Parameters,
    manifest: scan::StoredScanManifest,
    frames: Vec<Value>,
}

fn load_project(path: &Path, cancel: &AtomicBool, progress:&crate::operations::SharedProgress) -> Result<Project, String> {
    if cancel.load(Ordering::SeqCst) { return Err("Open scan project cancelled".into()); }
    if !path.is_dir() { return Err("Select an existing scan project folder containing manifest.json".into()); }
    let source = fs::read(path.join("manifest.json"))
        .map_err(|error| format!("Cannot read scan project manifest.json: {error}"))?;
    let manifest: scan::StoredScanManifest = serde_json::from_slice(&source)
        .map_err(|error| format!("Invalid scan project manifest: {error}"))?;
    if path.file_name().and_then(|name| name.to_str()) != Some(manifest.scan_id.as_str()) {
        return Err(format!("Select the original project folder named {}", manifest.scan_id));
    }
    let parameters = Parameters {
        task_id: manifest.scan_id.clone(),
        save_path: path.parent().ok_or("Scan project has no parent folder")?.display().to_string(),
        projection_count: manifest.projection_count,
        angle_step_deg: 360.0 / f64::from(manifest.projection_count),
        exposure_ms: manifest.exposure_ms,
    };
    parameters.validate().map_err(|code| format!("Invalid scan project parameters: {code}"))?;
    if !scan::completed_manifest_exists(&parameters) {
        return Err("This project is not a completed, sealed scan. Use Restore Unfinished Scan for an unfinished acquisition.".into());
    }
    reconstruction::validate_scan_project(path, cancel,|value| {if let Ok(mut current)=progress.lock(){*current=value;}})?;
    // Avoid committing a mixed dataset if another process replaces metadata
    // while the integrity check is in progress.
    if cancel.load(Ordering::SeqCst) { return Err("Open scan project cancelled".into()); }
    if fs::read(path.join("manifest.json")).map_err(|error| error.to_string())? != source {
        return Err("Scan project changed during validation; open it again".into());
    }
    let frames = manifest.frames.iter().map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    Ok(Project { parameters, manifest, frames })
}

impl Engine {
    pub(crate) fn enqueue_open_scan_project(&mut self, payload: &Value) -> Result<(), &'static str> {
        if self.preview { return Err("DESKTOP_SCAN_PROJECT_REQUIRED"); }
        if self.busy() || self.task_registry.locked() { return Err("SCAN_PROJECT_LOCKED"); }
        let path = payload.get("path").and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty()).ok_or("SCAN_PROJECT_PATH_REQUIRED")?;
        if !Path::new(path).is_absolute() { return Err("SCAN_PROJECT_ABSOLUTE_PATH_REQUIRED"); }
        let path = Path::new(path).to_path_buf();
        let progress=Arc::new(Mutex::new(crate::operations::OperationProgress::default()));
        let shared=progress.clone();
        let id=self.operations.spawn(&self.engine_session_id,"task_project", "open_project", move |cancel| {
            let project = load_project(&path, &cancel,&shared)?;
            Ok(Box::new(move |engine| {
                engine.metadata_generation += 1;
                engine.card_import = None;
                engine.parameters = project.parameters;
                engine.current = project.manifest.frames.len() as u32;
                engine.real_frames = project.frames;
                engine.scan_geometry = Some(project.manifest.geometry);
                engine.scan_references = project.manifest.references;
                engine.scan_stage = "completed";
                engine.scan_prompt = None;
                engine.restored_scan = None;
                engine.restored_frames.clear();
                engine.metadata_manifest = true;
                engine.metadata_resume = false;
                engine.metadata_completed = true;
                engine.preflight = false;
                engine.capture_profile = None;
                engine.scan_angle_deg = None;
                engine.reconstruction_method = None;
                engine.reconstruction_status = "idle";
                engine.reconstruction_percent = 0;
                engine.reconstruction_message = "Project verified. Ready for offline reconstruction.".into();
                engine.reconstruction_cache_path = None;
                engine.reconstruction_event_sequence = 0;
                // Phase, faults, beam, setpoints and live device evidence are
                // deliberately unchanged: historical data is not safety proof.
                engine.log("INFO", "system", &format!("Opened completed scan project {}: {} projections verified. Ready for offline reconstruction.", engine.parameters.task_id, engine.current));
                Ok(())
            }))
        })?;
        self.operations.track_progress(&id,progress);
        self.log("INFO", "system", "Opening scan project: validating sealed metadata, references and projection integrity. No device action is required.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{capture_profile::test_profile, Phase};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{path::PathBuf, time::{Duration, Instant}};

    fn fixture(schema: u32) -> (Engine, PathBuf, Value) {
        let engine = Engine::new(false);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine")
            .join(&engine.engine_session_id).join("completed-test");
        fs::create_dir_all(&root).unwrap();
        let frame = |index: u32, group: &str| {
            let name = format!("{group}-{index}.nef");
            let bytes = format!("synthetic integrity fixture {group} {index}").into_bytes();
            let path = root.join(&name);
            fs::write(&path, &bytes).unwrap();
            json!({"index":index,"exposureMs":200.0,"fileName":name,
                "path":path.display().to_string(),"bytes":bytes.len(),
                "sha256":format!("{:x}",Sha256::digest(&bytes))})
        };
        let projections = (1..=3).map(|i| {
            let mut value = frame(i,"projection"); value["angleDeg"] = json!(-(i as f64 - 1.0)*120.0); value
        }).collect::<Vec<_>>();
        let manifest = json!({"schemaVersion":schema,"scanId":"completed-test","completed":true,
            "voltageKv":59.9,"currentUa":200.0,"projectionCount":3,"exposureMs":200.0,
            "stage":"completed","captureProfile":test_profile(),"maxXraySec":600,
            "geometry":{"sodMm":80.0,"objectToDetectorMm":100.0,"detectorWidthMm":23.5,
                "centerOffsetXMm":0.0,"centerOffsetYMm":0.0,"rotationDirection":"clockwise",
                "mirrorX":false,"measurement":"measured"},
            "references":{"preDark":(1..=10).map(|i|frame(i,"dark")).collect::<Vec<_>>(),
                "preFlat":(1..=10).map(|i|frame(i,"flat")).collect::<Vec<_>>(),"postDark":[],"postFlat":[]},
            "frames":projections});
        save(&root,&manifest);
        (engine,root,manifest)
    }
    fn save(root: &Path, manifest: &Value) { fs::write(root.join("manifest.json"),serde_json::to_vec(manifest).unwrap()).unwrap(); }
    fn card_fixture() -> (Engine, PathBuf, Value) {
        let (engine,root,mut manifest)=fixture(3);
        manifest["schemaVersion"]=json!(4);
        manifest["captureProfile"]=json!(crate::capture_profile::test_card_profile("single"));
        manifest["captureDestination"]=json!("cameraCard");
        manifest["acquisitionStatus"]=json!(scan::ACQUISITION_ACQUIRED);
        manifest["dataStatus"]=json!(scan::DATA_VERIFIED);
        let mut records=vec![];
        for (kind,frames) in [("preDark",&manifest["references"]["preDark"]),("preFlat",&manifest["references"]["preFlat"]),("projection",&manifest["frames"])] {
            for frame in frames.as_array().unwrap() {
                let mut imported=frame.clone();
                if imported.get("angleDeg").is_none(){imported["angleDeg"]=json!(0.0);}
                records.push(json!({"index":frame["index"],"kind":kind,"angleDeg":imported["angleDeg"],"exposureMs":200.0,
                    "state":scan::DATA_VERIFIED,"cameraSessionId":"card-session","operationId":format!("{kind}-{}",frame["index"]),
                    "storageId":131073,"storageChanged":false,"handle":records.len()+1,"parent":7,
                    "sourceFileName":format!("DSC_{:04}.NEF",records.len()+1),"format":14336,
                    "size":frame["bytes"],"captureDate":"20261013T060000","importedFile":imported}));
            }
        }
        manifest["captureRecords"]=json!(records);
        save(&root,&manifest);
        (engine,root,manifest)
    }
    fn finish(engine: &mut Engine) {
        let deadline = Instant::now()+Duration::from_secs(3);
        while engine.operations.blocks_controls() && Instant::now()<deadline {
            engine.sync_operations(); std::thread::sleep(Duration::from_millis(2));
        }
        engine.sync_operations();
        assert!(!engine.operations.blocks_controls());
    }
    fn open(engine: &mut Engine, root: &Path) {
        engine.command("open_scan_project",json!({"path":root})).unwrap(); finish(engine);
    }

    #[test]
    fn opens_schema_two_and_three_without_devices_or_writing_inputs() {
        for schema in [2,3] {
            let (mut engine,root,_)=fixture(schema);
            let before=fs::read(root.join("manifest.json")).unwrap();
            let beam=engine.snapshot()["workstation"]["xray"].clone();
            let phase=engine.phase;
            open(&mut engine,&root);
            assert_eq!(engine.operations.history.last().unwrap().status,"completed");
            let progress=engine.operations.history.last().unwrap().progress.as_ref().unwrap();
            assert_eq!(progress.percent,100);assert_eq!(progress.estimated_remaining_sec,Some(0));
            assert_eq!(progress.verified_files,23);assert_eq!(progress.total_files,23);
            assert_eq!(progress.processed_bytes,progress.total_bytes);
            let receipt=engine.snapshot()["operations"].as_array().unwrap().last().unwrap().clone();
            assert_eq!(receipt["progress"]["percent"],100);
            assert_eq!(receipt["progress"]["verifiedFiles"],23);
            assert_eq!(engine.parameters.task_id,"completed-test");
            assert_eq!(engine.current,3); assert_eq!(engine.real_frames.len(),3);
            assert_eq!(engine.scan_references.pre_dark.len(),10);
            assert!(engine.snapshot()["workstation"]["reconstruction"]["unlocked"].as_bool().unwrap());
            assert_eq!(engine.snapshot()["workstation"]["xray"],beam);
            assert!(!engine.preflight); assert!(!engine.homed); assert!(engine.scan_angle_deg.is_none());
            assert_eq!(engine.phase,phase);
            assert!(engine.camera.is_none()&&engine.nano.is_none()&&engine.xray.is_none());
            assert!(engine.operations.history.iter().all(|op|op.device=="task_project"));
            assert_eq!(fs::read(root.join("manifest.json")).unwrap(),before);
            // The same metadata qualifies for the existing reconstruction entry.
            assert!(scan::completed_manifest_exists(&engine.parameters));
        }
    }
    #[test]
    fn opens_verified_schema_four_card_project_without_devices_or_writing_inputs() {
        let (mut engine,root,_)=card_fixture();
        let before=fs::read(root.join("manifest.json")).unwrap();
        open(&mut engine,&root);
        let receipt=engine.operations.history.last().unwrap();
        assert_eq!(receipt.status,"completed","{:?}",receipt.error);
        assert_eq!(receipt.progress.as_ref().unwrap().verified_files,23);
        assert!(engine.snapshot()["workstation"]["reconstruction"]["unlocked"].as_bool().unwrap());
        assert_eq!(engine.real_frames.len(),3);
        assert_eq!(fs::read(root.join("manifest.json")).unwrap(),before);
        assert!(engine.camera.is_none()&&engine.nano.is_none()&&engine.xray.is_none());
    }
    #[test]
    fn schema_four_read_gate_requires_verified_card_records_bound_to_every_input() {
        for problem in ["pending","acquisition","destination","stage","missing","unverified","duplicate","path","hash","size"] {
            let (_,root,mut manifest)=card_fixture();
            match problem {
                "pending"=>manifest["dataStatus"]=json!(scan::DATA_PENDING_IMPORT),
                "acquisition"=>manifest["acquisitionStatus"]=json!("running"),
                "destination"=>manifest["captureDestination"]=json!("hostRam"),
                "stage"=>manifest["stage"]=json!("finishing"),
                "missing"=>{manifest["captureRecords"].as_array_mut().unwrap().pop();},
                "unverified"=>manifest["captureRecords"][0]["state"]=json!(scan::DATA_PENDING_IMPORT),
                "duplicate"=>manifest["captureRecords"][1]=manifest["captureRecords"][0].clone(),
                "path"=>manifest["captureRecords"][0]["importedFile"]["path"]=json!("/unrelated.nef"),
                "hash"=>manifest["captureRecords"][0]["importedFile"]["sha256"]=json!("00".repeat(32)),
                "size"=>manifest["captureRecords"][0]["size"]=json!(1),
                _=>unreachable!(),
            }
            save(&root,&manifest);
            let before=fs::read(root.join("manifest.json")).unwrap();
            assert!(reconstruction::validate_scan_project(&root,&AtomicBool::new(false),|_|{}).is_err(),"{problem}");
            assert_eq!(fs::read(root.join("manifest.json")).unwrap(),before);
        }
    }
    #[test]
    fn bad_inputs_preserve_current_dataset_and_safety_evidence() {
        for problem in ["unsealed","stage","count","geometry","hash","missing","escape","profile","json"] {
            let (mut engine,root,mut manifest)=fixture(3);
            engine.parameters.task_id="keep-current".into(); engine.current=7;
            engine.real_frames=vec![json!({"retained":true})]; engine.metadata_completed=true;
            engine.preflight=true; engine.phase=Phase::Fault; engine.fault_latched=true;
            engine.manual_lock=Some("Keep fault".into());
            match problem {
                "unsealed"=>manifest["completed"]=json!(false),
                "stage"=>manifest["stage"]=json!("paused"),
                "count"=>manifest["projectionCount"]=json!(4),
                "geometry"=>manifest["geometry"]["measurement"]=json!("estimated"),
                "hash"=>fs::write(root.join("projection-1.nef"),b"tampered").unwrap(),
                "missing"=>fs::remove_file(root.join("flat-1.nef")).unwrap(),
                "escape"=> { let outside=root.parent().unwrap().join("outside.nef");fs::copy(root.join("projection-1.nef"),&outside).unwrap();manifest["frames"][0]["path"]=json!(outside); },
                "profile"=>manifest["captureProfile"]["parametersHash"]=json!("bad"),
                _=>{},
            }
            save(&root,&manifest);
            if problem=="json" {fs::write(root.join("manifest.json"),b"broken json").unwrap();}
            open(&mut engine,&root);
            assert_eq!(engine.operations.history.last().unwrap().status,"rejected","{problem}");
            if let Some(progress)=&engine.operations.history.last().unwrap().progress {
                assert!(progress.percent<100,"{problem}");assert_eq!(progress.estimated_remaining_sec,None);
            }
            assert_eq!(engine.parameters.task_id,"keep-current","{problem}");
            assert_eq!(engine.current,7); assert_eq!(engine.real_frames,vec![json!({"retained":true})]);
            assert!(engine.preflight&&engine.fault_latched&&engine.metadata_completed);
            assert_eq!(engine.phase,Phase::Fault);assert_eq!(engine.manual_lock.as_deref(),Some("Keep fault"));
            assert!(engine.operations.history.iter().all(|op|op.device=="task_project"));
        }
    }
    #[test]
    fn missing_manifest_and_wrong_folder_are_explained() {
        let (_,root,_)=fixture(2);
        let progress=Arc::new(Mutex::new(crate::operations::OperationProgress::default()));
        assert!(load_project(root.parent().unwrap(),&AtomicBool::new(false),&progress).err().unwrap().contains("manifest.json"));
        let alternate=root.parent().unwrap().join("wrong-name");fs::create_dir_all(&alternate).unwrap();
        fs::copy(root.join("manifest.json"),alternate.join("manifest.json")).unwrap();
        assert!(load_project(&alternate,&AtomicBool::new(false),&progress).err().unwrap().contains("completed-test"));
    }
    #[test]
    fn cancelled_or_late_load_does_not_commit() {
        let (mut engine,root,_)=fixture(2);
        engine.command("open_scan_project",json!({"path":root})).unwrap();
        // Let the worker finish but leave its completion in the mailbox.
        std::thread::sleep(Duration::from_millis(80));
        engine.operations.cancel(); finish(&mut engine);
        assert_eq!(engine.operations.history.last().unwrap().status,"cancelled");
        assert!(engine.parameters.task_id.is_empty()&&engine.real_frames.is_empty());
        assert!(!engine.metadata_completed);
        assert!(load_project(&root,&AtomicBool::new(true),&Arc::new(Mutex::new(crate::operations::OperationProgress::default()))).err().unwrap().contains("cancelled"));
    }
    #[test]
    fn validation_progress_is_monotone_and_counts_only_verified_files() {
        let (_,root,mut manifest)=fixture(3);
        let mut events=vec![];
        reconstruction::validate_scan_project(&root,&AtomicBool::new(false),|progress|events.push(progress)).unwrap();
        assert_eq!(events.first().unwrap().processed_bytes,0);
        assert_eq!(events.last().unwrap().verified_files,23);
        assert_eq!(events.last().unwrap().processed_bytes,events.last().unwrap().total_bytes);
        assert!(events.iter().all(|event|event.percent<100));
        assert!(events.windows(2).all(|pair|pair[0].processed_bytes<=pair[1].processed_bytes&&pair[0].percent<=pair[1].percent));
        // Same-size corruption reaches the byte budget but must not verify the last file.
        let path=root.join("projection-3.nef");let mut bytes=fs::read(&path).unwrap();bytes[0]^=1;fs::write(&path,bytes).unwrap();
        manifest["frames"][2]["sha256"]=json!("0".repeat(64));save(&root,&manifest);
        events.clear();
        assert!(reconstruction::validate_scan_project(&root,&AtomicBool::new(false),|progress|events.push(progress)).unwrap_err().contains("SHA-256 mismatch"));
        assert_eq!(events.last().unwrap().verified_files,22);
        assert!(events.iter().all(|event|event.percent<100));
    }
    #[test]
    fn acquisition_and_maintenance_ownership_cannot_be_bypassed() {
        let (mut engine,root,_)=fixture(3);
        engine.phase=Phase::Paused;
        assert_eq!(engine.command("open_scan_project",json!({"path":root})),Err("SCAN_PROJECT_LOCKED"));
        engine.phase=Phase::Idle; engine.maintenance_lock=true;
        assert_eq!(engine.command("open_scan_project",json!({"path":root})),Err("SOFTWARE_UPDATE_MAINTENANCE_LOCKED"));
        engine.maintenance_lock=false;
        engine.task_registry.register(&Parameters{task_id:"unfinished".into(),save_path:root.parent().unwrap().display().to_string(),projection_count:3,angle_step_deg:120.0,exposure_ms:200.0}).unwrap();
        assert_eq!(engine.command("open_scan_project",json!({"path":root})),Err("SCAN_PROJECT_LOCKED"));
        assert!(engine.operations.history.is_empty());
    }
}
