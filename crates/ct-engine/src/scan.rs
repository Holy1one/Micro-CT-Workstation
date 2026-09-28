//! Transactional projection-scan coordinator.
//!
//! A projection is committed only after turntable arrival, warning assertion,
//! verified X-ray exposure, host-side camera file confirmation, and
//! `CAPTURE_DONE`. A verified beam stays on across views until the configured
//! continuous limit, pause, stop, or final view. Pause is observed at transaction
//! boundaries. USB auto-shutdown is an operator-owned device setting: the
//! coordinator never changes it. Every exit path still attempts verified beam
//! OFF and warning OFF before returning.

use crate::devices::camera::{CameraError, DigiCamControlAdapter};
use crate::devices::turntable::{MoveTicket, NanoAdapter};
use crate::devices::xray::{MoxtekAdapter, XrayHealth};
use crate::{timestamp, Parameters};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanCompletion { Completed, Stopped }

#[derive(Clone)]
pub struct RealScanConfig {
    pub parameters: Parameters,
    pub max_xray_sec: u32,
    pub voltage_kv: f64,
    pub current_ua: f64,
    pub resume_frames: Vec<RealFrame>,
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
        self.cooldown_remaining_seconds = self.cooldown_until
            .map(|deadline| {
                let remaining = deadline.saturating_duration_since(now);
                remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
            });
        self.estimated_remaining_seconds = None;
        if !matches!(self.phase, "running" | "cooling") || self.error.is_some() { return; }
        let Some(sample) = self.measured_projection_time else { return; };
        let remaining = self.projection_count.saturating_sub(self.captured);
        if remaining == 0 { self.estimated_remaining_seconds = Some(0); return; }
        let cooling_in_progress = self.cooldown_wait_started
            .map(|started| now.saturating_duration_since(started))
            .unwrap_or_default();
        let current_elapsed = self.projection_started.map(|started| {
            now.saturating_duration_since(started)
                .saturating_sub(self.projection_cooling_elapsed + cooling_in_progress)
        }).unwrap_or_default();
        let work = sample.saturating_mul(remaining).saturating_sub(current_elapsed);
        let mut seconds = work.as_secs_f64();
        if let Some(deadline) = self.cooldown_until {
            let pending = deadline.saturating_duration_since(now);
            // Work still being done with the beam OFF overlaps this interval.
            let overlap = if self.phase == "running" && self.projection_started.is_some() {
                sample.saturating_sub(current_elapsed)
            } else { Duration::ZERO };
            seconds += pending.saturating_sub(overlap).as_secs_f64();
        }
        // Active projection time is an upper bound for future beam time. A
        // completed five-minute OFF interval must never enter that sample.
        if !self.block_limit.is_zero() {
            let active = self.beam_started
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
    pub camera: DigiCamControlAdapter,
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
}

impl RealScanHandle {
    pub fn start(
        nano: Arc<NanoAdapter>,
        camera: DigiCamControlAdapter,
        xray: MoxtekAdapter,
        config: RealScanConfig,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(RealScanProgress {
            captured: config.resume_frames.len() as u32,
            frames: config.resume_frames.clone(),
            ..RealScanProgress::default()
        }));
        let (sender, outcome) = mpsc::channel();
        let worker_cancel = cancel.clone();
        let worker_pause = pause.clone();
        let worker_progress = progress.clone();
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
                ))).unwrap_or_else(|_| {
                    let off = xray.force_off();
                    let warning = worker_nano.set_xray_warning(false);
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
                    if let Err(error) = persist_scan_log(&root, &worker_progress, &result) {
                        let detail = format!("scan log could not be saved: {error}");
                        result = Err(match result { Ok(_) => detail.clone(), Err(previous) => format!("{previous}; {detail}") });
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
        }
    }

    pub fn progress(&self) -> RealScanProgress {
        let mut snapshot = self.progress.lock().expect("scan progress mutex poisoned").clone();
        snapshot.refresh_estimate(Instant::now());
        snapshot
    }

    pub fn request_pause(&self) {
        self.pause.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.pause.store(false, Ordering::SeqCst);
    }

    pub fn request_stop(&self) -> Result<(), String> {
        if self.cancel.swap(true, Ordering::SeqCst) { return Ok(()); }
        update_phase(&self.progress, "stopping");
        self.nano
            .stop()
            .map_err(|error| format!("Nano STOP failed: {error}"))
    }

    pub fn try_finish(&mut self) -> Option<Result<ScanOutcome, String>> {
        poll_scan_outcome(&self.outcome, &mut self.worker)
    }
}

