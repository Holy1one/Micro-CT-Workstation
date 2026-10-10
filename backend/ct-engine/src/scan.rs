//! Transactional projection-scan coordinator.
//!
//! A projection is committed only after turntable arrival, warning assertion,
//! verified X-ray exposure, host-side camera file confirmation, and
//! `CAPTURE_DONE`. A verified beam stays on across views until the configured
//! continuous limit, pause, stop, or final view. Pause is observed at transaction
//! boundaries. USB auto-shutdown is an operator-owned device setting: the
//! coordinator never changes it. Every exit path still attempts verified beam
//! OFF and warning OFF before returning.

use crate::capture_profile::CaptureProfile;
use crate::devices::camera::{CameraAdapter, CameraCaptureEvent, CameraError};
use crate::devices::turntable::{
    plan_projection, verify_fixed_configuration, verify_plan_capture_hold, MoveTicket, NanoAdapter,
};
use crate::devices::xray::{MoxtekAdapter, XrayError, XrayHealth};
use crate::{timestamp, Parameters};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CAMERA_TRIGGER_MARGIN: Duration = Duration::from_secs(2);
// Leave time for camera/transport jitter before the hard 600-second beam cutoff.
const EXPOSURE_BUDGET_MARGIN: Duration = Duration::from_secs(30);
const MOVE_BUDGET_MARGIN: Duration = Duration::from_secs(30);
// Project OFF-interval policy, not a Moxtek-specified five-minute cooldown.
const XRAY_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const CANCELLED: &str = "scan cancelled";
const REFERENCE_COUNT: usize = 10;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanGeometry {
    pub sod_mm: f64,
    pub object_to_detector_mm: f64,
    pub detector_width_mm: f64,
    pub center_offset_x_mm: f64,
    pub center_offset_y_mm: f64,
    pub rotation_direction: String,
    pub mirror_x: bool,
    pub measurement: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scintillator_layers: Vec<crate::reconstruction::spectral::StackLayer>,
}

impl ScanGeometry {
    pub fn validate(&self) -> Result<(), &'static str> {
        crate::reconstruction::spectral::validate_layers(&self.scintillator_layers)
            .map_err(|_| "INVALID_SCINTILLATOR_STACK")?;
        if !self.sod_mm.is_finite()
            || self.sod_mm <= 0.0
            || !self.object_to_detector_mm.is_finite()
            || self.object_to_detector_mm <= 0.0
            || !self.detector_width_mm.is_finite()
            || self.detector_width_mm <= 0.0
            || !self.center_offset_x_mm.is_finite()
            || !self.center_offset_y_mm.is_finite()
            || !matches!(
                self.rotation_direction.as_str(),
                "clockwise" | "counterclockwise"
            )
            || !matches!(self.measurement.as_str(), "measured" | "estimated")
        {
            return Err("INVALID_SCAN_GEOMETRY");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceFrames {
    pub pre_dark: Vec<ReferenceFrame>,
    pub pre_flat: Vec<ReferenceFrame>,
    pub post_flat: Vec<ReferenceFrame>,
    pub post_dark: Vec<ReferenceFrame>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceFrame {
    pub index: u32,
    pub exposure_ms: f64,
    pub file_name: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScanResume {
    pub capture_profile: CaptureProfile,
    pub max_xray_sec: u32,
    pub geometry: ScanGeometry,
    pub frames: Vec<RealFrame>,
    pub references: ReferenceFrames,
    pub stage: String,
}

pub(crate) fn initial_scan_stage(resume: Option<&ScanResume>) -> &'static str {
    resume.map_or("preDark", |resume| match resume.stage.as_str() {
        "preDark" => "preDark",
        "preFlat" => "preFlat",
        "placeSample" => "placeSample",
        "projections" => "projections",
        "removeSample" | "postFlat" | "postDark" | "finalizing" => "finalizing",
        _ => "preDark",
    })
}

pub(crate) fn recovery_position_matches(resume: &ScanResume, parameters: &Parameters, position: i64, ppr: u32) -> bool {
    if resume.stage != "projections" || resume.frames.is_empty() || ppr != 96_000 || position > 0 {
        return false;
    }
    let phase = position.rem_euclid(i64::from(ppr));
    let last = resume.frames.last().unwrap().angle_deg;
    let next = projection_job(resume.frames.len() as u32, parameters.projection_count).angle_deg;
    [last, next].into_iter().any(|angle| {
        let pulses = -crate::devices::turntable::pulses_for_forward_mdeg((-angle * 1000.0).round() as i32);
        pulses.rem_euclid(i64::from(ppr)) == phase
    })
}

#[derive(Clone, Debug, Default)]
struct PromptGate {
    pending: Option<&'static str>,
    confirmed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanCompletion {
    Completed,
    Stopped,
}

#[derive(Clone)]
pub struct RealScanConfig {
    pub capture_profile: CaptureProfile,
    pub parameters: Parameters,
    pub max_xray_sec: u32,
    pub voltage_kv: f64,
    pub current_ua: f64,
    pub geometry: ScanGeometry,
    pub resume: Option<ScanResume>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ProjectionJob {
    index: u32,
    angle_mdeg: i32,
    angle_deg: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealFrame {
    pub index: u32,
    pub angle_deg: f64,
    pub exposure_ms: f64,
    pub file_name: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct ScanMessage {
    pub timestamp: String,
    pub level: &'static str,
    pub source: &'static str,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct RealScanProgress {
    pub revision: u64,
    pub phase: &'static str,
    pub captured: u32,
    pub angle_deg: f64,
    pub beam_on: bool,
    pub frames: Vec<RealFrame>,
    pub references: ReferenceFrames,
    pub stage: &'static str,
    pub prompt: Option<&'static str>,
    pub messages: Vec<ScanMessage>,
    pub error: Option<String>,
    pub xray_health: Option<XrayHealth>,
    output_dir_owned: bool,
    pub estimated_remaining_seconds: Option<u64>,
    pub cooldown_remaining_seconds: Option<u64>,
    projection_count: u32,
    projection_started: Option<Instant>,
    // Mean projection work time; completed OFF cooldown waits are excluded.
    measured_projection_time: Option<Duration>,
    projection_cooling_elapsed: Duration,
    cooldown_until: Option<Instant>,
    cooldown_wait_started: Option<Instant>,
    beam_started: Option<Instant>,
    block_limit: Duration,
}

impl Default for RealScanProgress {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: "running",
            captured: 0,
            angle_deg: 0.0,
            beam_on: false,
            frames: Vec::new(),
            references: ReferenceFrames::default(),
            stage: "idle",
            prompt: None,
            messages: Vec::new(),
            error: None,
            xray_health: None,
            output_dir_owned: false,
            estimated_remaining_seconds: None,
            cooldown_remaining_seconds: None,
            projection_count: 0,
            projection_started: None,
            measured_projection_time: None,
            projection_cooling_elapsed: Duration::ZERO,
            cooldown_until: None,
            cooldown_wait_started: None,
            beam_started: None,
            block_limit: Duration::ZERO,
        }
    }
}

impl RealScanProgress {
    fn refresh_estimate(&mut self, now: Instant) {
        self.cooldown_remaining_seconds = self.cooldown_until.map(|deadline| {
            let remaining = deadline.saturating_duration_since(now);
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        });
        self.estimated_remaining_seconds = None;
        if !matches!(self.phase, "running" | "cooling") || self.error.is_some() {
            return;
        }
        let Some(sample) = self.measured_projection_time else {
            return;
        };
        let remaining = self.projection_count.saturating_sub(self.captured);
        if remaining == 0 {
            self.estimated_remaining_seconds = Some(0);
            return;
        }
        let cooling_in_progress = self
            .cooldown_wait_started
            .map(|started| now.saturating_duration_since(started))
            .unwrap_or_default();
        let current_elapsed = self
            .projection_started
            .map(|started| {
                now.saturating_duration_since(started)
                    .saturating_sub(self.projection_cooling_elapsed + cooling_in_progress)
            })
            .unwrap_or_default();
        let work = sample
            .saturating_mul(remaining)
            .saturating_sub(current_elapsed);
        let mut seconds = work.as_secs_f64();
        if let Some(deadline) = self.cooldown_until {
            let pending = deadline.saturating_duration_since(now);
            // Work still being done with the beam OFF overlaps this interval.
            let overlap = if self.phase == "running" && self.projection_started.is_some() {
                sample.saturating_sub(current_elapsed)
            } else {
                Duration::ZERO
            };
            seconds += pending.saturating_sub(overlap).as_secs_f64();
        }
        // Active projection time is an upper bound for future beam time. A
        // completed five-minute OFF interval must never enter that sample.
        if !self.block_limit.is_zero() {
            let active = self
                .beam_started
                .map(|started| now.saturating_duration_since(started).as_secs_f64())
                .unwrap_or(0.0);
            let limit = self.block_limit.as_secs_f64();
            let future_beam_work = sample.saturating_mul(remaining.saturating_sub(1));
            let future_blocks = ((active + future_beam_work.as_secs_f64()) / limit).floor() as u64;
            seconds += future_blocks.saturating_mul(XRAY_COOLDOWN.as_secs()) as f64;
        }
        self.estimated_remaining_seconds = Some(seconds.ceil().min(u64::MAX as f64) as u64);
    }
}

pub struct ScanOutcome {
    pub camera: CameraAdapter,
    pub xray: MoxtekAdapter,
    pub result: Result<ScanCompletion, String>,
}

pub struct RealScanHandle {
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    progress: Arc<Mutex<RealScanProgress>>,
    outcome: mpsc::Receiver<ScanOutcome>,
    worker: Option<JoinHandle<()>>,
    nano: Arc<NanoAdapter>,
    prompt_gate: Arc<Mutex<PromptGate>>,
}

impl RealScanHandle {
    pub fn start(
        nano: Arc<NanoAdapter>,
        camera: CameraAdapter,
        xray: MoxtekAdapter,
        config: RealScanConfig,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let prompt_gate = Arc::new(Mutex::new(PromptGate::default()));
        let initial_stage = initial_scan_stage(config.resume.as_ref());
        let progress = Arc::new(Mutex::new(RealScanProgress {
            captured: config
                .resume
                .as_ref()
                .map_or(0, |resume| resume.frames.len() as u32),
            frames: config
                .resume
                .as_ref()
                .map_or_else(Vec::new, |resume| resume.frames.clone()),
            references: config
                .resume
                .as_ref()
                .map_or_else(ReferenceFrames::default, |resume| resume.references.clone()),
            stage: initial_stage,
            ..RealScanProgress::default()
        }));
        let (sender, outcome) = mpsc::channel();
        let worker_cancel = cancel.clone();
        let worker_pause = pause.clone();
        let worker_progress = progress.clone();
        let worker_prompt_gate = prompt_gate.clone();
        let worker_nano = nano.clone();
        let worker = thread::Builder::new()
            .name("rts9060-real-scan".into())
            .spawn(move || {
                let mut camera = camera;
                let mut xray = xray;
                let mut result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_scan(
                    &worker_nano,
                    &mut camera,
                    &mut xray,
                    &config,
                    &worker_cancel,
                    &worker_pause,
                    &worker_progress,
                    &worker_prompt_gate,
                ))).unwrap_or_else(|_| {
                    let off = xray.force_off();
                    let warning = warning_after_off(&worker_nano,&xray);
                    Err(format!("scan worker panicked; shutdown attempted: X-ray={off:?}, warning={warning:?}"))
                });
                if matches!(result, Ok(ScanCompletion::Stopped)) {
                    update_phase(&worker_progress, "stopped");
                    push_message(&worker_progress, "INFO", "system", "Scan ended by operator; output OFF confirmed".into());
                }
                if let Err(error) = &result {
                    update(&worker_progress, |value| {
                        value.phase = "fault";
                        value.beam_on = xray.health().beam_on;
                        value.xray_health = Some(xray.health());
                        value.error = Some(error.clone());
                    });
                    push_message(&worker_progress, "ERR", "system", format!("Real scan failed closed · {error}"));
                }
                if worker_progress.lock().expect("scan progress mutex poisoned").output_dir_owned {
                    let root = Path::new(config.parameters.save_path.trim()).join(config.parameters.task_id.trim());
                    log_xray_diagnostics(&xray, &worker_progress);
                    if let Err(error) = persist_scan_log(&root, &worker_progress, &result) {
                        let detail = format!("scan log could not be saved: {error}");
                        let rollback = if matches!(result, Ok(ScanCompletion::Completed)) {
                            persist_progress(&root, &config.parameters, &config,
                                &worker_progress, "finalizing", false).err()
                        } else { None };
                        result = Err(match result {
                            Ok(_) => detail.clone(),
                            Err(previous) => format!("{previous}; {detail}"),
                        });
                        if let Some(rollback_error) = rollback {
                            let previous = result.as_ref().err().cloned().unwrap_or_default();
                            result = Err(format!("{previous}; completion checkpoint rollback failed: {rollback_error}"));
                        }
                        update(&worker_progress, |value| {
                            value.phase = "fault";
                            value.error = result.as_ref().err().cloned();
                        });
                        push_message(&worker_progress, "ERR", "system", detail);
                    }
                }
                let _ = sender.send(ScanOutcome {
                    camera,
                    xray,
                    result,
                });
            })
            .expect("failed to spawn real scan worker");
        Self {
            cancel,
            pause,
            progress,
            outcome,
            worker: Some(worker),
            nano,
            prompt_gate,
        }
    }

    pub fn progress(&self) -> RealScanProgress {
        let mut snapshot = self
            .progress
            .lock()
            .expect("scan progress mutex poisoned")
            .clone();
        snapshot.refresh_estimate(Instant::now());
        snapshot
    }

    pub fn request_pause(&self) {
        self.pause.store(true, Ordering::SeqCst);
    }

    pub fn confirm_stage(&self, stage: &str) -> Result<(), &'static str> {
        let mut gate = self.prompt_gate.lock().expect("scan prompt mutex poisoned");
        if gate.pending != Some(stage) || gate.confirmed {
            return Err("SCAN_PROMPT_MISMATCH");
        }
        gate.confirmed = true;
        Ok(())
    }

    pub fn resume(&self) {
        self.pause.store(false, Ordering::SeqCst);
    }

    /// Signal cancellation without waiting for the domain's parallel STOP/OFF.
    pub fn cancel_only(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        update_phase(&self.progress, "stopping");
    }

    pub fn request_stop(&self) -> Result<(), String> {
        if self.cancel.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        update_phase(&self.progress, "stopping");
        self.nano
            .stop()
            .map_err(|error| format!("Nano STOP failed: {error}"))
    }

    pub fn try_finish(&mut self) -> Option<Result<ScanOutcome, String>> {
        poll_scan_outcome(&self.outcome, &mut self.worker)
    }
}

fn poll_scan_outcome<T>(
    receiver: &mpsc::Receiver<T>,
    worker: &mut Option<JoinHandle<()>>,
) -> Option<Result<T, String>> {
    match receiver.try_recv() {
        Ok(outcome) => {
            // Receipt is the terminal ownership handoff. Do not leave a handle
            // whose Drop could send STOP after a successful completion.
            if let Some(worker) = worker.take() {
                if worker.is_finished() {
                    let _ = worker.join();
                }
            }
            Some(Ok(outcome))
        }
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => {
            if worker.as_ref().is_some_and(JoinHandle::is_finished) {
                let _ = worker.take().unwrap().join();
            }
            Some(Err(
                "scan worker exited without a cleanup outcome; output state is unconfirmed".into(),
            ))
        }
    }
}

impl Drop for RealScanHandle {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            if !worker.is_finished() {
                self.cancel.store(true, Ordering::SeqCst);
                let _ = self.nano.stop();
            }
            // Never block IPC or process shutdown on an unbounded join.
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanManifest<'a> {
    schema_version: u32,
    capture_profile: &'a CaptureProfile,
    max_xray_sec: u32,
    scan_id: &'a str,
    created_at: String,
    completed: bool,
    voltage_kv: f64,
    current_ua: f64,
    projection_count: u32,
    exposure_ms: f64,
    geometry: &'a ScanGeometry,
    references: &'a ReferenceFrames,
    stage: &'a str,
    frames: &'a [RealFrame],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredScanManifest {
    schema_version: u32,
    #[serde(default)]
    capture_profile: Option<CaptureProfile>,
    #[serde(default)]
    max_xray_sec: Option<u32>,
    pub(crate) scan_id: String,
    completed: bool,
    voltage_kv: f64,
    current_ua: f64,
    pub(crate) projection_count: u32,
    pub(crate) exposure_ms: f64,
    pub(crate) geometry: ScanGeometry,
    pub(crate) references: ReferenceFrames,
    stage: String,
    pub(crate) frames: Vec<RealFrame>,
}

/// Only the completion bit is needed for command gating. Unknown manifest
/// fields are ignored so older checkpoint files remain inspectable.
#[derive(Deserialize)]
struct StoredScanManifestHeader {
    completed: bool,
}

pub fn manifest_file_exists(parameters: &Parameters) -> bool {
    Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("manifest.json")
        .is_file()
}

pub fn completed_manifest_exists(parameters: &Parameters) -> bool {
    let path = Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("manifest.json");
    let Ok(file) = File::open(path) else {
        return false;
    };
    serde_json::from_reader::<_, StoredScanManifest>(file).is_ok_and(|manifest| {
        matches!(manifest.schema_version, 2 | 3)
            && manifest.completed
            && manifest.stage == "completed"
            && (manifest.schema_version == 2
                || manifest
                    .capture_profile
                    .as_ref()
                    .is_some_and(|profile| profile.validate().is_ok())
                    && manifest
                        .max_xray_sec
                        .is_some_and(|maximum| (1..=600).contains(&maximum)))
            && manifest.scan_id == parameters.task_id
            && manifest.projection_count == parameters.projection_count
            && (manifest.exposure_ms - parameters.exposure_ms).abs() < 1e-6
            && manifest.frames.len() == parameters.projection_count as usize
            && [&manifest.references.pre_dark, &manifest.references.pre_flat]
                .iter()
                .all(|frames| frames.len() == REFERENCE_COUNT)
    })
}

pub fn preparation_checkpoint_exists(parameters: &Parameters) -> bool {
    Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("preparation-checkpoint.json")
        .is_file()
}

pub fn persist_preparation_checkpoint(
    parameters: &Parameters,
    voltage_kv: f64,
    current_ua: f64,
    max_xray_sec: u32,
    capture_profile: &CaptureProfile,
) -> Result<(), String> {
    capture_profile.validate()?;
    if !(1..=600).contains(&max_xray_sec) {
        return Err("Invalid frozen continuous X-ray limit".into());
    }
    let root = Path::new(parameters.save_path.trim()).join(parameters.task_id.trim());
    fs::create_dir_all(&root).map_err(io_error)?;
    if root.join("manifest.json").exists() {
        return Err("scan manifest already exists".into());
    }
    let path = root.join("preparation-checkpoint.json");
    let checkpoint = json!({"schemaVersion":3,"captureProfile":capture_profile,"maxXraySec":max_xray_sec,"scanId":parameters.task_id,"createdAt":timestamp(),
        "stage":"geometry","projectionCount":parameters.projection_count,
        "exposureMs":parameters.exposure_ms,"voltageKv":voltage_kv,"currentUa":current_ua});
    crate::task_registry::atomic_store(
        &path,
        &serde_json::to_vec_pretty(&checkpoint).map_err(|error| error.to_string())?,
    )
}

fn preparation_checkpoint_matches(root: &Path, config: &RealScanConfig) -> Result<bool, String> {
    let path = root.join("preparation-checkpoint.json");
    if !path.exists() {
        return Ok(false);
    }
    let data: serde_json::Value = serde_json::from_reader(File::open(path).map_err(io_error)?)
        .map_err(|error| error.to_string())?;
    let profile: CaptureProfile =
        serde_json::from_value(data["captureProfile"].clone()).map_err(|error| {
            format!("Preparation checkpoint lacks a frozen capture profile: {error}")
        })?;
    config.capture_profile.matches(&profile)?;
    Ok(data["schemaVersion"].as_u64() == Some(3)
        && data["stage"].as_str() == Some("geometry")
        && data["maxXraySec"].as_u64() == Some(u64::from(config.max_xray_sec))
        && data["scanId"].as_str() == Some(config.parameters.task_id.as_str())
        && data["projectionCount"].as_u64() == Some(u64::from(config.parameters.projection_count))
        && data["exposureMs"]
            .as_f64()
            .is_some_and(|value| (value - config.parameters.exposure_ms).abs() < 1e-6)
        && data["voltageKv"]
            .as_f64()
            .is_some_and(|value| (value - config.voltage_kv).abs() < 0.01)
        && data["currentUa"]
            .as_f64()
            .is_some_and(|value| (value - config.current_ua).abs() < 0.01))
}

pub fn resume_manifest_exists(parameters: &Parameters) -> bool {
    let path = Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("manifest.json");
    let Ok(file) = File::open(path) else {
        return false;
    };
    serde_json::from_reader::<_, StoredScanManifestHeader>(file)
        .is_ok_and(|manifest| !manifest.completed)
}

pub fn load_resume(
    parameters: &Parameters,
    voltage_kv: f64,
    current_ua: f64,
    actual_profile: &CaptureProfile,
) -> Result<ScanResume, String> {
    let root = Path::new(parameters.save_path.trim()).join(parameters.task_id.trim());
    let file = File::open(root.join("manifest.json")).map_err(io_error)?;
    let manifest: StoredScanManifest =
        serde_json::from_reader(file).map_err(|error| format!("invalid scan manifest: {error}"))?;
    if manifest.schema_version == 2 {
        return Err("LEGACY_SCHEMA_2_READ_ONLY: native acquisition cannot append projections without complete camera evidence; use the retained legacy version".into());
    }

    if manifest.schema_version != 3
        || manifest.completed
        || manifest.scan_id != parameters.task_id
        || manifest.projection_count != parameters.projection_count
        || (manifest.exposure_ms - parameters.exposure_ms).abs() > 1e-6
        || (manifest.voltage_kv - voltage_kv).abs() > 0.01
        || (manifest.current_ua - current_ua).abs() > 0.01
        || manifest.frames.len() > parameters.projection_count as usize
    {
        return Err("unfinished scan manifest does not match the current task, exposure, projection count or X-ray setpoints".into());
    }
    let capture_profile = manifest
        .capture_profile
        .ok_or("Frozen capture profile is missing")?;
    capture_profile.matches(actual_profile)?;
    let max_xray_sec = manifest
        .max_xray_sec
        .filter(|maximum| (1..=600).contains(maximum))
        .ok_or("Frozen continuous X-ray limit is missing or invalid")?;
    manifest.geometry.validate().map_err(str::to_owned)?;
    if !matches!(
        manifest.stage.as_str(),
        "preDark"
            | "preFlat"
            | "placeSample"
            | "projections"
            | "removeSample"
            | "postFlat"
            | "postDark"
            | "finalizing"
    ) {
        return Err("invalid scan checkpoint stage".into());
    }
    let pre_dark = manifest.references.pre_dark.len();
    let pre_flat = manifest.references.pre_flat.len();
    let post_flat = manifest.references.post_flat.len();
    let post_dark = manifest.references.post_dark.len();
    let projections = manifest.frames.len();
    let full_pre = pre_dark == REFERENCE_COUNT && pre_flat == REFERENCE_COUNT;
    let full_projection = projections == parameters.projection_count as usize;
    let stage_consistent = match manifest.stage.as_str() {
        "preDark" => pre_flat == 0 && post_flat == 0 && post_dark == 0 && projections == 0,
        "preFlat" => {
            pre_dark == REFERENCE_COUNT && post_flat == 0 && post_dark == 0 && projections == 0
        }
        "placeSample" => full_pre && post_flat == 0 && post_dark == 0,
        "projections" => full_pre && post_flat == 0 && post_dark == 0,
        "removeSample" => full_pre && full_projection && post_flat == 0 && post_dark == 0,
        "postFlat" => full_pre && full_projection && post_dark == 0,
        "postDark" => full_pre && full_projection && post_flat == REFERENCE_COUNT,
        "finalizing" => full_pre && full_projection,
        _ => false,
    };
    if !stage_consistent {
        return Err("scan checkpoint stage and committed frame counts disagree".into());
    }
    verify_references(&root, &manifest.references)?;
    for reference in manifest
        .references
        .pre_dark
        .iter()
        .chain(&manifest.references.pre_flat)
        .chain(&manifest.references.post_flat)
        .chain(&manifest.references.post_dark)
    {
        if (reference.exposure_ms - parameters.exposure_ms).abs() > 1e-6 {
            return Err("reference exposure does not match scan setup".into());
        }
    }

    let frames_dir = root.join("frames");
    for (zero_index, frame) in manifest.frames.iter().enumerate() {
        let index = zero_index as u32 + 1;
        let expected_name = format!("frame-{index:04}.nef");
        let expected_path = frames_dir.join(&expected_name);
        if frame.index != index
            || frame.file_name != expected_name
            || Path::new(&frame.path) != expected_path
            || (frame.angle_deg
                - projection_job(zero_index as u32, parameters.projection_count).angle_deg)
                .abs()
                > 0.01
            || (frame.exposure_ms - parameters.exposure_ms).abs() > 1e-6
        {
            return Err(format!(
                "resume frame {index} does not match the recorded scan geometry"
            ));
        }
        let (bytes, sha256) = hash_file(&expected_path)?;
        if bytes != frame.bytes || sha256 != frame.sha256 {
            return Err(format!(
                "resume frame {index} failed size or SHA-256 verification"
            ));
        }
    }
    let mut file_count = 0;
    for entry in fs::read_dir(&frames_dir).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if !entry.file_type().map_err(io_error)?.is_file() {
            return Err(
                "scan frames directory contains a non-file entry; inspect it before restoring"
                    .into(),
            );
        }
        file_count += 1;
    }
    if file_count != manifest.frames.len() {
        return Err("scan frames directory has an uncommitted or unexpected file; inspect it before restoring".into());
    }
    // Older recovery workers wrote placeSample even after committing projections.
    // Normalize only after checking every frozen setting and retained file.
    let stage = match manifest.stage.as_str() {
        "placeSample" if projections > 0 => "projections".to_owned(),
        "removeSample" | "postFlat" | "postDark" => "finalizing".to_owned(),
        _ => manifest.stage,
    };
    Ok(ScanResume {
        capture_profile,
        max_xray_sec,
        geometry: manifest.geometry,
        frames: manifest.frames,
        references: manifest.references,
        stage,
    })
}

fn verify_references(root: &Path, references: &ReferenceFrames) -> Result<(), String> {
    for (group, frames) in [
        ("pre-dark", &references.pre_dark),
        ("pre-flat", &references.pre_flat),
        ("post-flat", &references.post_flat),
        ("post-dark", &references.post_dark),
    ] {
        if frames.len() > REFERENCE_COUNT {
            return Err(format!("too many {group} references"));
        }
        let dir = root.join("references").join(group).join("frames");
        for (zero_index, frame) in frames.iter().enumerate() {
            let index = zero_index as u32 + 1;
            let expected_name = format!("frame-{index:04}.nef");
            let path = dir.join(&expected_name);
            if frame.index != index
                || frame.file_name != expected_name
                || Path::new(&frame.path) != path
                || !frame.exposure_ms.is_finite()
                || frame.exposure_ms <= 0.0
            {
                return Err(format!("{group} reference {index} path mismatch"));
            }
            let (bytes, sha256) = hash_file(&path)?;
            if bytes != frame.bytes || sha256 != frame.sha256 {
                return Err(format!(
                    "{group} reference {index} failed SHA-256 verification"
                ));
            }
        }
        if dir.exists() {
            let entries = fs::read_dir(&dir).map_err(io_error)?.count();
            if entries != frames.len() {
                return Err(format!("{group} has an uncommitted file"));
            }
        }
    }
    Ok(())
}

fn run_scan(
    nano: &NanoAdapter,
    camera: &mut CameraAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    pause: &AtomicBool,
    progress: &Mutex<RealScanProgress>,
    prompt_gate: &Mutex<PromptGate>,
) -> Result<ScanCompletion, String> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_scan_inner(
            nano,
            camera,
            xray,
            config,
            cancel,
            pause,
            progress,
            prompt_gate,
        )
    }))
    .unwrap_or_else(|_| Err("scan body panicked".into()));
    let off = xray.force_off();
    let warning_off = if off.is_ok() {
        warning_after_off(nano, xray)
    } else {
        // Unknown emission keeps the physical warning asserted.
        let _ = nano.set_xray_warning(true);
        Err(crate::devices::turntable::NanoError::Safety(
            "Warning retained because Moxtek OFF is unconfirmed".into(),
        ))
    };
    if off.is_ok() {
        push_message(
            progress,
            "PASS",
            "xray",
            "Moxtek OFF confirmed; operator-owned USB auto-shutdown setting unchanged".to_owned(),
        );
    }
    set_xray_health(progress, xray.health());
    set_beam(progress, xray.health().beam_on);
    if result.is_err() {
        let _ = nano.stop();
    }

    // An operator Stop and a device fault both cancel the inner loop, so the cancel flag
    // alone cannot label the outcome. Reporting a fault as "Scan ended by operator" would
    // hide the real cause from the operator and from the record; the worker owns the live
    // device evidence here, so it decides which of the two actually happened.
    let completion = match finish_scan_result(
        result,
        off.map(|_| ()).map_err(|e| e.to_string()),
        warning_off.map_err(|e| e.to_string()),
    ) {
        Ok(ScanCompletion::Stopped) => match device_fault_evidence(camera, xray) {
            Some(reason) => Err(reason),
            None => Ok(ScanCompletion::Stopped),
        },
        other => other,
    };
    let root = Path::new(config.parameters.save_path.trim()).join(config.parameters.task_id.trim());
    let owned = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .output_dir_owned;
    let checkpoint = if owned {
        let stage = progress.lock().expect("scan progress mutex poisoned").stage;
        persist_progress(
            &root,
            &config.parameters,
            config,
            progress,
            if completion == Ok(ScanCompletion::Completed) {
                "completed"
            } else {
                stage
            },
            completion == Ok(ScanCompletion::Completed),
        )
    } else {
        Ok(())
    };
    if let Err(error) = checkpoint {
        return Err(match completion {
            Err(previous) => format!("{previous}; checkpoint save failed: {error}"),
            _ => format!("checkpoint save failed: {error}"),
        });
    }
    if completion == Ok(ScanCompletion::Completed) {
        update_phase(progress, "completed");
        let frame_count = progress
            .lock()
            .expect("scan progress mutex poisoned")
            .frames
            .len();
        push_message(
            progress,
            "PASS",
            "system",
            format!(
                "Real scan complete · {} projections · X-ray OFF",
                frame_count
            ),
        );
    }
    completion
}

