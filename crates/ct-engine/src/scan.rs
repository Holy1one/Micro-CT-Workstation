//! Transactional projection-scan coordinator.
//!
//! A projection is committed only after turntable arrival, warning assertion,
//! verified X-ray exposure, host-side camera file confirmation, verified beam
//! shutdown, and `CAPTURE_DONE`. Pause is observed only at safe transaction
//! boundaries. USB auto-shutdown is an operator-owned device setting: the
//! coordinator never changes it. Every exit path still attempts verified beam
//! OFF and warning OFF before returning.

use crate::devices::camera::{CameraError, DigiCamControlAdapter};
use crate::devices::turntable::{MoveTicket, NanoAdapter};
use crate::devices::xray::{MoxtekAdapter, XrayHealth};
use crate::{timestamp, Parameters};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::collections::VecDeque;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CAMERA_TRIGGER_MARGIN: Duration = Duration::from_secs(2);
const XRAY_COOLDOWN: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub struct RealScanConfig {
    pub parameters: Parameters,
    pub max_xray_sec: u32,
    pub voltage_kv: f64,
    pub current_ua: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ProjectionJob {
    index: u32,
    angle_mdeg: i32,
    angle_deg: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealFrame {
    pub index: u32,
    pub angle_deg: f64,
    pub exposure_ms: u32,
    pub file_name: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct ScanMessage {
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
        }
    }
}

pub struct ScanOutcome {
    pub camera: DigiCamControlAdapter,
    pub xray: MoxtekAdapter,
    pub result: Result<(), String>,
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
        let progress = Arc::new(Mutex::new(RealScanProgress::default()));
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
                let result = run_scan(
                    &worker_nano,
                    &mut camera,
                    &mut xray,
                    &config,
                    &worker_cancel,
                    &worker_pause,
                    &worker_progress,
                );
                if let Err(error) = &result {
                    update(&worker_progress, |value| {
                        value.phase = "fault";
                        value.beam_on = false;
                        value.error = Some(error.clone());
                        value.messages.push(ScanMessage {
                            level: "ERR",
                            source: "system",
                            message: format!("Real scan failed closed · {error}"),
                        });
                    });
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
        self.progress.lock().expect("scan progress mutex poisoned").clone()
    }

    pub fn request_pause(&self) {
        self.pause.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.pause.store(false, Ordering::SeqCst);
    }

    pub fn request_stop(&self) -> Result<(), String> {
        self.cancel.store(true, Ordering::SeqCst);
        self.nano
            .stop()
            .map_err(|error| format!("Nano STOP failed: {error}"))
    }

    pub fn try_finish(&mut self) -> Option<ScanOutcome> {
        let outcome = self.outcome.try_recv().ok()?;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        Some(outcome)
    }
}

impl Drop for RealScanHandle {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.cancel.store(true, Ordering::SeqCst);
            let _ = self.nano.stop();
            let _ = worker.join();
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
    exposure_ms: u32,
    frames: &'a [RealFrame],
}

fn run_scan(
    nano: &NanoAdapter,
    camera: &mut DigiCamControlAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &AtomicBool,
    pause: &AtomicBool,
    progress: &Mutex<RealScanProgress>,
) -> Result<(), String> {
    let result = run_scan_inner(nano, camera, xray, config, cancel, pause, progress);
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
    set_beam(progress, false);
    if result.is_err() {
        let _ = nano.stop();
    }

    let mut errors = Vec::new();
    if let Err(error) = result {
        errors.push(error);
    }
    if let Err(error) = off {
        errors.push(format!("final Moxtek OFF failed: {error}"));
    }
    if let Err(error) = warning_off {
        errors.push(format!("final warning OFF failed: {error}"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
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
    if root.exists() && fs::read_dir(&root).map_err(io_error)?.next().is_some() {
        return Err(format!("scan output already exists: {}", root.display()));
    }
    fs::create_dir_all(root.join("frames")).map_err(io_error)?;
    push_message(progress, "ACTION", "operator", format!(
        "Real scan started · {} projections · {:.1} kV / {:.1} µA",
        parameters.projection_count, config.voltage_kv, config.current_ua
    ));
    let shutter = camera
        .set_exposure_ms(parameters.exposure_ms)
        .map_err(|error| format!("camera exposure configuration failed: {error}"))?;
    push_message(progress, "PASS", "camera", format!("D7100 shutter set to {shutter}"));
    xray
        .set_parameters(config.voltage_kv, config.current_ua)
        .map_err(|error| format!("Moxtek setpoint configuration failed: {error}"))?;
    set_xray_health(progress, xray.health());

    let block_limit = Duration::from_secs(u64::from(config.max_xray_sec));
    let mut beam_started: Option<Instant> = None;
    let mut cooldown_until: Option<Instant> = None;
    let mut jobs: VecDeque<ProjectionJob> = (0..parameters.projection_count)
        .map(|zero_index| projection_job(zero_index, parameters.projection_count))
        .collect();
    let mut prefetched: Option<(ProjectionJob, MoveTicket)> = None;
    persist(&root, parameters, config, &[], false)?;

    while let Some(job) = jobs.pop_front() {
        check_cancel(cancel)?;
        if let Some(cooldown_deadline) = cooldown_until.take() {
            update_phase(progress, "paused");
            push_message(
                progress,
                "WARN",
                "xray",
                "Continuous X-ray limit reached · cooling for 300 seconds before resuming"
                    .to_owned(),
            );
            while Instant::now() < cooldown_deadline {
                check_cancel(cancel)?;
                thread::sleep(Duration::from_millis(250));
            }
            update_phase(progress, "running");
            push_message(
                progress,
                "PASS",
                "xray",
                "Five-minute X-ray cooldown complete · scan resuming".to_owned(),
            );
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
                return Err("scan cancelled while paused".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        update_phase(progress, "running");
        let ticket = match prefetched.take() {
            Some((prefetched_job, ticket)) if prefetched_job == job => {
                verify_capture_hold(nano, &ticket, job)?;
                push_message(
                    progress,
                    "PASS",
                    "nano",
                    format!(
                        "Queued view={} already at {:.3}° · READY_TO_CAPTURE id={} pos={}",
                        job.index, job.angle_deg, ticket.command_id, ticket.position_pulses
                    ),
                );
                ticket
            }
            Some(_) => return Err("prefetched projection queue order mismatch".into()),
            None => move_to_projection(nano, progress, job)?,
        };
        set_angle(progress, job.angle_deg);

        if beam_started.is_none() {
            nano.set_xray_warning(true)
                .map_err(|error| format!("XRAY_WARNING ON failed: {error}"))?;
            xray.beam_on(cancel)
                .map_err(|error| format!("Moxtek beam-on failed: {error}"))?;
            set_xray_health(progress, xray.health());
            set_beam(progress, true);
            beam_started = Some(Instant::now());
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
        let exposure_evidence_timeout = Duration::from_millis(u64::from(parameters.exposure_ms))
            .saturating_add(CAMERA_TRIGGER_MARGIN);
        let mut projection_released = false;
        let mut transfer_announced = false;
        let camera_for_capture = &mut *camera;
        let capture_result = thread::scope(|scope| {
            let (capture_sender, capture_receiver) = mpsc::channel();
            let save_path = PathBuf::from(&parameters.save_path);
            let task_id = parameters.task_id.clone();
            scope.spawn(move || {
                let result = camera_for_capture.capture_frame(&save_path, &task_id, frame_index);
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
                                    set_beam(progress, false);
                                    let _ = nano.stop();
                                }
                            }
                        }
                        if abort_reason.is_none() && cancel.load(Ordering::SeqCst) {
                            abort_reason = Some("scan cancelled".into());
                            cancel.store(true, Ordering::SeqCst);
                            let _ = xray.force_off();
                            set_xray_health(progress, xray.health());
                            let _ = nano.set_xray_warning(false);
                            set_beam(progress, false);
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
                            let pause_requested = pause.load(Ordering::SeqCst);
                            if exposure_complete
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
                                        }
                                    }
                                    Err(error) => {
                                        abort_reason = Some(error);
                                        cancel.store(true, Ordering::SeqCst);
                                        let _ = nano.stop();
                                    }
                                }
                            }
                            if transfer_started && !projection_released {
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
                                let next_job = if pause.load(Ordering::SeqCst) {
                                    None
                                } else {
                                    jobs.front().copied()
                                };
                                match release_projection_and_prefetch(
                                    nano,
                                    progress,
                                    &ticket,
                                    next_job,
                                    &mut prefetched,
                                ) {
                                    Ok(()) => projection_released = true,
                                    Err(error) => {
                                        abort_reason = Some(error);
                                        cancel.store(true, Ordering::SeqCst);
                                        let _ = xray.force_off();
                                        let _ = nano.set_xray_warning(false);
                                        set_beam(progress, false);
                                        let _ = nano.stop();
                                    }
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
            }
        }
        if !projection_released {
            let next_job = if pause.load(Ordering::SeqCst) {
                None
            } else {
                jobs.front().copied()
            };
            release_projection_and_prefetch(
                nano,
                progress,
                &ticket,
                next_job,
                &mut prefetched,
            )?;
        }
        let (bytes, sha256) = hash_file(&path)?;
        let frame = RealFrame {
            index: frame_index,
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
        let frames = push_frame(progress, frame.clone());
        persist(&root, parameters, config, &frames, false)?;
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

    push_message(
        progress,
        "ACTION",
        "nano",
        "Returning turntable to 0.000° via the forward-only -360.000° target".to_owned(),
    );
    let return_ticket = nano
        .move_abs(-360_000)
        .map_err(|error| format!("final MOVE_ABS -360000 failed: {error}"))?;
    if return_ticket.position_pulses != 0 {
        return Err(format!(
            "final zero ticket reported {} pulses",
            return_ticket.position_pulses
        ));
    }
    let zero_status = nano
        .capture_done(return_ticket.command_id)
        .map_err(|error| format!("final zero CAPTURE_DONE failed: {error}"))?;
    if zero_status.position_pulses != 0
        || !zero_status.reference_valid
        || !zero_status.homed
        || !zero_status.rearmed
    {
        return Err("turntable did not finish at a valid zero reference".into());
    }
    set_angle(progress, 0.0);
    push_message(
        progress,
        "PASS",
        "nano",
        "Turntable returned to 0 pulses · reference valid".to_owned(),
    );

    let frames = progress.lock().expect("scan progress mutex poisoned").frames.clone();
    persist(&root, parameters, config, &frames, true)?;
    update_phase(progress, "completed");
    push_message(progress, "PASS", "system", format!(
        "Real scan complete · {} projections · X-ray OFF",
        frames.len()
    ));
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err("scan cancelled".into())
    } else {
        Ok(())
    }
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
) -> Result<(), String> {
    let status = nano
        .status()
        .map_err(|error| format!("Nano STATUS failed before view {}: {error}", job.index))?;
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
    Ok(())
}

fn move_to_projection(
    nano: &NanoAdapter,
    progress: &Mutex<RealScanProgress>,
    job: ProjectionJob,
) -> Result<MoveTicket, String> {
    set_angle(progress, job.angle_deg);
    push_message(
        progress,
        "ACTION",
        "nano",
        format!("MOVE_ABS view={} angle={:.3}°", job.index, job.angle_deg),
    );
    let ticket = nano
        .move_abs(job.angle_mdeg)
        .map_err(|error| format!("MOVE_ABS view {} failed: {error}", job.index))?;
    verify_capture_hold(nano, &ticket, job)?;
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

fn release_projection_and_prefetch(
    nano: &NanoAdapter,
    progress: &Mutex<RealScanProgress>,
    ticket: &MoveTicket,
    next_job: Option<ProjectionJob>,
    prefetched: &mut Option<(ProjectionJob, MoveTicket)>,
) -> Result<(), String> {
    nano.capture_done(ticket.command_id)
        .map_err(|error| format!("CAPTURE_DONE failed: {error}"))?;
    push_message(
        progress,
        "PASS",
        "nano",
        format!("CAPTURE_DONE id={} · current exposure released", ticket.command_id),
    );
    if let Some(job) = next_job {
        if prefetched.is_some() {
            return Err("projection prefetch queue is already occupied".into());
        }
        let next_ticket = move_to_projection(nano, progress, job)?;
        push_message(
            progress,
            "INFO",
            "nano",
            format!(
                "View {} positioned while previous NEF transfer continues; capture remains queued",
                job.index
            ),
        );
        *prefetched = Some((job, next_ticket));
    }
    Ok(())
}

fn exposure_is_complete(
    transfer_started: bool,
    capture_elapsed: Duration,
    evidence_timeout: Duration,
) -> bool {
    transfer_started || capture_elapsed >= evidence_timeout
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
            level,
            source,
            message,
        });
    });
}

fn push_frame(progress: &Mutex<RealScanProgress>, frame: RealFrame) -> Vec<RealFrame> {
    update(progress, |value| {
        value.captured = frame.index;
        value.angle_deg = frame.angle_deg;
        value.frames.push(frame);
    });
    progress.lock().expect("scan progress mutex poisoned").frames.clone()
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
    fn projection_jobs_are_fifo_and_opposite_for_two_views() {
        assert_eq!(projection_job(0, 2).angle_mdeg, 0);
        assert_eq!(projection_job(1, 2).angle_mdeg, -180_000);
        assert_eq!(expected_pulses(-180_000, 96_000), -48_000);
        assert_eq!(expected_pulses(-360_000, 96_000), 0);
    }
}