fn poll_scan_outcome<T>(receiver: &mpsc::Receiver<T>, worker: &mut Option<JoinHandle<()>>) -> Option<Result<T, String>> {
    match receiver.try_recv() {
        Ok(outcome) => {
            // Receipt is the terminal ownership handoff. Do not leave a handle
            // whose Drop could send STOP after a successful completion.
            if let Some(worker) = worker.take() { if worker.is_finished() { let _ = worker.join(); } }
            Some(Ok(outcome))
        }
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => {
            if worker.as_ref().is_some_and(JoinHandle::is_finished) {
                let _ = worker.take().unwrap().join();
            }
            Some(Err("scan worker exited without a cleanup outcome; output state is unconfirmed".into()))
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
            if worker.is_finished() { let _ = worker.join(); }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanManifest<'a> {
    schema_version: u32,
    scan_id: &'a str,
    created_at: String,
    completed: bool,
    voltage_kv: f64,
    current_ua: f64,
    projection_count: u32,
    exposure_ms: f64,
    frames: &'a [RealFrame],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredScanManifest {
    schema_version: u32,
    scan_id: String,
    completed: bool,
    voltage_kv: f64,
    current_ua: f64,
    projection_count: u32,
    exposure_ms: f64,
    frames: Vec<RealFrame>,
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

pub fn resume_manifest_exists(parameters: &Parameters) -> bool {
    let path = Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("manifest.json");
    let Ok(file) = File::open(path) else { return false; };
    serde_json::from_reader::<_, StoredScanManifestHeader>(file)
        .is_ok_and(|manifest| !manifest.completed)
}

pub fn load_resume_frames(
    parameters: &Parameters,
    voltage_kv: f64,
    current_ua: f64,
) -> Result<Vec<RealFrame>, String> {
    let root = Path::new(parameters.save_path.trim()).join(parameters.task_id.trim());
    let file = File::open(root.join("manifest.json")).map_err(io_error)?;
    let manifest: StoredScanManifest = serde_json::from_reader(file)
        .map_err(|error| format!("invalid scan manifest: {error}"))?;
    if manifest.schema_version != 1 || manifest.completed
        || manifest.scan_id != parameters.task_id
        || manifest.projection_count != parameters.projection_count
        || (manifest.exposure_ms - parameters.exposure_ms).abs() > 1e-6
        || (manifest.voltage_kv - voltage_kv).abs() > 0.01
        || (manifest.current_ua - current_ua).abs() > 0.01
        || manifest.frames.is_empty()
        || manifest.frames.len() >= parameters.projection_count as usize
    {
        return Err("unfinished scan manifest does not match the current task, exposure, projection count or X-ray setpoints".into());
    }
    let frames_dir = root.join("frames");
    for (zero_index, frame) in manifest.frames.iter().enumerate() {
        let index = zero_index as u32 + 1;
        let expected_name = format!("frame-{index:04}.nef");
        let expected_path = frames_dir.join(&expected_name);
        if frame.index != index || frame.file_name != expected_name
            || Path::new(&frame.path) != expected_path
            || (frame.angle_deg - projection_job(zero_index as u32, parameters.projection_count).angle_deg).abs() > 0.01
            || (frame.exposure_ms - parameters.exposure_ms).abs() > 1e-6
        {
            return Err(format!("resume frame {index} does not match the recorded scan geometry"));
        }
        let (bytes, sha256) = hash_file(&expected_path)?;
        if bytes != frame.bytes || sha256 != frame.sha256 {
            return Err(format!("resume frame {index} failed size or SHA-256 verification"));
        }
    }
    let mut file_count = 0;
    for entry in fs::read_dir(&frames_dir).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if !entry.file_type().map_err(io_error)?.is_file() {
            return Err("scan frames directory contains a non-file entry; inspect it before restoring".into());
        }
        file_count += 1;
    }
    if file_count != manifest.frames.len() {
        return Err("scan frames directory has an uncommitted or unexpected file; inspect it before restoring".into());
    }
    Ok(manifest.frames)
}

fn run_scan(
    nano: &NanoAdapter,
    camera: &mut DigiCamControlAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &AtomicBool,
    pause: &AtomicBool,
    progress: &Mutex<RealScanProgress>,
) -> Result<ScanCompletion, String> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || run_scan_inner(nano, camera, xray, config, cancel, pause, progress)
    )).unwrap_or_else(|_| Err("scan body panicked".into()));
    let off = xray.force_off();
    let warning_off = nano.set_xray_warning(false);
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

    let completion = finish_scan_result(result, off.map(|_| ()).map_err(|e| e.to_string()), warning_off.map_err(|e| e.to_string()))?;
    if completion == ScanCompletion::Completed {
        let frames = progress.lock().expect("scan progress mutex poisoned").frames.clone();
        let root = Path::new(config.parameters.save_path.trim()).join(config.parameters.task_id.trim());
        persist(&root, &config.parameters, config, &frames, true)?;
        update_phase(progress, "completed");
        push_message(progress, "PASS", "system", format!(
            "Real scan complete · {} projections · X-ray OFF", frames.len()
        ));
    }
    Ok(completion)
}