/// The device evidence that a stop was caused by a fault instead of by the operator.
///
/// A confirmed OFF plus a healthy, non-isolated camera leaves an operator Stop a stop. A
/// latched X-ray error, output that is still on or no longer confirmed off, or a camera
/// session left isolated with unconfirmed cleanup is a fault: it must be reported as one so
/// the operator and the retained record see the real cause.
fn device_fault_evidence(camera: &CameraAdapter, xray: &MoxtekAdapter) -> Option<String> {
    let xray_health = xray.health();
    if let Some(error) = xray_health
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    {
        return Some(format!("device fault ended the scan · X-ray: {error}"));
    }
    if xray_health.beam_on {
        return Some("device fault ended the scan · Moxtek output is still ON".to_owned());
    }
    if !xray_health.beam_off_confirmed {
        return Some("device fault ended the scan · Moxtek OFF was not confirmed".to_owned());
    }
    let camera_health = camera.health();
    if camera_health.isolated && !camera_health.cleanup_confirmed {
        return Some(
            "device fault ended the scan · camera session isolated with unconfirmed cleanup"
                .to_owned(),
        );
    }
    None
}

fn finish_scan_result(
    result: Result<(), String>,
    off: Result<(), String>,
    warning: Result<(), String>,
) -> Result<ScanCompletion, String> {
    let stopped = matches!(&result, Err(error) if error == CANCELLED);
    let mut errors = Vec::new();
    if let Err(error) = result {
        if !stopped {
            errors.push(error);
        }
    }
    if let Err(error) = off {
        errors.push(format!("final Moxtek OFF unconfirmed: {error}"));
    }
    if let Err(error) = warning {
        errors.push(format!("final warning OFF unconfirmed: {error}"));
    }
    if !errors.is_empty() {
        Err(errors.join("; "))
    } else if stopped {
        Ok(ScanCompletion::Stopped)
    } else {
        Ok(ScanCompletion::Completed)
    }
}

