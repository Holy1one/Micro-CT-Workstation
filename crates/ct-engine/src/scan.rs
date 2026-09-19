use crate::adapters::camera::DigiCamControlAdapter;
use crate::adapters::nano::NanoAdapter;
use crate::adapters::xray::{MoxtekAdapter, XrayHealth};
use crate::{timestamp, Parameters};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct RealScanConfig {
    pub parameters: Parameters,
    pub max_xray_sec: u32,
    pub voltage_kv: f64,
    pub current_ua: f64,
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
    set_beam(progress, false);
    if result.is_err() {
        let _ = nano.stop();
    }
    match (result, off, warning_off) {
        (Ok(()), Ok(_), Ok(())) => Ok(()),
        (Err(error), _, _) => Err(error),
        (Ok(()), Err(error), _) => Err(format!("final Moxtek OFF failed: {error}")),
        (Ok(()), Ok(_), Err(error)) => Err(format!("final warning OFF failed: {error}")),
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

    let mut total_on = Duration::ZERO;
    let max_on = Duration::from_secs(u64::from(config.max_xray_sec));
    persist(&root, parameters, config, &[], false)?;

    for zero_index in 0..parameters.projection_count {
        check_cancel(cancel)?;
        while pause.load(Ordering::SeqCst) {
            update_phase(progress, "paused");
            if cancel.load(Ordering::SeqCst) {
                return Err("scan cancelled while paused".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        update_phase(progress, "running");
        let angle_mdeg = -(((i64::from(zero_index) * 360_000)
            + i64::from(parameters.projection_count / 2))
            / i64::from(parameters.projection_count)) as i32;
        let angle_deg = f64::from(angle_mdeg) / 1_000.0;
        set_angle(progress, angle_deg);
        push_message(progress, "ACTION", "nano", format!(
            "MOVE_ABS view={} angle={angle_deg:.3}°",
            zero_index + 1
        ));
        let ticket = nano
            .move_abs(angle_mdeg)
            .map_err(|error| format!("MOVE_ABS failed: {error}"))?;
        let status = nano.status().map_err(|error| format!("Nano STATUS failed: {error}"))?;
        let magnitude = i64::from(angle_mdeg).unsigned_abs() % 360_000;
        let expected = ((u64::from(status.pulses_per_rev) * magnitude + 180_000) / 360_000)
            as i64
            * if angle_mdeg < 0 { -1 } else { 1 };
        if ticket.position_pulses != expected || status.position_pulses != expected {
            return Err(format!(
                "Nano position mismatch: ticket={} status={} expected={expected}",
                ticket.position_pulses, status.position_pulses
            ));
        }
        push_message(progress, "PASS", "nano", format!(
            "READY_TO_CAPTURE id={} pos={expected}", ticket.command_id
        ));

        nano.set_xray_warning(true)
            .map_err(|error| format!("XRAY_WARNING ON failed: {error}"))?;
        let on_started = Instant::now();
        xray.beam_on(cancel)
            .map_err(|error| format!("Moxtek beam-on failed: {error}"))?;
        set_xray_health(progress, xray.health());
        set_beam(progress, true);
        push_message(progress, "ACTION", "xray", format!(
            "Beam confirmed ON · view={} · {:.1} kV / {:.1} µA",
            zero_index + 1, config.voltage_kv, config.current_ua
        ));

        let frame_index = zero_index + 1;
        let mut aborted = false;
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
                        if !aborted
                            && (cancel.load(Ordering::SeqCst)
                                || total_on + on_started.elapsed() >= max_on)
                        {
                            aborted = true;
                            cancel.store(true, Ordering::SeqCst);
                            let _ = xray.force_off();
                            set_xray_health(progress, xray.health());
                            let _ = nano.set_xray_warning(false);
                            set_beam(progress, false);
                            let _ = nano.stop();
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        break Err(crate::adapters::camera::CameraError::Io(
                            "camera capture worker terminated without a result".into(),
                        ));
                    }
                }
            }
        });
        if aborted {
            return Err("scan cancelled or maximum X-ray time reached".into());
        }
        total_on += on_started.elapsed();
        xray.force_off()
            .map_err(|error| format!("Moxtek OFF after capture failed: {error}"))?;
        set_xray_health(progress, xray.health());
        nano.set_xray_warning(false)
            .map_err(|error| format!("XRAY_WARNING OFF failed: {error}"))?;
        set_beam(progress, false);
        let path = capture_result.map_err(|error| format!("D7100 capture failed: {error}"))?;
        let (bytes, sha256) = hash_file(&path)?;
        let frame = RealFrame {
            index: frame_index,
            angle_deg,
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
        nano.capture_done(ticket.command_id)
            .map_err(|error| format!("CAPTURE_DONE failed: {error}"))?;
        push_message(progress, "PASS", "camera", format!(
            "Projection committed · view={} · {} bytes · SHA-256 {}",
            frame.index, frame.bytes, frame.sha256
        ));
    }

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