fn finish_scan_result(result: Result<(), String>, off: Result<(), String>, warning: Result<(), String>) -> Result<ScanCompletion, String> {
    let stopped = matches!(&result, Err(error) if error == CANCELLED);
    let mut errors = Vec::new();
    if let Err(error) = result { if !stopped { errors.push(error); } }
    if let Err(error) = off { errors.push(format!("final Moxtek OFF unconfirmed: {error}")); }
    if let Err(error) = warning { errors.push(format!("final warning OFF unconfirmed: {error}")); }
    if !errors.is_empty() { Err(errors.join("; ")) }
    else if stopped { Ok(ScanCompletion::Stopped) }
    else { Ok(ScanCompletion::Completed) }
}

fn run_scan_inner(
    nano: &NanoAdapter,
    camera: &mut DigiCamControlAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &AtomicBool,
    pause: &AtomicBool,
    progress: &Mutex<RealScanProgress>,
) -> Result<(), String> {
    let parameters = &config.parameters;
    let root = Path::new(parameters.save_path.trim()).join(parameters.task_id.trim());
    let resume_count = config.resume_frames.len();
    if resume_count == 0 {
        if root.exists() && fs::read_dir(&root).map_err(io_error)?.next().is_some() {
            return Err(format!("scan output already exists: {} · use Restore after Preflight and HOME", root.display()));
        }
        fs::create_dir_all(root.join("frames")).map_err(io_error)?;
    } else if load_resume_frames(parameters, config.voltage_kv, config.current_ua)? != config.resume_frames {
        return Err("scan files changed after Restore; no device action was started".into());
    }
    update(progress, |value| value.output_dir_owned = true);
    push_message(progress, "ACTION", "operator", if resume_count == 0 {
        format!("Real scan started · {} projections · {:.1} kV / {:.1} µA",
            parameters.projection_count, config.voltage_kv, config.current_ua)
    } else {
        format!("Real scan restored · {resume_count} files verified · continuing at view {}",
            resume_count + 1)
    });
    let shutter = camera
        .set_exposure_ms(parameters.exposure_ms)
        .map_err(|error| format!("camera exposure configuration failed: {error}"))?;
    push_message(progress, "PASS", "camera", format!("D7100 shutter set to {shutter}"));
    xray
        .set_parameters(config.voltage_kv, config.current_ua)
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
    if resume_count == 0 {
        persist(&root, parameters, config, &[], false)?;
    }
    update(progress, |value| {
        value.projection_count = parameters.projection_count;
        value.block_limit = block_limit;
    });

    for zero_index in resume_count as u32..parameters.projection_count {
        let job = projection_job(zero_index, parameters.projection_count);
        check_cancel(cancel)?;
        if let Some(cooldown_deadline) = cooldown_until.take() {
            wait_for_cooldown(progress, cancel, cooldown_deadline)?;
        }
        if pause.load(Ordering::SeqCst) && beam_started.is_some() {
            close_scan_beam(
                nano,
                xray,
                progress,
                &mut beam_started,
                "Pause boundary",
            )?;
        }
        while pause.load(Ordering::SeqCst) {
            update_phase(progress, "paused");
            if cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        if beam_started.is_some_and(|started| needs_cooldown_before_exposure(
            started, Instant::now(), block_limit,
            exposure_budget.saturating_add(MOVE_BUDGET_MARGIN),
        )) {
            close_scan_beam(nano, xray, progress, &mut beam_started,
                "Cooling before the next projection's move and exposure")?;
            wait_for_cooldown(progress, cancel, Instant::now() + XRAY_COOLDOWN)?;
        }
        update_phase(progress, "running");
        update(progress, |value| {
            value.projection_started = Some(Instant::now());
            value.projection_cooling_elapsed = Duration::ZERO;
        });
        let ticket = move_to_projection_monitored(
            nano, xray, progress, job, cancel, &mut beam_started, block_limit,
            &mut cooldown_until,
        )?;

        // The limit can expire during a long MOVE_ABS. The next exposure must
        // wait for the entire OFF interval, even though the move has completed.
        if let Some(deadline) = cooldown_until.take() {
            wait_for_cooldown(progress, cancel, deadline)?;
            verify_capture_hold(nano, &ticket, job)?;
        }

        if beam_started.is_some_and(|started| needs_cooldown_before_exposure(
            started, Instant::now(), block_limit, exposure_budget,
        )) {
            close_scan_beam(nano, xray, progress, &mut beam_started,
                "Cooling before the current projection's exposure")?;
            wait_for_cooldown(progress, cancel, Instant::now() + XRAY_COOLDOWN)?;
            verify_capture_hold(nano, &ticket, job)?;
        }

        if beam_started.is_none() {
            nano.set_xray_warning(true)
                .map_err(|error| format!("XRAY_WARNING ON failed: {error}"))?;
            // Include the entire ON command latency in the continuous-output budget.
            beam_started = Some(Instant::now());
            update(progress, |value| value.beam_started = beam_started);
            let emission = xray.beam_on(cancel);
            check_cancel(cancel)?;
            emission.map_err(|error| format!("Moxtek beam-on failed: {error}"))?;
            set_xray_health(progress, xray.health());
            set_beam(progress, true);
            if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
                close_scan_beam(nano, xray, progress, &mut beam_started,
                    "Continuous X-ray limit reached during beam-on confirmation")?;
                return Err("continuous X-ray limit reached before exposure started".into());
            }
            if beam_started.is_some_and(|started| needs_cooldown_before_exposure(
                started, Instant::now(), block_limit, exposure_budget,
            )) {
                close_scan_beam(nano, xray, progress, &mut beam_started,
                    "Beam enable left insufficient time for a confirmed exposure")?;
                return Err("configured X-ray time cannot accommodate beam enable and the current exposure".into());
            }
            push_message(progress, "ACTION", "xray", format!(
                "Continuous beam block confirmed ON · view={} · {:.1} kV / {:.1} µA · limit {} s",
                job.index, config.voltage_kv, config.current_ua, config.max_xray_sec
            ));
        }

        let frame_index = job.index;
        let final_projection = frame_index == parameters.projection_count;
        let mut abort_reason = None;
        let mut last_xray_poll = Instant::now();
        let capture_started = Instant::now();
        let mut transfer_announced = false;
        let camera_for_capture = &mut *camera;
        let capture_result = thread::scope(|scope| {
            let (capture_sender, capture_receiver) = mpsc::channel();
            let save_path = PathBuf::from(&parameters.save_path);
            let task_id = parameters.task_id.clone();
            scope.spawn(move || {
                let result = camera_for_capture.capture_frame_cancellable(&save_path, &task_id, frame_index, cancel);
                let _ = capture_sender.send(result);
            });
            loop {
                match capture_receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(result) => break result,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if abort_reason.is_none()
                            && beam_started.is_some()
                            && last_xray_poll.elapsed() >= Duration::from_millis(250)
                        {
                            last_xray_poll = Instant::now();
                            match xray.refresh_emission_status() {
                                Ok(health) => {
                                    if let Some(warning) = health.telemetry_warning.clone() {
                                        push_message(progress, "WARN", "xray", warning);
                                    }
                                    set_xray_health(progress, health);
                                }
                                Err(error) => {
                                    abort_reason =
                                        Some(format!("Moxtek emission monitor failed: {error}"));
                                    cancel.store(true, Ordering::SeqCst);
                                    set_xray_health(progress, xray.health());
                                    let _ = nano.set_xray_warning(false);
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
                            let _ = nano.set_xray_warning(false);
                            set_beam(progress, xray.health().beam_on);
                            let _ = nano.stop();
                        }
                        if abort_reason.is_none() {
                            let transfer_started = DigiCamControlAdapter::frame_transfer_started(
                                Path::new(parameters.save_path.trim()),
                                parameters.task_id.trim(),
                                frame_index,
                            );
                            let exposure_complete = exposure_is_complete(
                                transfer_started,
                                capture_started.elapsed(),
                                exposure_evidence_timeout,
                            );
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
                                let _ = nano.set_xray_warning(false);
                                let _ = nano.stop();
                            }
                            let pause_requested = pause.load(Ordering::SeqCst);
                            if abort_reason.is_none() && exposure_complete
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
                                            update(progress, |value| value.cooldown_until = cooldown_until);
                                        }
                                    }
                                    Err(error) => {
                                        abort_reason = Some(error);
                                        cancel.store(true, Ordering::SeqCst);
                                        let _ = nano.stop();
                                    }
                                }
                            }
                            if transfer_started {
                                if !transfer_announced {
                                    push_message(
                                        progress,
                                        "PASS",
                                        "camera",
                                        format!(
                                            "NEF transfer started · view={} · exposure complete",
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
        let path = capture_result.map_err(|error| format!("D7100 capture failed: {error}"))?;
        let limit_reached = beam_started
            .is_some_and(|started| started.elapsed() >= block_limit);
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
            close_scan_beam(
                nano,
                xray,
                progress,
                &mut beam_started,
                reason,
            )?;
            if limit_reached && !final_projection {
                cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                update(progress, |value| value.cooldown_until = cooldown_until);
            }
        }
        let frame = commit_projection_monitored(
            nano, xray, progress, &root, parameters, config, cancel,
            &path, job, &ticket, &mut beam_started, block_limit,
            &mut cooldown_until, final_projection,
        )?;
        push_frame(progress, frame.clone());
        update(progress, |value| record_projection_sample(value, frame.index, Instant::now()));
        push_message(progress, "PASS", "nano", format!(
            "CAPTURE_DONE view={} id={} · committed file confirmed", job.index, ticket.command_id
        ));
        push_message(progress, "PASS", "camera", format!(
            "Projection committed · view={} · {} bytes · SHA-256 {}",
            frame.index, frame.bytes, frame.sha256
        ));
    }

    if beam_started.is_some() {
        close_scan_beam(
            nano,
            xray,
            progress,
            &mut beam_started,
            "Scan completed",
        )?;
    }

    update_phase(progress, "finishing");
    check_cancel(cancel)?;
    push_message(
        progress,
        "ACTION",
        "nano",
        "Verifying the final committed projection position without moving the turntable".to_owned(),
    );
    let final_status = nano.status()
        .map_err(|error| format!("final Nano STATUS failed: {error}"))?;
    check_cancel(cancel)?;
    let final_job = projection_job(parameters.projection_count - 1, parameters.projection_count);
    validate_final_position(final_job, &final_status)?;
    push_message(
        progress,
        "PASS",
        "nano",
        format!("Final projection position confirmed · view={} · {:.3}° · {} pulses · reference valid",
            final_job.index, final_job.angle_deg, final_status.position_pulses),
    );

    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn wait_for_cooldown(
    progress: &Mutex<RealScanProgress>, cancel: &AtomicBool, deadline: Instant,
) -> Result<(), String> {
    let wait_started = Instant::now();
    update(progress, |value| {
        value.phase = "cooling";
        value.cooldown_until = Some(deadline);
        value.cooldown_wait_started = Some(wait_started);
    });
    let remaining_duration = deadline.saturating_duration_since(wait_started);
    let remaining = remaining_duration.as_secs() + u64::from(remaining_duration.subsec_nanos() > 0);
    push_message(progress, "WARN", "xray", format!(
        "X-ray OFF interval · {remaining} seconds remain of the 300-second interval before resuming"
    ));
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
    push_message(progress, "PASS", "xray", "Five-minute X-ray cooldown complete · scan resuming".into());
    Ok(())
}

fn record_projection_sample(value: &mut RealScanProgress, index: u32, now: Instant) {
    if let Some(started) = value.projection_started.take() {
        let active = now.saturating_duration_since(started)
            .saturating_sub(value.projection_cooling_elapsed);
        value.measured_projection_time = Some(match value.measured_projection_time {
            Some(previous) => (previous.saturating_mul(index - 1) + active) / index,
            None => active,
        });
    }
    value.projection_cooling_elapsed = Duration::ZERO;
}

fn validate_final_position(job: ProjectionJob, status: &crate::devices::turntable::NanoStatus) -> Result<(), String> {
    if status.pulses_per_rev == 0
        || status.position_pulses != expected_pulses(job.angle_mdeg, status.pulses_per_rev)
        || status.capture_id != 0
        || !status.reference_valid || !status.homed || !status.rearmed
        || status.state != "IDLE"
    {
        return Err(format!("turntable did not finish at the confirmed final view {} position", job.index));
    }
    Ok(())
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
    nano.set_xray_warning(false)
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
    let angle_mdeg = -(((i64::from(zero_index) * 360_000)
        + i64::from(projection_count / 2))
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
    if status.pulses_per_rev == 0 {
        return Err("Nano feedback has an invalid pulse scale".into());
    }
    let expected = expected_pulses(job.angle_mdeg, status.pulses_per_rev);
    if status.state != "CAPTURE_HOLD"
        || status.capture_id != ticket.command_id
        || ticket.position_pulses != expected
        || status.position_pulses != expected
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
    cancel: &AtomicBool,
) -> Result<MoveTicket, String> {
    check_cancel(cancel)?;
    push_message(
        progress,
        "ACTION",
        "nano",
        format!("MOVE_ABS view={} angle={:.3}°", job.index, job.angle_deg),
    );
    let movement = nano.move_abs(job.angle_mdeg);
    check_cancel(cancel)?;
    let ticket = movement.map_err(|error| format!("MOVE_ABS view {} failed: {error}", job.index))?;
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
    cancel: &AtomicBool,
    beam_started: &mut Option<Instant>,
    block_limit: Duration,
    cooldown_until: &mut Option<Instant>,
) -> Result<MoveTicket, String> {
    if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
        close_scan_beam(nano, xray, progress, beam_started,
            "Continuous X-ray limit reached before turntable movement")?;
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
        scope.spawn(move || { let _ = sender.send(move_to_projection(nano, progress, job, cancel)); });
        let mut last_poll = Instant::now();
        let mut abort: Option<String> = None;
        let mut warning_off_after_move = false;
        loop {
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => {
                    if warning_off_after_move {
                        nano.set_xray_warning(false).map_err(|error|
                            format!("XRAY_WARNING OFF failed after timed move: {error}"))?;
                    }
                    if abort.is_none() && beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
                        close_scan_beam(nano, xray, progress, beam_started,
                            "Continuous X-ray limit reached at turntable arrival")?;
                        *cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                        update(progress, |value| value.cooldown_until = *cooldown_until);
                    }
                    return abort.map_or(result, Err);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) =>
                    return Err("Nano move worker terminated without a result".into()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if abort.is_some() { continue; }
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
                        push_message(progress, "WARN", "xray",
                            "Continuous X-ray limit reached during movement · beam OFF confirmed".into());
                    }
                    Err(error) => {
                        abort = Some(format!("Moxtek OFF failed during move: {error}"));
                        let _ = nano.stop();
                    }
                }
            } else if beam_started.is_some() && last_poll.elapsed() >= Duration::from_millis(250) {
                last_poll = Instant::now();
                match xray.refresh_emission_status() {
                    Ok(health) => set_xray_health(progress, health),
                    Err(error) => {
                        abort = Some(format!("Moxtek emission monitor failed during move: {error}"));
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
    cancel: &AtomicBool,
    path: &Path,
    job: ProjectionJob,
    ticket: &MoveTicket,
    beam_started: &mut Option<Instant>,
    block_limit: Duration,
    cooldown_until: &mut Option<Instant>,
    final_projection: bool,
) -> Result<RealFrame, String> {
    let mut frames = progress.lock().expect("scan progress mutex poisoned").frames.clone();
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        scope.spawn(move || {
            let result = (|| {
                let (bytes, sha256) = hash_file(path)?;
                let frame = RealFrame {
                    index: job.index,
                    angle_deg: job.angle_deg,
                    exposure_ms: parameters.exposure_ms,
                    file_name: path.file_name().and_then(|value| value.to_str())
                        .unwrap_or("frame.nef").to_owned(),
                    path: path.display().to_string(), bytes, sha256,
                };
                check_cancel(cancel)?;
                nano.capture_done(ticket.command_id)
                    .map_err(|error| format!("CAPTURE_DONE view {} failed: {error}", job.index))?;
                check_cancel(cancel)?;
                frames.push(frame.clone());
                persist(root, parameters, config, &frames, false)?;
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
                        nano.set_xray_warning(false).map_err(|error|
                            format!("XRAY_WARNING OFF failed after frame commit: {error}"))?;
                    }
                    return abort.map_or(result, Err);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) =>
                    return Err("frame commit worker terminated without a result".into()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if abort.is_some() || beam_started.is_none() { continue; }
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
                match xray.refresh_emission_status() {
                    Ok(health) => set_xray_health(progress, health),
                    Err(error) => {
                        abort = Some(format!("Moxtek emission monitor failed during frame commit: {error}"));
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        let _ = nano.stop();
                    }
                }
            }
        }
    })
}

fn exposure_is_complete(
    transfer_started: bool,
    capture_elapsed: Duration,
    evidence_timeout: Duration,
) -> bool {
    transfer_started || capture_elapsed >= evidence_timeout
}

fn needs_cooldown_before_exposure(
    beam_started: Instant,
    now: Instant,
    block_limit: Duration,
    required_budget: Duration,
) -> bool {
    now.saturating_duration_since(beam_started)
        .saturating_add(required_budget) >= block_limit
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
    let messages = progress.lock().expect("scan progress mutex poisoned").messages.clone();
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
        writeln!(writer, "{}\t{}\t{}\t{}", event.timestamp, event.level, event.source, event.message.replace(['\r', '\n'], " "))
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
    completed: bool,
) -> Result<(), String> {
    let manifest = ScanManifest {
        schema_version: 1,
        scan_id: &parameters.task_id,
        created_at: timestamp(),
        completed,
        voltage_kv: config.voltage_kv,
        current_ua: config.current_ua,
        projection_count: parameters.projection_count,
        exposure_ms: parameters.exposure_ms,
        frames,
    };
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(root.join("manifest.json"))
        .map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &manifest).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)?;

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
    Ok(())
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::turntable::{LineTransport, NanoStatus, EXPECTED_DEVICE};
    use std::collections::VecDeque;

    #[test]
    fn restore_keeps_274_verified_frames_and_rejects_an_uncommitted_275th() {
        let run_id = format!("{}-restore-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"), std::process::id());
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/tests/ct-engine").join(run_id);
        let root = run.join("demo6");
        let frames_dir = root.join("frames");
        fs::create_dir_all(&frames_dir).unwrap();
        fs::write(run.join("command.txt"), "cargo test -p ct-engine scan::tests::restore_keeps_274_verified_frames_and_rejects_an_uncommitted_275th\n").unwrap();
        let parameters = Parameters {
            task_id: "demo6".into(), save_path: run.to_string_lossy().into_owned(),
            projection_count: 360, angle_step_deg: 1.0, exposure_ms: 1000.0,
        };
        let config = RealScanConfig { parameters: parameters.clone(), max_xray_sec: 600,
            voltage_kv: 59.9, current_ua: 200.0, resume_frames: Vec::new() };
        let mut committed = Vec::new();
        for index in 1..=274 {
            let path = frames_dir.join(format!("frame-{index:04}.nef"));
            fs::write(&path, format!("verified projection {index}")).unwrap();
            let (bytes, sha256) = hash_file(&path).unwrap();
            committed.push(RealFrame {
                index, angle_deg: projection_job(index - 1, 360).angle_deg,
                exposure_ms: 1000.0, file_name: format!("frame-{index:04}.nef"),
                path: path.to_string_lossy().into_owned(), bytes, sha256,
            });
        }
        persist(&root, &parameters, &config, &committed, false).unwrap();
        assert!(manifest_file_exists(&parameters));
        assert!(resume_manifest_exists(&parameters));
        assert_eq!(load_resume_frames(&parameters, 59.9, 200.0).unwrap(), committed);
        assert_eq!(projection_job(committed.len() as u32, 360).index, 275);
        assert_eq!(projection_job(committed.len() as u32, 360).angle_deg, -274.0);
        let incomplete = frames_dir.join("frame-0275.nef");
        fs::write(&incomplete, b"incomplete exposure").unwrap();
        assert!(load_resume_frames(&parameters, 59.9, 200.0).unwrap_err().contains("uncommitted"));
        fs::remove_file(incomplete).unwrap();
        assert_eq!(load_resume_frames(&parameters, 59.9, 200.0).unwrap().len(), 274);
        persist(&root, &parameters, &config, &committed, true).unwrap();
        assert!(!resume_manifest_exists(&parameters), "a completed manifest is not a resumable checkpoint");
        fs::write(run.join("summary.md"), "PASS: 274 committed frames reload; an extra partial frame blocks restore; removing only the partial frame permits view 275.\n").unwrap();
    }

    #[test]
    fn stopped_outcome_requires_confirmed_beam_and_warning_off() {
        assert_eq!(finish_scan_result(Err(CANCELLED.into()), Ok(()), Ok(())).unwrap(), ScanCompletion::Stopped);
        assert!(finish_scan_result(Err(CANCELLED.into()), Err("readback lost".into()), Ok(())).is_err());
        assert!(finish_scan_result(Err(CANCELLED.into()), Ok(()), Err("warning stuck".into())).is_err());
        assert_eq!(finish_scan_result(Ok(()), Ok(()), Ok(())).unwrap(), ScanCompletion::Completed);
    }

    #[test]
    fn disconnected_worker_outcome_is_a_terminal_fault() {
        let (sender, receiver) = mpsc::channel::<u8>();
        drop(sender);
        assert!(poll_scan_outcome(&receiver, &mut None).unwrap().is_err());
    }

    #[test]
    fn scan_log_keeps_the_fault_and_all_scan_events() {
        let messages = vec![
            ScanMessage { timestamp: "2026-09-28T10:00:00Z".into(), level: "ACTION", source: "nano", message: "Verifying final position".into() },
            ScanMessage { timestamp: "2026-09-28T10:00:01Z".into(), level: "ERR", source: "system", message: "final Nano STATUS failed\nread timed out".into() },
        ];
        let mut output = Vec::new();
        write_scan_log(&mut output, &messages, &Err("final Nano STATUS failed: read timed out".into())).unwrap();
        let log = String::from_utf8(output).unwrap();
        assert!(log.contains("2026-09-28T10:00:00Z\tACTION\tnano\tVerifying final position"));
        assert!(log.contains("2026-09-28T10:00:01Z\tERR\tsystem\tfinal Nano STATUS failed read timed out"));
        assert!(log.contains("\tRESULT\tsystem\tFAULT: final Nano STATUS failed: read timed out"));
    }

    #[test]
    fn final_position_requires_the_last_committed_projection_and_idle_reference() {
        let final_job = projection_job(359, 360);
        assert_eq!(final_job.angle_mdeg, -359_000);
        assert_eq!(expected_pulses(final_job.angle_mdeg, 96_000), -95_733);
        let mut status = NanoStatus {
            state: "IDLE".into(), position_pulses: -95_733, target_pulses: -95_733,
            microsteps: 8, pulses_per_rev: 96_000, reference_valid: true,
            homed: true, rearmed: true, hall_active: false, capture_id: 0,
        };
        assert!(validate_final_position(final_job, &status).is_ok());
        status.position_pulses = 0;
        assert!(validate_final_position(final_job, &status).is_err());
        status.position_pulses = -95_733;
        status.capture_id = 1;
        assert!(validate_final_position(final_job, &status).is_err());
        status.capture_id = 0;
        status.reference_valid = false;
        assert!(validate_final_position(final_job, &status).is_err());
        status.reference_valid = true;
        status.state = "CAPTURE_HOLD".into();
        assert!(validate_final_position(final_job, &status).is_err());
        status.state = "IDLE".into();
        status.pulses_per_rev = 0;
        assert!(validate_final_position(final_job, &status).is_err());
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
                "PING" => { lines.push_back(format!("ACK {id} PING")); lines.push_back(format!("PONG {id}")); }
                "INFO" => {
                    lines.push_back(format!("ACK {id} INFO"));
                    lines.push_back(format!("INFO {id} DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"));
                }
                "HEARTBEAT" => lines.push_back(format!("HBACK {id}")),
                "MOVE_ABS" => {
                    self.capture_id = id.parse().unwrap();
                    lines.push_back(format!("ACK {id} MOVE_ABS"));
                    self.announced.send(self.capture_id).unwrap();
                    // The test withholds READY_TO_CAPTURE until it has sampled
                    // the live progress while this motion is still pending.
                }
                "STATUS" => {
                    lines.push_back(format!("ACK {id} STATUS"));
                    lines.push_back(format!("STATUS {id} state=CAPTURE_HOLD pos=-24000 target=-24000 microsteps=8 ppr=96000 reference=1 homed=1 rearmed=1 hall=0 capture_id={}", self.capture_id));
                }
                "STOP" => { lines.push_back(format!("ACK {id} STOP")); lines.push_back(format!("STOPPED {id} POS=-24000")); }
                "XRAY_WARNING" => {
                    lines.push_back(format!("ACK {id} XRAY_WARNING"));
                    lines.push_back(format!("OK {id} XRAY_WARNING={}", fields.get(2).unwrap_or(&"OFF")));
                }
                _ => {}
            }
            Ok(())
        }
        fn read_line(&mut self, timeout: Duration) -> std::io::Result<Option<String>> {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(line) = self.lines.lock().unwrap().pop_front() { return Ok(Some(line)); }
                if Instant::now() >= deadline { return Ok(None); }
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
        adapter.connect_transport_for_test("MEMORY-ONLY", Box::new(DeferredMoveTransport {
            lines: lines.clone(), announced, capture_id: 0,
        })).unwrap();
        let adapter = Arc::new(adapter);
        let progress = Arc::new(Mutex::new(RealScanProgress::default()));
        let worker_adapter = adapter.clone();
        let worker_progress = progress.clone();
        let worker = thread::spawn(move || move_to_projection(&worker_adapter, &worker_progress, projection_job(1, 4), &AtomicBool::new(false)));
        let arrival = receiver.recv_timeout(Duration::from_secs(2));
        let before = progress.lock().unwrap().angle_deg;
        if let Ok(id) = arrival {
            lines.lock().unwrap().push_back(format!("READY_TO_CAPTURE {id} POS=-24000"));
        } else {
            let _ = adapter.stop();
        }
        let result = worker.join().unwrap();
        assert!(arrival.is_ok(), "in-memory motion was not issued");
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(before, 0.0, "a target must not appear as current feedback before arrival");
        assert_eq!(progress.lock().unwrap().angle_deg, -90.0);
    }

    #[test]
    fn committing_an_older_frame_does_not_rewind_prefetched_live_position() {
        let progress = Mutex::new(RealScanProgress::default());
        set_angle(&progress, -90.0);
        push_frame(&progress, RealFrame {
            index:1, angle_deg:0.0, exposure_ms:200.0, file_name:"frame-0001.nef".into(),
            path:"memory-only".into(), bytes:1, sha256:"test".into(),
        });
        let state = progress.lock().unwrap();
        assert_eq!(state.captured, 1);
        assert_eq!(state.frames[0].angle_deg, 0.0);
        assert_eq!(state.angle_deg, -90.0);
    }

    #[test]
    fn exposure_boundary_accepts_transfer_start_or_configured_time_evidence() {
        let timeout = Duration::from_millis(2_200);
        assert!(!exposure_is_complete(
            false,
            Duration::from_millis(2_199),
            timeout
        ));
        assert!(exposure_is_complete(
            true,
            Duration::from_millis(100),
            timeout
        ));
        assert!(exposure_is_complete(false, timeout, timeout));
    }

    #[test]
    fn short_beam_budget_cools_before_starting_the_next_exposure() {
        let now = Instant::now();
        let started = now - Duration::from_secs(591);
        let limit = Duration::from_secs(600);
        let exposure_budget = Duration::from_secs(5);
        assert!(needs_cooldown_before_exposure(
            started, now, limit, exposure_budget + MOVE_BUDGET_MARGIN,
        ));
        assert!(!needs_cooldown_before_exposure(
            started, now, limit, exposure_budget,
        ));
        assert!(needs_cooldown_before_exposure(
            started, now + Duration::from_secs(4), limit, exposure_budget,
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
        assert!(jobs.iter().enumerate().all(|(index, job)|
            job.index == index as u32 + 1 && job.angle_mdeg == -(index as i32 * 2_000)));
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