fn run_scan_inner(
    nano: &NanoAdapter,
    camera: &mut CameraAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    pause: &AtomicBool,
    progress: &Mutex<RealScanProgress>,
    prompt_gate: &Mutex<PromptGate>,
) -> Result<(), String> {
    let parameters = &config.parameters;
    config.capture_profile.validate()?;
    if !(1..=600).contains(&config.max_xray_sec)
        || config
            .resume
            .as_ref()
            .is_some_and(|resume| resume.max_xray_sec != config.max_xray_sec)
    {
        return Err("Continuous X-ray limit differs from the frozen acquisition".into());
    }
    let root = Path::new(parameters.save_path.trim()).join(parameters.task_id.trim());
    let resume_count = config
        .resume
        .as_ref()
        .map_or(0, |resume| resume.frames.len());
    if config.resume.is_none() {
        if root.exists() {
            let entries = fs::read_dir(&root)
                .map_err(io_error)?
                .map(|entry| entry.map(|value| value.file_name().to_string_lossy().into_owned()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(io_error)?;
            if !entries.is_empty()
                && !(entries.len() == 1
                    && entries[0] == "preparation-checkpoint.json"
                    && preparation_checkpoint_matches(&root, config)?)
            {
                return Err(format!(
                    "scan output already exists: {} · use Restore after Preflight and HOME",
                    root.display()
                ));
            }
        }
        fs::create_dir_all(root.join("frames")).map_err(io_error)?;
    } else if load_resume(
        parameters,
        config.voltage_kv,
        config.current_ua,
        &config.capture_profile,
    )? != *config.resume.as_ref().unwrap()
    {
        return Err("scan files changed after Restore; no device action was started".into());
    }
    update(progress, |value| value.output_dir_owned = true);
    push_message(
        progress,
        "ACTION",
        "operator",
        if resume_count == 0 {
            format!(
                "Real scan started · {} projections · {:.1} kV / {:.1} µA",
                parameters.projection_count, config.voltage_kv, config.current_ua
            )
        } else {
            format!(
                "Real scan restored · {resume_count} files verified · continuing at view {}",
                resume_count + 1
            )
        },
    );
    if camera.health().live_view.active { return Err("LIVE_VIEW_CAPTURE_FORBIDDEN: explicitly stop Live View".into()); }
    config.capture_profile.require_current_policy()?;
    camera
        .read_properties()
        .map_err(|error| format!("Camera profile readback failed: {error}"))?;
    let actual_profile = CaptureProfile::from_camera_readback(
        &camera.health(),
        camera
            .profile_readback()
            .map_err(|error| error.to_string())?,
        config.capture_profile.preset.clone(),
    )?;
    config.capture_profile.matches(&actual_profile)?;
    if actual_profile.exposure_ms() != Some(parameters.exposure_ms) {
        return Err(
            "Native shutter readback does not exactly match the scan Exposure target".into(),
        );
    }
    push_message(
        progress,
        "PASS",
        "camera",
        format!("D7100 native shutter verified at {:.6} ms",parameters.exposure_ms),
    );
    xray.set_parameters(config.voltage_kv, config.current_ua)
        .map_err(|error| format!("Moxtek setpoint configuration failed: {error}"))?;
    set_xray_health(progress, xray.health());

    let block_limit = Duration::from_secs(u64::from(config.max_xray_sec));
    let exposure_evidence_timeout = Duration::from_secs_f64(parameters.exposure_ms / 1000.0)
        .saturating_add(CAMERA_TRIGGER_MARGIN);
    let exposure_budget = exposure_evidence_timeout.saturating_add(EXPOSURE_BUDGET_MARGIN);
    if exposure_budget >= block_limit {
        return Err(format!(
            "max X-ray time of {} s cannot accommodate the {:.3} ms exposure and confirmation margin",
            config.max_xray_sec, parameters.exposure_ms
        ));
    }
    let mut beam_started: Option<Instant> = None;
    let mut cooldown_until: Option<Instant> = None;
    if config.resume.is_none() {
        persist_progress(&root, parameters, config, progress, "preDark", false)?;
        if preparation_checkpoint_exists(parameters) {
            fs::remove_file(root.join("preparation-checkpoint.json")).map_err(io_error)?;
        }
    }
    update(progress, |value| {
        value.projection_count = parameters.projection_count;
        value.block_limit = block_limit;
    });

    let resume_stage = initial_scan_stage(config.resume.as_ref());
    if !matches!(resume_stage, "projections" | "finalizing") {
        let pre_references_incomplete = {
            let state = progress.lock().expect("scan progress mutex poisoned");
            state.references.pre_dark.len() < REFERENCE_COUNT
                || state.references.pre_flat.len() < REFERENCE_COUNT
        };
        if pre_references_incomplete {
            wait_for_prompt(nano, xray, progress, prompt_gate, cancel, "preReferences")?;
        }
        capture_reference_group(
            nano, camera, xray, config, cancel, progress, &root, "pre-dark", "preDark", false,
        )?;
        capture_reference_group(
            nano, camera, xray, config, cancel, progress, &root, "pre-flat", "preFlat", true,
        )?;
        if resume_count < parameters.projection_count as usize {
            update(progress, |value| value.stage = "placeSample");
            persist_progress(&root, parameters, config, progress, "placeSample", false)?;
            wait_for_prompt(nano, xray, progress, prompt_gate, cancel, "placeSample")?;
            persist_progress(&root, parameters, config, progress, "projections", false)?;
            update(progress, |value| value.stage = "projections");
        }
    } else if resume_stage == "projections" {
        persist_progress(&root, parameters, config, progress, "projections", false)?;
        update(progress, |value| value.stage = "projections");
        push_message(progress, "INFO", "system", format!(
            "Verified pre-scan references retained; continuing at projection {}",
            resume_count + 1
        ));
    }

    for zero_index in resume_count as u32..parameters.projection_count {
        let job = projection_job(zero_index, parameters.projection_count);
        check_cancel(cancel)?;
        if let Some(cooldown_deadline) = cooldown_until.take() {
            wait_for_cooldown(progress, cancel, cooldown_deadline)?;
        }
        if pause.load(Ordering::SeqCst) && beam_started.is_some() {
            close_scan_beam(nano, xray, progress, &mut beam_started, "Pause boundary")?;
        }
        while pause.load(Ordering::SeqCst) {
            update_phase(progress, "paused");
            if cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        if beam_started.is_some_and(|started| {
            needs_cooldown_before_exposure(
                started,
                Instant::now(),
                block_limit,
                exposure_budget.saturating_add(MOVE_BUDGET_MARGIN),
            )
        }) {
            close_scan_beam(
                nano,
                xray,
                progress,
                &mut beam_started,
                "Cooling before the next projection's move and exposure",
            )?;
            wait_for_cooldown(progress, cancel, Instant::now() + XRAY_COOLDOWN)?;
        }
        update_phase(progress, "running");
        update(progress, |value| {
            value.projection_started = Some(Instant::now());
            value.projection_cooling_elapsed = Duration::ZERO;
        });
        let ticket = move_to_projection_monitored(
            nano,
            xray,
            progress,
            job,
            cancel,
            &mut beam_started,
            block_limit,
            &mut cooldown_until,
        )?;

        // The limit can expire during a long MOVE_ABS. The next exposure must
        // wait for the entire OFF interval, even though the move has completed.
        if let Some(deadline) = cooldown_until.take() {
            wait_for_cooldown(progress, cancel, deadline)?;
            verify_capture_hold(nano, &ticket, job)?;
        }

        if beam_started.is_some_and(|started| {
            needs_cooldown_before_exposure(started, Instant::now(), block_limit, exposure_budget)
        }) {
            close_scan_beam(
                nano,
                xray,
                progress,
                &mut beam_started,
                "Cooling before the current projection's exposure",
            )?;
            wait_for_cooldown(progress, cancel, Instant::now() + XRAY_COOLDOWN)?;
            verify_capture_hold(nano, &ticket, job)?;
        }

        if beam_started.is_none() {
            nano.set_xray_warning(true)
                .map_err(|error| format!("XRAY_WARNING ON failed: {error}"))?;
            // Include the entire ON command latency in the continuous-output budget.
            beam_started = Some(Instant::now());
            update(progress, |value| value.beam_started = beam_started);
            let emission = xray.beam_on_shared(
                cancel.clone(),
                beam_started.expect("beam start set") + block_limit,
            );
            check_cancel(cancel)?;
            emission.map_err(|error| xray_operation_error(error, "Moxtek beam-on failed"))?;
            set_xray_health(progress, xray.health());
            set_beam(progress, true);
            if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
                close_scan_beam(
                    nano,
                    xray,
                    progress,
                    &mut beam_started,
                    "Continuous X-ray limit reached during beam-on confirmation",
                )?;
                return Err("continuous X-ray limit reached before exposure started".into());
            }
            if beam_started.is_some_and(|started| {
                needs_cooldown_before_exposure(
                    started,
                    Instant::now(),
                    block_limit,
                    exposure_budget,
                )
            }) {
                close_scan_beam(
                    nano,
                    xray,
                    progress,
                    &mut beam_started,
                    "Beam enable left insufficient time for a confirmed exposure",
                )?;
                return Err(
                    "configured X-ray time cannot accommodate beam enable and the current exposure"
                        .into(),
                );
            }
            push_message(
                progress,
                "ACTION",
                "xray",
                format!(
                "Continuous beam block confirmed ON · view={} · {:.1} kV / {:.1} µA · limit {} s",
                job.index, config.voltage_kv, config.current_ua, config.max_xray_sec
            ),
            );
        }

        let frame_index = job.index;
        let final_projection = frame_index == parameters.projection_count;
        let mut abort_reason = None;
        let mut last_xray_poll = Instant::now();
        let mut transfer_announced = false;
        let camera_for_capture = camera.clone();
        let capture_session = camera
            .health()
            .session_id
            .ok_or("Capture session is unknown")?;
        let capture_operation = format!(
            "scan-{}-{}-{}",
            parameters.task_id,
            frame_index,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let mut evidence =
            ExposureEvidence::new(capture_session.clone(), capture_operation.clone());
        let capture_result = thread::scope(|scope| {
            let (capture_sender, capture_receiver) = mpsc::channel();
            let (event_sender, event_receiver) = mpsc::channel();
            let capture_path = root
                .join("frames")
                .join(format!("frame-{frame_index:04}.nef"));
            let worker_operation = capture_operation.clone();
            scope.spawn(move || {
                let result = camera_for_capture.capture_to_path(
                    &capture_path,
                    &worker_operation,
                    cancel,
                    Some(event_sender),
                );
                let _ = capture_sender.send(result);
            });
            loop {
                for event in event_receiver.try_iter() {
                    if let Err(error) = evidence.observe(&event) {
                        abort_reason = Some(error);
                        cancel.store(true, Ordering::SeqCst);
                        let _ = xray.force_off();
                        let _ = nano.stop();
                    }
                }
                match capture_receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(result) => {
                        for event in event_receiver.try_iter() {
                            if let Err(error) = evidence.observe(&event) {
                                abort_reason = Some(error);
                            }
                        }
                        break result;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if abort_reason.is_none()
                            && beam_started.is_some()
                            && last_xray_poll.elapsed() >= Duration::from_millis(250)
                        {
                            last_xray_poll = Instant::now();
                            match refresh_emission_monitored(xray, progress, cancel) {
                                Ok(health) => {
                                    if let Some(warning) = health.telemetry_warning.clone() {
                                        push_message(progress, "WARN", "xray", warning);
                                    }
                                    set_xray_health(progress, health);
                                }
                                Err(error) => {
                                    abort_reason = Some(xray_operation_error(
                                        error,
                                        "Moxtek emission monitor failed",
                                    ));
                                    cancel.store(true, Ordering::SeqCst);
                                    set_xray_health(progress, xray.health());
                                    let _ = warning_after_off(nano, xray);
                                    set_beam(progress, xray.health().beam_on);
                                    let _ = nano.stop();
                                }
                            }
                        }
                        if abort_reason.is_none() && cancel.load(Ordering::SeqCst) {
                            abort_reason = Some(CANCELLED.into());
                            cancel.store(true, Ordering::SeqCst);
                            let _ = xray.force_off();
                            set_xray_health(progress, xray.health());
                            let _ = warning_after_off(nano, xray);
                            set_beam(progress, xray.health().beam_on);
                            let _ = nano.stop();
                        }
                        if abort_reason.is_none() {
                            let exposure_complete = evidence.exposure_ended;
                            let limit_reached = beam_started
                                .is_some_and(|started| started.elapsed() >= block_limit);
                            if limit_reached && !exposure_complete {
                                abort_reason = Some(format!(
                                    "continuous X-ray limit of {} s reached before exposure completion",
                                    config.max_xray_sec
                                ));
                                cancel.store(true, Ordering::SeqCst);
                                let _ = xray.force_off();
                                set_xray_health(progress, xray.health());
                                set_beam(progress, xray.health().beam_on);
                                let _ = warning_after_off(nano, xray);
                                let _ = nano.stop();
                            }
                            let pause_requested = pause.load(Ordering::SeqCst);
                            if abort_reason.is_none()
                                && exposure_complete
                                && beam_started.is_some()
                                && (limit_reached || final_projection || pause_requested)
                            {
                                let reason = if limit_reached {
                                    "Continuous X-ray limit reached after current exposure"
                                } else if final_projection {
                                    "Final projection exposure complete"
                                } else {
                                    "Pause requested after current exposure"
                                };
                                match close_scan_beam(
                                    nano,
                                    xray,
                                    progress,
                                    &mut beam_started,
                                    reason,
                                ) {
                                    Ok(_) => {
                                        if limit_reached && !final_projection {
                                            cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                                            update(progress, |value| {
                                                value.cooldown_until = cooldown_until
                                            });
                                        }
                                    }
                                    Err(error) => {
                                        abort_reason = Some(error);
                                        cancel.store(true, Ordering::SeqCst);
                                        let _ = nano.stop();
                                    }
                                }
                            }
                            if evidence.transfer_started {
                                if !transfer_announced {
                                    push_message(
                                        progress,
                                        "PASS",
                                        "camera",
                                        format!(
                                            "NEF transfer started · view={} · file confirmation pending",
                                            frame_index
                                        ),
                                    );
                                    transfer_announced = true;
                                }
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        break Err(CameraError::Io(
                            "camera capture worker terminated without a result".into(),
                        ));
                    }
                }
            }
        });
        if let Some(reason) = abort_reason {
            return Err(reason);
        }
        check_cancel(cancel)?;
        let image = capture_result.map_err(|error| format!("D7100 capture failed: {error}"))?;
        if image.session_id != capture_session
            || image.operation_id != capture_operation
            || !image.exposure_ended
            || !evidence.exposure_ended
        {
            return Err(
                "Capture completed without this operation's reliable ExposureEnded event".into(),
            );
        }
        let path = image.path;
        let limit_reached = beam_started.is_some_and(|started| started.elapsed() >= block_limit);
        if beam_started.is_some()
            && (limit_reached || final_projection || pause.load(Ordering::SeqCst))
        {
            let reason = if limit_reached {
                "Continuous X-ray limit reached after completed exposure"
            } else if final_projection {
                "Final projection capture complete"
            } else {
                "Pause requested after completed exposure"
            };
            close_scan_beam(nano, xray, progress, &mut beam_started, reason)?;
            if limit_reached && !final_projection {
                cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                update(progress, |value| value.cooldown_until = cooldown_until);
            }
        }
        let frame = commit_projection_monitored(
            nano,
            xray,
            progress,
            &root,
            parameters,
            config,
            cancel,
            &path,
            job,
            &ticket,
            &mut beam_started,
            block_limit,
            &mut cooldown_until,
            final_projection,
        )?;
        update(progress, |value| {
            record_projection_sample(value, frame.index, Instant::now())
        });
        push_message(
            progress,
            "PASS",
            "nano",
            format!(
                "CAPTURE_DONE view={} id={} · committed file confirmed",
                job.index, ticket.command_id
            ),
        );
        push_message(
            progress,
            "PASS",
            "camera",
            format!(
                "Projection committed · view={} · {} bytes · SHA-256 {}",
                frame.index, frame.bytes, frame.sha256
            ),
        );
    }

    if beam_started.is_some() {
        close_scan_beam(nano, xray, progress, &mut beam_started, "Scan completed")?;
    }

    update_phase(progress, "finishing");
    check_cancel(cancel)?;
    update(progress, |value| value.stage = "finalizing");
    Ok(())
}

fn xray_operation_error(error: crate::devices::xray::XrayError, context: &str) -> String {
    if matches!(error, crate::devices::xray::XrayError::Cancelled) {
        CANCELLED.into()
    } else {
        format!("{context}: {error}")
    }
}
fn warning_after_off(
    nano: &NanoAdapter,
    xray: &MoxtekAdapter,
) -> Result<(), crate::devices::turntable::NanoError> {
    let health = xray.health();
    if health.beam_off_confirmed && !health.beam_on {
        nano.set_xray_warning(false)
    } else {
        let _ = nano.set_xray_warning(true);
        Err(crate::devices::turntable::NanoError::Safety(
            "Radiation warning retained while output is unknown".into(),
        ))
    }
}

fn check_cancel(cancel: &Arc<AtomicBool>) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn wait_for_cooldown(
    progress: &Mutex<RealScanProgress>,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<(), String> {
    let wait_started = Instant::now();
    update(progress, |value| {
        value.phase = "cooling";
        value.cooldown_until = Some(deadline);
        value.cooldown_wait_started = Some(wait_started);
    });
    let remaining_duration = deadline.saturating_duration_since(wait_started);
    let remaining = remaining_duration.as_secs() + u64::from(remaining_duration.subsec_nanos() > 0);
    push_message(
        progress,
        "WARN",
        "xray",
        format!(
        "X-ray OFF interval · {remaining} seconds remain of the 300-second interval before resuming"
    ),
    );
    while Instant::now() < deadline {
        check_cancel(cancel)?;
        thread::sleep(Duration::from_millis(250));
    }

    let waited = wait_started.elapsed();
    update(progress, |value| {
        if value.projection_started.is_some() {
            value.projection_cooling_elapsed += waited;
        }
        value.cooldown_wait_started = None;
        value.cooldown_until = None;
        value.phase = "running";
    });
    push_message(
        progress,
        "PASS",
        "xray",
        "Five-minute X-ray cooldown complete · scan resuming".into(),
    );
    Ok(())
}

fn wait_for_prompt(
    nano: &NanoAdapter,
    xray: &mut MoxtekAdapter,
    progress: &Mutex<RealScanProgress>,
    gate: &Mutex<PromptGate>,
    cancel: &Arc<AtomicBool>,
    stage: &'static str,
) -> Result<(), String> {
    let off = xray
        .force_off()
        .map_err(|error| format!("{stage} prompt requires confirmed X-ray OFF: {error}"))?;
    warning_after_off(nano, xray)
        .map_err(|error| format!("{stage} prompt warning OFF failed: {error}"))?;
    set_xray_health(progress, off);
    set_beam(progress, false);
    {
        let mut state = gate.lock().expect("scan prompt mutex poisoned");
        state.pending = Some(stage);
        state.confirmed = false;
    }
    update(progress, |state| state.prompt = Some(stage));
    let mut last_poll = Instant::now();
    loop {
        check_cancel(cancel)?;
        if last_poll.elapsed() >= Duration::from_millis(250) {
            last_poll = Instant::now();
            let health = xray
                .refresh_status()
                .map_err(|error| format!("{stage} prompt X-ray status unavailable: {error}"))?;
            let beam_on = health.beam_on;
            let off_confirmed = health.beam_off_confirmed;
            set_xray_health(progress, health);
            set_beam(progress, beam_on);
            if beam_on || !off_confirmed {
                return Err(format!(
                    "{stage} prompt interrupted: X-ray OFF is unconfirmed"
                ));
            }
        }
        {
            let mut state = gate.lock().expect("scan prompt mutex poisoned");
            if state.confirmed {
                state.pending = None;
                state.confirmed = false;
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    update(progress, |state| state.prompt = None);
    check_cancel(cancel)?;
    let off = xray
        .force_off()
        .map_err(|error| format!("{stage} confirmation requires confirmed X-ray OFF: {error}"))?;
    warning_after_off(nano, xray)
        .map_err(|error| format!("{stage} confirmation warning OFF failed: {error}"))?;
    set_xray_health(progress, off);
    set_beam(progress, false);
    Ok(())
}

fn reference_list<'a>(references: &'a ReferenceFrames, group: &str) -> &'a Vec<ReferenceFrame> {
    match group {
        "pre-dark" => &references.pre_dark,
        "pre-flat" => &references.pre_flat,
        "post-flat" => &references.post_flat,
        "post-dark" => &references.post_dark,
        _ => unreachable!("reference group is fixed by the scan coordinator"),
    }
}

fn reference_list_mut<'a>(
    references: &'a mut ReferenceFrames,
    group: &str,
) -> &'a mut Vec<ReferenceFrame> {
    match group {
        "pre-dark" => &mut references.pre_dark,
        "pre-flat" => &mut references.pre_flat,
        "post-flat" => &mut references.post_flat,
        "post-dark" => &mut references.post_dark,
        _ => unreachable!("reference group is fixed by the scan coordinator"),
    }
}

fn capture_reference_group(
    nano: &NanoAdapter,
    camera: &mut CameraAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    progress: &Mutex<RealScanProgress>,
    root: &Path,
    group: &'static str,
    stage: &'static str,
    flat: bool,
) -> Result<(), String> {
    let starting_at = {
        let state = progress.lock().expect("scan progress mutex poisoned");
        reference_list(&state.references, group).len()
    };
    if starting_at == REFERENCE_COUNT {
        return Ok(());
    }
    update(progress, |state| state.stage = stage);
    persist_progress(root, &config.parameters, config, progress, stage, false)?;
    // Every dark image is taken only after verified OFF; no warning is asserted.
    if !flat {
        xray.force_off()
            .map_err(|error| format!("{group} beam OFF failed: {error}"))?;
        warning_after_off(nano, xray)
            .map_err(|error| format!("{group} warning OFF failed: {error}"))?;
        set_xray_health(progress, xray.health());
        set_beam(progress, false);
    }
    let save_path = root.join("references");
    let block_limit = Duration::from_secs(u64::from(config.max_xray_sec));
    let exposure_timeout = Duration::from_secs_f64(config.parameters.exposure_ms / 1000.0)
        .saturating_add(CAMERA_TRIGGER_MARGIN);
    let flat_started_at = if flat {
        check_cancel(cancel)?;
        nano.set_xray_warning(true)
            .map_err(|error| format!("{group} warning ON failed: {error}"))?;
        let started = Instant::now();
        xray.beam_on_shared(cancel.clone(), started + block_limit)
            .map_err(|error| format!("{group} beam ON failed: {error}"))?;
        set_xray_health(progress, xray.health());
        set_beam(progress, true);
        push_message(
            progress,
            "ACTION",
            "xray",
            format!(
                "{group} continuous beam confirmed ON · remaining references {}",
                REFERENCE_COUNT - starting_at
            ),
        );
        Some(started)
    } else {
        None
    };
    for index in starting_at as u32 + 1..=REFERENCE_COUNT as u32 {
        check_cancel(cancel)?;
        if !flat {
            let health = xray
                .refresh_status()
                .map_err(|error| format!("{group} dark capture X-ray status failed: {error}"))?;
            if !health.beam_off_confirmed || health.beam_on {
                return Err(format!("{group} dark capture requires confirmed X-ray OFF"));
            }
            set_xray_health(progress, health);
        } else {
            let started = flat_started_at.expect("flat group has one continuous beam block");
            if started
                .elapsed()
                .saturating_add(exposure_timeout)
                .saturating_add(EXPOSURE_BUDGET_MARGIN)
                >= block_limit
            {
                return Err(format!("{group} cannot start reference {index}: continuous X-ray limit would be exceeded"));
            }
            let health = refresh_emission_monitored(xray, progress, cancel)
                .map_err(|error| {
                    xray_operation_error(
                        error,
                        &format!("{group} emission check before reference {index} failed"),
                    )
                })?;
            set_xray_health(progress, health);
        }
        let mut abort = None;
        let mut beam_shutdown_attempted = false;
        let capture_result = thread::scope(|scope| {
            let (sender, receiver) = mpsc::channel();
            let save_path_for_capture = save_path.clone();
            let camera_for_capture = &mut *camera;
            scope.spawn(move || {
                let result = camera_for_capture.capture_frame_cancellable(
                    &save_path_for_capture,
                    group,
                    index,
                    cancel,
                );
                let _ = sender.send(result);
            });
            let mut last_poll = Instant::now();
            loop {
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(result) => break result,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        break Err(CameraError::Io(
                            "reference capture worker disconnected".into(),
                        ))
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if cancel.load(Ordering::SeqCst) && abort.is_none() {
                    abort = Some(CANCELLED.to_owned());
                }
                if !flat && last_poll.elapsed() >= Duration::from_millis(250) && abort.is_none() {
                    last_poll = Instant::now();
                    match xray.refresh_status() {
                        Ok(health) if health.beam_off_confirmed && !health.beam_on => {
                            set_xray_health(progress, health);
                        }
                        Ok(health) => {
                            set_xray_health(progress, health);
                            abort = Some(format!("{group} dark capture lost confirmed X-ray OFF"));
                        }
                        Err(error) => {
                            abort =
                                Some(format!("{group} dark capture X-ray status failed: {error}"));
                        }
                    }
                    if abort.is_some() {
                        cancel.store(true, Ordering::SeqCst);
                        let _ = xray.force_off();
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        let _ = nano.stop();
                    }
                }
                if flat {
                    if last_poll.elapsed() >= Duration::from_millis(250) && abort.is_none() {
                        last_poll = Instant::now();
                        if let Err(error) = refresh_emission_monitored(xray, progress, cancel) {
                            abort = Some(xray_operation_error(
                                error,
                                &format!("{group} emission monitor failed"),
                            ));
                            cancel.store(true, Ordering::SeqCst);
                        } else {
                            set_xray_health(progress, xray.health());
                        }
                    }
                    if abort.is_none()
                        && flat_started_at.is_some_and(|started| started.elapsed() >= block_limit)
                    {
                        abort = Some(format!(
                            "{group} continuous X-ray limit reached during reference {index}"
                        ));
                        cancel.store(true, Ordering::SeqCst);
                    }
                    if abort.is_some() && !beam_shutdown_attempted {
                        beam_shutdown_attempted = true;
                        let off = xray.force_off();
                        let warning_off = warning_after_off(nano, xray);
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        if let Err(error) = off {
                            abort = Some(format!("{group} beam OFF failed: {error}"));
                            cancel.store(true, Ordering::SeqCst);
                        }
                        if let Err(error) = warning_off {
                            abort = Some(format!("{group} warning OFF failed: {error}"));
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                }
                if abort.is_some() && !flat {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        });
        if let Some(error) = abort {
            return Err(error);
        }
        check_cancel(cancel)?;
        let path =
            capture_result.map_err(|error| format!("{group} capture {index} failed: {error}"))?;
        if flat {
            if flat_started_at.is_some_and(|started| started.elapsed() >= block_limit) {
                return Err(format!(
                    "{group} continuous X-ray limit reached during reference {index}"
                ));
            }
            if index == REFERENCE_COUNT as u32 {
                let off = xray
                    .force_off()
                    .map_err(|error| format!("{group} beam OFF failed: {error}"))?;
                set_xray_health(progress, off);
                warning_after_off(nano, xray)
                    .map_err(|error| format!("{group} warning OFF failed: {error}"))?;
                set_beam(progress, false);
                push_message(
                    progress,
                    "PASS",
                    "xray",
                    format!(
                        "{group} continuous beam OFF confirmed after {REFERENCE_COUNT} references"
                    ),
                );
            }
        }
        let (bytes, sha256) = hash_file(&path)?;
        let frame = ReferenceFrame {
            index,
            exposure_ms: config.parameters.exposure_ms,
            file_name: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned(),
            path: path.display().to_string(),
            bytes,
            sha256,
        };
        update(progress, |state| {
            reference_list_mut(&mut state.references, group).push(frame)
        });
        persist_progress(root, &config.parameters, config, progress, stage, false)?;
        push_message(
            progress,
            "PASS",
            "camera",
            format!("{group} reference {index}/{REFERENCE_COUNT} saved"),
        );
    }
    Ok(())
}

fn record_projection_sample(value: &mut RealScanProgress, index: u32, now: Instant) {
    if let Some(started) = value.projection_started.take() {
        let active = now
            .saturating_duration_since(started)
            .saturating_sub(value.projection_cooling_elapsed);
        value.measured_projection_time = Some(match value.measured_projection_time {
            Some(previous) => (previous.saturating_mul(index - 1) + active) / index,
            None => active,
        });
    }
    value.projection_cooling_elapsed = Duration::ZERO;
}

fn close_scan_beam(
    nano: &NanoAdapter,
    xray: &mut MoxtekAdapter,
    progress: &Mutex<RealScanProgress>,
    beam_started: &mut Option<Instant>,
    reason: &str,
) -> Result<Duration, String> {
    let elapsed = beam_started
        .take()
        .map(|started| started.elapsed())
        .unwrap_or_default();
    let health = xray
        .force_off()
        .map_err(|error| format!("Moxtek OFF failed at {reason}: {error}"))?;
    set_xray_health(progress, health);
    warning_after_off(nano, xray)
        .map_err(|error| format!("XRAY_WARNING OFF failed at {reason}: {error}"))?;
    set_beam(progress, false);
    update(progress, |value| value.beam_started = None);
    push_message(
        progress,
        "PASS",
        "xray",
        format!(
            "{reason} · beam OFF after {:.3} s continuous output",
            elapsed.as_secs_f64()
        ),
    );
    Ok(elapsed)
}

fn projection_job(zero_index: u32, projection_count: u32) -> ProjectionJob {
    let angle_mdeg = -(((i64::from(zero_index) * 360_000) + i64::from(projection_count / 2))
        / i64::from(projection_count)) as i32;
    ProjectionJob {
        index: zero_index + 1,
        angle_mdeg,
        angle_deg: f64::from(angle_mdeg) / 1_000.0,
    }
}

fn expected_pulses(angle_mdeg: i32, pulses_per_rev: u32) -> i64 {
    let magnitude = i64::from(angle_mdeg).unsigned_abs() % 360_000;
    ((u64::from(pulses_per_rev) * magnitude + 180_000) / 360_000) as i64
        * if angle_mdeg < 0 { -1 } else { 1 }
}

fn verify_capture_hold(
    nano: &NanoAdapter,
    ticket: &MoveTicket,
    job: ProjectionJob,
) -> Result<f64, String> {
    let status = nano
        .status()
        .map_err(|error| format!("Nano STATUS failed before view {}: {error}", job.index))?;
    verify_fixed_configuration(&status).map_err(|error| error.to_string())?;
    let expected = expected_pulses(job.angle_mdeg, status.pulses_per_rev);
    if status.state != "CAPTURE_HOLD"
        || status.capture_id != ticket.command_id
        || ticket.position_pulses != status.position_pulses
        || status.target_pulses != status.position_pulses
        || status
            .position_pulses
            .rem_euclid(i64::from(status.pulses_per_rev))
            != expected.rem_euclid(i64::from(status.pulses_per_rev))
        || !status.reference_valid
        || !status.homed
        || !status.rearmed
    {
        return Err(format!(
            "view {} threshold check failed: state={} capture_id={} ticket={} status={} expected={} reference={} homed={} rearmed={}",
            job.index,
            status.state,
            status.capture_id,
            ticket.position_pulses,
            status.position_pulses,
            expected,
            status.reference_valid,
            status.homed,
            status.rearmed
        ));
    }
    Ok(status.position_pulses as f64 * 360.0 / f64::from(status.pulses_per_rev))
}

fn move_to_projection(
    nano: &NanoAdapter,
    progress: &Mutex<RealScanProgress>,
    job: ProjectionJob,
    cancel: &Arc<AtomicBool>,
) -> Result<MoveTicket, String> {
    check_cancel(cancel)?;
    push_message(
        progress,
        "ACTION",
        "nano",
        format!(
            "Prescribed-direction MOVE_REL view={} nominal angle={:.3}°",
            job.index, job.angle_deg
        ),
    );
    let starting_status = nano
        .status()
        .map_err(|error| format!("STATUS before view {} failed: {error}", job.index))?;
    let plan = plan_projection(&starting_status, job.angle_mdeg)
        .map_err(|error| format!("Plan view {} failed: {error}", job.index))?;
    let movement = nano.execute_plan_cancelled(&plan, cancel.clone());
    check_cancel(cancel)?;
    let ticket =
        movement.map_err(|error| format!("MOVE_REL view {} failed: {error}", job.index))?;
    let status = nano.status().map_err(|error| error.to_string())?;
    verify_plan_capture_hold(&plan, &ticket, &status).map_err(|error| error.to_string())?;
    let confirmed_angle = verify_capture_hold(nano, &ticket, job)?;
    // Only confirmed device position may advance the live scene. The planned
    // target is not a measurement, including while the move is still pending.
    set_angle(progress, confirmed_angle);
    push_message(
        progress,
        "PASS",
        "nano",
        format!(
            "READY_TO_CAPTURE view={} id={} pos={}",
            job.index, ticket.command_id, ticket.position_pulses
        ),
    );
    Ok(ticket)
}

fn move_to_projection_monitored(
    nano: &NanoAdapter,
    xray: &mut MoxtekAdapter,
    progress: &Mutex<RealScanProgress>,
    job: ProjectionJob,
    cancel: &Arc<AtomicBool>,
    beam_started: &mut Option<Instant>,
    block_limit: Duration,
    cooldown_until: &mut Option<Instant>,
) -> Result<MoveTicket, String> {
    if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
        close_scan_beam(
            nano,
            xray,
            progress,
            beam_started,
            "Continuous X-ray limit reached before turntable movement",
        )?;
        let deadline = Instant::now() + XRAY_COOLDOWN;
        wait_for_cooldown(progress, cancel, deadline)?;
    }
    if beam_started.is_none() {
        return move_to_projection(nano, progress, job, cancel);
    }
    // Nano MOVE_ABS may block for minutes. Keep the Moxtek watchdog alive and
    // enforce the continuous-output deadline while Nano owns its serial worker.
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        scope.spawn(move || {
            let _ = sender.send(move_to_projection(nano, progress, job, cancel));
        });
        let mut last_poll = Instant::now();
        let mut abort: Option<String> = None;
        let mut warning_off_after_move = false;
        loop {
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => {
                    if warning_off_after_move {
                        warning_after_off(nano, xray).map_err(|error| {
                            format!("XRAY_WARNING OFF failed after timed move: {error}")
                        })?;
                    }
                    if abort.is_none()
                        && beam_started.is_some_and(|started| started.elapsed() >= block_limit)
                    {
                        close_scan_beam(
                            nano,
                            xray,
                            progress,
                            beam_started,
                            "Continuous X-ray limit reached at turntable arrival",
                        )?;
                        *cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                        update(progress, |value| value.cooldown_until = *cooldown_until);
                    }
                    return abort.map_or(result, Err);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Nano move worker terminated without a result".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if abort.is_some() {
                continue;
            }
            if cancel.load(Ordering::SeqCst) {
                abort = Some(CANCELLED.into());
                let _ = xray.force_off();
                set_xray_health(progress, xray.health());
                set_beam(progress, xray.health().beam_on);
                let _ = nano.stop();
                continue;
            }
            if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
                // Moxtek is independent of Nano's serial worker. Shut the beam
                // immediately; queue warning OFF only once MOVE_ABS returns.
                match xray.force_off() {
                    Ok(health) => {
                        set_xray_health(progress, health);
                        set_beam(progress, false);
                        update(progress, |value| value.beam_started = None);
                        *beam_started = None;
                        warning_off_after_move = true;
                        *cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                        update(progress, |value| value.cooldown_until = *cooldown_until);
                        push_message(
                            progress,
                            "WARN",
                            "xray",
                            "Continuous X-ray limit reached during movement · beam OFF confirmed"
                                .into(),
                        );
                    }
                    Err(error) => {
                        abort = Some(format!("Moxtek OFF failed during move: {error}"));
                        let _ = nano.stop();
                    }
                }
            } else if beam_started.is_some() && last_poll.elapsed() >= Duration::from_millis(250) {
                last_poll = Instant::now();
                match refresh_emission_monitored(xray, progress, cancel) {
                    Ok(health) => set_xray_health(progress, health),
                    Err(error) => {
                        abort = Some(xray_operation_error(
                            error,
                            "Moxtek emission monitor failed during move",
                        ));
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        let _ = nano.stop();
                    }
                }
            }
        }
    })
}

fn commit_projection_monitored(
    nano: &NanoAdapter,
    xray: &mut MoxtekAdapter,
    progress: &Mutex<RealScanProgress>,
    root: &Path,
    parameters: &Parameters,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    path: &Path,
    job: ProjectionJob,
    ticket: &MoveTicket,
    beam_started: &mut Option<Instant>,
    block_limit: Duration,
    cooldown_until: &mut Option<Instant>,
    final_projection: bool,
) -> Result<RealFrame, String> {
    let mut frames = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .frames
        .clone();
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        scope.spawn(move || {
            let result = (|| {
                let (bytes, sha256) = hash_file(path)?;
                let frame = RealFrame {
                    index: job.index,
                    angle_deg: job.angle_deg,
                    exposure_ms: parameters.exposure_ms,
                    file_name: path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("frame.nef")
                        .to_owned(),
                    path: path.display().to_string(),
                    bytes,
                    sha256,
                };
                check_cancel(cancel)?;
                nano.capture_done_cancelled(ticket.command_id, cancel.clone())
                    .map_err(|error| format!("CAPTURE_DONE view {} failed: {error}", job.index))?;
                check_cancel(cancel)?;
                frames.push(frame.clone());
                let references = progress
                    .lock()
                    .expect("scan progress mutex poisoned")
                    .references
                    .clone();
                persist(
                    root,
                    parameters,
                    config,
                    &frames,
                    &references,
                    "projections",
                    false,
                )?;
                push_frame(progress, frame.clone());
                Ok(frame)
            })();
            let _ = sender.send(result);
        });
        let mut last_poll = Instant::now();
        let mut abort: Option<String> = None;
        let mut warning_off_after_commit = false;
        loop {
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => {
                    if warning_off_after_commit {
                        warning_after_off(nano, xray).map_err(|error| {
                            format!("XRAY_WARNING OFF failed after frame commit: {error}")
                        })?;
                    }
                    return abort.map_or(result, Err);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("frame commit worker terminated without a result".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if abort.is_some() || beam_started.is_none() {
                continue;
            }
            if cancel.load(Ordering::SeqCst) {
                abort = Some(CANCELLED.into());
                let _ = xray.force_off();
                set_xray_health(progress, xray.health());
                set_beam(progress, xray.health().beam_on);
                let _ = nano.stop();
            } else if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
                match xray.force_off() {
                    Ok(health) => {
                        set_xray_health(progress, health);
                        set_beam(progress, false);
                        update(progress, |value| value.beam_started = None);
                        *beam_started = None;
                        warning_off_after_commit = true;
                        if !final_projection {
                            *cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                            update(progress, |value| value.cooldown_until = *cooldown_until);
                        }
                    }
                    Err(error) => {
                        abort = Some(format!("Moxtek OFF failed during frame commit: {error}"));
                        let _ = nano.stop();
                    }
                }
            } else if last_poll.elapsed() >= Duration::from_millis(250) {
                last_poll = Instant::now();
                match refresh_emission_monitored(xray, progress, cancel) {
                    Ok(health) => set_xray_health(progress, health),
                    Err(error) => {
                        abort = Some(xray_operation_error(
                            error,
                            "Moxtek emission monitor failed during frame commit",
                        ));
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        let _ = nano.stop();
                    }
                }
            }
        }
    })
}

struct ExposureEvidence {
    session_id: String,
    operation_id: String,
    exposure_ended: bool,
    transfer_started: bool,
}
impl ExposureEvidence {
    fn new(session_id: String, operation_id: String) -> Self {
        Self {
            session_id,
            operation_id,
            exposure_ended: false,
            transfer_started: false,
        }
    }
    fn observe(&mut self, event: &CameraCaptureEvent) -> Result<(), String> {
        let (session, operation) = match event {
            CameraCaptureEvent::ExposureEnded {
                session_id,
                operation_id,
            }
            | CameraCaptureEvent::ObjectAdded {
                session_id,
                operation_id,
                ..
            }
            | CameraCaptureEvent::TransferStarted {
                session_id,
                operation_id,
                ..
            } => (session_id, operation_id),
            CameraCaptureEvent::FileComplete { image } => (&image.session_id, &image.operation_id),
        };
        if session != &self.session_id || operation != &self.operation_id {
            return Err("Capture event belongs to a different session or operation".into());
        }
        match event {
            CameraCaptureEvent::ExposureEnded { .. } => {
                if self.exposure_ended {
                    return Err("Duplicate ExposureEnded event".into());
                }
                self.exposure_ended = true;
            }
            CameraCaptureEvent::TransferStarted { .. } => self.transfer_started = true,
            _ => {}
        }
        Ok(())
    }
}

fn needs_cooldown_before_exposure(
    beam_started: Instant,
    now: Instant,
    block_limit: Duration,
    required_budget: Duration,
) -> bool {
    now.saturating_duration_since(beam_started)
        .saturating_add(required_budget)
        >= block_limit
}

fn update(progress: &Mutex<RealScanProgress>, apply: impl FnOnce(&mut RealScanProgress)) {
    if let Ok(mut progress) = progress.lock() {
        apply(&mut progress);
        progress.revision = progress.revision.wrapping_add(1);
    }
}

fn update_phase(progress: &Mutex<RealScanProgress>, phase: &'static str) {
    update(progress, |value| value.phase = phase);
}

fn set_angle(progress: &Mutex<RealScanProgress>, angle_deg: f64) {
    update(progress, |value| value.angle_deg = angle_deg);
}

fn set_beam(progress: &Mutex<RealScanProgress>, beam_on: bool) {
    update(progress, |value| value.beam_on = beam_on);
}

fn log_xray_diagnostics(xray: &MoxtekAdapter, progress: &Mutex<RealScanProgress>) {
    for message in xray.drain_diagnostics() { push_message(progress, "WARN", "xray", message); }
}

fn refresh_emission_monitored(xray: &mut MoxtekAdapter, progress: &Mutex<RealScanProgress>, cancel: &Arc<AtomicBool>) -> Result<XrayHealth, XrayError> {
    let result = xray.refresh_emission_status_shared(cancel.clone());
    // Scan owns the proxy while running. Persist serial diagnostics in scan.log
    // even when the domain's standalone observer has no X-ray adapter.
    log_xray_diagnostics(xray, progress);
    result
}

fn set_xray_health(progress: &Mutex<RealScanProgress>, health: XrayHealth) {
    update(progress, |value| value.xray_health = Some(health));
}

fn push_message(
    progress: &Mutex<RealScanProgress>,
    level: &'static str,
    source: &'static str,
    message: String,
) {
    update(progress, |value| {
        value.messages.push(ScanMessage {
            timestamp: timestamp(),
            level,
            source,
            message,
        });
    });
}

fn persist_scan_log(
    root: &Path,
    progress: &Mutex<RealScanProgress>,
    result: &Result<ScanCompletion, String>,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("scan.log"))
        .map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    let messages = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .messages
        .clone();
    write_scan_log(&mut writer, &messages, result)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)?;
    Ok(())
}

fn write_scan_log(
    writer: &mut impl Write,
    messages: &[ScanMessage],
    result: &Result<ScanCompletion, String>,
) -> Result<(), String> {
    writeln!(writer, "Micro-CT scan log").map_err(io_error)?;
    for event in messages {
        writeln!(
            writer,
            "{}\t{}\t{}\t{}",
            event.timestamp,
            event.level,
            event.source,
            event.message.replace(['\r', '\n'], " ")
        )
        .map_err(io_error)?;
    }
    let outcome = match result {
        Ok(ScanCompletion::Completed) => "COMPLETED".to_owned(),
        Ok(ScanCompletion::Stopped) => "STOPPED".to_owned(),
        Err(error) => format!("FAULT: {}", error.replace(['\r', '\n'], " ")),
    };
    writeln!(writer, "{}\tRESULT\tsystem\t{outcome}", timestamp()).map_err(io_error)?;
    Ok(())
}

fn push_frame(progress: &Mutex<RealScanProgress>, frame: RealFrame) {
    update(progress, |value| {
        value.captured = frame.index;
        // A prefetched move may already have reached the next view while this
        // older image transfers. Committing a frame must not rewind live position.
        value.frames.push(frame);
    });
}

fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = File::open(path).map_err(io_error)?;
    let bytes = file.metadata().map_err(io_error)?.len();
    if bytes == 0 {
        return Err(format!("captured file is empty: {}", path.display()));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}

fn persist(
    root: &Path,
    parameters: &Parameters,
    config: &RealScanConfig,
    frames: &[RealFrame],
    references: &ReferenceFrames,
    stage: &str,
    completed: bool,
) -> Result<(), String> {
    let manifest = ScanManifest {
        schema_version: 3,
        capture_profile: &config.capture_profile,
        max_xray_sec: config.max_xray_sec,
        scan_id: &parameters.task_id,
        created_at: timestamp(),
        completed,
        voltage_kv: config.voltage_kv,
        current_ua: config.current_ua,
        projection_count: parameters.projection_count,
        exposure_ms: parameters.exposure_ms,
        geometry: &config.geometry,
        references,
        stage,
        frames,
    };
    let staging_path = root.join("manifest.next.json");
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&staging_path)
        .map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &manifest).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)?;
    drop(writer);

    let csv = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(root.join("acquisition.csv"))
        .map_err(io_error)?;
    let mut csv = BufWriter::new(csv);
    csv.write_all(b"index,angle_deg,exposure_ms,file_name,bytes,sha256\n")
        .map_err(io_error)?;
    for frame in frames {
        writeln!(
            csv,
            "{},{:.6},{},{},{},{}",
            frame.index,
            frame.angle_deg,
            frame.exposure_ms,
            frame.file_name,
            frame.bytes,
            frame.sha256
        )
        .map_err(io_error)?;
    }
    csv.flush().map_err(io_error)?;
    csv.get_ref().sync_all().map_err(io_error)?;
    drop(csv);
    replace_manifest(&staging_path, &root.join("manifest.json"))?;
    Ok(())
}

