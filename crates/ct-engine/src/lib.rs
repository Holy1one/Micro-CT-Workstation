//! Desktop scan-state owner and protocol-v1 command processor.
//! Production domain engine for the Micro-CT workstation.
//!
//! The engine is the single owner of device state, safety gates, scan state,
//! and the versioned JSONL API exposed to the Tauri desktop shell. Device-
//! specific I/O lives under `devices`; scan transaction execution lives under
//! `scan`. Keeping those directions one-way prevents hardware drivers from
//! acquiring a second copy of application state.

pub mod devices;
pub mod reconstruction;
mod scan;
mod operations;
mod device_controls;
mod software_update;
pub mod capture_profile;
pub mod task_registry;
pub mod presets;

use devices::camera::{CameraHealth, CameraAdapter, valid_exposure_ms, EXPOSURE_MIN_MS, EXPOSURE_MAX_MS};
use devices::turntable::{NanoAdapter, NanoConnectionState, NanoHealth, NanoStatus};
use devices::xray::{
    MoxtekAdapter, XrayHealth, MAX_CURRENT_UA, MAX_SETPOINT_POWER_W, MAX_VOLTAGE_KV,
    MIN_VOLTAGE_KV,
};
use scan::{RealFrame, RealScanConfig, RealScanHandle, ReferenceFrames, ScanCompletion, ScanGeometry, ScanResume};
use reconstruction::{ReconstructionHandle, ReconstructionMethod, ReconstructionRequest};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u32 = 2;
const MAX_LOGS: usize = 100;

pub fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Last confirmed physical position, never the pending motion target.
fn confirmed_nano_angle(health: &NanoHealth) -> Option<f64> {
    if health.state != NanoConnectionState::Connected || health.last_error.is_some() {
        return None;
    }
    let status = health.status.as_ref()?;
    if !status.reference_valid || !status.homed || status.pulses_per_rev == 0
        || matches!(status.state.as_str(), "FAULT" | "LOCKED" | "HOMING")
    {
        return None;
    }
    Some(status.position_pulses as f64 * 360.0 / f64::from(status.pulses_per_rev))
}

/// Presentation of current output evidence. Retained OFF after disconnect is
/// still useful to cleanup, but cannot be displayed as a monitored current OFF.
fn observed_xray_beam_state(health: Option<&XrayHealth>, engine_beam_on: bool) -> &'static str {
    let live = health.filter(|health| health.connected && health.last_error.is_none());
    if live.is_some_and(|health| health.beam_on) {
        "on"
    } else if live.is_some_and(|health| health.beam_off_confirmed) && !engine_beam_on {
        "off"
    } else {
        "unknown"
    }
}

/// A live, idle Nano STATUS can retain the Hall reference established earlier
/// in the same controller session. Cached engine flags alone never qualify.
fn nano_reference_ready(status: &NanoStatus) -> bool {
    status.state == "IDLE" && status.capture_id == 0
        && status.reference_valid && status.homed && status.rearmed
        && status.pulses_per_rev > 0 && status.target_pulses == status.position_pulses
}

fn may_reuse_home(calibrated_this_process: bool, status: &NanoStatus) -> bool {
    calibrated_this_process && nano_reference_ready(status)
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Request {
    #[serde(default)]
    pub engine_session_id: Option<String>,
    pub protocol_version: u32,
    pub request_id: String,
    pub command: String,
    #[serde(default)]
    pub payload: Value,
    pub timestamp: String,
    pub sequence: u64,
    #[serde(default)]
    pub error_code: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    pub engine_session_id: String,
    pub protocol_version: u32,
    pub request_id: String,
    pub command: String,
    pub payload: Value,
    pub timestamp: String,
    pub sequence: u64,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Parameters {
    pub task_id: String,
    pub save_path: String,
    pub projection_count: u32,
    pub angle_step_deg: f64,
    pub exposure_ms: f64,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            task_id: String::new(),
            save_path: String::new(),
            projection_count: 0,
            angle_step_deg: 0.0,
            exposure_ms: 0.0,
        }
    }
}

impl Parameters {
    fn validate(&self) -> Result<(), &'static str> {
        if self.task_id.trim().is_empty() || self.save_path.trim().is_empty() {
            return Err("INVALID_PARAMETERS");
        }
        let task_id = self.task_id.trim();
        validate_task_id(task_id)?;
        if task_id == "." || task_id == ".." || task_id.contains(['/', '\\', '\0']) {
            return Err("INVALID_PARAMETERS");
        }
        if !(1..=3600).contains(&self.projection_count)
            || !self.angle_step_deg.is_finite()
            || self.angle_step_deg <= 0.0
            || self.angle_step_deg > 360.0
            || (self.angle_step_deg * f64::from(self.projection_count) - 360.0).abs() > 0.01
            || !valid_exposure_ms(self.exposure_ms)
        {
            return Err("INVALID_PARAMETERS");
        }
        Ok(())
    }
}

fn validate_task_id(task_id:&str)->Result<(),&'static str>{
    let stem=task_id.split('.').next().unwrap_or("").to_ascii_uppercase();
    if task_id.eq_ignore_ascii_case("CameraCaptures")||task_id.ends_with(['.',' '])||task_id.chars().any(|c|c.is_control()||"<>:\\|?*/".contains(c))||matches!(stem.as_str(),"CON"|"PRN"|"AUX"|"NUL")||stem.strip_prefix("COM").or_else(||stem.strip_prefix("LPT")).is_some_and(|n|matches!(n,"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9")){return Err("RESERVED_OR_INVALID_TASK_ID");}Ok(())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScanSetupInput {
    #[serde(default)]
    save_path: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    projection_count: Option<u32>,
    #[serde(default)]
    exposure_ms: Option<f64>,
    #[serde(default)]
    max_xray_sec: Option<u32>,
}

impl ScanSetupInput {
    fn apply(
        &self,
        current_parameters: &Parameters,
        current_max_xray_sec: u32,
    ) -> Result<(Parameters, u32), &'static str> {
        if self.save_path.is_none()
            && self.task_id.is_none()
            && self.projection_count.is_none()
            && self.exposure_ms.is_none()
            && self.max_xray_sec.is_none()
        {
            return Err("INVALID_PARAMETERS");
        }

        let mut parameters = current_parameters.clone();
        let mut max_xray_sec = current_max_xray_sec;

        if let Some(save_path) = &self.save_path {
            if save_path.trim().is_empty() {
                return Err("INVALID_PARAMETERS");
            }
            parameters.save_path = save_path.clone();
        }
        if let Some(task_id) = &self.task_id {
            let value = task_id.trim();
            if value.is_empty() || value == "." || value == ".."
                || value.contains(['/', '\\', '\0']) {
                return Err("INVALID_PARAMETERS");
            }
            validate_task_id(value)?;
            parameters.task_id = task_id.clone();
        }
        if let Some(projection_count) = self.projection_count {
            if !(1..=3600).contains(&projection_count) {
                return Err("INVALID_PARAMETERS");
            }
            parameters.projection_count = projection_count;
            parameters.angle_step_deg = 360.0 / f64::from(projection_count);
        }
        if let Some(exposure_ms) = self.exposure_ms {
            if !valid_exposure_ms(exposure_ms) {
                return Err("INVALID_PARAMETERS");
            }
            parameters.exposure_ms = exposure_ms;
        }
        if let Some(value) = self.max_xray_sec {
            if !(1..=600).contains(&value) {
                return Err("INVALID_PARAMETERS");
            }
            max_xray_sec = value;
        }

        Ok((parameters, max_xray_sec))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    ReadyForHome,
    Ready,
    Running,
    Paused,
    Finishing,
    Stopping,
    Stopped,
    Completed,
    Fault,
}

impl Phase {
    fn value(self) -> (&'static str, &'static str) {
        match self {
            Self::Idle => ("idle", "Idle"),
            Self::ReadyForHome => ("ready_for_home", "Ready for Home"),
            Self::Ready => ("ready", "Ready"),
            Self::Running => ("running", "Running"),
            Self::Paused => ("paused", "Paused"),
            Self::Finishing => ("finishing", "Returning to start"),
            Self::Stopping => ("stopping", "Ending scan"),
            Self::Stopped => ("stopped", "Stopped"),
            Self::Completed => ("completed", "Completed"),
            Self::Fault => ("fault", "Fault"),
        }
    }
}

pub struct Engine {
    #[cfg(test)]
    test_connections: device_controls::TestConnections,
    engine_session_id: String,
    operations: operations::Operations,
    request_ids: std::collections::VecDeque<String>,
    bench_mode: bool,
    ever_xray: bool,
    manual_lock: Option<String>,
    software_update_preparation: Option<software_update::SoftwareUpdatePreparation>,
    maintenance_lock: bool,
    ever_nano: bool,
    ever_camera: bool,
    captures: std::collections::VecDeque<device_controls::RegisteredCapture>,
    log_counter: u64,
    task_registry: task_registry::TaskRegistry,
    preset_store: presets::PresetStore,
    preset_tag: Option<capture_profile::PresetTag>,
    capture_profile: Option<capture_profile::CaptureProfile>,
    camera_draft: devices::camera::CameraProperties,
    preset_export_json: Option<String>,
    preset_comparison: Value,
    scan_stop_requested: bool,
    metadata_generation:u64,metadata_manifest:bool,metadata_resume:bool,metadata_completed:bool,
    preview: bool,
    connected: bool,
    preflight: bool,
    homed: bool,
    phase: Phase,
    parameters: Parameters,
    max_xray_sec: u32,
    current: u32,
    sequence: u64,
    request_sequence: u64,
    last_tick: Instant,
    preview_view_ms: Option<u64>,
    logs: Vec<Value>,
    beam_on: bool,
    fault_latched: bool,
    timer_on: bool,
    usb_auto_shut_down: bool,
    usb_auto_shut_down_confirmed: bool,
    set_kv: f64,
    set_ua: f64,
    set_kv_confirmed: bool,
    set_ua_confirmed: bool,
    nano: Option<Arc<NanoAdapter>>,
    camera: Option<CameraAdapter>,
    xray: Option<MoxtekAdapter>,
    real_scan: Option<RealScanHandle>,
    real_scan_revision: u64,
    real_scan_message_count: usize,
    scan_stage: &'static str,
    scan_prompt: Option<&'static str>,
    scan_geometry: Option<ScanGeometry>,
    scan_references: ReferenceFrames,
    restored_scan: Option<ScanResume>,
    reconstruction: Option<ReconstructionHandle>,
    reconstruction_method: Option<ReconstructionMethod>,
    reconstruction_status: &'static str,
    reconstruction_percent: u8,
    reconstruction_event_sequence:u64,
    reconstruction_message: String,
    reconstruction_cache_path: Option<String>,
    scan_angle_deg: Option<f64>,
    real_frames: Vec<Value>,
    restored_frames: Vec<RealFrame>,
    home_performed_this_process: bool,
    cached_camera_health: Option<CameraHealth>,
    cached_xray_health: Option<XrayHealth>,
    last_error: Option<String>,
}

impl Engine {
    pub fn new(preview: bool) -> Self {
        #[cfg(test)]
        let storage = {
            static TEST_STORAGE_SEQUENCE: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine").join(format!("{}-engine-{}",Utc::now().format("%Y%m%d-%H%M%S%3f"),TEST_STORAGE_SEQUENCE.fetch_add(1,std::sync::atomic::Ordering::SeqCst)))
        };
        #[cfg(not(test))]
        let storage = std::env::var_os("MICRO_CT_ENGINE_DATA_DIR").map(std::path::PathBuf::from)
            .unwrap_or_else(||std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("MicroCTWorkstation"));
        Self::new_with_storage(preview,storage)
    }
    pub fn new_with_storage(preview: bool, storage: impl AsRef<Path>) -> Self {
        let mut engine = Self {
            #[cfg(test)]
            test_connections: device_controls::TestConnections::default(),
            engine_session_id: format!("engine-{}-{}", std::process::id(), Utc::now().timestamp_nanos_opt().unwrap_or_default()),
            operations: operations::Operations::default(),
            request_ids: std::collections::VecDeque::new(),
            bench_mode: false,
            ever_xray: false,
            manual_lock: None,
            software_update_preparation: None,maintenance_lock:false,ever_nano:false,ever_camera:false,captures:Default::default(),log_counter:0,
            task_registry: task_registry::TaskRegistry::load(storage.as_ref().join("active-task.json")),
            preset_store: presets::PresetStore::load(storage.as_ref().join("camera-presets.json")),
            preset_tag: None,
            capture_profile: None,
            camera_draft: Default::default(),
            preset_export_json: None,
            preset_comparison: Value::Null,
            scan_stop_requested: false,
            metadata_generation:0,metadata_manifest:false,metadata_resume:false,metadata_completed:false,
            preview,
            connected: preview,
            preflight: false,
            homed: false,
            phase: Phase::Idle,
            parameters: Parameters::default(),
            max_xray_sec: 600,
            current: 0,
            sequence: 0,
            request_sequence: 0,
            last_tick: Instant::now(),
            preview_view_ms: None,
            logs: Vec::new(),
            beam_on: false,
            fault_latched: false,
            timer_on: false,
            usb_auto_shut_down: true,
            usb_auto_shut_down_confirmed: preview,
            set_kv: 4.0,
            set_ua: 10.0,
            set_kv_confirmed: false,
            set_ua_confirmed: false,
            nano: None,
            camera: None,
            xray: None,
            real_scan: None,
            real_scan_revision: 0,
            real_scan_message_count: 0,
            scan_stage: "idle",
            scan_prompt: None,
            scan_geometry: None,
            scan_references: ReferenceFrames::default(),
            restored_scan: None,
            reconstruction: None,
            reconstruction_method: None,
            reconstruction_status: "idle",
            reconstruction_percent: 0,
            reconstruction_event_sequence:0,
            reconstruction_message: String::new(),
            reconstruction_cache_path: None,
            scan_angle_deg: None,
            real_frames: Vec::new(),
            restored_frames: Vec::new(),
            home_performed_this_process: false,
            cached_camera_health: None,
            cached_xray_health: None,
            last_error: None,
        };
        if let Some(parameters)=engine.task_registry.restore_parameters(){engine.parameters=parameters;}
        engine.schedule_metadata_probe();
        engine.log("INFO", "system", "ct-engine started; waiting for an explicit connection");
        engine.log("WARN", "xray", "NO REAL HARDWARE · output remains fail-closed");
        engine
    }

    pub fn handle(&mut self, request: Request) -> Response {
        self.tick();
        let descriptor_id=request.payload.get("captureId").and_then(Value::as_str).map(str::to_owned);
        let error = if request.protocol_version != PROTOCOL_VERSION {
            Some("PROTOCOL_VERSION_MISMATCH")
        } else if request.error_code.as_deref()==Some("COMMAND_QUEUE_FULL") {
            Some("COMMAND_QUEUE_FULL")
        } else if request.request_id.is_empty()
            || request.timestamp.is_empty()
            || request.error_code.is_some()
        {
            Some("INVALID_ENVELOPE")
        } else if request.sequence <= self.request_sequence {
            Some("STALE_REQUEST")
        } else if self.request_ids.contains(&request.request_id) {
            Some("DUPLICATE_REQUEST_ID")
        } else if request.engine_session_id.as_deref() != Some(&self.engine_session_id)
            && !(self.request_sequence == 0 && request.command == "snapshot" && request.engine_session_id.is_none()) {
            Some("ENGINE_SESSION_MISMATCH")
        } else {
            self.request_sequence = request.sequence;
            self.request_ids.push_back(request.request_id.clone());
            if self.request_ids.len()>256 { self.request_ids.pop_front(); }
            self.validate_payload_type(&request.command, &request.payload)
                .and_then(|_| self.command(&request.command, request.payload))
                .err()
        };
        self.sequence += 1;
        if let Some(code) = error {
            self.log("WARN", "system", &format!("Command rejected: {code}"));
        }
        let mut snapshot=self.snapshot();
        if error.is_none() && request.command=="camera_capture_descriptor" {
            snapshot["cameraCaptureDescriptor"]=self.captures.iter().find(|capture|Some(capture.capture_id.as_str())==descriptor_id.as_deref()).map(|capture|serde_json::to_value(capture).unwrap_or(Value::Null)).unwrap_or(Value::Null);
        }
        if error.is_none() && request.command=="camera_pipe_descriptor" {
            snapshot["liveViewPipeDescriptor"]=serde_json::to_value(self.camera.as_ref().and_then(|c|c.pipe_descriptor())).unwrap_or(Value::Null);
        }
        Response {
            engine_session_id: self.engine_session_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            request_id: request.request_id,
            command: request.command,
            payload: snapshot,
            timestamp: timestamp(),
            sequence: self.sequence,
            error_code: error.map(str::to_owned),
        }
    }