#[cfg(windows)]
fn replace_manifest(staging: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "Kernel32")]
    extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *const std::ffi::c_void,
            reserved: *const std::ffi::c_void,
        ) -> i32;
    }
    if !destination.exists() {
        return fs::rename(staging, destination).map_err(io_error);
    }
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let destination_wide = wide(destination);
    let staging_wide = wide(staging);
    // ReplaceFileW keeps the prior manifest intact if replacement fails.
    let replaced = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            staging_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        Err(io_error(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_manifest(staging: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(staging, destination).map_err(io_error)
}

fn persist_progress(
    root: &Path,
    parameters: &Parameters,
    config: &RealScanConfig,
    progress: &Mutex<RealScanProgress>,
    stage: &str,
    completed: bool,
) -> Result<(), String> {
    let state = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .clone();
    persist(
        root,
        parameters,
        config,
        &state.frames,
        &state.references,
        stage,
        completed,
    )
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn scintillator_layers_round_trip_with_geometry_and_legacy_geometry_stays_readable() {
        let legacy=serde_json::json!({"sodMm":40.0,"objectToDetectorMm":25.0,"detectorWidthMm":40.0,
            "centerOffsetXMm":0.0,"centerOffsetYMm":0.0,"rotationDirection":"counterclockwise","mirrorX":false,"measurement":"measured"});
        let mut geometry:ScanGeometry=serde_json::from_value(legacy.clone()).unwrap();
        assert!(geometry.scintillator_layers.is_empty());assert_eq!(serde_json::to_value(&geometry).unwrap(),legacy);
        geometry.scintillator_layers=vec![crate::reconstruction::spectral::StackLayer{material_name:"scintillator glass".into(),emission_color:"green".into(),thickness_mm:1.},
            crate::reconstruction::spectral::StackLayer{material_name:"scintillator glass".into(),emission_color:"blue".into(),thickness_mm:3.}];
        geometry.validate().unwrap();
        let frozen:ScanGeometry=serde_json::from_value(serde_json::to_value(&geometry).unwrap()).unwrap();assert_eq!(frozen,geometry);
        geometry.scintillator_layers[1].thickness_mm=0.;assert_eq!(geometry.validate(),Err("INVALID_SCINTILLATOR_STACK"));
    }
    use super::*;
    use crate::devices::turntable::{LineTransport, EXPECTED_DEVICE};
    use std::collections::VecDeque;

    fn test_profile_for_exposure(exposure_ms: f64) -> CaptureProfile {
        let profile = crate::capture_profile::test_profile();
        let mut readback = profile.readback;
        assert_eq!(exposure_ms, 1000.0);
        readback.insert(
            "shutter".into(),
            crate::devices::camera::CameraValue::Integer(0x00010001),
        );
        CaptureProfile::from_readback(profile.identity, readback, None, 0xd100).unwrap()
    }

    fn test_reference_series(root: &Path, group: &str) -> Vec<ReferenceFrame> {
        let dir = root.join("references").join(group).join("frames");
        fs::create_dir_all(&dir).unwrap();
        (1..=REFERENCE_COUNT as u32)
            .map(|index| {
                let file_name = format!("frame-{index:04}.nef");
                let path = dir.join(&file_name);
                fs::write(&path, format!("{group} reference {index}")).unwrap();
                let (bytes, sha256) = hash_file(&path).unwrap();
                ReferenceFrame {
                    index,
                    exposure_ms: 1000.0,
                    file_name,
                    path: path.to_string_lossy().into_owned(),
                    bytes,
                    sha256,
                }
            })
            .collect()
    }

    #[test]
    fn restore_keeps_274_verified_frames_and_rejects_an_uncommitted_275th() {
        let run_id = format!(
            "{}-restore-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            std::process::id()
        );
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(run_id);
        let root = run.join("demo6");
        let frames_dir = root.join("frames");
        fs::create_dir_all(&frames_dir).unwrap();
        fs::write(run.join("command.txt"), "cargo test -p ct-engine scan::tests::restore_keeps_274_verified_frames_and_rejects_an_uncommitted_275th\n").unwrap();
        let parameters = Parameters {
            task_id: "demo6".into(),
            save_path: run.to_string_lossy().into_owned(),
            projection_count: 360,
            angle_step_deg: 1.0,
            exposure_ms: 1000.0,
        };
        let config = RealScanConfig {
            capture_profile: test_profile_for_exposure(parameters.exposure_ms),
            parameters: parameters.clone(),
            max_xray_sec: 600,
            voltage_kv: 59.9,
            current_ua: 200.0,
            resume: None,
            geometry: ScanGeometry {
                sod_mm: 100.0,
                object_to_detector_mm: 100.0,
                detector_width_mm: 100.0,
                center_offset_x_mm: 0.0,
                center_offset_y_mm: 0.0,
                rotation_direction: "clockwise".into(),
                mirror_x: false,
                measurement: "measured".into(),
                scintillator_layers:vec![],
            },
        };
        let mut committed = Vec::new();
        for index in 1..=274 {
            let path = frames_dir.join(format!("frame-{index:04}.nef"));
            fs::write(&path, format!("verified projection {index}")).unwrap();
            let (bytes, sha256) = hash_file(&path).unwrap();
            committed.push(RealFrame {
                index,
                angle_deg: projection_job(index - 1, 360).angle_deg,
                exposure_ms: 1000.0,
                file_name: format!("frame-{index:04}.nef"),
                path: path.to_string_lossy().into_owned(),
                bytes,
                sha256,
            });
        }
        persist(
            &root,
            &parameters,
            &config,
            &committed,
            &ReferenceFrames::default(),
            "projections",
            false,
        )
        .unwrap();
        assert!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap_err()
                .contains("stage and committed"),
            "projection restore must reject missing pre-scan references"
        );
        let mut references = ReferenceFrames {
            pre_dark: test_reference_series(&root, "pre-dark"),
            pre_flat: test_reference_series(&root, "pre-flat"),
            ..ReferenceFrames::default()
        };
        persist(
            &root,
            &parameters,
            &config,
            &committed,
            &references,
            "projections",
            false,
        )
        .unwrap();
        assert!(manifest_file_exists(&parameters));
        assert!(resume_manifest_exists(&parameters));
        assert_eq!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap()
                .frames,
            committed
        );
        assert_eq!(projection_job(committed.len() as u32, 360).index, 275);
        assert_eq!(
            projection_job(committed.len() as u32, 360).angle_deg,
            -274.0
        );
        let incomplete = frames_dir.join("frame-0275.nef");
        fs::write(&incomplete, b"incomplete exposure").unwrap();
        assert!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap_err()
                .contains("uncommitted")
        );
        fs::remove_file(incomplete).unwrap();
        assert_eq!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap()
                .frames
                .len(),
            274
        );
        references.post_flat = test_reference_series(&root, "post-flat");
        references.post_dark = test_reference_series(&root, "post-dark");
        persist(
            &root,
            &parameters,
            &config,
            &committed,
            &references,
            "postDark",
            false,
        )
        .unwrap();
        assert!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap_err()
                .contains("stage and committed"),
            "post-dark checkpoint cannot claim completion before the last projection"
        );
        persist(
            &root,
            &parameters,
            &config,
            &committed,
            &references,
            "completed",
            true,
        )
        .unwrap();
        assert!(
            !resume_manifest_exists(&parameters),
            "a completed manifest is not a resumable checkpoint"
        );
        assert!(
            !completed_manifest_exists(&parameters),
            "incomplete projections cannot unlock reconstruction"
        );
        fs::write(run.join("summary.md"), "PASS: 274 committed frames reload; an extra partial frame blocks restore; removing only the partial frame permits view 275.\n").unwrap();
    }

    #[test]
    fn completed_scan_needs_only_pre_scan_references() {
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(format!(
                "{}-pre-only-{}",
                chrono::Utc::now().format("%Y%m%d-%H%M%S"),
                std::process::id()
            ));
        let root = run.join("pre-only");
        let frames_dir = root.join("frames");
        fs::create_dir_all(&frames_dir).unwrap();
        let path = frames_dir.join("frame-0001.nef");
        fs::write(&path, b"projection").unwrap();
        let (bytes, sha256) = hash_file(&path).unwrap();
        let frame = RealFrame {
            index: 1,
            angle_deg: 0.0,
            exposure_ms: 1000.0,
            file_name: "frame-0001.nef".into(),
            path: path.to_string_lossy().into_owned(),
            bytes,
            sha256,
        };
        let parameters = Parameters {
            task_id: "pre-only".into(),
            save_path: run.to_string_lossy().into_owned(),
            projection_count: 1,
            angle_step_deg: 360.0,
            exposure_ms: 1000.0,
        };
        let config = RealScanConfig {
            capture_profile: test_profile_for_exposure(parameters.exposure_ms),
            parameters: parameters.clone(),
            voltage_kv: 59.9,
            current_ua: 200.0,
            max_xray_sec: 600,
            geometry: ScanGeometry {
                sod_mm: 100.0,
                object_to_detector_mm: 100.0,
                detector_width_mm: 40.0,
                center_offset_x_mm: 0.0,
                center_offset_y_mm: 0.0,
                rotation_direction: "counterclockwise".into(),
                mirror_x: false,
                measurement: "measured".into(),
                scintillator_layers:vec![],
            },
            resume: None,
        };
        let references = ReferenceFrames {
            pre_dark: test_reference_series(&root, "pre-dark"),
            pre_flat: test_reference_series(&root, "pre-flat"),
            ..ReferenceFrames::default()
        };
        persist(
            &root,
            &parameters,
            &config,
            &[frame],
            &references,
            "completed",
            true,
        )
        .unwrap();
        assert!(completed_manifest_exists(&parameters));
        let path = root.join("manifest.json");
        let mut legacy: serde_json::Value =
            serde_json::from_reader(File::open(&path).unwrap()).unwrap();
        legacy["schemaVersion"] = json!(2);
        legacy.as_object_mut().unwrap().remove("captureProfile");
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            completed_manifest_exists(&parameters),
            "legacy completed scans remain readable"
        );
        legacy["completed"] = json!(false);
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        let unfinished_before = fs::read(&path).unwrap();
        assert!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap_err()
                .contains("LEGACY_SCHEMA_2")
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            unfinished_before,
            "rejected legacy resume never rewrites data"
        );
        fs::write(&path, before).unwrap();
    }

    #[test]
    fn stopped_outcome_requires_confirmed_beam_and_warning_off() {
        assert_eq!(
            finish_scan_result(Err(CANCELLED.into()), Ok(()), Ok(())).unwrap(),
            ScanCompletion::Stopped
        );
        assert!(
            finish_scan_result(Err(CANCELLED.into()), Err("readback lost".into()), Ok(())).is_err()
        );
        assert!(
            finish_scan_result(Err(CANCELLED.into()), Ok(()), Err("warning stuck".into())).is_err()
        );
        assert_eq!(
            finish_scan_result(Ok(()), Ok(()), Ok(())).unwrap(),
            ScanCompletion::Completed
        );
    }

    #[test]
    fn disconnected_worker_outcome_is_a_terminal_fault() {
        let (sender, receiver) = mpsc::channel::<u8>();
        drop(sender);
        assert!(poll_scan_outcome(&receiver, &mut None).unwrap().is_err());
    }

    #[test]
    fn scan_monitor_keeps_serial_diagnostics_in_the_durable_scan_log() {
        let (mut xray, probe) = crate::devices::xray::test_worker(60.0, 200.0);
        xray.beam_on(&AtomicBool::new(false)).unwrap();
        probe.fail_next_read();
        // A fresh verified pair may be reused for 250 ms. Wait for the real
        // autonomous failure before checking its diagnostic delivery.
        let deadline = Instant::now() + Duration::from_secs(2);
        while xray.health().last_error.is_none() && Instant::now() < deadline { thread::sleep(Duration::from_millis(5)); }
        assert!(xray.health().last_error.is_some());
        let progress = Mutex::new(RealScanProgress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(refresh_emission_monitored(&mut xray, &progress, &cancel).is_err());
        let messages = progress.lock().unwrap().messages.clone();
        assert!(messages.iter().any(|entry| entry.source == "xray" && entry.message.contains("Moxtek serial command")));
        let mut output = Vec::new();
        write_scan_log(&mut output, &messages, &Err("communication fault".into())).unwrap();
        let log = String::from_utf8(output).unwrap();
        assert!(log.contains("Moxtek serial command"));
        assert!(log.contains("budget="));
        assert!(xray.drain_diagnostics().is_empty(), "diagnostics must not be logged twice");
    }

    #[test]
    fn scan_log_keeps_the_fault_and_all_scan_events() {
        let messages = vec![
            ScanMessage {
                timestamp: "2026-09-28T10:00:00Z".into(),
                level: "ACTION",
                source: "nano",
                message: "Verifying final position".into(),
            },
            ScanMessage {
                timestamp: "2026-09-28T10:00:01Z".into(),
                level: "ERR",
                source: "system",
                message: "final Nano STATUS failed\nread timed out".into(),
            },
        ];
        let mut output = Vec::new();
        write_scan_log(
            &mut output,
            &messages,
            &Err("final Nano STATUS failed: read timed out".into()),
        )
        .unwrap();
        let log = String::from_utf8(output).unwrap();
        assert!(log.contains("2026-09-28T10:00:00Z\tACTION\tnano\tVerifying final position"));
        assert!(log.contains(
            "2026-09-28T10:00:01Z\tERR\tsystem\tfinal Nano STATUS failed read timed out"
        ));
        assert!(log.contains("\tRESULT\tsystem\tFAULT: final Nano STATUS failed: read timed out"));
    }

    #[derive(Default)]
    struct PipelineNanoState {
        lines: VecDeque<String>,
        writes: Vec<String>,
        position: i64,
        capture_id: u32,
        rearmed: bool,
        homed: bool,
        on_capture_done: Option<Box<dyn FnOnce() + Send>>,
    }
    struct PipelineNano(Arc<Mutex<PipelineNanoState>>);
    impl LineTransport for PipelineNano {
        fn write_line(&mut self, line: &str) -> std::io::Result<()> {
            let fields: Vec<_> = line.split_whitespace().collect();
            let id = fields.get(1).copied().unwrap_or("0");
            let mut state = self.0.lock().unwrap();
            state.writes.push(line.into());
            match fields[0] {
                "PING" => {
                    state.lines.push_back(format!("ACK {id} PING"));
                    state.lines.push_back(format!("PONG {id}"));
                }
                "INFO" => {
                    state.lines.push_back(format!("ACK {id} INFO"));
                    state.lines.push_back(format!("INFO {id} DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=offline-test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"));
                }
                "HEARTBEAT" => state.lines.push_back(format!("HBACK {id}")),
                "STATUS" => {
                    let position = state.position;
                    let capture_id = state.capture_id;
                    let phase = if !state.rearmed {
                        "LOCKED"
                    } else if capture_id == 0 {
                        "IDLE"
                    } else {
                        "CAPTURE_HOLD"
                    };
                    let homed = u8::from(state.homed);
                    let rearmed = u8::from(state.rearmed);
                    state.lines.push_back(format!("ACK {id} STATUS"));
                    state.lines.push_back(format!("STATUS {id} state={phase} pos={position} target={position} microsteps=8 ppr=96000 reference={homed} homed={homed} rearmed={rearmed} hall=0 capture_id={capture_id}"));
                }
                "MOVE_REL" => {
                    let delta = fields[2].parse::<i64>().unwrap();
                    assert!((-360000..=0).contains(&delta));
                    state.position -= ((-delta) * 96000 + 180000) / 360000;
                    state.capture_id = id.parse().unwrap();
                    let position = state.position;
                    state.lines.push_back(format!("ACK {id} MOVE_REL"));
                    state
                        .lines
                        .push_back(format!("READY_TO_CAPTURE {id} POS={position}"));
                }
                "CAPTURE_DONE" => {
                    if id.parse::<u32>().unwrap() != state.capture_id {
                        state
                            .lines
                            .push_back(format!("ERR {id} CAPTURE_ID_MISMATCH"));
                    } else {
                        if let Some(inject) = state.on_capture_done.take() {
                            inject();
                        }
                        state.capture_id = 0;
                        state.lines.push_back(format!("ACK {id} CAPTURE_DONE"));
                        state.lines.push_back(format!("IDLE {id} CAPTURE_RELEASED"));
                    }
                }
                "XRAY_WARNING" => {
                    state.lines.push_back(format!("ACK {id} XRAY_WARNING"));
                    state
                        .lines
                        .push_back(format!("OK {id} XRAY_WARNING={}", fields[2]));
                }
                "STOP" => {
                    state.rearmed = false;
                    state.homed = false;
                    state.capture_id = 0;
                    state.lines.push_back(format!("ACK {id} STOP"));
                    state
                        .lines
                        .push_back(format!("STOPPED {id} POSITION_UNKNOWN reason=HOST_STOP"));
                }
                _ => panic!("Unexpected physical command in offline scan fake: {line}"),
            }
            Ok(())
        }
        fn read_line(&mut self, timeout: Duration) -> std::io::Result<Option<String>> {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(line) = self.0.lock().unwrap().lines.pop_front() {
                    return Ok(Some(line));
                }
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    fn pipeline_nano() -> (Arc<NanoAdapter>, Arc<Mutex<PipelineNanoState>>) {
        let state = Arc::new(Mutex::new(PipelineNanoState {
            rearmed: true,
            homed: true,
            ..Default::default()
        }));
        state.lock().unwrap().lines.push_back(format!("READY DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=offline-test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"));
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("MEMORY-SCAN-ONLY", Box::new(PipelineNano(state.clone())))
            .unwrap();
        (Arc::new(adapter), state)
    }
    fn pipeline_config(run: &Path, task: &str, camera: &CameraAdapter) -> RealScanConfig {
        let profile = CaptureProfile::from_camera_readback(
            &camera.health(),
            camera.profile_readback().unwrap(),
            None,
        )
        .unwrap();
        RealScanConfig {
            parameters: Parameters {
                task_id: task.into(),
                save_path: run.to_string_lossy().into(),
                projection_count: 3,
                angle_step_deg: 120.0,
                exposure_ms: 200.0,
            },
            capture_profile: profile,
            max_xray_sec: 600,
            voltage_kv: 20.0,
            current_ua: 100.0,
            geometry: ScanGeometry {
                sod_mm: 100.0,
                object_to_detector_mm: 100.0,
                detector_width_mm: 40.0,
                center_offset_x_mm: 0.0,
                center_offset_y_mm: 0.0,
                rotation_direction: "counterclockwise".into(),
                mirror_x: false,
                measurement: "measured".into(),
                scintillator_layers:vec![],
            },
            resume: None,
        }
    }
    fn complete_pipeline(
        mut handle: RealScanHandle,
        camera: &CameraAdapter,
        probe: &crate::devices::xray::TestXrayProbe,
        task: &str,
    ) -> (ScanOutcome, bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut confirmed = std::collections::BTreeSet::new();
        let mut early_off = false;
        loop {
            let progress = handle.progress();
            if let Some(stage) = progress.prompt {
                if !confirmed.contains(stage) && handle.confirm_stage(stage).is_ok() {
                    confirmed.insert(stage);
                }
            }
            let health = camera.health();
            if health.busy
                && health
                    .operation_id
                    .as_ref()
                    .is_some_and(|id| id.starts_with(&format!("scan-{task}-3-")))
                && !probe.output_enabled()
                && progress.captured < 3
            {
                early_off = true;
            }
            if let Some(outcome) = handle.try_finish() {
                return (outcome.unwrap(), early_off);
            }
            if Instant::now() >= deadline {
                handle.cancel_only();
                panic!(
                    "Offline production coordinator did not complete: {:?}",
                    handle.progress()
                );
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn production_coordinator_two_rounds_preserve_continuous_groups_and_end_beam_before_final_file()
    {
        use crate::devices::camera::{scripted_camera, ScriptedCameraConfig};
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(format!(
                "{}-native-production-coordinator",
                chrono::Local::now().format("%Y%m%d-%H%M%S")
            ));
        fs::create_dir_all(&run).unwrap();
        fs::write(
            run.join("command.txt"),
            "cargo test -p ct-engine scan::tests::production_coordinator_two_rounds\n",
        )
        .unwrap();
        let (nano, state) = pipeline_nano();
        let camera = scripted_camera(ScriptedCameraConfig {
            exposure_delay: Duration::from_millis(5),
            transfer_delay: Duration::from_millis(150),
            omit_exposure_end: false,
            truncate_raw: false,
            close_failure: false,
            native_exposure_ms: None,
            capture_started_signal: None,
            discovered: 1,
        });
        let (xray, probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let initial = probe.beam_transitions().len();
        for (task, expected_position) in [("round-a", -64000), ("round-b", -160000)] {
            let config = pipeline_config(&run, task, &camera);
            let parameters = config.parameters.clone();
            let handle = RealScanHandle::start(nano.clone(), camera.clone(), xray.clone(), config);
            let (outcome, early_off) = complete_pipeline(handle, &camera, &probe, task);
            assert_eq!(outcome.result.as_ref().unwrap(), &ScanCompletion::Completed);
            assert!(
                early_off,
                "final ExposureEnded closes output while transfer/file commit remains pending"
            );
            assert!(completed_manifest_exists(&parameters));
            let manifest: serde_json::Value =
                serde_json::from_reader(File::open(run.join(task).join("manifest.json")).unwrap())
                    .unwrap();
            assert_eq!(manifest["schemaVersion"], 3);
            assert_eq!(manifest["frames"].as_array().unwrap().len(), 3);
            assert_eq!(
                manifest["references"]["preDark"].as_array().unwrap().len(),
                10
            );
            assert_eq!(
                manifest["references"]["preFlat"].as_array().unwrap().len(),
                10
            );
            assert_eq!(
                manifest["references"]["postDark"].as_array().unwrap().len(),
                0
            );
            assert_eq!(
                manifest["references"]["postFlat"].as_array().unwrap().len(),
                0
            );
            assert_eq!(nano.status().unwrap().position_pulses, expected_position);
            assert!(!probe.output_enabled());
        }
        let transitions = probe.beam_transitions();
        assert_eq!(
            transitions[initial..]
                .iter()
                .filter(|(enabled, _)| *enabled)
                .count(),
            4,
            "one pre-flat block and one projection block per scan"
        );
        let state = state.lock().unwrap();
        assert!(!state
            .writes
            .iter()
            .any(|line| line.starts_with("HOME ") || line.starts_with("MOVE_ABS ")));
        assert_eq!(
            state
                .writes
                .iter()
                .filter(|line| line.starts_with("CAPTURE_DONE "))
                .count(),
            6
        );
        fs::write(run.join("summary.md"),"PASS: same production scan coordinator and all three production workers; two rounds, 10+10 front references, 3 projections, continuous blocks, ExposureEnded before file commit, matching hold release, no terminal HOME, original synthetic NEF retained. Only offline verification.\n").unwrap();
    }

    // R0 exercises the production coordinator and all three device owners.
    // Only their physical transports are scripted; UNKNOWN and file outcomes
    // are asserted after the complete cleanup path, never inferred from ACKs.
    fn r0_run(topic: &str) -> std::path::PathBuf {
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(format!("{}-system-coordinator-{topic}-{}",
                chrono::Local::now().format("%Y%m%d-%H%M%S"),
                chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("command.txt"), "cargo test -p ct-engine scan::tests::r0_ -- --nocapture\n").unwrap();
        println!("R0 EVIDENCE: {}", run.display());
        run
    }

    fn r0_reference_hashes(root: &Path) -> Vec<(String, u64, String)> {
        let references = root.join("references");
        let mut files: Vec<_> = ["pre-dark", "pre-flat"].into_iter()
            .flat_map(|group| {
                let files: Vec<_> = fs::read_dir(references.join(group).join("frames")).unwrap()
                    .map(|entry| entry.unwrap().path()).collect();
                assert_eq!(files.len(), REFERENCE_COUNT);
                files
            }).collect();
        files.sort();
        assert_eq!(files.len(), 20, "complete pre-dark and pre-flat groups exist before fault injection");
        files.into_iter().map(|path| {
            let (bytes, hash) = hash_file(&path).unwrap();
            (path.strip_prefix(&references).unwrap().to_string_lossy().into_owned(), bytes, hash)
        }).collect()
    }

    fn projection_resume_fixture(
        run: &Path, task: &str, camera: &CameraAdapter, stage: &str, committed_count: u32,
    ) -> RealScanConfig {
        let root = run.join(task);
        fs::create_dir_all(root.join("frames")).unwrap();
        let mut config = pipeline_config(run, task, camera);
        let mut references = ReferenceFrames {
            pre_dark: test_reference_series(&root, "pre-dark"),
            pre_flat: test_reference_series(&root, "pre-flat"),
            ..Default::default()
        };
        for frame in references.pre_dark.iter_mut().chain(&mut references.pre_flat) {
            frame.exposure_ms = config.parameters.exposure_ms;
        }
        let frames: Vec<_> = (1..=committed_count).map(|index| {
            let file_name = format!("frame-{index:04}.nef");
            let path = root.join("frames").join(&file_name);
            fs::write(&path, format!("retained projection {index}")).unwrap();
            let (bytes, sha256) = hash_file(&path).unwrap();
            RealFrame {
                index, angle_deg: projection_job(index - 1, config.parameters.projection_count).angle_deg,
                exposure_ms: config.parameters.exposure_ms, file_name,
                path: path.to_string_lossy().into_owned(), bytes, sha256,
            }
        }).collect();
        persist(&root, &config.parameters, &config, &frames, &references, stage, false).unwrap();
        let original_manifest = fs::read(root.join("manifest.json")).unwrap();
        config.resume = Some(load_resume(
            &config.parameters, config.voltage_kv, config.current_ua, &config.capture_profile,
        ).unwrap());
        assert_eq!(fs::read(root.join("manifest.json")).unwrap(), original_manifest,
            "Restore verifies and normalizes in memory without rewriting the checkpoint");
        config
    }

    #[test]
    fn projection_checkpoint_resume_skips_reference_and_sample_preparation() {
        use crate::devices::camera::{scripted_camera_with_probe, ScriptedCameraConfig};
        for (stage, retained) in [("projections", 1), ("placeSample", 1), ("projections", 0), ("finalizing", 3)] {
            let run = r0_run(&format!("projection-resume-{stage}-{retained}"));
            let root = run.join("resume-task");
            let (nano, nano_state) = pipeline_nano();
            let (camera, camera_probe) = scripted_camera_with_probe(ScriptedCameraConfig {
                exposure_delay: Duration::from_millis(5), ..Default::default()
            });
            let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
            let config = projection_resume_fixture(&run, "resume-task", &camera, stage, retained);
            let before = config.resume.as_ref().unwrap().clone();
            if retained == 1 {
                assert!(recovery_position_matches(&before, &config.parameters, 0, 96_000));
                assert!(recovery_position_matches(&before, &config.parameters, -32_000, 96_000));
                assert!(!recovery_position_matches(&before, &config.parameters, -31_999, 96_000));
                assert!(!recovery_position_matches(&before, &config.parameters, -32_000, 48_000));
            }
            let expected_stage = if stage == "finalizing" { "finalizing" } else { "projections" };
            assert_eq!(before.stage, expected_stage);
            assert_eq!(initial_scan_stage(config.resume.as_ref()), expected_stage);
            let references_before = r0_reference_hashes(&root);
            let mut handle = RealScanHandle::start(nano, camera, xray, config);
            let deadline = Instant::now() + Duration::from_secs(20);
            let outcome = loop {
                let progress = handle.progress();
                if progress.prompt.is_some() || matches!(progress.stage, "preDark" | "preFlat" | "placeSample") {
                    handle.cancel_only();
                    panic!("verified projection resume regressed to preparation: {progress:?}");
                }
                if let Some(outcome) = handle.try_finish() { break outcome.unwrap(); }
                if Instant::now() >= deadline {
                    handle.cancel_only();
                    panic!("projection resume did not finish: {:?}", handle.progress());
                }
                thread::sleep(Duration::from_millis(1));
            };
            assert_eq!(outcome.result.as_ref().unwrap(), &ScanCompletion::Completed);
            let progress = handle.progress();
            assert_eq!(&progress.frames[..retained as usize], before.frames.as_slice());
            assert_eq!(progress.references, before.references);
            assert_eq!(r0_reference_hashes(&root), references_before);
            for frame in &before.frames {
                assert_eq!(hash_file(Path::new(&frame.path)).unwrap(), (frame.bytes, frame.sha256.clone()));
            }
            assert_eq!(camera_probe.operation_count(0x90c0), (3 - retained) as usize,
                "only missing projections may cause physical captures");
            assert_eq!(nano_state.lock().unwrap().writes.iter().filter(|line| line.starts_with("CAPTURE_DONE ")).count(), (3 - retained) as usize);
            assert_eq!(xray_probe.beam_transitions().iter().filter(|(on, _)| *on).count(), usize::from(retained < 3),
                "no reference group may open the beam during projection recovery");
            assert!(!xray_probe.output_enabled());
        }
    }

    #[test]
    fn genuine_place_sample_checkpoint_still_requires_operator_confirmation() {
        use crate::devices::camera::{scripted_camera_with_probe, ScriptedCameraConfig};
        let run = r0_run("unfinished-sample-confirmation");
        let (nano, _) = pipeline_nano();
        let (camera, camera_probe) = scripted_camera_with_probe(ScriptedCameraConfig::default());
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config = projection_resume_fixture(&run, "sample-task", &camera, "placeSample", 0);
        assert_eq!(config.resume.as_ref().unwrap().stage, "placeSample");
        let mut handle = RealScanHandle::start(nano, camera, xray, config);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let progress = handle.progress();
            assert!(!xray_probe.output_enabled());
            assert_eq!(camera_probe.operation_count(0x90c0), 0);
            if progress.prompt == Some("placeSample") { break; }
            assert!(handle.try_finish().is_none(), "unfinished sample confirmation must remain pending");
            if Instant::now() >= deadline { handle.cancel_only(); panic!("sample prompt did not appear"); }
            thread::sleep(Duration::from_millis(1));
        }
        handle.cancel_only();
        while handle.try_finish().is_none() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(camera_probe.operation_count(0x90c0), 0);
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum R0ProjectionFault { CameraLoss, Interlock, StopTransfer, UnknownOff }

    fn r0_projection_fault(case: R0ProjectionFault) {
        use crate::devices::camera::{scripted_camera_with_probe, ScriptedCameraConfig};
        let topic = match case {
            R0ProjectionFault::CameraLoss => "camera-loss",
            R0ProjectionFault::Interlock => "interlock",
            R0ProjectionFault::StopTransfer => "stop-transfer",
            R0ProjectionFault::UnknownOff => "off-unconfirmed",
        };
        let run = r0_run(topic);
        let task = "fault-task";
        let root = run.join(task);
        let (nano, nano_state) = pipeline_nano();
        let (trigger_sender, trigger_receiver) = mpsc::channel();
        let (camera, camera_probe) = scripted_camera_with_probe(ScriptedCameraConfig {
            exposure_delay: Duration::from_millis(5),
            capture_started_signal: Some(trigger_sender),
            ..Default::default()
        });
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config = pipeline_config(&run, task, &camera);
        let parameters = config.parameters.clone();
        let mut handle = RealScanHandle::start(nano.clone(), camera.clone(), xray, config);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut references = None;
        let mut transfer_count = 0;
        let mut armed = false;
        let mut injected_at = None;
        let outcome = loop {
            let progress = handle.progress();
            if progress.prompt == Some("preReferences") {
                let _ = handle.confirm_stage("preReferences");
            }
            if progress.prompt == Some("placeSample") && !armed {
                references = Some(r0_reference_hashes(&root));
                trigger_receiver.try_iter().for_each(drop);
                transfer_count = camera_probe.operation_count(0x1009); // GetObject
                if case == R0ProjectionFault::StopTransfer {
                    camera_probe.set_transfer_delay(Duration::from_secs(2));
                } else {
                    camera_probe.set_exposure_delay(Duration::from_secs(2));
                }
                armed = true;
                handle.confirm_stage("placeSample").unwrap();
            }
            let accepted = armed && trigger_receiver.try_iter().count() > 0;
            let transfer_pending = armed && camera_probe.operation_count(0x1009) > transfer_count;
            if injected_at.is_none() && (if case == R0ProjectionFault::StopTransfer { transfer_pending } else { accepted }) {
                assert!(xray_probe.output_enabled(), "fault must occur in an actual scripted beam block");
                assert!(camera.health().operation_id.as_deref().is_some_and(|id| id.starts_with("scan-fault-task-1-")));
                injected_at = Some(Instant::now());
                match case {
                    R0ProjectionFault::CameraLoss => camera_probe.fail_transport_operation(0x90c7),
                    R0ProjectionFault::Interlock => xray_probe.set_interlock(true),
                    R0ProjectionFault::StopTransfer => handle.request_stop().unwrap(),
                    R0ProjectionFault::UnknownOff => {
                        xray_probe.ignore_off(true);
                        handle.request_pause(); // First projection's associated C102 must request OFF.
                    }
                }
            }
            if let Some(outcome) = handle.try_finish() { break outcome.unwrap(); }
            if Instant::now() >= deadline {
                handle.cancel_only();
                panic!("R0 {case:?} did not terminate: {:?}", handle.progress());
            }
            thread::sleep(Duration::from_millis(1));
        };
        let injected_at = injected_at.expect("the intended fault boundary was reached");
        let progress = handle.progress();
        assert_eq!(progress.captured, 0);
        assert!(progress.frames.is_empty(), "failed/cancelled projection cannot advance progress");
        assert!(!completed_manifest_exists(&parameters));
        let manifest: serde_json::Value = serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["completed"], false);
        assert_eq!(manifest["frames"].as_array().unwrap().len(), 0);
        assert_eq!(r0_reference_hashes(&root), references.unwrap(), "original reference files are retained byte-for-byte");
        let writes = nano_state.lock().unwrap().writes.clone();
        assert!(writes.iter().any(|line| line.starts_with("STOP ")));
        assert!(!writes.iter().any(|line| line.starts_with("CAPTURE_DONE ")), "failed first projection cannot release a completed capture ticket");
        let camera_health = outcome.camera.health();
        let xray_health = outcome.xray.health();
        if case == R0ProjectionFault::StopTransfer {
            assert_eq!(outcome.result.as_ref().unwrap(), &ScanCompletion::Stopped);
            assert_eq!(progress.phase, "stopped");
            assert!(camera_health.cleanup_confirmed);
        } else {
            assert!(outcome.result.is_err());
            assert_eq!(progress.phase, "fault");
        }
        if case == R0ProjectionFault::CameraLoss {
            assert!(camera_health.isolated && !camera_health.cleanup_confirmed);
            assert!(camera_health.last_error.as_deref().is_some_and(|error| error.contains("scripted camera disconnected")));
        }
        if case == R0ProjectionFault::UnknownOff {
            assert!(!xray_health.beam_off_confirmed);
            assert!(xray_probe.output_enabled(), "fake OFF ACK alone cannot prove output is off");
            assert!(outcome.result.as_ref().unwrap_err().contains("OFF"));
            let last_warning = writes.iter().rev().find(|line| line.starts_with("XRAY_WARNING ")).unwrap();
            assert_eq!(last_warning.split_whitespace().last(), Some("ON"),
                "unknown output must retain warning ON; writes={writes:?}");
            xray_probe.ignore_off(false);
        } else {
            assert!(xray_health.beam_off_confirmed && !xray_probe.output_enabled());
            if case == R0ProjectionFault::StopTransfer {
                assert!(xray_probe.beam_transitions().iter().any(|(on, at)| !*on && *at >= injected_at && at.duration_since(injected_at) < Duration::from_secs(1)),
                    "OFF must precede the two-second stalled transfer deadline; this is an offline timing assertion");
            }
            assert_eq!(fs::read_dir(root.join("frames")).unwrap().count(), 0, "no late NEF commit after cancellation/fault");
        }
        fs::write(run.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "status":"PASS", "boundary":"offline production coordinator and scripted device owners", "case":topic,
            "outcome":format!("{:?}",outcome.result), "phase":progress.phase, "captured":progress.captured,
            "cameraCleanupConfirmed":camera_health.cleanup_confirmed, "cameraIsolated":camera_health.isolated,
            "xrayOffConfirmed":xray_health.beam_off_confirmed, "referenceFilesRetained":20, "nanoWrites":writes,
            "fullLinkedHardwareCertified":false
        })).unwrap()).unwrap();
    }

    #[test]
    fn r0_camera_loss_during_projection_isolates_camera_and_never_commits() {
        r0_projection_fault(R0ProjectionFault::CameraLoss);
    }

    #[test]
    fn r0_interlock_during_projection_confirms_off_and_never_commits() {
        r0_projection_fault(R0ProjectionFault::Interlock);
    }

    #[test]
    fn r0_stop_during_projection_transfer_confirms_off_before_transfer_and_never_commits() {
        r0_projection_fault(R0ProjectionFault::StopTransfer);
    }

    #[test]
    fn r0_unconfirmed_off_retains_warning_and_cannot_report_safe_completion() {
        r0_projection_fault(R0ProjectionFault::UnknownOff);
    }

    #[test]
    fn r0_manifest_write_failure_retains_nef_and_refuses_projection_progress() {
        use crate::devices::camera::{scripted_camera, ScriptedCameraConfig};
        let run = r0_run("manifest-failure");
        let root = run.join("commit-task");
        let (nano, nano_state) = pipeline_nano();
        let camera = scripted_camera(ScriptedCameraConfig { exposure_delay:Duration::from_millis(5), ..Default::default() });
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config = pipeline_config(&run, "commit-task", &camera);
        let parameters = config.parameters.clone();
        let mut handle = RealScanHandle::start(nano, camera, xray, config);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut references = None;
        let mut armed = false;
        let outcome = loop {
            let progress = handle.progress();
            if progress.prompt == Some("preReferences") { let _ = handle.confirm_stage("preReferences"); }
            if progress.prompt == Some("placeSample") && !armed {
                references = Some(r0_reference_hashes(&root));
                let fault_root = root.clone();
                nano_state.lock().unwrap().on_capture_done = Some(Box::new(move || {
                    assert!(fault_root.join("frames/frame-0001.nef").is_file());
                    // Fail persistence after the matching device release, before
                    // the sole domain owner can publish committed frame progress.
                    fs::rename(fault_root.join("acquisition.csv"), fault_root.join("acquisition-before-fault.csv")).unwrap();
                    fs::create_dir(fault_root.join("acquisition.csv")).unwrap();
                }));
                armed = true;
                handle.confirm_stage("placeSample").unwrap();
            }
            if let Some(outcome) = handle.try_finish() { break outcome.unwrap(); }
            if Instant::now() >= deadline { handle.cancel_only(); panic!("R0 manifest fault did not finish: {:?}", handle.progress()); }
            thread::sleep(Duration::from_millis(1));
        };
        assert!(armed && root.join("acquisition.csv").is_dir(), "the commit fault was actually injected");
        assert!(outcome.result.is_err());
        let progress = handle.progress();
        assert_eq!(progress.phase, "fault");
        assert_eq!(progress.captured, 0);
        assert!(progress.frames.is_empty());
        assert!(!completed_manifest_exists(&parameters));
        let manifest:serde_json::Value = serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["completed"], false);
        assert_eq!(manifest["frames"].as_array().unwrap().len(), 0);
        let (bytes, sha256) = hash_file(&root.join("frames/frame-0001.nef")).unwrap();
        assert!(bytes > 0 && sha256.len() == 64, "captured NEF survives failed manifest commit");
        assert_eq!(r0_reference_hashes(&root), references.unwrap());
        assert!(!xray_probe.output_enabled() && outcome.xray.health().beam_off_confirmed);
        let writes=nano_state.lock().unwrap().writes.clone();
        assert_eq!(writes.iter().filter(|line|line.starts_with("CAPTURE_DONE ")).count(),1);
        assert!(writes.iter().any(|line|line.starts_with("STOP ")));
        fs::write(run.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({
            "status":"PASS", "boundary":"offline production coordinator and scripted device owners", "case":"manifest-failure",
            "phase":progress.phase, "captured":progress.captured, "nefBytes":bytes, "nefSha256":sha256,
            "referenceFilesRetained":20, "manifestCompleted":false, "fullLinkedHardwareCertified":false
        })).unwrap()).unwrap();
    }

    #[test]
    fn r0_pause_commits_current_projection_before_resume_without_repeating_files() {
        use crate::devices::camera::{scripted_camera_with_probe, ScriptedCameraConfig};
        let run = r0_run("pause-resume");
        let root = run.join("pause-task");
        let (nano, nano_state) = pipeline_nano();
        let (trigger_sender, trigger_receiver) = mpsc::channel();
        let (camera, camera_probe) = scripted_camera_with_probe(ScriptedCameraConfig {
            exposure_delay:Duration::from_millis(5), capture_started_signal:Some(trigger_sender), ..Default::default()
        });
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config=pipeline_config(&run,"pause-task",&camera);
        let parameters=config.parameters.clone();
        let mut handle=RealScanHandle::start(nano,camera,xray,config);
        let deadline=Instant::now()+Duration::from_secs(30);
        let mut armed=false;
        let mut pause_requested=false;
        let mut pause_verified=false;
        let mut reference_hashes=None;
        let outcome=loop {
            let progress=handle.progress();
            if progress.prompt==Some("preReferences"){let _=handle.confirm_stage("preReferences");}
            if progress.prompt==Some("placeSample")&&!armed {
                reference_hashes=Some(r0_reference_hashes(&root));
                trigger_receiver.try_iter().for_each(drop);
                camera_probe.set_exposure_delay(Duration::from_millis(200));
                camera_probe.set_transfer_delay(Duration::from_millis(100));
                armed=true;handle.confirm_stage("placeSample").unwrap();
            }
            if armed&&!pause_requested&&trigger_receiver.try_iter().count()>0 {
                assert!(xray_probe.output_enabled());
                handle.request_pause();pause_requested=true;
            }
            if progress.phase=="paused"&&!pause_verified {
                assert_eq!(progress.captured,1);
                assert_eq!(progress.frames.len(),1);
                assert!(!xray_probe.output_enabled());
                let manifest:serde_json::Value=serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
                assert_eq!(manifest["frames"].as_array().unwrap().len(),1);
                assert_eq!(manifest["completed"],false);
                assert_eq!(nano_state.lock().unwrap().writes.iter().filter(|line|line.starts_with("CAPTURE_DONE ")).count(),1);
                pause_verified=true;handle.resume();
            }
            if let Some(outcome)=handle.try_finish(){break outcome.unwrap();}
            if Instant::now()>=deadline{handle.cancel_only();panic!("R0 pause/resume did not finish: {:?}",handle.progress());}
            thread::sleep(Duration::from_millis(1));
        };
        assert!(pause_requested&&pause_verified);
        assert_eq!(outcome.result.as_ref().unwrap(),&ScanCompletion::Completed);
        assert!(completed_manifest_exists(&parameters));
        assert_eq!(handle.progress().captured,3);
        assert_eq!(camera_probe.operation_count(0x90c0),23,"20 references plus exactly three projections");
        assert_eq!(fs::read_dir(root.join("frames")).unwrap().count(),3);
        assert_eq!(r0_reference_hashes(&root),reference_hashes.unwrap());
        let transitions=xray_probe.beam_transitions();
        assert_eq!(transitions.iter().filter(|(on,_)|*on).count(),3,"one flat group and two projection blocks separated by pause");
        let mut last_off=None;
        for (on,at) in &transitions {
            if *on {assert!(last_off.is_some_and(|off|at.duration_since(off)>=Duration::from_secs(2)),"pause/resume cannot bypass the adapter OFF-to-ON gate");}
            else {last_off=Some(*at);}
        }
        assert!(!xray_probe.output_enabled()&&outcome.xray.health().beam_off_confirmed);
        fs::write(run.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({
            "status":"PASS", "boundary":"offline production coordinator and scripted device owners", "case":"pause-resume",
            "pauseCaptured":1, "completedCaptured":3, "totalCaptureCommands":23, "beamOnBlocks":3,
            "referenceFilesRetained":20, "fullLinkedHardwareCertified":false
        })).unwrap()).unwrap();
    }

    struct DeferredMoveTransport {
        lines: Arc<Mutex<VecDeque<String>>>,
        announced: std::sync::mpsc::Sender<u32>,
        capture_id: u32,
    }

    impl LineTransport for DeferredMoveTransport {
        fn write_line(&mut self, line: &str) -> std::io::Result<()> {
            let fields: Vec<_> = line.split_whitespace().collect();
            let mut lines = self.lines.lock().unwrap();
            let id = fields.get(1).copied().unwrap_or("0");
            match fields[0] {
                "PING" => {
                    lines.push_back(format!("ACK {id} PING"));
                    lines.push_back(format!("PONG {id}"));
                }
                "INFO" => {
                    lines.push_back(format!("ACK {id} INFO"));
                    lines.push_back(format!("INFO {id} DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"));
                }
                "HEARTBEAT" => lines.push_back(format!("HBACK {id}")),
                "MOVE_REL" => {
                    self.capture_id = id.parse().unwrap();
                    lines.push_back(format!("ACK {id} MOVE_REL"));
                    self.announced.send(self.capture_id).unwrap();
                    // The test withholds READY_TO_CAPTURE until it has sampled
                    // the live progress while this motion is still pending.
                }
                "STATUS" => {
                    lines.push_back(format!("ACK {id} STATUS"));
                    let (state, position) = if self.capture_id == 0 {
                        ("IDLE", 0)
                    } else {
                        ("CAPTURE_HOLD", -24000)
                    };
                    lines.push_back(format!("STATUS {id} state={state} pos={position} target={position} microsteps=8 ppr=96000 reference=1 homed=1 rearmed=1 hall=0 capture_id={}", self.capture_id));
                }
                "STOP" => {
                    lines.push_back(format!("ACK {id} STOP"));
                    lines.push_back(format!("STOPPED {id} POS=-24000"));
                }
                "XRAY_WARNING" => {
                    lines.push_back(format!("ACK {id} XRAY_WARNING"));
                    lines.push_back(format!(
                        "OK {id} XRAY_WARNING={}",
                        fields.get(2).unwrap_or(&"OFF")
                    ));
                }
                _ => {}
            }
            Ok(())
        }
        fn read_line(&mut self, timeout: Duration) -> std::io::Result<Option<String>> {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(line) = self.lines.lock().unwrap().pop_front() {
                    return Ok(Some(line));
                }
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn pending_move_keeps_old_angle_until_device_confirms_arrival() {
        let lines = Arc::new(Mutex::new(VecDeque::from([format!(
            "READY DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"
        )])));
        let (announced, receiver) = std::sync::mpsc::channel();
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test(
                "MEMORY-ONLY",
                Box::new(DeferredMoveTransport {
                    lines: lines.clone(),
                    announced,
                    capture_id: 0,
                }),
            )
            .unwrap();
        let adapter = Arc::new(adapter);
        let progress = Arc::new(Mutex::new(RealScanProgress::default()));
        let worker_adapter = adapter.clone();
        let worker_progress = progress.clone();
        let worker = thread::spawn(move || {
            move_to_projection(
                &worker_adapter,
                &worker_progress,
                projection_job(1, 4),
                &Arc::new(AtomicBool::new(false)),
            )
        });
        let arrival = receiver.recv_timeout(Duration::from_secs(2));
        let before = progress.lock().unwrap().angle_deg;
        if let Ok(id) = arrival {
            lines
                .lock()
                .unwrap()
                .push_back(format!("READY_TO_CAPTURE {id} POS=-24000"));
        } else {
            let _ = adapter.stop();
        }
        let result = worker.join().unwrap();
        assert!(arrival.is_ok(), "in-memory motion was not issued");
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            before, 0.0,
            "a target must not appear as current feedback before arrival"
        );
        assert_eq!(progress.lock().unwrap().angle_deg, -90.0);
    }

    #[test]
    fn committing_an_older_frame_does_not_rewind_prefetched_live_position() {
        let progress = Mutex::new(RealScanProgress::default());
        set_angle(&progress, -90.0);
        push_frame(
            &progress,
            RealFrame {
                index: 1,
                angle_deg: 0.0,
                exposure_ms: 200.0,
                file_name: "frame-0001.nef".into(),
                path: "memory-only".into(),
                bytes: 1,
                sha256: "test".into(),
            },
        );
        let state = progress.lock().unwrap();
        assert_eq!(state.captured, 1);
        assert_eq!(state.frames[0].angle_deg, 0.0);
        assert_eq!(state.angle_deg, -90.0);
    }

    #[test]
    fn exposure_boundary_requires_this_operations_event_and_rejects_old_or_duplicate_events() {
        let mut evidence = ExposureEvidence::new("session-1".into(), "capture-1".into());
        evidence
            .observe(&CameraCaptureEvent::TransferStarted {
                session_id: "session-1".into(),
                operation_id: "capture-1".into(),
                size: 50,
            })
            .unwrap();
        assert!(
            !evidence.exposure_ended,
            "file transfer does not prove exposure end"
        );
        assert!(evidence
            .observe(&CameraCaptureEvent::ExposureEnded {
                session_id: "old-session".into(),
                operation_id: "capture-1".into()
            })
            .is_err());
        assert!(evidence
            .observe(&CameraCaptureEvent::ExposureEnded {
                session_id: "session-1".into(),
                operation_id: "old-capture".into()
            })
            .is_err());
        assert!(!evidence.exposure_ended);
        let event = CameraCaptureEvent::ExposureEnded {
            session_id: "session-1".into(),
            operation_id: "capture-1".into(),
        };
        evidence.observe(&event).unwrap();
        assert!(evidence.exposure_ended);
        assert!(evidence.observe(&event).is_err());
    }

    #[test]
    fn short_beam_budget_cools_before_starting_the_next_exposure() {
        let now = Instant::now();
        let started = now - Duration::from_secs(591);
        let limit = Duration::from_secs(600);
        let exposure_budget = Duration::from_secs(5);
        assert!(needs_cooldown_before_exposure(
            started,
            now,
            limit,
            exposure_budget + MOVE_BUDGET_MARGIN,
        ));
        assert!(!needs_cooldown_before_exposure(
            started,
            now,
            limit,
            exposure_budget,
        ));
        assert!(needs_cooldown_before_exposure(
            started,
            now + Duration::from_secs(4),
            limit,
            exposure_budget,
        ));
    }

    #[test]
    fn projection_jobs_are_fifo_and_opposite_for_two_views() {
        assert_eq!(projection_job(0, 2).angle_mdeg, 0);
        assert_eq!(projection_job(1, 2).angle_mdeg, -180_000);
        assert_eq!(expected_pulses(-180_000, 96_000), -48_000);
        assert_eq!(expected_pulses(-360_000, 96_000), 0);
        let jobs: Vec<_> = (0..180).map(|index| projection_job(index, 180)).collect();
        assert_eq!(jobs.len(), 180);
        assert!(jobs
            .iter()
            .enumerate()
            .all(|(index, job)| job.index == index as u32 + 1
                && job.angle_mdeg == -(index as i32 * 2_000)));
        assert_eq!(jobs.last().unwrap().angle_mdeg, -358_000);
    }

    #[test]
    fn estimate_requires_completed_sample_and_hides_operator_pause_or_fault() {
        let mut state = RealScanProgress::default();
        state.projection_count = 3;
        state.refresh_estimate(Instant::now());
        assert_eq!(state.estimated_remaining_seconds, None);
        state.captured = 1;
        state.measured_projection_time = Some(Duration::from_secs(12));
        state.block_limit = Duration::from_secs(10);
        state.refresh_estimate(Instant::now());
        assert_eq!(state.estimated_remaining_seconds, Some(324));
        state.phase = "paused";
        state.refresh_estimate(Instant::now());
        assert_eq!(state.estimated_remaining_seconds, None);
        state.phase = "fault";
        state.refresh_estimate(Instant::now());
        assert_eq!(state.estimated_remaining_seconds, None);
    }

    #[test]
    fn estimate_does_not_count_a_completed_cooldown_twice() {
        let now = Instant::now();
        let mut state = RealScanProgress::default();
        state.projection_count = 360;
        state.captured = 133;
        state.measured_projection_time = Some(Duration::from_secs(9));
        state.projection_started = Some(now - Duration::from_secs(309));
        state.projection_cooling_elapsed = Duration::from_secs(300);
        record_projection_sample(&mut state, 134, now);
        assert_eq!(state.measured_projection_time, Some(Duration::from_secs(9)));
        state.captured = 134;
        state.block_limit = Duration::from_secs(600);
        state.refresh_estimate(now);
        assert_eq!(state.estimated_remaining_seconds, Some(2_934));
    }

    #[test]
    fn cooldown_countdown_uses_monotonic_elapsed_time() {
        let now = Instant::now();
        let mut state = RealScanProgress::default();
        state.cooldown_until = Some(now + XRAY_COOLDOWN);
        state.refresh_estimate(now);
        assert_eq!(state.cooldown_remaining_seconds, Some(300));
        state.refresh_estimate(now + Duration::from_secs(301));
        assert_eq!(state.cooldown_remaining_seconds, Some(0));
    }

    #[test]
    fn estimate_overlaps_remaining_off_interval_with_current_file_work() {
        let now = Instant::now();
        let mut state = RealScanProgress::default();
        state.projection_count = 2;
        state.captured = 1;
        state.measured_projection_time = Some(Duration::from_secs(10));
        state.projection_started = Some(now - Duration::from_secs(4));
        state.cooldown_until = Some(now + Duration::from_secs(300));
        state.refresh_estimate(now);
        assert_eq!(state.estimated_remaining_seconds, Some(300));
    }
}