    fn validate_payload_type(&self, command: &str, payload: &Value) -> Result<(), &'static str> {
        if command == "snapshot" {
            return Ok(());
        }
        if payload.get("type").and_then(Value::as_str) == Some(command) {
            Ok(())
        } else {
            Err("COMMAND_PAYLOAD_MISMATCH")
        }
    }

    fn busy(&self) -> bool {
        self.real_scan.is_some() || self.reconstruction.is_some() || self.operations.blocks_controls()
            || matches!(self.phase, Phase::Running | Phase::Paused | Phase::Finishing | Phase::Stopping)
    }

    fn sync_real_scan(&mut self) {
        let Some(handle) = self.real_scan.as_mut() else { return; };
        let progress = handle.progress();
        let outcome = handle.try_finish();
        if progress.revision != self.real_scan_revision {
            self.real_scan_revision = progress.revision;
            self.current = progress.captured;
            self.scan_stage = progress.stage;
            self.scan_prompt = progress.prompt;
            self.scan_references = progress.references.clone();
            self.scan_angle_deg = Some(progress.angle_deg);
            self.beam_on = progress.beam_on;
            if self.fault_latched || progress.phase == "fault" {
                self.phase = Phase::Fault;
            } else if self.phase != Phase::Stopping {
                self.phase = match progress.phase {
                    "paused" => Phase::Paused,
                    "completed" | "finishing" => Phase::Finishing,
                    "stopping" | "stopped" => Phase::Stopping,
                    _ => Phase::Running,
                };
            }
            self.real_frames = progress.frames.iter()
                .map(|frame| serde_json::to_value(frame).unwrap_or_else(|_| json!({}))).collect();
            if let Some(health) = progress.xray_health {
                self.usb_auto_shut_down = health.usb_auto_shutdown.unwrap_or(self.usb_auto_shut_down);
                self.cached_xray_health = Some(health.clone());
            }
            let messages = progress.messages.iter().skip(self.real_scan_message_count).cloned().collect::<Vec<_>>();
            self.real_scan_message_count = progress.messages.len();
            for message in messages { self.log(message.level, message.source, &message.message); }
            if let Some(error) = progress.error { self.last_error = Some(error); }
        }
        let Some(outcome) = outcome else { return; };
        drop(self.real_scan.take());
        match outcome {
            Err(error) => {
                self.last_error = Some(error.clone());
                self.scan_stage = "fault";
                self.scan_prompt = None;
                self.fault_latched = true;
                self.preflight = false;
                self.homed = false;
                self.phase = Phase::Fault;
                if let Some(health) = self.cached_xray_health.as_mut() {
                    health.connected = false;
                    health.beam_off_confirmed = false;
                    health.last_error = Some(error.clone());
                }
                let _=self.enqueue_stop_for_shutdown();self.phase=Phase::Fault;
                self.log("ERR", "system", &error);
            }
            Ok(outcome) => {
                self.beam_on = outcome.xray.health().beam_on;
                self.cached_camera_health = Some(outcome.camera.health());
                self.cached_xray_health = Some(outcome.xray.health());
                self.camera = Some(outcome.camera);
                self.xray = Some(outcome.xray);
                match outcome.result {
                    Ok(ScanCompletion::Completed) if !self.fault_latched && !self.scan_stop_requested => {
                        self.phase = Phase::Finishing;
                        self.scan_stage = "persisting_completed";
                        self.scan_prompt = None;
                        self.last_error = None;
                        if let Err(error)=self.enqueue_task_completed(){self.fault_latched=true;self.phase=Phase::Fault;self.last_error=Some(error.into());}
                    }
                    Ok(ScanCompletion::Stopped) if !self.fault_latched => {
                        self.metadata_manifest=true;self.metadata_resume=true;self.metadata_completed=false;
                        self.invalidate(Phase::Stopped);
                        self.scan_stage = "stopped";
                        self.scan_prompt = None;
                        self.last_error = None;
                        self.log("INFO", "system", "Scan ended; saved projections retained. Repeat Preflight and HOME before a new scan.");
                    }
                    Ok(ScanCompletion::Completed) if self.scan_stop_requested&&!self.fault_latched=>{
                        self.invalidate(Phase::Stopped);self.scan_stage="stopped";self.scan_prompt=None;
                        self.log("INFO","system","Stop superseded a late completion; sealed files retained and persistent task lock remains");
                    }
                    result => {
                        if let Err(error) = result { self.last_error = Some(error); }
                        self.scan_stage = "fault";
                        self.scan_prompt = None;
                        self.fault_latched = true;
                        self.preflight = false;
                        self.homed = false;
                        self.phase = Phase::Fault;
                    }
                }
            }
        }
    }

    fn stop_real_scan(&mut self) -> Result<(), &'static str> {
        let Some(handle) = self.real_scan.as_ref() else { return Ok(()); };
        let result = handle.request_stop();
        self.phase = Phase::Stopping;
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.real_scan.is_some() && Instant::now() < deadline {
            self.sync_real_scan();
            if self.real_scan.is_some() { thread::sleep(Duration::from_millis(25)); }
        }
        if let Err(error) = result { return Err(self.nano_error("Scan STOP failed", error)); }
        if self.real_scan.is_some() { return Err("SCAN_CLEANUP_PENDING"); }
        if self.phase == Phase::Fault { return Err("SCAN_STOP_FAILED"); }
        Ok(())
    }

    fn invalidate(&mut self, phase: Phase) {
        self.beam_on = false;
        self.preflight = false;
        self.homed = false;
        self.phase = phase;
        self.restored_frames.clear();
        self.restored_scan = None;
        self.scan_prompt = None;
    }

    fn nano_error(&mut self, context: &str, error: impl std::fmt::Display) -> &'static str {
        let message = format!("{context}: {error}");
        self.last_error = Some(message.clone());
        self.connected = false;
        self.home_performed_this_process = false;
        self.fault_latched = true;
        self.invalidate(Phase::Fault);
        self.log("ERR", "nano", &format!("{message} · STOP/OFF recovery required"));
        "NANO_COMMUNICATION_FAILED"
    }

    fn force_xray_off(&mut self, context: &str) -> Result<(), &'static str> {
        if self.real_scan.is_some() { return Err("SCAN_CLEANUP_PENDING"); }
        if self.xray.is_none() && self.cached_xray_health.as_ref().is_some_and(|h| !h.beam_off_confirmed) {
            return Err("XRAY_OFF_UNCONFIRMED");
        }
        if let Some(xray) = self.xray.as_mut() {
            if let Err(error) = xray.force_off() {
                let message = format!("{context}: Moxtek OFF was not confirmed: {error}");
                self.last_error = Some(message.clone());
                self.fault_latched = true;
                self.invalidate(Phase::Fault);
                self.log("ERR", "xray", &message);
                return Err("XRAY_OFF_UNCONFIRMED");
            }
        }
        if let Some(nano) = self.nano.as_ref() {
            if let Err(error) = nano.set_xray_warning(false) {
                let message = format!("{context}: Nano XRAY_WARNING OFF was not confirmed: {error}");
                self.last_error = Some(message.clone());
                self.connected = false;
                self.fault_latched = true;
                self.invalidate(Phase::Fault);
                self.log("ERR", "nano", &message);
                return Err("NANO_COMMUNICATION_FAILED");
            }
        }
        self.beam_on = false;
        Ok(())
    }

    /// Polls the Moxtek for its live beam state (throttled, production only).
    /// Active emission is checked more frequently and any safety failure is
    /// latched after the device module has attempted an immediate OFF.
    fn sync_xray_live_status(&mut self) {
        if self.preview {
            return;
        }
        let Some(xray) = self.xray.as_ref() else {
            return;
        };
        let health = xray.health();
        let result = match health.last_error.as_ref() { Some(error) => Err(error.clone()), None=>Ok(health.clone()) };
        match result {
            Ok(health) => {
                if let Some(warning) = health.telemetry_warning.as_deref() {
                    self.log("WARN", "xray", warning);
                }
                if health.beam_on != self.beam_on {
                    self.log(
                        "WARN",
                        "xray",
                        if health.beam_on {
                            "Beam state changed externally · output is ON"
                        } else {
                            "Beam state changed externally · output is OFF"
                        },
                    );
                }
                self.beam_on = health.beam_on;
                self.usb_auto_shut_down = health
                    .usb_auto_shutdown
                    .unwrap_or(self.usb_auto_shut_down);
                self.cached_xray_health = Some(health);
            }
            Err(error) => {
                self.beam_on = health.beam_on;
                self.usb_auto_shut_down = health
                    .usb_auto_shutdown
                    .unwrap_or(self.usb_auto_shut_down);
                self.cached_xray_health = Some(health.clone());
                if health.last_error.is_some() || !health.beam_off_confirmed {
                    let newly_faulted=!self.fault_latched;
                    self.fault_latched = true;
                    self.invalidate(Phase::Fault);
                    if newly_faulted {let _=self.enqueue_stop_for_shutdown();self.phase=Phase::Fault;}
                }
                self.log(
                    "ERR",
                    "xray",
                    &format!("Moxtek status poll failed: {error} · X-ray controls unavailable"),
                );
            }
        }
    }

    /// Deterministic shutdown used when the control channel closes (window
    /// closed, shell crashed or killed): stop any scan, then force the Moxtek
    /// OFF and release every device before the process exits.
    pub fn shutdown(&mut self) -> Result<(), String> {
        self.shutdown_with_timeout(Duration::from_secs(65))
    }
    fn shutdown_with_timeout(&mut self, timeout: Duration) -> Result<(), String> {
        if self.preview {self.beam_on=false;return Ok(());}
        let deadline=Instant::now()+timeout;
        let previous_error=self.last_error.clone();
        let xray_was_managed=self.ever_xray || self.xray.is_some() || self.cached_xray_health.is_some();
        // A missing owner cannot erase previously unconfirmed device evidence.
        let orphaned_xray=(self.xray.is_none()&&self.real_scan.is_none()).then(||self.cached_xray_health.clone()).flatten();
        let orphaned_camera=(self.camera.is_none()&&self.real_scan.is_none()).then(||self.cached_camera_health.clone()).flatten();
        let camera_session=self.camera.as_ref().and_then(|camera|camera.health().session_id);
        let previous_operations=self.operations.history.iter().map(|op|op.operation_id.clone()).collect::<std::collections::HashSet<_>>();
        let mut errors=Vec::new();
        let mut stop_id=match self.enqueue_stop_for_shutdown() {
            Ok(())=>self.operations.last_id.clone(),
            Err(error)=>{errors.push(format!("Shutdown STOP admission failed: {error}"));None}
        };
        let mut camera_was_pending=false;
        let mut retried=false;
        while Instant::now()<deadline {
            self.tick();
            let live_camera=self.camera.as_ref().map(|camera|camera.health());
            let camera_pending=live_camera.as_ref().is_some_and(|health|!health.cleanup_confirmed);
            camera_was_pending|=camera_pending;
            let stop_failed=stop_id.as_ref().and_then(|id|self.operations.history.iter().find(|op|op.operation_id==*id))
                .is_some_and(|op|matches!(op.status.as_str(),"failed"|"cleanup_unconfirmed"|"cancelled"));
            // The camera owner may confirm C102 after the six-second domain
            // Stop deadline. One new safety ticket must confirm the current
            // OFF/Nano STOP/cleanup; an old failed ticket never becomes success.
            let late_camera_confirmed=camera_was_pending && camera_session.is_some()
                && live_camera.as_ref().is_some_and(|health|health.cleanup_confirmed&&health.session_id==camera_session);
            if !retried && stop_failed && late_camera_confirmed && self.real_scan.is_none() && !self.operations.blocks_controls() {
                retried=true;
                eprintln!("ct-engine shutdown: late camera cleanup confirmed; retrying safety STOP once after ticket {:?}",stop_id);
                match self.enqueue_stop_for_shutdown() {
                    Ok(())=>{stop_id=self.operations.last_id.clone();continue;}
                    Err(error)=>errors.push(format!("Shutdown STOP retry admission failed: {error}")),
                }
            }
            if self.real_scan.is_none() && !self.operations.blocks_controls() && !camera_pending {break;}
            thread::sleep(Duration::from_millis(20));
        }
        self.tick();
        if self.camera.is_none() && self.cached_camera_health.is_none(){self.cached_camera_health=orphaned_camera.clone();}
        if self.xray.is_none() && self.cached_xray_health.is_none(){self.cached_xray_health=orphaned_xray.clone();}
        let stop_confirmed=stop_id.as_ref().and_then(|id|self.operations.history.iter().find(|op|op.operation_id==*id))
            .is_some_and(|op|op.device=="safety"&&op.kind=="stop"&&op.status=="completed");
        if !stop_confirmed {
            let evidence=stop_id.as_ref().and_then(|id|self.operations.history.iter().find(|op|op.operation_id==*id))
                .map(|op|format!("{}: {}{}",op.operation_id,op.status,op.error.as_ref().map(|error|format!("; {error}")).unwrap_or_default()))
                .unwrap_or_else(||"no matching shutdown STOP ticket".into());
            errors.push(format!("Shutdown STOP unconfirmed: {evidence}"));
            for operation in self.operations.history.iter().filter(|op|op.device=="safety"&&op.kind=="stop"&&!previous_operations.contains(&op.operation_id)) {
                if let Some(error)=operation.error.as_ref() {errors.push(format!("Shutdown safety result {}: {error}",operation.operation_id));}
            }
        }
        let mut camera_clean=self.camera.as_ref().is_none_or(|camera|camera.health().cleanup_confirmed)
            && self.cached_camera_health.as_ref().is_none_or(|health|health.cleanup_confirmed)
            && orphaned_camera.as_ref().is_none_or(|health|health.cleanup_confirmed);
        if camera_clean {
            if let Some(camera)=self.camera.as_ref() {
                match camera.disconnect_confirmed() {
                    Ok(health) if health.cleanup_confirmed=>{self.cached_camera_health=Some(health);self.camera=None;}
                    Ok(health)=>{camera_clean=false;self.cached_camera_health=Some(health);errors.push("Camera disconnect cleanup unconfirmed".into());}
                    Err(error)=>{camera_clean=false;errors.push(format!("Camera disconnect cleanup unconfirmed: {error}"));}
                }
            }
        } else {errors.push("Camera cleanup unconfirmed".into());}
        if let Some(xray)=self.xray.as_mut() {
            let result=xray.disconnect();
            self.cached_xray_health=Some(xray.health());
            match result {Ok(())=>self.xray=None,Err(error)=>errors.push(format!("X-ray disconnect/OFF unconfirmed: {error}"))}
        }
        let off=self.xray.as_ref().is_none_or(|xray|xray.health().beam_off_confirmed)
            && self.cached_xray_health.as_ref().is_none_or(|health|health.beam_off_confirmed)
            && orphaned_xray.as_ref().is_none_or(|health|health.beam_off_confirmed)
            && (!xray_was_managed || self.xray.is_some() || self.cached_xray_health.is_some());
        if !off {errors.push("OFF unconfirmed".into());}else{self.beam_on=false;}
        let unresolved=self.real_scan.is_some() || self.operations.blocks_controls() || self.manual_lock.is_some();
        if unresolved {errors.push(format!("Shutdown cleanup unfinished: scan={}, operations={}, lock={:?}",self.real_scan.is_some(),self.operations.blocks_controls(),self.manual_lock));}
        if stop_confirmed {self.nano=None;}
        eprintln!("ct-engine shutdown: OFF required={}, OFF confirmed={:?}, camera cleanup={}, STOP confirmed={}, unresolved cleanup={}",xray_was_managed,xray_was_managed.then_some(off),camera_clean,stop_confirmed,unresolved);
        if errors.is_empty(){return Ok(());}
        if let Some(error)=previous_error {errors.insert(0,format!("Previous failure: {error}"));}
        let error=errors.join("; ");
        self.last_error=Some(error.clone());self.fault_latched=true;self.phase=Phase::Fault;
        eprintln!("ct-engine shutdown failed: {error}");
        Err(error)
    }
    fn sync_nano_health(&mut self) {
        let Some(nano) = self.nano.as_ref() else {
            return;
        };
        let health = nano.health();
        if !matches!(health.state, NanoConnectionState::Connected) {
            let newly_faulted=!self.fault_latched;
            let detail = health
                .last_error
                .unwrap_or_else(|| format!("Nano state {:?}", health.state));
            self.last_error = Some(detail.clone());
            self.connected = false;
            self.home_performed_this_process = false;
            self.fault_latched = true;
            self.invalidate(Phase::Fault);
            if newly_faulted {let _=self.enqueue_stop_for_shutdown();self.phase=Phase::Fault;}
        }
    }

    fn command(&mut self, command: &str, payload: Value) -> Result<(), &'static str> {
        if command=="prepare_software_update" { return self.prepare_software_update(); }
        if command=="cancel_software_update_prepare" { return self.cancel_software_update_prepare(&payload); }
        if self.maintenance_lock && !matches!(command,"snapshot"|"stop"|"xray_off"|"camera_status"|"camera_identity"|"turntable_status"|"turntable_identity"|"xray_status"|"xray_identity") { return Err("SOFTWARE_UPDATE_MAINTENANCE_LOCKED"); }
        if command=="prepare_scan" { return self.prepare_scan(&payload); }
        if command=="set_capture_root" { return self.set_capture_root(&payload); }
        if command=="camera_capture_descriptor" { return if self.captures.iter().any(|capture|Some(capture.capture_id.as_str())==payload.get("captureId").and_then(Value::as_str)){Ok(())}else{Err("CAPTURE_NOT_REGISTERED")}; }

        if !self.preview {
            if let Some(result) = self.device_command(command, payload.clone()) { return result; }
        }
        self.command_core(command,payload)
    }

    fn command_core(&mut self, command: &str, payload: Value) -> Result<(), &'static str> {
        if command == "snapshot" {
            return Ok(());
        }
        if command == "stop" {
            if !matches!(self.phase, Phase::Running | Phase::Paused | Phase::Finishing) { return Ok(()); }
            if let Some(handle) = self.real_scan.as_ref() {
                let result = handle.request_stop();
                self.phase = Phase::Stopping;
                if let Err(error) = result { return Err(self.nano_error("Scan STOP failed", error)); }
                self.log("ACTION", "operator", "Ending scan; waiting for capture cleanup and confirmed output OFF");
                return Ok(());
            }
            self.force_xray_off("End scan")?;
            self.invalidate(Phase::Stopped);
            self.last_error = None;
            self.log("INFO", "system", "Preview scan ended; saved progress retained; repeat Preflight and HOME");
            return Ok(());
        }
        if command == "disconnect" {
            let _ = self.stop_real_scan();
            if self.real_scan.is_some() {
                let deadline = Instant::now() + Duration::from_secs(70);
                while self.real_scan.is_some() && Instant::now() < deadline {
                    self.sync_real_scan();
                    thread::sleep(Duration::from_millis(50));
                }
            }
            if self.real_scan.is_some() {
                return Err("SCAN_SHUTDOWN_TIMEOUT");
            }
            let mut xray_off = self.force_xray_off("Disconnect");
            let mut failed_xray_health = None;
            if let Some(mut xray) = self.xray.take() {
                if let Err(error) = xray.disconnect() {
                    let message = format!("Disconnect cleanup could not reconfirm Moxtek OFF: {error}");
                    failed_xray_health = Some(xray.health());
                    self.last_error = Some(message.clone());
                    self.fault_latched = true;
                    self.log("ERR", "xray", &message);
                    if xray_off.is_ok() { xray_off = Err("XRAY_DISCONNECT_FAILED"); }
                }
            }
            if let Some(nano) = self.nano.take() {
                if let Ok(mut nano) = Arc::try_unwrap(nano) {
                    nano.disconnect();
                }
            }
            if let Some(camera) = self.camera.take() {
                camera.disconnect();
            }
            self.cached_camera_health = None;
            self.cached_xray_health = failed_xray_health;
            self.set_kv_confirmed = false;
            self.set_ua_confirmed = false;
            self.usb_auto_shut_down = true;
            self.usb_auto_shut_down_confirmed = false;
            self.connected = false;
            self.home_performed_this_process = false;
            self.invalidate(if xray_off.is_ok() { Phase::Idle } else { Phase::Fault });
            self.log("WARN", "system", "Engine disconnected · Moxtek OFF, warning OFF and Nano STOP attempted · safety conditions invalidated");
            return xray_off;
        }
        match command {
            "connect" => {
                if self.connected || self.busy() {
                    return Err("ALREADY_CONNECTED");
                }
                match payload.get("adapter").and_then(Value::as_str) {
                    Some("developer_preview") if self.preview => {
                        self.invalidate(Phase::Idle);
                        self.connected = true;
                        self.fault_latched = false;
                        self.last_error = None;
                        self.log("INFO", "system", "DEVELOPER PREVIEW connected · NO REAL HARDWARE");
                    }
                    Some("real_hardware") if !self.preview => {
                        let mut nano = NanoAdapter::new();
                        let identity = nano
                            .discover_and_connect()
                            .map_err(|error| self.nano_error("Nano auto-identification failed", error))?;
                        let status = nano
                            .status()
                            .map_err(|error| self.nano_error("Nano initial STATUS failed", error))?;
                        self.nano = Some(Arc::new(nano));
                        self.connected = true;
                        self.home_performed_this_process = false;
                        self.invalidate(Phase::Idle);
                        self.fault_latched = false;
                        self.last_error = None;
                        self.log(
                            "INFO",
                            "nano",
                            &format!(
                                "{} v{} build {} connected · state={} · {}",
                                identity.device, identity.version, identity.build, status.state,
                                if nano_reference_ready(&status) { "live reference available for Preflight" }
                                else { "HOME required if reference remains invalid" }
                            ),
                        );
                    }
                    Some("real_hardware") => return Err("PREVIEW_ADAPTER_MISMATCH"),
                    Some("developer_preview") => return Err("PRODUCTION_LOCKED"),
                    _ => return Err("INVALID_ADAPTER"),
                }
            }
            "set_parameters" => {
                if !self.preview {
                    return Err("STAGE_1_NANO_ONLY");
                }
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                let parameters: Parameters = serde_json::from_value(
                    payload.get("parameters").cloned().ok_or("INVALID_PARAMETERS")?,
                )
                .map_err(|_| "INVALID_PARAMETERS")?;
                parameters.validate()?;
                self.parameters = parameters;
                self.current = 0;
                self.scan_geometry = None;
                self.scan_references = ReferenceFrames::default();
                self.scan_stage = "idle";
                self.reconstruction_method = None;
                self.reconstruction_status = "idle";
                self.reconstruction_percent = 0;
                self.reconstruction_message.clear();
                self.reconstruction_cache_path = None;
                self.invalidate(Phase::Idle);
                self.log("INFO", "system", "Scan parameters updated · safety checks invalidated");
            }
            "update_scan_setup" => {
                if self.task_registry.locked(){return Err("UNFINISHED_TASK_LOCKED");}
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                let setup: ScanSetupInput = serde_json::from_value(
                    payload.get("setup").cloned().ok_or("INVALID_PARAMETERS")?,
                )
                .map_err(|_| "INVALID_PARAMETERS")?;
                let (parameters, max_xray_sec) =
                    setup.apply(&self.parameters, self.max_xray_sec)?;
                if setup.exposure_ms.is_some() && !self.preview { return Err("EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY"); }
                if setup.exposure_ms.is_some() {
                    let health = self.camera.as_ref().map(CameraAdapter::health)
                        .or_else(|| self.cached_camera_health.clone());
                    if let Some(health) = health.filter(|health| health.connected) {
                        if let Some((min, max)) = health.exposure_min_ms.zip(health.exposure_max_ms) {
                            if !(min..=max).contains(&parameters.exposure_ms) {
                                return Err("EXPOSURE_OUTSIDE_CAMERA_RANGE");
                            }
                        }
                    }
                }
                if setup.exposure_ms.is_some() {
                    if let Some(camera)=self.camera.as_ref().filter(|camera|camera.health().connected) {
                        camera.set_exposure_target(parameters.exposure_ms).map_err(|_|"EXPOSURE_NOT_NATIVE_CAMERA_VALUE")?;
                    }
                }
                self.parameters = parameters;
                self.max_xray_sec = max_xray_sec;
                self.schedule_metadata_probe();
                self.current = 0;
                self.restored_scan = None;
                self.scan_geometry = None;
                self.scan_references = ReferenceFrames::default();
                self.scan_prompt = None;
                self.scan_stage = "idle";
                self.reconstruction_method = None;
                self.reconstruction_status = "idle";
                self.reconstruction_percent = 0;
                self.reconstruction_message.clear();
                self.reconstruction_cache_path = None;
                self.invalidate(Phase::Idle);
                self.log("INFO", "system", "Scan setup updated · repeat preflight and HOME");
            }
            "preflight" => {
                self.require_connected()?;
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                if !self.preview {
                    self.parameters.validate()?;
                    if scan::manifest_file_exists(&self.parameters)
                        && !scan::resume_manifest_exists(&self.parameters) {
                        return Err("SCAN_OUTPUT_EXISTS_USE_NEW_TASK");
                    }
                    if !(1..=600).contains(&self.max_xray_sec) {
                        return Err("INVALID_PARAMETERS");
                    }
                    if !self
                        .camera
                        .as_ref()
                        .is_some_and(|camera| camera.health().connected)
                    {
                        let camera = CameraAdapter::discover()
                            .map_err(|_| "CAMERA_BACKEND_UNAVAILABLE")?;
                        let health = match camera.connect() {
                            Ok(health) => health,
                            Err(error) => {
                                let message = format!("D7100 auto-connect failed: {error}");
                                self.last_error = Some(message.clone());
                                self.log("ERR", "camera", &message);
                                return Err("CAMERA_CONNECTION_FAILED");
                            }
                        };
                        self.cached_camera_health = Some(health.clone());
                        self.camera = Some(camera);
                        self.log(
                            "PASS",
                            "camera",
                            &format!(
                                "D7100 auto-connected · serial={} · {}",
                                health.serial.as_deref().unwrap_or("unknown"),
                                health.transfer_policy.as_deref().unwrap_or("unverified")
                            ),
                        );
                    }
                    if self.xray.is_none() {
                        let mut xray = MoxtekAdapter::new();
                        let health = match xray.discover_and_connect() {
                            Ok(health) => health,
                            Err(error) => {
                                let message = format!("Moxtek auto-connect failed: {error}");
                                self.last_error = Some(message.clone());
                                self.log("ERR", "xray", &message);
                                return Err("XRAY_CONNECTION_FAILED");
                            }
                        };
                        self.usb_auto_shut_down = true;
                        self.usb_auto_shut_down_confirmed = false;
                        self.cached_xray_health = Some(health.clone());
                        self.xray = Some(xray);
                        if health.beam_on {
                            self.log(
                                "WARN",
                                "xray",
                                &format!(
                                    "Moxtek auto-connected · port={} · BEAM WAS ON from a previous session · Preflight will force it OFF",
                                    health.port.as_deref().unwrap_or("unknown")
                                ),
                            );
                        } else {
                            self.log(
                                "PASS",
                                "xray",
                                &format!(
                                    "Moxtek auto-connected SAFE OFF · port={} · serial={}",
                                    health.port.as_deref().unwrap_or("unknown"),
                                    health.serial.as_deref().unwrap_or("unknown")
                                ),
                            );
                        }
                    }
                    if !self.usb_auto_shut_down_confirmed || self.usb_auto_shut_down {
                        self.log(
                            "WARN",
                            "xray",
                            "CT scan requires the operator to disable USB Auto Shut Down in the X-ray panel",
                        );
                        return Err("USB_AUTO_SHUTDOWN_RELEASE_REQUIRED");
                    }
                    if !self.set_kv_confirmed || !self.set_ua_confirmed {
                        self.log(
                            "WARN",
                            "xray",
                            "Setpoint confirmation required · click SEND V and SEND I before Preflight",
                        );
                        return Err("XRAY_SETPOINT_CONFIRMATION_REQUIRED");
                    }
                    self.force_xray_off("Preflight")?;
                    let xray_result = self
                        .xray
                        .as_mut()
                        .ok_or("XRAY_NOT_CONNECTED")?
                        .set_parameters(self.set_kv, self.set_ua);
                    let xray_health = match xray_result {
                        Ok(health) => health,
                        Err(error) => {
                            let message = format!("Moxtek preflight failed: {error}");
                            self.last_error = Some(message.clone());
                            self.log("ERR", "xray", &message);
                            return Err("XRAY_PREFLIGHT_FAILED");
                        }
                    };
                    self.cached_xray_health = Some(xray_health.clone());
                    if xray_health.locked != Some(false)
                        || !xray_health.beam_off_confirmed
                        || xray_health
                            .temperature_c
                            .is_none_or(|temperature| !temperature.is_finite() || temperature > 65.0)
                    {
                        return Err("XRAY_PREFLIGHT_FAILED");
                    }
                    let nano = self.nano.as_ref().ok_or("NANO_NOT_CONNECTED")?.clone();
                    let nano_status = nano
                        .status()
                        .map_err(|error| self.nano_error("Nano preflight STATUS failed", error))?;
                    let reusable_reference = may_reuse_home(self.home_performed_this_process, &nano_status);
                    if nano_status.state == "FAULT" {
                        nano.clear_fault()
                            .map_err(|error| self.nano_error("Nano CLEAR_FAULT failed", error))?;
                        self.log("WARN", "nano", "Previous Nano fault cleared · reference remains invalid until HOME");
                    }
                    let status = if reusable_reference {
                        nano_status
                    } else {
                        nano.rearm()
                            .map_err(|error| self.nano_error("Nano REARM failed", error))?
                    };
                    self.fault_latched = false;
                    self.preflight = true;
                    self.scan_geometry = None;
                    self.scan_references = ReferenceFrames::default();
                    let restoring = scan::resume_manifest_exists(&self.parameters);
                    self.scan_prompt = if restoring { None } else { Some("geometry") };
                    self.scan_stage = if restoring { "idle" } else { "geometry" };
                    self.homed = reusable_reference;
                    self.phase = if reusable_reference { Phase::Ready } else { Phase::ReadyForHome };
                    self.last_error = None;
                    self.log(
                        "PASS",
                        "preflight",
                        &format!(
                            "8/8 real checks passed · Nano={} · reference={} · D7100 host-only · Moxtek OFF/unlocked {:.1} kV / {:.1} µA · {:.1} C",
                            status.state,
                            if reusable_reference { "live STATUS confirmed; HOME reused" } else { "HOME required" },
                            self.set_kv,
                            self.set_ua,
                            xray_health.temperature_c.unwrap_or_default()
                        ),
                    );
                } else {
                    self.parameters.validate()?;
                    if !(1..=600).contains(&self.max_xray_sec) {
                        return Err("INVALID_PARAMETERS");
                    }
                    self.fault_latched = false;
                    self.preflight = true;
                    self.scan_references = ReferenceFrames::default();
                    self.scan_prompt = if self.scan_geometry.is_some(){None}else{Some("geometry")};
                    self.scan_stage = if self.scan_geometry.is_some(){"idle"}else{"geometry"};
                    self.homed = false;
                    self.phase = Phase::ReadyForHome;
                    self.log("PASS", "preflight", "8/8 preview checks passed · real interlocks unverified");
                }
            }
            "home" => {
                self.require_connected()?;
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                if !self.preflight {
                    return Err("PREFLIGHT_REQUIRED");
                }
                if self.fault_latched {
                    return Err("FAULT_RECOVERY_REQUIRED");
                }
                if !self.preview {
                    self.force_xray_off("Before HOME")?;
                    let nano = self.nano.as_ref().ok_or("NANO_NOT_CONNECTED")?;
                    let status = nano
                        .home()
                        .map_err(|error| self.nano_error("Nano HOME failed", error))?;
                    self.homed = true;
                    self.home_performed_this_process = true;
                    self.current = 0;
                    self.real_frames.clear();
                    self.restored_frames.clear();
                    self.phase = Phase::Ready;
                    self.last_error = None;
                    self.log(
                        "INFO",
                        "nano",
                        &format!(
                            "Nano HOME verified · pos={} pulses · reference valid",
                            status.position_pulses
                        ),
                    );
                } else {
                    self.homed = true;
                    self.current = 0;
                    self.phase = Phase::Ready;
                    self.log("INFO", "nano", "Preview HOME complete · 0.00°");
                }
            }
            "confirm_scan_stage" => {
                let stage = payload.get("stage").and_then(Value::as_str).ok_or("INVALID_SCAN_STAGE")?;
                if stage == "geometry" {
                    if self.scan_prompt != Some("geometry") || !self.preflight { return Err("SCAN_PROMPT_MISMATCH"); }
                    let geometry: ScanGeometry = serde_json::from_value(payload.get("geometry").cloned()
                        .ok_or("INVALID_SCAN_GEOMETRY")?).map_err(|_| "INVALID_SCAN_GEOMETRY")?;
                    geometry.validate()?;
                    if geometry.rotation_direction != "counterclockwise" || geometry.measurement != "measured" {
                        return Err("INVALID_SCAN_GEOMETRY");
                    }
                    self.scan_geometry = Some(geometry);
                    self.scan_prompt = None;
                    self.scan_stage = "idle";
                    return Ok(());
                }
                if !matches!(stage, "preReferences" | "placeSample") {
                    return Err("INVALID_SCAN_STAGE");
                }
                self.real_scan.as_ref().ok_or("SCAN_PROMPT_MISMATCH")?.confirm_stage(stage)?;
                self.scan_prompt = None;
                return Ok(());
            }
            "cancel_scan_stage" => {
                let stage = payload.get("stage").and_then(Value::as_str).ok_or("INVALID_SCAN_STAGE")?;
                if self.scan_prompt != Some(stage) { return Err("SCAN_PROMPT_MISMATCH"); }
                if let Some(scan) = self.real_scan.as_ref() {
                    scan.request_stop().map_err(|error| self.nano_error("Scan cancel STOP failed", error))?;
                    self.phase = Phase::Stopping;
                } else {
                    self.enqueue_checkpoint()?;
                }
                self.scan_prompt = None;
                return Ok(());
            }
            "start_reconstruction" => {
                if self.real_scan.is_some() || self.busy() {
                    return Err("RECONSTRUCTION_LOCKED");
                }
                if self.reconstruction.is_some() { return Err("RECONSTRUCTION_ACTIVE"); }
                let method = match payload.get("method").and_then(Value::as_str) {
                    Some("fdk") => ReconstructionMethod::Fdk,
                    Some("sirt") => ReconstructionMethod::Sirt,
                    Some("cgls") => ReconstructionMethod::Cgls,
                    _ => return Err("INVALID_RECONSTRUCTION_METHOD"),
                };
                if !reconstruction::available_methods().iter().any(|entry| entry.method == method && entry.enabled) {
                    return Err("RECONSTRUCTION_METHOD_UNAVAILABLE");
                }
                if self.parameters.projection_count < 3 { return Err("RECONSTRUCTION_METHOD_UNAVAILABLE"); }
                if method == ReconstructionMethod::Sirt
                    && !reconstruction::sirt_feasible(self.parameters.projection_count) {
                    return Err("RECONSTRUCTION_METHOD_UNAVAILABLE");
                }
                let scan_dir = Path::new(self.parameters.save_path.trim()).join(self.parameters.task_id.trim());
                if !scan::completed_manifest_exists(&self.parameters) {
                    return Err("RECONSTRUCTION_LOCKED");
                }
                self.reconstruction_method = Some(method);
                match reconstruction::cached_result(&scan_dir, method) {
                    Ok(Some(result)) => {
                        self.reconstruction_status = "completed";
                        self.reconstruction_percent = 100;
                        self.reconstruction_message = "重构已缓存".into();
                        self.reconstruction_cache_path = Some(result.cache_path.display().to_string());
                        return Ok(());
                    }
                    Err(error) => {
                        self.last_error = Some(error);
                        return Err("RECONSTRUCTION_CACHE_INVALID");
                    }
                    Ok(None) => {}
                }
                let handle = ReconstructionHandle::start(ReconstructionRequest { scan_dir, method })
                    .map_err(|error| { self.last_error = Some(error); "RECONSTRUCTION_START_FAILED" })?;
                self.reconstruction = Some(handle);
                self.reconstruction_status = "running";
                self.reconstruction_percent = 0;
                self.reconstruction_message = "准备重构".into();
                self.reconstruction_cache_path = None;
                return Ok(());
            }
            "start_scan" => {
                if !self.preview {
                    self.require_connected()?;
                    if self.busy() {
                        return Err("SCAN_ACTIVE");
                    }
                    if !self.preflight {
                        return Err("PREFLIGHT_REQUIRED");
                    }
                    if !self.homed {
                        return Err("HOME_REQUIRED");
                    }
                    if self.scan_prompt.is_some() || self.scan_geometry.is_none() {
                        return Err("SCAN_GEOMETRY_REQUIRED");
                    }
                    if self.fault_latched {
                        return Err("FAULT_RECOVERY_REQUIRED");
                    }
                    self.verify_current_nano_reference()?;
                    if !self.usb_auto_shut_down_confirmed || self.usb_auto_shut_down {
                        return Err("USB_AUTO_SHUTDOWN_RELEASE_REQUIRED");
                    }
                    self.parameters.validate()?;
                    // The confirmed setpoint is the verified readback, which can sit a
                    // few counts below the nominal minimum (hardware: 3.99971 kV for a
                    // 4.000 kV request). Admission must accept the operator's own
                    // confirmed minimum; nominal_voltage() only restores that bound.
                    self.validate_setpoint(crate::devices::xray::nominal_voltage(self.set_kv), self.set_ua)?;
                    let starting_new_scan = self.restored_scan.is_none();
                    self.scan_stop_requested=false;
                    self.metadata_generation+=1;
                    let nano = self.nano.as_ref().ok_or("NANO_NOT_CONNECTED")?.clone();
                    let camera = self.camera.as_ref().cloned().ok_or("CAMERA_NOT_CONNECTED")?;
                    let xray = match self.xray.as_ref().cloned() {
                        Some(xray) => xray,
                        None => {
                            return Err("XRAY_NOT_CONNECTED");
                        }
                    };
                    self.cached_camera_health = Some(camera.health());
                    self.cached_xray_health = Some(xray.health());
                    self.current = self.restored_frames.len() as u32;
                    self.restored_frames.clear();
                    self.scan_angle_deg = Some(0.0);
                    if starting_new_scan { self.real_frames.clear(); }
                    self.real_scan_revision = 0;
                    self.real_scan_message_count = 0;
                    self.scan_stage = "preDark";
                    self.phase = Phase::Running;
                    self.beam_on = false;
                    self.last_error = None;
                    self.real_scan = Some(RealScanHandle::start(
                        nano,
                        camera,
                        xray,
                        RealScanConfig {
                            capture_profile: self.capture_profile.clone().ok_or("CAPTURE_PROFILE_REQUIRED")?,
                            parameters: self.parameters.clone(),
                            max_xray_sec: self.max_xray_sec,
                            voltage_kv: self.set_kv,
                            current_ua: self.set_ua,
                            geometry: self.scan_geometry.clone().ok_or("SCAN_GEOMETRY_REQUIRED")?,
                            resume: self.restored_scan.take(),
                        },
                    ));
                    self.log("ACTION", "operator", &format!(
                        "Real scan worker started at view {} · output remains OFF until READY_TO_CAPTURE",
                        self.current + 1
                    ));
                    return Ok(());
                }
                self.require_connected()?;
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                if !self.preflight {
                    return Err("PREFLIGHT_REQUIRED");
                }
                if !self.homed {
                    return Err("HOME_REQUIRED");
                }
                if self.fault_latched {
                    return Err("FAULT_RECOVERY_REQUIRED");
                }
                self.current = 0;
                self.scan_angle_deg = None;
                self.phase = Phase::Running;
                self.beam_on = true;
                self.last_tick = Instant::now();
                self.preview_view_ms = None;
                self.log("ACTION", "operator", "Developer preview scan started · no device output");
            }
            "pause" => {
                if self.phase != Phase::Running {
                    return Err("NOT_RUNNING");
                }
                if let Some(scan) = self.real_scan.as_ref() {
                    scan.request_pause();
                    self.log("ACTION", "operator", "Pause requested · takes effect after current projection closes");
                    return Ok(());
                }
                self.phase = Phase::Paused;
                self.beam_on = false;
                self.log("WARN", "system", "Preview paused · output visualization disabled");
            }
            "resume" => {
                self.require_connected()?;
                if self.fault_latched || self.phase == Phase::Fault {
                    return Err("FAULT_RECOVERY_REQUIRED");
                }
                if self.phase != Phase::Paused || !self.preflight || !self.homed {
                    return Err("NOT_RESUMABLE");
                }
                if let Some(scan) = self.real_scan.as_ref() {
                    scan.resume();
                    self.phase = Phase::Running;
                    self.log("ACTION", "operator", "Real scan resumed");
                    return Ok(());
                }
                self.phase = Phase::Running;
                self.beam_on = true;
                self.last_tick = Instant::now();
                self.log("INFO", "system", "Preview resumed");
            }
            "restore_previous" => {
                self.require_connected()?;
                if self.fault_latched || self.phase == Phase::Fault {
                    return Err("FAULT_RECOVERY_REQUIRED");
                }
                if self.phase == Phase::Stopped {
                    return Err("RECOVERY_REQUIRED");
                }
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                if !self.preflight {
                    return Err("PREFLIGHT_REQUIRED");
                }
                if !self.homed {
                    return Err("HOME_REQUIRED");
                }
                if !self.preview {
                    self.verify_current_nano_reference()?;
                    let restored = match scan::load_resume(&self.parameters, self.set_kv, self.set_ua,self.capture_profile.as_ref().ok_or("CAPTURE_PROFILE_REQUIRED")?) {
                        Ok(restored) => restored,
                        Err(error) => {
                            self.last_error = Some(error.clone());
                            self.log("ERR", "system", &format!("Scan Restore rejected: {error}"));
                            return Err("RESTORE_VALIDATION_FAILED");
                        }
                    };
                    self.current = restored.frames.len() as u32;
                    self.real_frames = restored.frames.iter()
                        .map(|frame| serde_json::to_value(frame).unwrap_or_else(|_| json!({})))
                        .collect();
                    self.scan_geometry = Some(restored.geometry.clone());
                    self.scan_references = restored.references.clone();
                    self.scan_stage = "idle";
                    self.scan_prompt = None;
                    self.restored_frames = restored.frames.clone();
                    self.restored_scan = Some(restored);
                    self.scan_angle_deg = Some(0.0);
                    self.phase = Phase::Ready;
                    self.last_error = None;
                    self.log("PASS", "system", &format!(
                        "Restore verified {} committed NEFs · live Nano reference confirmed · Start continues at view {}",
                        self.current, self.current + 1
                    ));
                    return Ok(());
                }
                self.current = (self.parameters.projection_count / 3).max(1);
                self.phase = Phase::Paused;
                self.log("WARN", "system", "Preview checkpoint restored · no real task file was read");
            }
            "retry_device" => {
                let device = payload.get("device").and_then(Value::as_str).ok_or("INVALID_DEVICE")?;
                if !matches!(device, "turntable" | "camera" | "xray") {
                    return Err("INVALID_DEVICE");
                }
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                if !self.preview {
                    match device {
                        "turntable" => {
                            let ping_result = self
                                .nano
                                .as_ref()
                                .ok_or("RECONNECT_REQUIRED")?
                                .ping();
                            if let Err(error) = ping_result {
                                return Err(self.nano_error("Nano PING failed", error));
                            }
                            let status_result = self
                                .nano
                                .as_ref()
                                .ok_or("RECONNECT_REQUIRED")?
                                .status();
                            let status = match status_result {
                                Ok(status) => status,
                                Err(error) => return Err(self.nano_error("Nano STATUS retry failed", error)),
                            };
                            self.last_error = None;
                            self.log(
                                "INFO",
                                "nano",
                                &format!("Nano link verified · state={}", status.state),
                            );
                        }
                        "camera" => {
                            let camera = CameraAdapter::discover()
                                .map_err(|_| "CAMERA_BACKEND_UNAVAILABLE")?;
                            let health = camera.connect().map_err(|_| "CAMERA_CONNECTION_FAILED")?;
                            self.cached_camera_health = Some(health.clone());
                            self.log(
                                "PASS",
                                "camera",
                                &format!(
                                    "D7100 connected · serial={} · transfer={}",
                                    health.serial.as_deref().unwrap_or("unknown"),
                                    health.transfer_policy.as_deref().unwrap_or("unverified")
                                ),
                            );
                            self.camera = Some(camera);
                            self.last_error = None;
                        }
                        "xray" => {
                            if self.xray.as_ref().is_some_and(|xray|
                                xray.health().connected && xray.health().beam_off_confirmed
                                    && !self.fault_latched) {
                                return Err("XRAY_ALREADY_CONNECTED");
                            }
                            if let Some(mut stale) = self.xray.take() {
                                if let Err(error) = stale.disconnect() {
                                    self.log("WARN", "xray", &format!(
                                        "Moxtek stale link cleanup could not confirm OFF: {error}; reconnect will read and force OFF"
                                    ));
                                }
                                self.cached_xray_health = None;
                            }
                            let mut xray = MoxtekAdapter::new();
                            let health = match xray.discover_and_connect() {
                                Ok(health) => health,
                                Err(error) => {
                                    let message = format!("Moxtek connection failed: {error}");
                                    self.last_error = Some(message.clone());
                                    self.log("ERR", "xray", &message);
                                    return Err("XRAY_CONNECTION_FAILED");
                                }
                            };
                            let shutdown_delay = xray.read_usb_shutdown_timer().ok();
                            self.xray = Some(xray);
                            self.cached_xray_health = Some(health.clone());
                            self.usb_auto_shut_down = true;
                            self.usb_auto_shut_down_confirmed = false;
                            self.set_kv_confirmed = false;
                            self.set_ua_confirmed = false;
                            if health.beam_on {
                                // A previous session left the tube emitting.
                                // Report it (the switch turns red) and let the
                                // operator explicitly disable the beam.
                                self.beam_on = true;
                                self.log(
                                    "WARN",
                                    "xray",
                                    &format!(
                                        "Moxtek 12 W connected · port={} · BEAM CURRENTLY ON from a previous session · click Xray Disable to stop output",
                                        health.port.as_deref().unwrap_or("unknown")
                                    ),
                                );
                            } else {
                                self.beam_on = false;
                                self.log(
                                    "PASS",
                                    "xray",
                                    &format!(
                                        "Moxtek 12 W connected · port={} · serial={} · beam OFF confirmed · USB auto-shutdown awaits operator selection (delay raw {:?})",
                                        health.port.as_deref().unwrap_or("unknown"),
                                        health.serial.as_deref().unwrap_or("unknown"),
                                        shutdown_delay
                                    ),
                                );
                            }
                            self.last_error = None;
                        }
                        _ => unreachable!(),
                    }
                } else {
                    self.require_connected()?;
                    self.log("INFO", "system", &format!("{device} preview link check complete"));
                }
            }
            "xray_disconnect" => {
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                if self.preview {
                    self.log("ACTION", "xray", "Preview X-ray disconnected · NO REAL HARDWARE");
                    return Ok(());
                }
                if self.beam_on {
                    return Err("XRAY_BEAM_ACTIVE");
                }
                let mut xray = self.xray.take().ok_or("XRAY_NOT_CONNECTED")?;
                let result = xray.disconnect();
                self.cached_xray_health = result.as_ref().err().map(|_| xray.health());
                self.set_kv_confirmed = false;
                self.set_ua_confirmed = false;
                self.usb_auto_shut_down = true;
                self.usb_auto_shut_down_confirmed = false;
                self.preflight = false;
                self.homed = false;
                result.map_err(|error| {
                    let message = format!("Moxtek disconnect failed: {error}");
                    self.last_error = Some(message.clone());
                    self.fault_latched = true;
                    self.invalidate(Phase::Fault);
                    self.log("ERR", "xray", &message);
                    "XRAY_DISCONNECT_FAILED"
                })?;
                self.beam_on = false;
                self.phase = Phase::Idle;
                self.last_error = None;
                self.log("PASS", "xray", "Moxtek OFF confirmed and manually disconnected");
            }
            "camera_test_capture" => return Err("REAL_CAMERA_REQUIRED"),
            "xray_toggle" => {
                if !self.preview {
                    if self.busy() {
                        return Err("SCAN_ACTIVE");
                    }
                    if self.phase == Phase::Fault || self.fault_latched {
                        return Err("FAULT_RECOVERY_REQUIRED");
                    }
                    if self.xray.is_none() {
                        return Err("XRAY_NOT_CONNECTED");
                    }
                    if self.beam_on {
                        // Turning output OFF is never locked: a detected
                        // leftover beam must always be killable.
                        self.force_xray_off("Manual X-ray disable")?;
                        self.cached_xray_health = self.xray.as_ref().map(MoxtekAdapter::health);
                        self.log("PASS", "xray", "Manual beam OFF confirmed · output disabled");
                        return Ok(());
                    }
                    let xray = self.xray.as_mut().ok_or("XRAY_NOT_CONNECTED")?;
                    if !self.set_kv_confirmed || !self.set_ua_confirmed {
                        self.log(
                            "WARN",
                            "xray",
                            "Setpoint confirmation required · click SEND V and SEND I before enabling the beam",
                        );
                        return Err("XRAY_SETPOINT_CONFIRMATION_REQUIRED");
                    }
                    let cancel = std::sync::atomic::AtomicBool::new(false);
                    match xray.beam_on(&cancel) {
                        Ok(health) => {
                            self.beam_on = true;
                            self.usb_auto_shut_down = health
                                .usb_auto_shutdown
                                .unwrap_or(true);
                            self.cached_xray_health = Some(health.clone());
                            self.last_error = None;
                            self.log(
                                "WARN",
                                "xray",
                                &format!(
                                    "MANUAL BEAM ON · {:.1} kV / {:.1} µA · click Xray Disable to stop output",
                                    self.set_kv, self.set_ua
                                ),
                            );
                        }
                        Err(error) => {
                            self.beam_on = false;
                            self.cached_xray_health = Some(xray.health());
                            let message = format!("Manual beam-on failed closed: {error}");
                            self.last_error = Some(message.clone());
                            self.log("ERR", "xray", &message);
                            return Err("XRAY_BEAM_ON_FAILED");
                        }
                    }
                    return Ok(());
                }
                self.require_connected()?;
                if self.phase == Phase::Fault || self.fault_latched {
                    return Err("FAULT_RECOVERY_REQUIRED");
                }
                self.beam_on = !self.beam_on;
                self.log("WARN", "xray", if self.beam_on { "Preview beam visualization enabled · NO REAL HARDWARE" } else { "Preview beam visualization disabled" });
            }
            "timer_toggle" => {
                if !self.preview {
                    return Err("XRAY_STAGE_LOCKED");
                }
                if self.phase == Phase::Running {
                    return Err("SCAN_ACTIVE");
                }
                self.timer_on = !self.timer_on;
                self.log("INFO", "xray", if self.timer_on { "Preview timer enabled" } else { "Preview timer disabled" });
            }
            "usb_auto_shut_down_toggle" => {
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                let new_state = !self.usb_auto_shut_down;
                if !self.preview {
                    if self.xray.is_none() {
                        return Err("XRAY_NOT_CONNECTED");
                    }
                    if new_state && self.beam_on {
                        // Re-arming the deadman while emitting: stop output
                        // first so the transition is deliberate, not a timeout.
                        self.force_xray_off("USB auto-shutdown re-arm")?;
                    }
                    if let Some(xray) = self.xray.as_mut() {
                        if let Err(error) = xray.set_usb_auto_shutdown(new_state) {
                            let message = format!("Moxtek USB auto-shutdown write failed: {error}");
                            self.last_error = Some(message.clone());
                            self.log("ERR", "xray", &message);
                            return Err("XRAY_CONFIG_FAILED");
                        }
                        self.cached_xray_health = Some(xray.health());
                    }
                }
                self.usb_auto_shut_down = new_state;
                self.usb_auto_shut_down_confirmed = true;
                self.log(
                    "INFO",
                    "xray",
                    if self.usb_auto_shut_down {
                        "USB Auto Shut Down ARMED · continuous output bounded by the device timer"
                    } else {
                        "USB Auto Shut Down RELEASED by operator · CT scan will not change it"
                    },
                );
            }
            "set_usb_shutdown_delay" => {
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                let delay_raw = payload
                    .get("delay")
                    .and_then(Value::as_u64)
                    .ok_or("INVALID_PARAMETERS")?;
                let delay = u16::try_from(delay_raw).map_err(|_| "INVALID_PARAMETERS")?;
                if delay == 0 {
                    return Err("INVALID_PARAMETERS");
                }
                if self.preview {
                    self.log("INFO", "xray", &format!("Preview USB shutdown delay set to {delay}"));
                    return Ok(());
                }
                let xray = self.xray.as_mut().ok_or("XRAY_NOT_CONNECTED")?;
                match xray.set_usb_shutdown_delay(delay) {
                    Ok(health) => {
                        let readback = health.usb_shutdown_delay.unwrap_or(u32::from(delay));
                        self.cached_xray_health = Some(health);
                        self.last_error = None;
                        self.log(
                            "PASS",
                            "xray",
                            &format!("USB shutdown delay written · requested {delay} · readback {readback}"),
                        );
                    }
                    Err(error) => {
                        let message = format!("USB shutdown delay write failed: {error}");
                        self.last_error = Some(message.clone());
                        self.log("ERR", "xray", &message);
                        return Err("XRAY_CONFIG_FAILED");
                    }
                }
            }
            "send_voltage" => {
                let kv = payload.get("kv").and_then(Value::as_f64).ok_or("INVALID_VOLTAGE")?;
                if !kv.is_finite() || !(MIN_VOLTAGE_KV..=MAX_VOLTAGE_KV).contains(&kv) {
                    return Err("INVALID_XRAY_SETPOINT");
                }
                let requested_ua = self.set_ua;
                let applied_ua =
                    requested_ua.min(MAX_SETPOINT_POWER_W * 1_000.0 / kv);
                let power_clamped = (applied_ua - requested_ua).abs() > 1e-12;
                self.validate_setpoint(kv, applied_ua)?;
                if power_clamped {
                    self.log(
                        "WARN",
                        "xray",
                        &format!(
                            "SEND V requested {kv:.3} kV · current auto-adjusted from {requested_ua:.3} µA to {applied_ua:.3} µA · {MAX_SETPOINT_POWER_W:.0} W limit"
                        ),
                    );
                }
                if !self.preview {
                    if self.beam_on {
                        return Err("XRAY_BEAM_ACTIVE");
                    }
                    let result = self
                        .xray
                        .as_mut()
                        .ok_or("XRAY_NOT_CONNECTED")?
                        .set_parameters(kv, applied_ua);
                    let health = match result {
                        Ok(health) => health,
                        Err(error) => {
                            let message = format!("Moxtek setpoint write failed: {error}");
                            self.last_error = Some(message.clone());
                            self.log("ERR", "xray", &message);
                            return Err("XRAY_SETPOINT_FAILED");
                        }
                    };
                    // set_parameters already verifies both readbacks and OFF.
                    // A second OFF transaction can fail after a successful
                    // write and make SEND V appear to have done nothing.
                    let accepted_kv = health.set_voltage_kv.unwrap_or(kv);
                    let accepted_ua = health.set_current_ua.unwrap_or(applied_ua);
                    self.set_kv = accepted_kv;
                    self.set_ua = accepted_ua;
                    self.set_kv_confirmed = true;
                    if power_clamped {
                        self.set_ua_confirmed = true;
                    }
                    self.cached_xray_health = Some(health.clone());
                    self.last_error = None;
                    self.log(
                        "PASS",
                        "xray",
                        &format!(
                            "SEND V applied and verified OFF · final {accepted_kv:.3} kV / {accepted_ua:.3} µA"
                        ),
                    );
                    return Ok(());
                }
                self.set_kv = kv;
                self.set_ua = applied_ua;
                self.log(
                    "INFO",
                    "xray",
                    &format!(
                        "Preview SEND V applied · final {kv:.3} kV / {applied_ua:.3} µA"
                    ),
                );
            }
            "send_current" => {
                let ua = payload.get("ua").and_then(Value::as_f64).ok_or("INVALID_CURRENT")?;
                if !ua.is_finite() || !(0.0..=MAX_CURRENT_UA).contains(&ua) {
                    return Err("INVALID_XRAY_SETPOINT");
                }
                let requested_kv = self.set_kv;
                let applied_kv = if ua > 0.0 {
                    requested_kv.min(MAX_SETPOINT_POWER_W * 1_000.0 / ua)
                } else {
                    requested_kv
                };
                // Same verified-readback rounding as the admission path above.
                let applied_kv = crate::devices::xray::nominal_voltage(applied_kv);
                let power_clamped = (applied_kv - requested_kv).abs() > 1e-12;
                self.validate_setpoint(applied_kv, ua)?;
                if power_clamped {
                    self.log(
                        "WARN",
                        "xray",
                        &format!(
                            "SEND I requested {ua:.3} µA · voltage auto-adjusted from {requested_kv:.3} kV to {applied_kv:.3} kV · {MAX_SETPOINT_POWER_W:.0} W limit"
                        ),
                    );
                }
                if !self.preview {
                    if self.beam_on {
                        return Err("XRAY_BEAM_ACTIVE");
                    }
                    let result = self
                        .xray
                        .as_mut()
                        .ok_or("XRAY_NOT_CONNECTED")?
                        .set_parameters(applied_kv, ua);
                    let health = match result {
                        Ok(health) => health,
                        Err(error) => {
                            let message = format!("Moxtek setpoint write failed: {error}");
                            self.last_error = Some(message.clone());
                            self.log("ERR", "xray", &message);
                            return Err("XRAY_SETPOINT_FAILED");
                        }
                    };
                    // The adapter has already confirmed OFF and both values.
                    let accepted_kv = health.set_voltage_kv.unwrap_or(applied_kv);
                    let accepted_ua = health.set_current_ua.unwrap_or(ua);
                    self.set_kv = accepted_kv;
                    self.set_ua = accepted_ua;
                    self.set_ua_confirmed = true;
                    if power_clamped {
                        self.set_kv_confirmed = true;
                    }
                    self.cached_xray_health = Some(health.clone());
                    self.last_error = None;
                    self.log(
                        "PASS",
                        "xray",
                        &format!(
                            "SEND I applied and verified OFF · final {accepted_kv:.3} kV / {accepted_ua:.3} µA"
                        ),
                    );
                    return Ok(());
                }
                self.set_kv = applied_kv;
                self.set_ua = ua;
                self.log(
                    "INFO",
                    "xray",
                    &format!(
                        "Preview SEND I applied · final {applied_kv:.3} kV / {ua:.3} µA"
                    ),
                );
            }
            _ => return Err("UNKNOWN_COMMAND"),
        }
        Ok(())
    }

    fn validate_setpoint(&self, kv: f64, ua: f64) -> Result<(), &'static str> {
        if !kv.is_finite()
            || !ua.is_finite()
            || !(MIN_VOLTAGE_KV..=MAX_VOLTAGE_KV).contains(&kv)
            || !(0.0..=MAX_CURRENT_UA).contains(&ua)
        {
            return Err("INVALID_XRAY_SETPOINT");
        }
        if kv * ua / 1000.0 > MAX_SETPOINT_POWER_W + 1e-12 {
            return Err("XRAY_POWER_LIMIT");
        }
        Ok(())
    }

    fn require_connected(&self) -> Result<(), &'static str> {
        if self.connected { Ok(()) } else { Err("NOT_CONNECTED") }
    }

    fn sync_reconstruction(&mut self) {
        let Some(handle) = self.reconstruction.as_mut() else { return; };
        let progress = handle.progress();
        let events=progress.events.iter().filter(|event|event.sequence>self.reconstruction_event_sequence).cloned().collect::<Vec<_>>();
        let outcome=handle.try_finish();
        self.reconstruction_status = if progress.status == "fault" { "failed" } else { progress.status };
        self.reconstruction_percent = progress.percent;
        self.reconstruction_message = progress.message;
        for event in events {
            self.reconstruction_event_sequence=event.sequence;
            self.log(if event.status=="fault"{"ERR"}else if event.status=="completed"{"PASS"}else{"INFO"},"reconstruction",&format!("{}% · {}",event.percent,event.message));
            if let Some(log)=self.logs.first_mut(){log["timestamp"]=json!(event.timestamp);}
        }
        let Some(result) = outcome else { return; };
        self.reconstruction = None;
        match result {
            Ok(result) => {
                if self.reconstruction_status!="completed"{self.log("PASS","reconstruction","重构完成");}
                self.reconstruction_status = "completed";
                self.reconstruction_percent = 100;
                self.reconstruction_message = "重构完成".into();
                self.reconstruction_cache_path = Some(result.cache_path.display().to_string());
            }
            Err(error) => {
                if self.reconstruction_status!="failed"{self.log("ERR","reconstruction",&error);}
                self.reconstruction_status = "failed";
                self.reconstruction_message = error;
            }
        }
    }

    fn verify_current_nano_reference(&mut self) -> Result<(), &'static str> {
        let nano = self.nano.as_ref().ok_or("NANO_NOT_CONNECTED")?.clone();
        let status = nano.health().status.ok_or("NANO_STATUS_UNKNOWN")?;
        if !nano_reference_ready(&status) {
            self.homed = false;
            self.phase = Phase::ReadyForHome;
            self.last_error = Some("Nano reference is no longer valid; HOME required".into());
            self.log("WARN", "nano", "Nano reference invalid or position unsettled · HOME required before exposure");
            return Err("HOME_REQUIRED");
        }
        Ok(())
    }

    fn log(&mut self, level: &str, source: &str, message: &str) {
        self.log_counter+=1;
        self.logs.insert(0, json!({
            "id": format!("{}-log-{}", self.engine_session_id,self.log_counter),
            "timestamp": timestamp(),
            "level": level,
            "source": source,
            "message": message,
        }));
        self.logs.truncate(MAX_LOGS);
    }

    pub fn tick(&mut self) {
        self.sync_operations();
        self.sync_software_update_preparation();
        self.sync_real_scan();
        self.sync_reconstruction();
        self.sync_nano_health();
        self.sync_camera_health();
        self.sync_xray_live_status();
        if !self.preview || !matches!(self.phase, Phase::Running | Phase::Finishing) {
            return;
        }
        if self.phase == Phase::Finishing {
            if self.last_tick.elapsed() < Duration::from_millis(850) { return; }
            self.scan_angle_deg = Some(0.0);
            self.phase = Phase::Completed;
            self.log("PASS", "system", "Developer preview completed at start orientation · no projection files created");
            return;
        }
        let steps = (self.last_tick.elapsed().as_millis() / 850) as u32;
        if steps == 0 {
            return;
        }
        self.last_tick += Duration::from_millis(u64::from(steps) * 850);
        self.preview_view_ms = Some(850);
        self.current = self.current.saturating_add(steps).min(self.parameters.projection_count);
        if self.current == self.parameters.projection_count {
            self.phase = Phase::Finishing;
            self.beam_on = false;
            self.last_tick = Instant::now();
            self.log("ACTION", "system", "Developer preview returning turntable to start orientation");
        }
    }

    fn angle_deg(&self) -> f64 {
        if !self.preview {
            return self.nano.as_ref().and_then(|nano| confirmed_nano_angle(&nano.health()))
                .or(self.scan_angle_deg).unwrap_or(0.0);
        }
        if let Some(angle) = self.scan_angle_deg {
            angle
        } else if self.current == 0 {
            0.0
        } else {
            f64::from(self.current - 1) * self.parameters.angle_step_deg
        }
    }

    fn estimated_remaining_seconds(&self) -> Option<u64> {
        if self.phase != Phase::Running { return None; }
        if self.preview {
            return self.preview_view_ms.map(|millis| {
                u64::from(self.parameters.projection_count.saturating_sub(self.current))
                    .saturating_mul(millis).saturating_add(999) / 1000
            });
        }
        self.real_scan.as_ref().and_then(|scan| scan.progress().estimated_remaining_seconds)
    }

    fn data_state(&self) -> &'static str {
        match self.phase {
            Phase::Running | Phase::Finishing | Phase::Stopping => "scanning",
            Phase::Paused => "paused",
            Phase::Fault => "fault",
            _ => "ready",
        }
    }

    fn phase_word(&self) -> &'static str {
        match self.phase {
            Phase::Running => "SCANNING",
            Phase::Paused => "PAUSED",
            Phase::Finishing => "FINISHING",
            Phase::Stopping => "STOPPING",
            Phase::Fault => "FAULT",
            Phase::Completed => "COMPLETE",
            Phase::Stopped => "STANDBY",
            Phase::Idle => "IDLE",
            _ => "READY",
        }
    }

    fn workstation_view(&self) -> Value {
        let total = self.parameters.projection_count;
        let captured = self.current;
        let percent = if total == 0 { 0.0 } else { (100.0 * f64::from(captured) / f64::from(total)).round() };
        let angle = self.angle_deg();
        let data_state = self.data_state();
        let configured = self.parameters.validate().is_ok()
            && (1..=600).contains(&self.max_xray_sec);
        let checkpoint_available = if self.preview { self.current > 0 || self.logs.iter().any(|entry| entry["message"].as_str().is_some_and(|message| message.contains("Preview scan ended"))) }
            else { configured && self.phase != Phase::Completed && self.metadata_resume };
        let pending_restore = !self.preview && checkpoint_available && self.restored_frames.is_empty();
        let is_preview = self.preview;
        let nano_health = self.nano.as_ref().map(|nano| nano.health());
        let angle_known = self.phase != Phase::Stopping && (self.preview || nano_health.as_ref().and_then(confirmed_nano_angle).is_some());
        let nano_connected = nano_health
            .as_ref()
            .is_some_and(|health| health.state == NanoConnectionState::Connected);
        let nano_faulted = nano_health.as_ref().is_some_and(|health| {
            matches!(health.state, NanoConnectionState::Fault | NanoConnectionState::Lost)
                || health.status.as_ref().is_some_and(|status| status.state == "FAULT")
        });
        let camera_health = self
            .camera
            .as_ref()
            .map(CameraAdapter::health)
            .or_else(|| self.cached_camera_health.clone());
        let camera_connected = camera_health
            .as_ref()
            .is_some_and(|health| health.connected && !health.isolated);
        let xray_health = self
            .xray
            .as_ref()
            .map(MoxtekAdapter::health)
            .or_else(|| self.cached_xray_health.clone());
        let xray_connected = xray_health
            .as_ref()
            .is_some_and(|health| health.connected);
        let actual_usb_auto_shutdown = xray_health
            .as_ref()
            .and_then(|health| health.usb_auto_shutdown)
            .unwrap_or(self.usb_auto_shut_down);
        // Display only confirmed telemetry. Missing/disconnected/error readback is
        // unknown, never an assumed zero, setpoint, room temperature, or safe OFF.
        let live_xray = xray_health.as_ref()
            .filter(|health| health.connected && health.last_error.is_none());
        let mon_kv = if self.preview { Some(if self.beam_on { self.set_kv } else { 0.0 }) }
            else { live_xray.and_then(|health| health.voltage_kv) };
        let mon_ua = if self.preview { Some(if self.beam_on { self.set_ua } else { 0.0 }) }
            else { live_xray.and_then(|health| health.current_ua) };
        let temperature = if self.preview { Some(24.0) }
            else { live_xray.and_then(|health| health.temperature_c) };
        let beam_state = if self.preview {
            if self.beam_on { "on" } else { "off" }
        } else {
            observed_xray_beam_state(xray_health.as_ref(), self.beam_on)
        };
        let identity = if is_preview {
            "DEVELOPER PREVIEW · NO REAL HARDWARE".to_owned()
        } else if let Some(health) = nano_health.as_ref() {
            if let Some(device) = health.identity.as_ref() {
                format!(
                    "{} v{} · {}",
                    device.device,
                    device.version,
                    health.port.as_deref().unwrap_or("unknown port")
                )
            } else {
                "PRODUCTION LOCKED · NANO DISCONNECTED".to_owned()
            }
        } else {
            "PRODUCTION LOCKED · NANO DISCONNECTED".to_owned()
        };
        let device_tone = if nano_faulted { "danger" } else if nano_connected || self.preview { "accent" } else { "muted" };
        let xray_text = match beam_state {
            "on" => if self.preview { "PREVIEW BEAM" } else { "EMITTING" },
            "off" => if self.preview { "PREVIEW OFF" } else { "OFF CONFIRMED" },
            _ => "OUTPUT UNKNOWN",
        };
        let safety_text = if self.phase == Phase::Fault {
            "FAULT · VERIFY OUTPUT STATE"
        } else if beam_state == "on" {
            if self.preview { "PREVIEW BEAM · NO REAL HARDWARE" } else { "OUTPUT ON · MONITOR DEVICE STATUS" }
        } else if beam_state == "off" {
            if self.preview { "PREVIEW ONLY · NO REAL HARDWARE" } else { "OUTPUT OFF CONFIRMED" }
        } else {
            "OUTPUT UNKNOWN · READBACK REQUIRED"
        };
        let eta = if self.phase == Phase::Paused {
            "held".to_owned()
        } else if let Some(seconds) = self.estimated_remaining_seconds() {
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        } else {
            "—".to_owned()
        };
        let cooldown = self.real_scan.as_ref().and_then(|scan| scan.progress().cooldown_remaining_seconds);
        let console_logs: Vec<Value> = self.logs.iter().map(|entry| {
            let level = entry["level"].as_str().unwrap_or("INFO");
            let source = entry["source"].as_str().unwrap_or("system");
            json!({
                "id": entry["id"],
                "timestamp": entry["timestamp"],
                "level": if matches!(level, "PASS" | "INFO" | "OK" | "WARN" | "ERR" | "ACTION") { level } else { "INFO" },
                "source": if matches!(source, "system" | "xray" | "nano" | "camera" | "preflight" | "operator" | "reconstruction") { source } else { "system" },
                "message": entry["message"],
            })
        }).collect();
        let frames: Vec<Value> = if self.preview {
            (1..=captured).map(|index| json!({
                "index": index,
                "angleDeg": f64::from(index - 1) * self.parameters.angle_step_deg,
                "exposureMs": self.parameters.exposure_ms,
                "fileName": format!("preview-{index:04}.frame"),
            })).collect()
        } else {
            self.real_frames.clone()
        };
        let scan_prompt = self.scan_prompt.map(|stage| {
            let (title, message, confirm_label) = match stage {
                "geometry" => ("几何参数", "输入扫描几何参数。", "确认"),
                "preReferences" => ("扫描前参考", "移走样品。确认后拍暗场，随后开射线拍空场。", "确认"),
                "placeSample" => ("放置样品", "射线已关闭。放好样品；确认后开始扫描，扫描时会开射线。", "确认"),
                _ => ("扫描确认", "确认后继续。", "确认"),
            };
            json!({"stage":stage,"title":title,"message":message,"confirmLabel":confirm_label,"cancelLabel":"取消"})
        });
        let reconstruction_unlocked = !self.preview && self.real_scan.is_none()
            && self.metadata_completed;
        let reconstruction_methods: Vec<Value> = reconstruction::available_methods().into_iter().map(|entry| {
            let id = entry.method.id();
            let projections_ok = self.parameters.projection_count >= 3;
            let sirt_ok = entry.method != ReconstructionMethod::Sirt
                || reconstruction::sirt_feasible(self.parameters.projection_count);
            let enabled = entry.enabled && projections_ok && sirt_ok;
            let reason = if !projections_ok { Some("至少需要 3 个投影".to_owned()) }
                else if !sirt_ok { Some("预计计算较慢，当前数据规模不可用".to_owned()) }
                else { entry.reason };
            json!({"id":id,"label":id.to_uppercase(),"enabled":enabled,"reason":reason})
        }).collect();
        json!({
            "dataState": data_state,
            "cameraExposure":{
                "minMs":camera_health.as_ref().filter(|health| health.connected).and_then(|health| health.exposure_min_ms).unwrap_or(EXPOSURE_MIN_MS),
                "maxMs":camera_health.as_ref().filter(|health| health.connected).and_then(|health| health.exposure_max_ms).unwrap_or(EXPOSURE_MAX_MS),
                "known":camera_health.as_ref().is_some_and(|health| health.connected && health.exposure_min_ms.is_some() && health.exposure_max_ms.is_some())
            },
            "phaseWord": self.phase_word(),
            "phaseTone": if self.phase == Phase::Fault { "danger" } else if self.phase == Phase::Running { "warn" } else { "accent" },
            "devices": [
                {"id":"xray","name":"X-Ray Source","word":if beam_state == "on" {"EMITTING"} else if beam_state == "off" {"OFF VERIFIED"} else if xray_connected {"UNKNOWN"} else {"OFFLINE"},"tone":if beam_state == "on" {"danger"} else if beam_state == "off" {"accent"} else {"muted"},"spec":if let Some(health) = xray_health.as_ref() {format!("12 W · {} · PORT {} · SERIAL {}", xray_text, health.port.as_deref().unwrap_or("unknown"), health.serial.as_deref().unwrap_or("unknown"))} else {"Moxtek · 12 W · awaiting connection".to_owned()}},
                {"id":"turntable","name":"Turntable-Nano","word":if nano_faulted {"FAULT"} else if self.phase == Phase::Running {"MOVING"} else if nano_connected {"ONLINE"} else if self.preview {"PREVIEW"} else {"OFFLINE"},"tone":device_tone,"spec":if nano_connected {format!("{identity} · POS {angle:.2}° · 60:1 · 8 µSTEP")} else {format!("POS {angle:.2}° · 60:1 · 8 µSTEP · {}", if self.preview {"PREVIEW"} else {"DISCONNECTED"})}},
                {"id":"camera","name":"Camera","word":if camera_connected {"ONLINE"} else if self.preview {"PREVIEW"} else {"OFFLINE"},"tone":if camera_connected {"accent"} else if self.preview {device_tone} else {"muted"},"spec":if let Some(health) = camera_health.as_ref() {format!("{} · {} · SERIAL {} · {}", health.identity.as_ref().map(|i|i.model.as_str()).unwrap_or("Camera"), health.backend, health.serial.as_deref().unwrap_or("unknown"), health.transfer_policy.as_deref().unwrap_or("transfer unverified"))} else if self.preview {"D7100 · NO REAL CAMERA CONNECTION".to_owned()} else {"Native WPD · OPEN DEVICES / CAMERA".to_owned()}}
            ],
            "onlineSummary": if self.preview { "PREVIEW · 0 REAL DEVICES".to_owned() } else { let count = u8::from(nano_connected) + u8::from(camera_connected) + u8::from(xray_connected); if count == 0 { "LOCKED · 0 REAL DEVICES".to_owned() } else { format!("{count} REAL DEVICES · {xray_text}") } },
            "preflight": {
                "word": if self.preflight {"PASSED"} else {"WAITING"},
                "percent": if self.preflight {100} else {0},
                "tone": if self.preflight {"pass"} else {"warn"},
                "subline": if self.preflight && self.preview {"8/8 preview checks · real interlocks unverified"} else if self.preflight {"8/8 real checks · Nano + D7100 + Moxtek"} else if nano_connected {"Nano online · REARM and HOME not executed"} else {"0/8 checks · no real hardware"}
            },
            "floats": [
                {"key":"X-RAY","text":xray_text,"tone":if self.phase == Phase::Fault || self.beam_on {"danger"} else {"muted"}},
                {"key":"CAMERA","text":if camera_connected {"CAMERA ONLINE"} else if self.preview {"PREVIEW ONLY"} else {"OFFLINE"},"tone":if camera_connected {"accent"} else {"muted"}},
                {"key":"SAMPLE","text":if angle_known {format!("{angle:.2}°")} else {"UNKNOWN".to_owned()},"tone":if !angle_known {"muted"} else if self.phase == Phase::Fault {"danger"} else {"accent"}}
            ],
            "safetyBar":{"text":safety_text,"tone":if self.phase == Phase::Fault {"dangerBold"} else if self.beam_on {"danger"} else {"muted"}},
            "scene":{"angleDeg":angle,"rotated":angle.abs() > 0.005,"angleKnown":angle_known,"rotationDirection":if self.preview {1} else {-1}},
            "xray":{
                "connected":xray_connected,
                "setKv":self.set_kv,"setUa":self.set_ua,
                "monKv":mon_kv,"monUa":mon_ua,
                "powerW":mon_kv.zip(mon_ua).map(|(kv, ua)| kv * ua / 1000.0),
                "tempC":temperature,"beamOn":self.beam_on,"beamState":beam_state,
                "latched":self.fault_latched,"onSec":10,"offSec":20,"timerOn":self.timer_on,
                "usbAutoShutDown":actual_usb_auto_shutdown,
                "usbAutoShutDownKnown":self.preview || (live_xray.is_some() && self.usb_auto_shut_down_confirmed),
                "usbShutdownDelay":live_xray.and_then(|health| health.usb_shutdown_delay),
                "manualControlsEnabled":self.preview || (xray_connected && !self.busy() && !self.fault_latched),
                "timerControlsEnabled":self.preview,
                "setpointControlsEnabled":self.preview || (xray_connected && !self.busy() && !self.beam_on)
                ,"voltageConfirmed":self.preview || self.set_kv_confirmed
                ,"currentConfirmed":self.preview || self.set_ua_confirmed
                ,"setpointConfirmed":self.preview || (self.set_kv_confirmed && self.set_ua_confirmed)
            },
            "progress":{
                "captured":captured,"total":total,"percent":percent,"angleDeg":angle,"etaText":eta,
                "barTone":if self.phase == Phase::Fault {"danger"} else if self.phase == Phase::Paused {"warn"} else {"accent"},
                "barLabel":if let Some(seconds) = cooldown {format!("{captured} / {total} · {percent:.0}% · cooling {:02}:{:02}", seconds / 60, seconds % 60)} else {format!("{captured} / {total} · {percent:.0}%")}
            },
            "summary":{
                "savePath":self.parameters.save_path,
                "acquisition":if configured {format!("{total} views · {:.2}° · {} ms", self.parameters.angle_step_deg, self.parameters.exposure_ms)} else {"Scan setup not configured".to_owned()},
                "output":if self.preview {format!("PREVIEW · SET {:.1} kV / {:.1} µA",self.set_kv,self.set_ua)} else {xray_text.to_owned()}
            },
            "statusbar":{
                "left":if configured {format!("{identity} · {} · {captured} / {total} · {angle:.2}°",self.phase_word())} else {format!("{identity} · SETUP REQUIRED")},
                "right":format!("ct-engine · JSONL v{} · {}",PROTOCOL_VERSION,if self.preview{"PREVIEW"}else if nano_connected{"NANO ONLINE"}else{"LOCKED"}),
                "dotTone":if self.phase == Phase::Fault {"danger"} else if self.phase == Phase::Running {"warn"} else if self.phase == Phase::Paused {"accent"} else {"muted"}
            },
            "dock":{
                "home":self.connected && self.preflight && !self.busy() && !self.fault_latched,
                "play":self.connected && !self.fault_latched && !pending_restore && ((self.phase == Phase::Running || self.phase == Phase::Paused) || (!self.busy() && self.preflight && self.homed && (self.preview || (self.usb_auto_shut_down_confirmed && !self.usb_auto_shut_down)))),
                "restore":checkpoint_available && self.connected && !self.busy() && self.preflight && self.homed && !self.fault_latched && !matches!(self.phase, Phase::Fault | Phase::Stopped),
                "stop":matches!(self.phase, Phase::Running | Phase::Paused | Phase::Finishing),
                "playMode":if self.phase == Phase::Running {"pause"} else if self.phase == Phase::Paused {"resume"} else if matches!(self.phase, Phase::Fault | Phase::Finishing | Phase::Stopping) {"disabled"} else {"start"},
                "homeReason":if !self.connected {"Connect the Nano first"} else if self.fault_latched || self.phase == Phase::Fault {"Run Preflight to revalidate device safety"} else if !self.preflight {"Complete scan setup and run Preflight first"} else if self.busy() {"HOME is unavailable while a scan is active"} else {""},
                "restoreReason":if !configured {"Complete valid scan parameters first"} else if !self.connected {"Connect the Nano first"} else if self.fault_latched || self.phase == Phase::Fault {"Run Preflight to recover from the device fault"} else if self.busy() {"Restore is unavailable while a scan is active"} else if !self.preflight {"Run Preflight first"} else if !self.homed {"Run HOME first"} else if !checkpoint_available {"No unfinished scan checkpoint was found"} else if !self.restored_frames.is_empty() {"Checkpoint already verified; Start continues at the next view"} else {""},
                "playReason":if self.phase == Phase::Running {"Pause after the active transaction closes"} else if self.phase == Phase::Paused {"Resume the paused scan"} else if matches!(self.phase, Phase::Finishing | Phase::Stopping) {"Wait for scan cleanup"} else if !self.connected {"Connect the Nano first"} else if self.fault_latched || self.phase == Phase::Fault {"Run Preflight to recover"} else if !self.preflight {"Run Preflight first"} else if !self.homed {"Run HOME first"} else if pending_restore {"Run Restore to verify the saved frames before Start"} else if !self.preview && (!self.usb_auto_shut_down_confirmed || self.usb_auto_shut_down) {"Disable USB Auto Shut Down manually in the X-ray panel"} else {""}
            },
            "scanSetup":{
                "savePath":self.parameters.save_path,"taskId":self.parameters.task_id,"projectionCount":total,
                "angleStepDeg":self.parameters.angle_step_deg,"exposureMs":self.parameters.exposure_ms,"maxXraySec":self.max_xray_sec
            },
            "scanFlow":{
                "stage":self.scan_stage,
                "prompt":scan_prompt,
                "references":{"preDark":self.scan_references.pre_dark.len(),"preFlat":self.scan_references.pre_flat.len(),
                    "postFlat":self.scan_references.post_flat.len(),"postDark":self.scan_references.post_dark.len(),"required":10},
                "geometry":self.scan_geometry,
                "checkpointPath":if self.preview || self.parameters.save_path.trim().is_empty()
                    || self.parameters.task_id.trim().is_empty() { None }
                    else if self.metadata_manifest {
                        Some(Path::new(self.parameters.save_path.trim())
                            .join(self.parameters.task_id.trim()).join("manifest.json").display().to_string())
                    } else if scan::preparation_checkpoint_exists(&self.parameters) {
                        Some(Path::new(self.parameters.save_path.trim())
                            .join(self.parameters.task_id.trim()).join("preparation-checkpoint.json").display().to_string())
                    } else { None }
            },
            "reconstruction":{
                "unlocked":reconstruction_unlocked,
                "methods":reconstruction_methods,
                "selectedMethod":self.reconstruction_method.map(ReconstructionMethod::id),
                "status":self.reconstruction_status,
                "percent":self.reconstruction_percent,
                "message":self.reconstruction_message,
                "cachePath":self.reconstruction_cache_path
            },
            "consoleLogs":console_logs,"frames":frames,"checkpointAvailable":checkpoint_available
        })
    }

    pub fn snapshot(&self) -> Value {
        let (phase, label) = self.phase.value();
        let mode = if self.preview { "developer_preview" } else { "production_locked" };
        let nano_health = self.nano.as_ref().map(|nano| nano.health());
        let nano_connected = nano_health
            .as_ref()
            .is_some_and(|health| health.state == NanoConnectionState::Connected);
        let nano_faulted = nano_health.as_ref().is_some_and(|health| {
            matches!(health.state, NanoConnectionState::Fault | NanoConnectionState::Lost)
                || health.status.as_ref().is_some_and(|status| status.state == "FAULT")
        });
        let camera_health = self.camera.as_ref().map(CameraAdapter::health)
            .or_else(|| self.cached_camera_health.clone());
        let camera_connected = camera_health
            .as_ref()
            .is_some_and(|health| health.connected && !health.isolated);
        let xray_health = self.xray.as_ref().map(MoxtekAdapter::health)
            .or_else(|| self.cached_xray_health.clone());
        let xray_connected = xray_health
            .as_ref()
            .is_some_and(|health| health.connected);
        let mut online_devices=Vec::new();
        if nano_connected {online_devices.push("NANO".to_owned());}
        if camera_connected {online_devices.push(camera_health.as_ref().and_then(|health|health.identity.as_ref()).map(|identity|identity.model.clone()).unwrap_or_else(||"CAMERA".into()));}
        if xray_connected {online_devices.push("MOXTEK".to_owned());}
        let mode_label = if self.preview {
            "DEVELOPER PREVIEW · NO REAL HARDWARE".to_owned()
        } else if nano_faulted {
            format!("PRODUCTION · NANO FAULT · {}",if online_devices.is_empty(){"NO DEVICES CONNECTED".to_owned()}else{online_devices.join(" + ")})
        } else if !online_devices.is_empty() {
            format!("PRODUCTION · {} CONNECTED",online_devices.join(" + "))
        } else {
            "PRODUCTION LOCKED · NO DEVICES CONNECTED".to_owned()
        };
        let adapter_label=format!("ct-engine · JSONL v{} · Nano {} · Camera {} · Moxtek {}",PROTOCOL_VERSION,if nano_connected{"connected"}else{"disconnected"},if camera_connected{"connected"}else{"disconnected"},if xray_connected{"connected"}else{"disconnected"});
        let beam_state=observed_xray_beam_state(xray_health.as_ref(),self.beam_on);
        let live_xray=xray_health.as_ref().filter(|health|health.connected&&health.last_error.is_none());
        let setpoints_confirmed=live_xray.is_some_and(|health|health.set_voltage_kv.is_some()&&health.set_current_ua.is_some())&&self.set_kv_confirmed&&self.set_ua_confirmed;
        let output_evidence=match beam_state{"on"=>"beam ON readback","off"=>"beam OFF confirmed by current readback",_=>"current beam UNKNOWN"};
        let lock_reason=if self.preview{"Developer preview has no real X-ray hardware".to_owned()}else if xray_connected{format!("Moxtek connected · {} · setpoints {}",output_evidence,if setpoints_confirmed{"confirmed"}else{"unconfirmed"})}else{"X-ray not monitored · current output UNKNOWN".to_owned()};
        let xray_detail=if let Some(health)=xray_health.as_ref(){format!("{} · port {} · {}{}",if xray_connected{"Moxtek connected"}else{"Moxtek disconnected"},health.port.as_deref().unwrap_or("unknown"),output_evidence,if !xray_connected&&health.beam_off_confirmed{" · last OFF readback retained for cleanup"}else{""})}else{"X-ray not monitored · current beam UNKNOWN".to_owned()};
        let connection_state = if nano_faulted {
            "degraded"
        } else if nano_health
            .as_ref()
            .is_some_and(|health| matches!(health.state, NanoConnectionState::Lost | NanoConnectionState::Fault))
        {
            "lost"
        } else if self.connected {
            "connected"
        } else {
            "disconnected"
        };
        let device_state = if self.preview {
            if !self.connected { "offline" } else if self.phase == Phase::Running { "busy" } else { "connected" }
        } else if nano_faulted {
            "fault"
        } else if nano_connected {
            "connected"
        } else if self.phase == Phase::Fault {
            "fault"
        } else {
            "offline"
        };
        let nano_detail = nano_health
            .as_ref()
            .and_then(|health| health.identity.as_ref().map(|identity| {
                format!(
                    "{} v{} build {} · {}",
                    identity.device,
                    identity.version,
                    identity.build,
                    health.port.as_deref().unwrap_or("unknown port")
                )
            }))
            .unwrap_or_else(|| mode_label.clone());
        json!({
            "engineSessionId":self.engine_session_id,"snapshotSequence":self.sequence,"operationId":self.operations.last_id,
            "operations":self.operations.history,"deviceControls":self.device_controls_snapshot(),
            "softwareUpdatePreparation":self.software_update_preparation,"cameraCaptureDescriptor":Value::Null,
            "cameraScanReadiness":self.camera_scan_readiness(),
            "mode":mode,"modeLabel":mode_label,"connectionState":connection_state,"adapterLabel":adapter_label,
            "phase":phase,"phaseLabel":label,"preflightPassed":self.preflight,"homed":self.homed,
            "requiresPreflight":!self.preflight,"requiresHome":!self.homed,
            "safety":{
                "xrayAvailable":xray_connected && self.preflight && self.homed && !self.fault_latched,"xrayEnabled":self.beam_on,
                "interlockOk":live_xray.and_then(|health| health.locked).is_some_and(|locked| !locked),
                "lockReason":lock_reason
            },
            "devices":[
                {"id":"turntable","label":"Precision Turntable","state":device_state,"detail":nano_detail},
                {"id":"camera","label":camera_health.as_ref().and_then(|h|h.identity.as_ref()).map(|i|i.model.as_str()).unwrap_or("Camera"),"state":if camera_connected {"connected"} else if self.preview {device_state} else {"offline"},"detail":if let Some(health) = camera_health.as_ref() {format!("{} · serial {} · {}", health.backend, health.serial.as_deref().unwrap_or("unknown"), health.transfer_policy.as_deref().unwrap_or("transfer unverified"))} else if self.preview {mode_label.clone()} else {"Open Devices / Camera, discover and explicitly select a native camera".to_owned()}},
                {"id":"xray","label":"Moxtek 12 W","state":if xray_connected {"connected"} else {"offline"},"detail":xray_detail}
            ],
            "parameters":self.parameters,
            "progress":{
                "current":self.current,"total":self.parameters.projection_count,
                "percent":if self.parameters.projection_count == 0 {0.0} else {(100.0 * f64::from(self.current) / f64::from(self.parameters.projection_count)).round()},
                "angleDeg":self.angle_deg(),
                "etaSeconds":self.estimated_remaining_seconds()
            },
            "imageCount":self.real_frames.len(),"logs":self.logs,"lastError":self.last_error,"updatedAt":timestamp(),
            "workstation":self.workstation_view()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(command: &str, extra: Value) -> Value {
        let mut map = extra.as_object().cloned().unwrap_or_default();
        map.insert("type".into(), Value::String(command.into()));
        Value::Object(map)
    }

    fn send(engine: &mut Engine, command: &str, extra: Value) -> Response {
        engine.handle(Request {
            engine_session_id: Some(engine.engine_session_id.clone()),
            protocol_version: PROTOCOL_VERSION,
            request_id: format!("test-{}", engine.request_sequence + 1),
            command: command.into(),
            payload: if command == "snapshot" { json!({}) } else { payload(command, extra) },
            timestamp: timestamp(),
            sequence: engine.request_sequence + 1,
            error_code: None,
        })
    }

    fn ready(engine: &mut Engine) {
        if !engine.connected {
            assert!(send(engine, "connect", json!({"adapter":"developer_preview"})).error_code.is_none());
        }
        assert!(send(
            engine,
            "update_scan_setup",
            json!({"setup":{
                "taskId":"test-scan",
                "savePath":"E:\\Images\\Camera Roll",
                "projectionCount":5,
                "exposureMs":120,
                "maxXraySec":30
            }}),
        )
        .error_code
        .is_none());
        assert!(send(engine, "preflight", json!({})).error_code.is_none());
        assert!(send(engine, "confirm_scan_stage", json!({"stage":"geometry","geometry":{
            "sodMm":100.0,"objectToDetectorMm":100.0,"detectorWidthMm":100.0,
            "centerOffsetXMm":0.0,"centerOffsetYMm":0.0,
            "rotationDirection":"counterclockwise","mirrorX":false,"measurement":"measured"
        }})).error_code.is_none());
        assert!(send(engine, "home", json!({})).error_code.is_none());
    }

    #[test]
    fn production_is_fail_closed_and_has_complete_workstation() {
        let mut engine = Engine::new(false);
        assert_eq!(send(&mut engine, "connect", json!({"adapter":"developer_preview"})).error_code.as_deref(), Some("PRODUCTION_LOCKED"));
        let snapshot = engine.snapshot();
        assert_eq!(snapshot["mode"], "production_locked");
        assert_eq!(snapshot["workstation"]["onlineSummary"], "LOCKED · 0 REAL DEVICES");
        assert_eq!(snapshot["workstation"]["xray"]["beamOn"], false);
        assert_eq!(snapshot["workstation"]["scene"]["angleKnown"], false);
        assert_eq!(snapshot["workstation"]["scene"]["rotationDirection"], -1);
        assert_eq!(snapshot["workstation"]["xray"]["beamState"], "unknown");
        for field in ["monKv", "monUa", "powerW", "tempC"] {
            assert!(snapshot["workstation"]["xray"][field].is_null(), "{field} must not be fabricated");
        }
        assert!(send(&mut engine, "stop", json!({})).error_code.is_none());
        assert!(send(&mut engine, "disconnect", json!({})).error_code.is_none());
    }

    #[test]
    fn scene_angle_uses_confirmed_pulses_not_pending_target() {
        use devices::turntable::NanoStatus;
        let mut health = NanoHealth {
            state: NanoConnectionState::Connected,
            status: Some(NanoStatus {
                state:"MOVING".into(), position_pulses:-24000, target_pulses:-48000,
                microsteps:8, pulses_per_rev:96000, reference_valid:true, homed:true,
                rearmed:true, hall_active:false, capture_id:1,
            }), ..NanoHealth::default()
        };
        assert_eq!(confirmed_nano_angle(&health), Some(-90.0));
        health.state = NanoConnectionState::Lost;
        assert_eq!(confirmed_nano_angle(&health), None);
        health.state = NanoConnectionState::Connected;
        health.status.as_mut().unwrap().reference_valid = false;
        assert_eq!(confirmed_nano_angle(&health), None);
        health.status.as_mut().unwrap().reference_valid = true;
        health.status.as_mut().unwrap().pulses_per_rev = 0;
        assert_eq!(confirmed_nano_angle(&health), None);
    }

    #[test]
    fn a_second_scan_may_reuse_only_a_live_idle_nano_reference() {
        let mut status = NanoStatus {
            state: "IDLE".into(), position_pulses: -73_067, target_pulses: -73_067,
            microsteps: 8, pulses_per_rev: 96_000, reference_valid: true,
            homed: true, rearmed: true, hall_active: false, capture_id: 0,
        };
        assert!(nano_reference_ready(&status));
        assert!(!may_reuse_home(false, &status), "a new app process must perform its first HOME");
        assert!(may_reuse_home(true, &status), "later scans may reuse a live valid reference");
        status.reference_valid = false;
        assert!(!nano_reference_ready(&status));
        status.reference_valid = true;
        status.rearmed = false;
        assert!(!nano_reference_ready(&status));
        status.rearmed = true;
        status.state = "FAULT".into();
        assert!(!nano_reference_ready(&status));
        status.state = "IDLE".into();
        status.target_pulses -= 1;
        assert!(!nano_reference_ready(&status));
        status.target_pulses = status.position_pulses;
        status.capture_id = 1;
        assert!(!nano_reference_ready(&status));
    }

    #[test]
    fn display_telemetry_requires_live_readback_and_confirmed_off() {
        let mut engine = Engine::new(false);
        engine.cached_xray_health = Some(XrayHealth {
            connected: true, voltage_kv: Some(59.5), current_ua: Some(195.0),
            temperature_c: Some(32.0), ..XrayHealth::default()
        });
        let snapshot = engine.workstation_view();
        assert_eq!(snapshot["xray"]["monKv"], 59.5);
        assert_eq!(snapshot["xray"]["tempC"], 32.0);
        assert_eq!(snapshot["xray"]["beamState"], "unknown");
        assert_eq!(snapshot["summary"]["output"], "OUTPUT UNKNOWN");
        engine.cached_xray_health.as_mut().unwrap().beam_off_confirmed = true;
        assert_eq!(engine.workstation_view()["xray"]["beamState"], "off");
        engine.cached_xray_health.as_mut().unwrap().last_error = Some("readback timeout".into());
        let failed = engine.workstation_view();
        assert_eq!(failed["xray"]["beamState"], "unknown");
        for field in ["monKv", "monUa", "powerW", "tempC"] {
            assert!(failed["xray"][field].is_null(), "stale {field} must be unknown");
        }
        engine.cached_xray_health.as_mut().unwrap().last_error = None;
        engine.cached_xray_health.as_mut().unwrap().connected = false;
        assert!(engine.workstation_view()["xray"]["monKv"].is_null());
        assert_eq!(engine.workstation_view()["xray"]["beamState"], "unknown");
    }

    #[test]
    fn snapshot_metadata_uses_current_device_and_output_evidence() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine").join(format!("{}-snapshot-metadata-{}",Utc::now().format("%Y%m%d-%H%M%S"),std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("command.txt"),"cargo test -p ct-engine snapshot_metadata_uses_current_device_and_output_evidence --offline -- --nocapture\nScripted Nano/Moxtek only; no real device or camera.\n").unwrap();
        let mut engine=Engine::new_with_storage(false,root.join("engine-data"));
        let cold=engine.snapshot();
        engine.nano=Some(devices::turntable::test_homed_nano());engine.connected=true;
        let(mut xray,_)=devices::xray::test_worker(20.0,100.0);xray.set_usb_auto_shutdown(false).unwrap();
        engine.xray=Some(xray.clone());engine.set_kv=20.0;engine.set_ua=100.0;
        let unconfirmed=engine.snapshot();
        engine.set_kv_confirmed=true;engine.set_ua_confirmed=true;
        let off=engine.snapshot();
        xray.beam_on(&std::sync::atomic::AtomicBool::new(false)).unwrap();engine.beam_on=true;
        let on=engine.snapshot();
        xray.force_off().unwrap();engine.beam_on=false;xray.disconnect().unwrap();
        engine.cached_xray_health=Some(xray.health());engine.xray=None;
        let disconnected=engine.snapshot();
        let records=json!({"cold":cold,"setpointsUnconfirmed":unconfirmed,"offConfirmed":off,"on":on,"disconnectedLastOff":disconnected});
        std::fs::write(root.join("snapshots.json"),serde_json::to_vec_pretty(&records).unwrap()).unwrap();
        println!("snapshot evidence: {}",root.display());
        assert!(!records["cold"]["safety"]["lockReason"].as_str().unwrap().contains("locked OFF"));
        for name in ["cold","setpointsUnconfirmed","offConfirmed","on","disconnectedLastOff"] {
            let snapshot=&records[name];let label=snapshot["modeLabel"].as_str().unwrap();
            assert!(!label.contains("D7100"),"{name}: offline camera must not be invented in {label}");
            let adapter=snapshot["adapterLabel"].as_str().unwrap();assert!(adapter.contains("Camera")&&adapter.contains("Moxtek"),"{name}: adapter scope must include independent camera and X-ray owners");
        }
        let reason=records["setpointsUnconfirmed"]["safety"]["lockReason"].as_str().unwrap();assert!(!reason.contains("setpoints verified")&&!reason.contains("setpoints confirmed"));
        for(name,state)in[("offConfirmed","off"),("on","on"),("disconnectedLastOff","unknown")] {
            assert_eq!(records[name]["workstation"]["xray"]["beamState"],state);
        }
        let detail=|name:&str|records[name]["devices"].as_array().unwrap().iter().find(|device|device["id"]=="xray").unwrap()["detail"].as_str().unwrap();
        assert!(detail("on").contains("ON")&&!detail("on").contains("OFF confirmed"));
        assert!(detail("offConfirmed").contains("OFF")&&detail("offConfirmed").contains("confirmed"));
        assert!(detail("disconnectedLastOff").contains("UNKNOWN")&&!detail("disconnectedLastOff").contains("beam OFF confirmed"));
        assert!(!records["disconnectedLastOff"]["safety"]["interlockOk"].as_bool().unwrap());
        assert!(engine.cached_xray_health.as_ref().unwrap().beam_off_confirmed,"closed-owner last OFF cleanup evidence must remain intact");
        std::fs::write(root.join("result.json"),r#"{"status":"PASS","hardware":"NO HARDWARE","validation":"scripted snapshot metadata"}"#).unwrap();
    }

    #[test]
    fn production_requires_connected_devices_before_scan_or_manual_controls() {
        let mut engine = Engine::new(false);
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("NOT_CONNECTED")
        );
        assert_eq!(
            send(&mut engine, "xray_toggle", json!({}))
                .error_code
                .as_deref(),
            Some("XRAY_NOT_CONNECTED")
        );
        assert_eq!(
            send(&mut engine, "send_voltage", json!({"kv":60.0}))
                .error_code
                .as_deref(),
            Some("XRAY_NOT_CONNECTED")
        );
        assert_eq!(
            send(&mut engine, "send_current", json!({"ua":100.0}))
                .error_code
                .as_deref(),
            Some("XRAY_NOT_CONNECTED")
        );
        let snapshot = engine.snapshot();
        assert_eq!(snapshot["safety"]["xrayAvailable"], false);
        assert_eq!(snapshot["workstation"]["xray"]["beamOn"], false);
    }

    #[test]
    fn scan_input_limits_accept_3600_and_fractional_ms_with_default_ten_minutes() {
        let mut engine = Engine::new(true);
        assert_eq!(engine.max_xray_sec, 600);
        assert!(send(&mut engine, "update_scan_setup", json!({"setup":{
            "projectionCount":3600,"exposureMs":0.125
        }})).error_code.is_none());
        assert_eq!(engine.parameters.projection_count, 3600);
        assert_eq!(engine.parameters.angle_step_deg, 0.1);
        assert_eq!(engine.parameters.exposure_ms, 0.125);
        assert!(send(&mut engine, "update_scan_setup", json!({"setup":{"exposureMs":30000}})).error_code.is_none());
        let before = engine.parameters.clone();
        for patch in [
            json!({"projectionCount":3601}), json!({"projectionCount":0}),
            json!({"projectionCount":1.5}), json!({"exposureMs":0.124}),
            json!({"exposureMs":30000.1}),
        ] {
            assert!(send(&mut engine, "update_scan_setup", json!({"setup":patch})).error_code.is_some());
            assert_eq!(engine.parameters, before);
        }
        engine.cached_camera_health = Some(CameraHealth {
            connected:true, exposure_min_ms:Some(1.0), exposure_max_ms:Some(2000.0),
            ..CameraHealth::default()
        });
        assert_eq!(send(&mut engine, "update_scan_setup", json!({"setup":{"exposureMs":2500}}))
            .error_code.as_deref(), Some("EXPOSURE_OUTSIDE_CAMERA_RANGE"));
        assert_eq!(engine.parameters, before);
    }

    #[test]
    fn command_and_payload_type_must_match() {
        let mut engine = Engine::new(true);
        let response = engine.handle(Request {
            engine_session_id: Some(engine.engine_session_id.clone()),
            protocol_version: PROTOCOL_VERSION,
            request_id: "mismatch".into(),
            command: "stop".into(),
            payload: json!({"type":"start_scan"}),
            timestamp: timestamp(),
            sequence: 1,
            error_code: None,
        });
        assert_eq!(response.error_code.as_deref(), Some("COMMAND_PAYLOAD_MISMATCH"));
    }

    #[test]
    fn reconstruction_worker_events_are_aggregated_once_with_stable_ids(){
        let mut engine=Engine::new(false);let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine").join(format!("{}-reconstruction-event-failure",Utc::now().timestamp_nanos_opt().unwrap()));
        engine.reconstruction=Some(ReconstructionHandle::start(ReconstructionRequest{scan_dir:root,method:ReconstructionMethod::Fdk}).unwrap());let deadline=Instant::now()+Duration::from_secs(2);while engine.reconstruction.is_some()&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(5));}
        assert_eq!(engine.reconstruction_status,"failed");let snapshot=engine.snapshot();let events=snapshot["workstation"]["consoleLogs"].as_array();
        let events=events.or_else(||snapshot["consoleLogs"].as_array()).expect("console logs");let events:Vec<_>=events.iter().filter(|entry|entry["source"]=="reconstruction").collect();assert!(!events.is_empty());assert!(events.iter().any(|entry|entry["level"]=="ERR"));let ids:Vec<_>=events.iter().map(|entry|entry["id"].clone()).collect();assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(),ids.len());engine.tick();let after=engine.snapshot();assert_eq!(snapshot["workstation"]["consoleLogs"],after["workstation"]["consoleLogs"]);
    }
    #[test]
    fn preflight_requires_every_scan_setup_field_before_home_and_start() {
        let mut engine = Engine::new(true);
        assert_eq!(engine.parameters, Parameters::default());
        assert_eq!(engine.max_xray_sec, 600);
        assert_eq!(
            send(&mut engine, "preflight", json!({}))
                .error_code
                .as_deref(),
            Some("INVALID_PARAMETERS")
        );
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("PREFLIGHT_REQUIRED")
        );

        for setup in [
            json!({"taskId":"test-scan"}),
            json!({"savePath":"E:\\Images\\Camera Roll"}),
            json!({"projectionCount":5}),
            json!({"exposureMs":120}),
            json!({"maxXraySec":30}),
        ] {
            assert!(send(
                &mut engine,
                "update_scan_setup",
                json!({"setup":setup}),
            )
            .error_code
            .is_none());
            let expected = if engine.parameters.validate().is_ok() && engine.max_xray_sec > 0 {
                None
            } else {
                Some("INVALID_PARAMETERS")
            };
            assert_eq!(
                send(&mut engine, "preflight", json!({})).error_code.as_deref(),
                expected
            );
        }

        assert!(engine.preflight);
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("HOME_REQUIRED")
        );
    }

    #[test]
    fn ordinary_stop_ends_active_scan_and_requires_fresh_preflight_and_home() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        assert_eq!(engine.snapshot()["workstation"]["dock"]["stop"], false);
        assert!(send(&mut engine, "stop", json!({})).error_code.is_none());
        assert_eq!(engine.phase, Phase::Ready);
        send(&mut engine, "start_scan", json!({}));
        assert_eq!(engine.snapshot()["workstation"]["dock"]["stop"], true);
        send(&mut engine, "stop", json!({}));
        assert_eq!(engine.phase, Phase::Stopped);
        assert!(!engine.fault_latched);
        assert!(!engine.beam_on && !engine.preflight && !engine.homed);
        assert_eq!(engine.snapshot()["workstation"]["dock"]["restore"], false);
        assert_eq!(engine.snapshot()["workstation"]["dock"]["stop"], false);
        assert_eq!(
            send(&mut engine, "restore_previous", json!({}))
                .error_code
                .as_deref(),
            Some("RECOVERY_REQUIRED")
        );
        assert_eq!(
            send(&mut engine, "resume", json!({})).error_code.as_deref(),
            Some("NOT_RESUMABLE")
        );
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("PREFLIGHT_REQUIRED")
        );

        assert!(send(&mut engine, "preflight", json!({}))
            .error_code
            .is_none());
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("HOME_REQUIRED")
        );
        assert!(engine.scan_geometry.is_some());assert!(engine.scan_prompt.is_none(),"preflight reuses atomically prepared geometry");
        assert!(send(&mut engine, "home", json!({}))
            .error_code
            .is_none());
        assert_eq!(engine.snapshot()["workstation"]["dock"]["restore"], true);
        assert!(send(&mut engine, "restore_previous", json!({}))
            .error_code
            .is_none());
        assert_eq!(engine.phase, Phase::Paused);
        assert!(send(&mut engine, "resume", json!({}))
            .error_code
            .is_none());
        assert_eq!(engine.phase, Phase::Running);
        assert!(engine.beam_on);
        assert!(send(&mut engine, "pause", json!({})).error_code.is_none());
        assert_eq!(engine.phase, Phase::Paused);
        assert!(send(&mut engine, "stop", json!({})).error_code.is_none());
        assert_eq!(engine.phase, Phase::Stopped);
        assert!(!engine.fault_latched);
        engine.phase = Phase::Fault;
        engine.fault_latched = true;
        assert_eq!(engine.snapshot()["workstation"]["dock"]["stop"], false);
        assert!(send(&mut engine, "stop", json!({})).error_code.is_none());
        assert_eq!(engine.phase, Phase::Fault);
    }

    #[test]
    fn preview_commands_are_implemented() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        for (command, extra) in [
            ("retry_device", json!({"device":"camera"})),
            ("timer_toggle", json!({})),
            ("usb_auto_shut_down_toggle", json!({})),
            ("send_voltage", json!({"kv":60.0})),
            ("send_current", json!({"ua":150.0})),
            ("xray_toggle", json!({})),
        ] {
            assert!(send(&mut engine, command, extra).error_code.is_none(), "{command}");
        }
    }

    #[test]
    fn xray_send_buttons_preserve_the_clicked_axis_and_clamp_the_other_to_12_w() {
        let mut current_engine = Engine::new(true);
        current_engine.set_kv = 60.0;
        let current_response = send(&mut current_engine, "send_current", json!({"ua":500.0}));
        assert!(current_response.error_code.is_none());
        assert!((current_engine.set_kv - 24.0).abs() < 1e-9);
        assert!((current_engine.set_ua - 500.0).abs() < 1e-9);
        assert!((current_engine.set_kv * current_engine.set_ua / 1_000.0 - 12.0).abs() < 1e-9);

        let mut voltage_engine = Engine::new(true);
        voltage_engine.set_ua = 500.0;
        let voltage_response = send(&mut voltage_engine, "send_voltage", json!({"kv":70.0}));
        assert!(voltage_response.error_code.is_none());
        assert!((voltage_engine.set_kv - 70.0).abs() < 1e-9);
        assert!((voltage_engine.set_ua - (12_000.0 / 70.0)).abs() < 1e-9);
        assert!((voltage_engine.set_kv * voltage_engine.set_ua / 1_000.0 - 12.0).abs() < 1e-9);

        let snapshot = voltage_engine.snapshot();
        assert_eq!(snapshot["workstation"]["xray"]["setKv"], 70.0);
        assert_eq!(
            snapshot["workstation"]["xray"]["setUa"].as_f64(),
            Some(12_000.0 / 70.0)
        );
        assert!(voltage_engine.logs.iter().any(|entry| {
            entry["level"] == "WARN"
                && entry["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("SEND V requested 70.000 kV")
                        && message.contains("500.000 µA to 171.429 µA")
                        && message.contains("12 W limit"))
        }));
    }

    #[test]
    fn xray_send_zero_exact_limit_and_invalid_axis_values_are_safe_and_atomic() {
        let mut engine = Engine::new(true);

        assert!(send(&mut engine, "send_current", json!({"ua":0.0}))
            .error_code
            .is_none());
        assert_eq!(engine.set_kv, 4.0);
        assert_eq!(engine.set_ua, 0.0);

        assert!(send(&mut engine, "send_voltage", json!({"kv":60.0}))
            .error_code
            .is_none());
        assert!(send(&mut engine, "send_current", json!({"ua":200.0}))
            .error_code
            .is_none());
        assert_eq!(engine.set_kv, 60.0);
        assert_eq!(engine.set_ua, 200.0);

        let before = (engine.set_kv, engine.set_ua);
        for (command, payload) in [
            ("send_voltage", json!({"kv":3.999})),
            ("send_voltage", json!({"kv":70.001})),
            ("send_current", json!({"ua":-0.001})),
            ("send_current", json!({"ua":1000.001})),
        ] {
            assert_eq!(
                send(&mut engine, command, payload).error_code.as_deref(),
                Some("INVALID_XRAY_SETPOINT")
            );
            assert_eq!((engine.set_kv, engine.set_ua), before);
        }
    }

    /// Hardware finding (2026-10-08): the confirmed 4.000 kV setpoint reads back as
    /// 3.99971 kV. Scan admission revalidates that verified value, so the nominal
    /// minimum inside readback noise must be accepted while a genuinely lower
    /// verified setpoint stays refused.
    #[test]
    fn scan_admission_accepts_the_verified_minimum_voltage_readback() {
        let engine = Engine::new(false);
        let verified = 3.999710083007813_f64;
        assert!(engine.validate_setpoint(verified, 9.9945068359375).is_err());
        assert!(engine.validate_setpoint(crate::devices::xray::nominal_voltage(verified), 9.9945068359375).is_ok());
        assert!(engine.validate_setpoint(crate::devices::xray::nominal_voltage(2.0), 10.0).is_err());
    }

    #[test]
    fn xray_panel_defaults_to_four_kv_ten_ua_without_confirming_device_output() {
        let engine = Engine::new(false);
        let xray = &engine.snapshot()["workstation"]["xray"];
        assert_eq!(xray["setKv"], 4.0);
        assert_eq!(xray["setUa"], 10.0);
        assert_eq!(xray["voltageConfirmed"], false);
        assert_eq!(xray["currentConfirmed"], false);
        assert_eq!(xray["beamState"], "unknown");
    }

    #[test]
    fn update_scan_setup_accepts_partial_patches_and_rejects_invalid_values_atomically() {
        let mut engine = Engine::new(true);
        let before = engine.parameters.clone();
        let before_max_xray_sec = engine.max_xray_sec;

        assert_eq!(
            send(
                &mut engine,
                "update_scan_setup",
                json!({"setup":{"projectionCount":0,"exposureMs":200}}),
            )
            .error_code
            .as_deref(),
            Some("INVALID_PARAMETERS")
        );
        assert_eq!(engine.parameters, before);
        assert_eq!(engine.max_xray_sec, before_max_xray_sec);

        assert!(send(
            &mut engine,
            "update_scan_setup",
            json!({"setup":{"savePath":"E:\\Images\\Camera Roll"}}),
        )
        .error_code
        .is_none());
        assert_eq!(engine.parameters.save_path, r"E:\Images\Camera Roll");
        assert_eq!(engine.parameters.projection_count, 0);

        assert!(send(
            &mut engine,
            "update_scan_setup",
            json!({"setup":{"projectionCount":10,"exposureMs":200,"maxXraySec":60}}),
        )
        .error_code
        .is_none());
        assert_eq!(engine.parameters.angle_step_deg, 36.0);
        assert_eq!(engine.parameters.exposure_ms, 200.0);
        assert_eq!(engine.max_xray_sec, 60);

        assert!(send(
            &mut engine,
            "update_scan_setup",
            json!({"setup":{"maxXraySec":600}}),
        )
        .error_code
        .is_none());
        assert_eq!(engine.max_xray_sec, 600);
        assert_eq!(
            send(
                &mut engine,
                "update_scan_setup",
                json!({"setup":{"maxXraySec":601}}),
            )
            .error_code
            .as_deref(),
            Some("INVALID_PARAMETERS")
        );
        assert_eq!(engine.max_xray_sec, 600);
    }

    #[test]
    fn update_scan_setup_rejects_empty_and_unknown_patches() {
        let mut engine = Engine::new(true);
        let before = engine.parameters.clone();

        assert_eq!(
            send(&mut engine, "update_scan_setup", json!({"setup":{}}))
                .error_code
                .as_deref(),
            Some("INVALID_PARAMETERS")
        );
        assert_eq!(
            send(
                &mut engine,
                "update_scan_setup",
                json!({"setup":{"angleStepDeg":1.0}}),
            )
            .error_code
            .as_deref(),
            Some("INVALID_PARAMETERS")
        );
        assert_eq!(engine.parameters, before);
    }

    #[test]
    fn preview_can_complete_without_images() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        send(&mut engine, "start_scan", json!({}));
        engine.last_tick -= Duration::from_secs(10);
        engine.tick();
        assert_eq!(engine.phase, Phase::Finishing);
        assert_eq!(engine.snapshot()["workstation"]["dock"]["play"], false);
        assert_eq!(engine.snapshot()["workstation"]["dock"]["stop"], true);
        engine.last_tick -= Duration::from_secs(1);
        engine.tick();
        assert_eq!(engine.phase, Phase::Completed);
        assert_eq!(engine.angle_deg(), 0.0);
        assert_eq!(engine.snapshot()["workstation"]["dock"]["stop"], false);
        assert!(send(&mut engine, "stop", json!({})).error_code.is_none());
        assert_eq!(engine.phase, Phase::Completed);
        assert_eq!(engine.snapshot()["imageCount"], 0);
        assert_eq!(engine.snapshot()["workstation"]["frames"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn preview_stop_during_final_return_is_an_ordinary_end() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        send(&mut engine, "start_scan", json!({}));
        engine.last_tick -= Duration::from_secs(10);
        engine.tick();
        assert_eq!(engine.phase, Phase::Finishing);
        assert!(send(&mut engine, "stop", json!({})).error_code.is_none());
        assert_eq!(engine.phase, Phase::Stopped);
        assert!(!engine.fault_latched);
        assert_eq!(engine.snapshot()["workstation"]["dock"]["stop"], false);
    }

    #[test]
    fn malformed_protocol_and_replayed_sequence_are_rejected() {
        let mut engine = Engine::new(true);
        let request = || Request {
            engine_session_id: None,
            protocol_version: PROTOCOL_VERSION,
            request_id: "r-1".into(),
            command: "snapshot".into(),
            payload: json!({}),
            timestamp: timestamp(),
            sequence: 1,
            error_code: None,
        };
        assert!(engine.handle(request()).error_code.is_none());
        assert_eq!(engine.handle(request()).error_code.as_deref(), Some("STALE_REQUEST"));
        let mut incompatible = request();
        incompatible.protocol_version = 1;
        incompatible.sequence = 2;
        assert_eq!(engine.handle(incompatible).error_code.as_deref(), Some("PROTOCOL_VERSION_MISMATCH"));
    }

    #[test]
    fn queued_camera_transfer_cannot_block_snapshot_stop_or_moxtek_off() {
        use devices::camera::{scripted_camera,ScriptedCameraConfig};
        let mut engine=Engine::new(false);
        engine.camera=Some(scripted_camera(ScriptedCameraConfig{transfer_delay:Duration::from_secs(2),..Default::default()}));
        engine.cached_camera_health=engine.camera.as_ref().map(|camera|camera.health());
        let (mut xray,probe)=devices::xray::test_worker(60.0,200.0);
        xray.beam_on_shared(Arc::new(std::sync::atomic::AtomicBool::new(false)),Instant::now()+Duration::from_secs(10)).unwrap();
        engine.xray=Some(xray);engine.beam_on=true;
        let directory=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/camera").join(format!("{}-domain-independent-capture",Utc::now().timestamp_nanos_opt().unwrap()));
        let started=Instant::now();
        engine.parameters.save_path=directory.to_string_lossy().into();
        let directory=directory.join("CameraCaptures");
        let accepted=send(&mut engine,"camera_capture",json!({}));
        assert!(accepted.error_code.is_none());assert!(started.elapsed()<Duration::from_millis(250));
        assert!(accepted.payload["operationId"].is_string());
        thread::sleep(Duration::from_millis(80));
        let queried=Instant::now();assert!(send(&mut engine,"snapshot",json!({})).error_code.is_none());assert!(queried.elapsed()<Duration::from_millis(250));
        let stop_started=Instant::now();assert!(send(&mut engine,"stop",json!({})).error_code.is_none());
        let stop_admission_ms=stop_started.elapsed().as_millis();
        assert!(stop_admission_ms<250,"Stop admission must not wait for COM transfer");
        let mut off_confirmed_ms=None;
        let deadline=Instant::now()+Duration::from_secs(5);
        while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();if off_confirmed_ms.is_none()&&engine.xray.as_ref().is_some_and(|xray|xray.health().beam_off_confirmed){off_confirmed_ms=Some(stop_started.elapsed().as_millis());}thread::sleep(Duration::from_millis(10));}
        let cleanup_ms=stop_started.elapsed().as_millis();let off_write_ms=probe.beam_transitions().iter().find(|(enabled,time)|!*enabled&&*time>=stop_started).map(|(_,time)|time.duration_since(stop_started).as_millis()).unwrap();
        println!("OFFLINE STOP TIMING: admission={stop_admission_ms}ms; OFF write={off_write_ms}ms; OFF readback={}ms; all cleanup={cleanup_ms}ms",off_confirmed_ms.unwrap_or(cleanup_ms));
        std::fs::write(directory.join("stop-timing.json"),serde_json::to_vec_pretty(&json!({"boundary":"offline scripted production workers; no physical OFF timing claim","stopAdmissionMs":stop_admission_ms,"offWriteMs":off_write_ms,"offReadbackConfirmationMs":off_confirmed_ms.unwrap_or(cleanup_ms),"allCleanupMs":cleanup_ms})).unwrap()).unwrap();
        assert!(!probe.output_enabled());
        assert!(probe.beam_transitions().iter().any(|(enabled,time)|!*enabled&&*time>=stop_started&&time.duration_since(stop_started)<Duration::from_secs(1)),"OFF write must be independent from camera transfer");
        let camera_cleanup=engine.camera.as_ref().map(|camera|{let health=camera.health();json!({"connected":health.connected,"isolated":health.isolated,"cleanupConfirmed":health.cleanup_confirmed,"phase":health.phase,"error":health.last_error,"diagnostic":health.last_cleanup_diagnostic})});
        assert!(!engine.fault_latched,"ordinary Stop with confirmed cleanup must not latch emergency fault: camera={camera_cleanup:?}; operations={}; logs={:?}",serde_json::to_string(&engine.operations.history).unwrap(),engine.logs);
        assert!(engine.operations.history.iter().any(|operation|operation.kind=="camera_capture"&&operation.status=="cancelled"));
        assert_eq!(engine.current,0);assert!(engine.real_frames.is_empty());
        if directory.exists(){assert!(std::fs::read_dir(directory).unwrap().all(|entry|entry.unwrap().path().extension().is_none_or(|extension|extension!="nef")),"late cancelled standalone image must not commit");}
    }

    #[test]
    fn domain_rejects_live_view_capture_task_occupation_and_persistent_lock_bypass() {
        use devices::camera::{scripted_camera,ScriptedCameraConfig};
        let mut engine=Engine::new(false);let camera=scripted_camera(ScriptedCameraConfig::default());
        camera.start_live_view().unwrap();engine.camera=Some(camera);
        assert_eq!(send(&mut engine,"camera_capture",json!({"directory":"unused"})).error_code.as_deref(),Some("LIVE_VIEW_CAPTURE_FORBIDDEN"));
        engine.phase=Phase::Paused;
        assert_eq!(send(&mut engine,"camera_focus",json!({"step":1})).error_code.as_deref(),Some("DEVICE_CONTROL_LOCKED"));
        assert_eq!(send(&mut engine,"turntable_relative",json!({"angleDeg":1.0})).error_code.as_deref(),Some("DEVICE_CONTROL_LOCKED"));
        engine.phase=Phase::Idle;
        let storage=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine").join(format!("{}-domain-invalid-register",Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&storage).unwrap();std::fs::write(storage.join("active-task.json"),b"broken registry").unwrap();
        engine.task_registry=task_registry::TaskRegistry::load(storage.join("active-task.json"));
        assert_eq!(send(&mut engine,"turntable_home",json!({})).error_code.as_deref(),Some("UNFINISHED_TASK_LOCKED"));
        assert_eq!(send(&mut engine,"update_scan_setup",json!({"setup":{"taskId":"other"}})).error_code.as_deref(),Some("UNFINISHED_TASK_LOCKED"));
        assert_eq!(send(&mut engine,"abandon_scan",json!({})).error_code.as_deref(),Some("TASK_REGISTRY_INVALID"));
    }

    #[test]
    fn stop_invalidates_late_domain_completion_without_unlocking_or_beam_enable() {
        let mut engine=Engine::new(false);
        engine.operations.spawn(&engine.engine_session_id,"camera","delayed_result",|_|{thread::sleep(Duration::from_millis(80));Ok(Box::new(|engine|{engine.beam_on=true;engine.preflight=true;Ok(())}))}).unwrap();
        assert!(send(&mut engine,"stop",json!({})).error_code.is_none());
        let deadline=Instant::now()+Duration::from_secs(1);
        while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(5));}
        assert!(!engine.beam_on);assert!(!engine.preflight);assert!(!engine.fault_latched);
        assert_eq!(engine.operations.history.iter().find(|operation|operation.kind=="delayed_result").unwrap().status,"cancelled");
    }

    #[test]
    fn protocol_requires_current_session_unique_ids_and_snapshot_evidence() {
        let mut engine=Engine::new(true);
        let handshake=engine.handle(Request{engine_session_id:None,protocol_version:PROTOCOL_VERSION,request_id:"handshake".into(),command:"snapshot".into(),payload:json!({}),timestamp:timestamp(),sequence:1,error_code:None});
        assert!(handshake.error_code.is_none());assert_eq!(handshake.payload["engineSessionId"],handshake.engine_session_id);assert_eq!(handshake.payload["snapshotSequence"],handshake.sequence);
        let wrong=engine.handle(Request{engine_session_id:Some("old-engine".into()),protocol_version:PROTOCOL_VERSION,request_id:"foreign".into(),command:"snapshot".into(),payload:json!({}),timestamp:timestamp(),sequence:2,error_code:None});
        assert_eq!(wrong.error_code.as_deref(),Some("ENGINE_SESSION_MISMATCH"));
        let duplicate=engine.handle(Request{engine_session_id:Some(engine.engine_session_id.clone()),protocol_version:PROTOCOL_VERSION,request_id:"handshake".into(),command:"snapshot".into(),payload:json!({}),timestamp:timestamp(),sequence:3,error_code:None});
        assert_eq!(duplicate.error_code.as_deref(),Some("DUPLICATE_REQUEST_ID"));
    }

    #[test]
    fn ordinary_stop_during_beam_enable_wait_retains_confirmed_off_without_fault() {
        let mut engine=Engine::new(false);let (xray,probe)=devices::xray::test_worker(60.0,200.0);
        engine.xray=Some(xray);engine.set_kv=60.0;engine.set_ua=200.0;engine.set_kv_confirmed=true;engine.set_ua_confirmed=true;
        assert!(send(&mut engine,"xray_toggle",json!({})).error_code.is_none());
        thread::sleep(Duration::from_millis(50));assert!(send(&mut engine,"stop",json!({})).error_code.is_none());
        let deadline=Instant::now()+Duration::from_secs(2);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(1));}
        assert!(!engine.fault_latched);assert!(!probe.output_enabled());assert!(engine.xray.as_ref().unwrap().health().beam_off_confirmed);
        assert!(!probe.beam_transitions().iter().any(|(enabled,_)|*enabled));
    }

    #[test]
    fn failed_off_confirmation_remains_unknown_and_latches_fault() {
        let mut engine=Engine::new(false);let (xray,probe)=devices::xray::test_worker(60.0,200.0);
        engine.xray=Some(xray);probe.fail_next_read();
        assert!(send(&mut engine,"stop",json!({})).error_code.is_none());
        let deadline=Instant::now()+Duration::from_secs(2);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(1));}
        assert!(engine.fault_latched);assert_eq!(engine.workstation_view()["xray"]["beamState"],"unknown");
        assert!(engine.operations.history.iter().any(|operation|operation.status=="cleanup_unconfirmed"));
    }

    #[test]
    fn registered_task_without_checkpoint_can_be_explicitly_abandoned_after_restart() {
        let storage=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine").join(format!("{}-abandon-missing-checkpoint",Utc::now().timestamp_nanos_opt().unwrap()));
        let parameters=Parameters{task_id:"interrupted-before-checkpoint".into(),save_path:storage.to_string_lossy().into(),projection_count:2,angle_step_deg:180.0,exposure_ms:200.0};
        let mut registry=task_registry::TaskRegistry::load(storage.join("active-task.json"));registry.register(&parameters).unwrap();
        let scan_dir=storage.join(&parameters.task_id);std::fs::create_dir_all(&scan_dir).unwrap();let retained=scan_dir.join("original.nef");std::fs::write(&retained,b"untouched original").unwrap();
        let mut engine=Engine::new_with_storage(false,&storage);assert!(engine.task_registry.error().is_some());assert!(engine.task_registry.can_abandon());
        assert!(send(&mut engine,"abandon_scan",json!({})).error_code.is_none());
        let deadline=Instant::now()+Duration::from_secs(1);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(5));}
        assert!(!engine.task_registry.locked());assert_eq!(std::fs::read(retained).unwrap(),b"untouched original");
        assert!(!task_registry::TaskRegistry::load(storage.join("active-task.json")).locked());
        assert!(engine.task_registry.check_target_available(&parameters).is_err());
    }

    #[test]
    fn idle_camera_stop_that_cleans_before_next_tick_is_expected_retirement() {
        use devices::camera::{scripted_camera,ScriptedCameraConfig};
        let mut engine=Engine::new(false);engine.camera=Some(scripted_camera(ScriptedCameraConfig::default()));
        engine.cached_camera_health=engine.camera.as_ref().map(|camera|camera.health());
        assert!(engine.device_command("stop",json!({})).unwrap().is_ok());
        let stop_id=engine.operations.last_id.clone().expect("matching Stop receipt");
        let deadline=Instant::now()+Duration::from_secs(8);
        // Establish the scenario without an Engine tick: the camera owner has
        // retired first. A fixed sleep cannot guarantee this under test load.
        while !engine.camera.as_ref().unwrap().health().cleanup_confirmed&&Instant::now()<deadline{thread::sleep(Duration::from_millis(5));}
        assert!(engine.camera.as_ref().unwrap().health().cleanup_confirmed,"camera cleanup must precede the first Engine tick");
        engine.tick();assert!(!engine.fault_latched,"confirmed retirement must not look like unexpected device loss");
        // Cleanup may precede the safety worker's result enqueue. Wait for this
        // admitted Stop receipt, keeping the production six-second budget intact.
        while engine.operations.history.iter().find(|op|op.operation_id==stop_id).is_some_and(|op|matches!(op.status.as_str(),"queued"|"running"|"cancelling"))&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(5));}
        let stopped=engine.operations.history.iter().find(|op|op.operation_id==stop_id).expect("Stop receipt retained");
        assert_eq!(stopped.status,"completed","matching Stop must confirm before Stopped: {:?}",stopped.error);
        assert!(!engine.fault_latched);assert_eq!(engine.phase,Phase::Stopped);
        assert!(engine.camera.as_ref().unwrap().health().cleanup_confirmed);
    }

    #[test]
    fn unfinished_task_restarts_connects_and_restores_only_through_confirmed_frozen_preflight() {
        use devices::camera::{scripted_camera,ScriptedCameraConfig};
        fn pump(engine:&mut Engine){
            let expected=engine.operations.last_id.clone().expect("requested operation ID");
            let deadline=Instant::now()+Duration::from_secs(8);
            while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(5));}
            assert!(!engine.operations.is_active(),"operation timeout: {}",engine.snapshot());
            assert!(engine.operations.history.iter().find(|operation|operation.operation_id==expected).is_some_and(|operation|operation.status=="completed"),"operation {expected} failed: {}",engine.snapshot());
        }
        let run=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine").join(format!("{}-unfinished-full-restart",Utc::now().timestamp_nanos_opt().unwrap()));
        let storage=run.join("engine-data");
        let parameters=Parameters{task_id:"resume-task".into(),save_path:run.to_string_lossy().into(),projection_count:3,angle_step_deg:120.0,exposure_ms:200.0};
        let camera=scripted_camera(ScriptedCameraConfig{exposure_delay:Duration::from_millis(5),transfer_delay:Duration::from_millis(120),..Default::default()});
        let nano=devices::turntable::test_homed_nano();let(xray,_)=devices::xray::test_worker(20.0,100.0);
        let profile=capture_profile::CaptureProfile::from_camera_readback(&camera.health(),camera.profile_readback().unwrap(),None).unwrap();
        let geometry=ScanGeometry{sod_mm:100.0,object_to_detector_mm:100.0,detector_width_mm:40.0,center_offset_x_mm:0.0,center_offset_y_mm:0.0,rotation_direction:"counterclockwise".into(),mirror_x:false,measurement:"measured".into()};
        let mut original=Engine::new_with_storage(false,&storage);original.parameters=parameters.clone();original.max_xray_sec=120;original.connected=true;original.preflight=true;original.homed=true;original.phase=Phase::Running;
        original.nano=Some(nano.clone());original.camera=Some(camera.clone());original.xray=Some(xray.clone());
        original.task_registry.register(&parameters).unwrap();
        original.real_scan=Some(RealScanHandle::start(nano,camera,xray,RealScanConfig{parameters:parameters.clone(),capture_profile:profile,max_xray_sec:120,voltage_kv:20.0,current_ua:100.0,geometry,resume:None}));
        let deadline=Instant::now()+Duration::from_secs(20);
        while original.current<1&&Instant::now()<deadline {
            original.tick();
            if let Some(stage)=original.scan_prompt {assert!(send(&mut original,"confirm_scan_stage",json!({"stage":stage})).error_code.is_none());}
            thread::sleep(Duration::from_millis(5));
        }
        assert!(original.current>=1,"first projection was not committed: {}",original.snapshot());
        assert!(send(&mut original,"stop",json!({})).error_code.is_none());
        let stopped=Instant::now()+Duration::from_secs(8);
        while (original.real_scan.is_some()||original.operations.is_active())&&Instant::now()<stopped{original.tick();thread::sleep(Duration::from_millis(5));}
        assert!(!original.fault_latched,"ordinary pipeline stop must be confirmed: {}",original.snapshot());assert!(original.task_registry.locked());
        let committed=original.current;assert!((1..3).contains(&committed));original.shutdown().unwrap();drop(original);
        let mut restored=Engine::new_with_storage(false,&storage);assert!(restored.task_registry.locked());assert_eq!(restored.parameters,parameters);
        restored.test_connections.nano=Some(Arc::new(||Ok(devices::turntable::test_homed_nano())));
        let recovery_probe=Arc::new(std::sync::Mutex::new(None));let probe_sink=recovery_probe.clone();
        restored.test_connections.camera=Some(Arc::new(move||{let(camera,probe)=devices::camera::scripted_camera_with_probe(ScriptedCameraConfig::default());*probe_sink.lock().unwrap()=Some(probe);camera.disconnect_confirmed().map_err(|error|error.to_string())?;Ok(camera)}));
        restored.test_connections.xray=Some(Arc::new(||Ok(devices::xray::test_worker(4.0,10.0).0)));
        for (command,payload) in [("turntable_connect",json!({})),("camera_connect",json!({"deviceId":"fake-camera-1"})),("xray_connect",json!({}))] {
            let response=send(&mut restored,command,payload);assert!(response.error_code.is_none(),"{command}: {:?}",response.error_code);assert!(response.payload["operationId"].is_string());pump(&mut restored);
            assert!(restored.task_registry.locked(),"connection repair cannot unlock retained task");
        }
        assert!(send(&mut restored,"camera_read_properties",json!({})).error_code.is_none());pump(&mut restored);
        for(command,payload)in[("send_voltage",json!({"kv":30.0})),("camera_focus",json!({"step":1})),("turntable_relative",json!({"angleDeg":1.0}))]{assert_eq!(send(&mut restored,command,payload).error_code.as_deref(),Some("UNFINISHED_TASK_LOCKED"));}
        assert_eq!(send(&mut restored,"preflight",json!({})).error_code.as_deref(),Some("RESTORE_PREFLIGHT_CONFIRMATION_REQUIRED"));
        assert!(send(&mut restored,"preflight",json!({"restore":true,"confirmUsbAutoShutdownRelease":true})).error_code.is_none());pump(&mut restored);
        assert!(restored.preflight);assert_eq!(restored.max_xray_sec,120);assert!((restored.set_kv-20.0).abs()<0.01);assert!((restored.set_ua-100.0).abs()<0.01);assert!(!restored.usb_auto_shut_down);
        assert!(send(&mut restored,"home",json!({})).error_code.is_none());pump(&mut restored);
        assert!(restored.homed);assert!(restored.task_registry.locked());
        assert!(send(&mut restored,"restore_previous",json!({})).error_code.is_none());pump(&mut restored);
        assert_eq!(restored.current,committed);assert!(restored.restored_scan.is_some());assert_eq!(restored.max_xray_sec,120);
        assert!(restored.task_registry.locked());assert_eq!(restored.phase,Phase::Ready);
        assert_eq!(recovery_probe.lock().unwrap().as_ref().unwrap().write_count(),0,"recovery Preflight and Restore never write core camera configuration");
        std::fs::write(run.join("summary.md"),"PASS: same production scan coordinator and camera/Nano/Moxtek workers; committed partial acquisition survives restart; explicit device repair remains locked; controlled preflight restores frozen V/I/profile/max only after USB-release confirmation; required HOME and verified restore reach Ready without changing retained NEFs. Only offline validation.\n").unwrap();
        restored.shutdown().unwrap();
    }

    #[test]
    fn unconfirmed_camera_disconnect_keeps_proxy_and_abandon_cannot_release_task_lock() {
        use devices::camera::{scripted_camera,ScriptedCameraConfig};
        let storage=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine").join(format!("{}-camera-close-failure",Utc::now().timestamp_nanos_opt().unwrap()));
        let mut engine=Engine::new_with_storage(false,storage.join("engine-data"));
        engine.parameters=Parameters{task_id:"retained-task".into(),save_path:storage.to_string_lossy().into(),projection_count:2,angle_step_deg:180.0,exposure_ms:200.0};
        engine.task_registry.register(&engine.parameters).unwrap();
        let retained=storage.join("retained-task/original.nef");std::fs::create_dir_all(retained.parent().unwrap()).unwrap();std::fs::write(&retained,b"retained original").unwrap();
        engine.camera=Some(scripted_camera(ScriptedCameraConfig{close_failure:true,..Default::default()}));engine.cached_camera_health=engine.camera.as_ref().map(|camera|camera.health());
        let response=send(&mut engine,"camera_disconnect",json!({}));assert!(response.error_code.is_none());let id=response.payload["operationId"].as_str().unwrap().to_owned();
        let deadline=Instant::now()+Duration::from_secs(8);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(10));}
        assert!(engine.camera.is_some());assert!(!engine.camera.as_ref().unwrap().health().cleanup_confirmed);
        assert_eq!(engine.operations.history.iter().find(|operation|operation.operation_id==id).unwrap().status,"cleanup_unconfirmed");
        assert!(send(&mut engine,"abandon_scan",json!({})).error_code.is_none());
        // An OFF-only completion while full abandonment cleanup is pending cannot unlock it.
        assert!(send(&mut engine,"xray_off",json!({})).error_code.is_none());
        let deadline=Instant::now()+Duration::from_secs(8);while engine.operations.is_active()&&Instant::now()<deadline{engine.tick();thread::sleep(Duration::from_millis(10));}
        assert!(engine.task_registry.locked());assert_eq!(std::fs::read(retained).unwrap(),b"retained original");assert!(engine.fault_latched);
    }

    #[test]
    fn independent_capture_rejects_the_configured_scan_directory_before_io() {
        use devices::camera::{scripted_camera,ScriptedCameraConfig};
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/camera").join(format!("{}-independent-directory-isolation",Utc::now().format("%Y%m%d-%H%M%S%3f")));
        let mut engine=Engine::new(false);engine.camera=Some(scripted_camera(ScriptedCameraConfig::default()));
        engine.parameters=Parameters{task_id:"selected-scan".into(),save_path:root.to_string_lossy().into(),projection_count:3,angle_step_deg:120.0,exposure_ms:200.0};
        let directory=root.join("selected-scan");let before=engine.real_frames.len();
        assert_eq!(send(&mut engine,"camera_test_capture",json!({"directory":directory.to_string_lossy()})).error_code.as_deref(),Some("CAPTURE_DIRECTORY_IS_SCAN_TASK"));
        assert!(!directory.exists());assert_eq!(engine.real_frames.len(),before);assert_eq!(engine.current,0);assert!(engine.camera.as_ref().unwrap().health().last_capture.is_none());
    }

    #[test]
    fn shutdown_without_devices_and_with_confirmed_devices_succeeds() {
        let mut empty=Engine::new(false);
        empty.shutdown_with_timeout(Duration::from_secs(2)).unwrap();
        assert!(empty.last_error.is_none());assert!(!empty.beam_on);

        let mut engine=Engine::new(false);
        let nano=devices::turntable::test_homed_nano();
        assert!(nano.health().status.is_some());
        engine.nano=Some(nano.clone());
        engine.camera=Some(devices::camera::scripted_camera(Default::default()));
        engine.xray=Some(devices::xray::test_worker(20.0,100.0).0);
        engine.shutdown_with_timeout(Duration::from_secs(4)).unwrap();
        assert!(engine.nano.is_none());assert!(engine.camera.is_none());assert!(engine.xray.is_none());
        assert!(nano.health().status.is_none(),"confirmed STOP invalidates the position; a final IDLE snapshot is not its receipt");
        assert!(engine.operations.history.iter().any(|op|op.device=="safety"&&op.kind=="stop"&&op.status=="completed"));
    }

    #[test]
    fn shutdown_preserves_unconfirmed_camera_evidence_without_an_owner() {
        let mut engine=Engine::new(false);
        let health=CameraHealth{cleanup_confirmed:false,last_error:Some("exposure termination unknown".into()),..Default::default()};
        engine.cached_camera_health=Some(health);engine.last_error=Some("capture failed before C102".into());
        let error=engine.shutdown_with_timeout(Duration::from_secs(2)).unwrap_err();
        assert!(error.contains("Camera cleanup unconfirmed"));assert!(error.contains("capture failed before C102"));
        assert_eq!(engine.last_error.as_deref(),Some(error.as_str()));assert!(engine.fault_latched);
    }

    #[test]
    fn shutdown_checks_the_matching_nano_stop_result_before_releasing_the_owner() {
        let mut engine=Engine::new(false);
        engine.nano=Some(Arc::new(NanoAdapter::new()));
        let error=engine.shutdown_with_timeout(Duration::from_secs(2)).unwrap_err();
        assert!(error.contains("Shutdown STOP unconfirmed"));assert!(error.contains("not connected"));
        assert!(engine.nano.is_some(),"a failed STOP result must not be erased by unconditional owner release");
        assert!(engine.operations.history.iter().any(|op|op.device=="safety"&&op.kind=="stop"&&op.status!="completed"));
    }

    #[test]
    fn shutdown_without_xray_owner_preserves_unconfirmed_off() {
        let mut engine=Engine::new(false);
        let (xray,_)=devices::xray::test_worker(20.0,100.0);
        let mut health=xray.health();health.beam_off_confirmed=false;health.last_error=Some("OFF readback missing".into());
        engine.cached_xray_health=Some(health);
        let error=engine.shutdown_with_timeout(Duration::from_secs(2)).unwrap_err();
        assert!(error.contains("OFF unconfirmed"));assert!(!engine.cached_xray_health.as_ref().unwrap().beam_off_confirmed);
        assert_eq!(engine.workstation_view()["xray"]["beamState"],"unknown");
        let mut lost=Engine::new(false);lost.ever_xray=true;
        assert!(lost.shutdown_with_timeout(Duration::from_secs(2)).unwrap_err().contains("OFF unconfirmed"),"a previously managed source without retained OFF evidence is unknown");
    }

    #[test]
    fn shutdown_waits_for_late_camera_receipt_and_requires_one_fresh_stop() {
        use devices::camera::{scripted_camera,ScriptedCameraConfig};
        let (started,receipt)=std::sync::mpsc::channel();
        let camera=scripted_camera(ScriptedCameraConfig {
            native_exposure_ms:Some(7000),exposure_delay:Duration::from_secs(7),
            capture_started_signal:Some(started),..Default::default()
        });
        let directory=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/tests/ct-engine")
            .join(format!("{}-shutdown-late-camera-receipt",Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&directory).unwrap();let destination=directory.join("cancelled.nef");
        let mut engine=Engine::new(false);engine.camera=Some(camera.clone());engine.cached_camera_health=Some(camera.health());
        let capture_path=destination.clone();
        let capture=thread::spawn(move||camera.capture_to_path(&capture_path,"shutdown-late-camera",&std::sync::atomic::AtomicBool::new(false),None));
        receipt.recv_timeout(Duration::from_secs(2)).expect("scripted boundary must receive the RAM trigger before EOF cleanup");
        engine.shutdown_with_timeout(Duration::from_secs(12)).unwrap();
        assert!(capture.join().unwrap().is_err());assert!(!destination.exists(),"cancelled late capture must not commit a host file");
        assert!(engine.cached_camera_health.as_ref().unwrap().cleanup_confirmed);
        let diagnostic=engine.cached_camera_health.as_ref().unwrap().last_cleanup_diagnostic.as_ref().unwrap();
        assert!(diagnostic.pending_at_start&&diagnostic.exposure_ended_observed&&diagnostic.close_confirmed);
        assert_eq!(diagnostic.operation_id.as_deref(),Some("shutdown-late-camera"));
        assert!(diagnostic.last_event_diagnostic.as_ref().unwrap().events.iter().any(|event|event.code==0xc102),"cleanup must retain the actual associated C102, not a timeout or DeviceReady inference");
        let stops=engine.operations.history.iter().filter(|op|op.device=="safety"&&op.kind=="stop").collect::<Vec<_>>();
        assert_eq!(stops.len(),2,"late camera proof permits exactly one fresh safety STOP retry");
        assert_eq!(stops[0].status,"cleanup_unconfirmed");assert_eq!(stops[1].status,"completed");
        assert_ne!(stops[0].operation_id,stops[1].operation_id);
    }

    #[test]
    fn shutdown_deadline_does_not_promote_unfinished_cleanup_to_success() {
        let mut engine=Engine::new(false);
        let (release,waiting)=std::sync::mpsc::channel();
        let (started,ready)=std::sync::mpsc::channel();
        engine.operations.spawn(&engine.engine_session_id,"camera","blocked_cleanup",move |_|{
            started.send(()).unwrap();waiting.recv().unwrap();Ok(Box::new(|engine|{engine.beam_on=true;Ok(())}))
        }).unwrap();
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        let error=engine.shutdown_with_timeout(Duration::from_millis(80)).unwrap_err();
        assert!(error.contains("Shutdown cleanup unfinished"));assert!(engine.operations.blocks_controls());
        release.send(()).unwrap();
        let deadline=Instant::now()+Duration::from_secs(1);
        while engine.operations.is_active()&&Instant::now()<deadline {engine.tick();thread::sleep(Duration::from_millis(5));}
        assert!(!engine.operations.is_active());assert!(!engine.beam_on,"late cancelled completion must remain discarded");
    }
}
