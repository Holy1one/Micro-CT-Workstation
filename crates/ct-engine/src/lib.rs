//! Desktop scan-state owner and protocol-v1 command processor.
//! Production domain engine for the Micro-CT workstation.
//!
//! The engine is the single owner of device state, safety gates, scan state,
//! and the versioned JSONL API exposed to the Tauri desktop shell. Device-
//! specific I/O lives under `devices`; scan transaction execution lives under
//! `scan`. Keeping those directions one-way prevents hardware drivers from
//! acquiring a second copy of application state.

pub mod devices;
mod scan;

use devices::camera::{CameraHealth, DigiCamControlAdapter, valid_exposure_ms, EXPOSURE_MIN_MS, EXPOSURE_MAX_MS};
use devices::turntable::{NanoAdapter, NanoConnectionState, NanoHealth};
use devices::xray::{
    MoxtekAdapter, XrayHealth, MAX_CURRENT_UA, MAX_SETPOINT_POWER_W, MAX_VOLTAGE_KV,
    MIN_VOLTAGE_KV,
};
use scan::{RealScanConfig, RealScanHandle, ScanCompletion};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u32 = 1;
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

#[derive(Debug, Deserialize, Serialize)]
pub struct Request {
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
            if task_id.trim().is_empty() {
                return Err("INVALID_PARAMETERS");
            }
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
    camera: Option<DigiCamControlAdapter>,
    xray: Option<MoxtekAdapter>,
    real_scan: Option<RealScanHandle>,
    real_scan_revision: u64,
    real_scan_message_count: usize,
    scan_angle_deg: Option<f64>,
    real_frames: Vec<Value>,
    cached_camera_health: Option<CameraHealth>,
    cached_xray_health: Option<XrayHealth>,
    last_xray_poll: Instant,
    last_error: Option<String>,
}

impl Engine {
    pub fn new(preview: bool) -> Self {
        let mut engine = Self {
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
            scan_angle_deg: None,
            real_frames: Vec::new(),
            cached_camera_health: None,
            cached_xray_health: None,
            last_xray_poll: Instant::now() - Duration::from_secs(10),
            last_error: None,
        };
        engine.log("INFO", "system", "ct-engine started; waiting for an explicit connection");
        engine.log("WARN", "xray", "NO REAL HARDWARE · output remains fail-closed");
        engine
    }

    pub fn handle(&mut self, request: Request) -> Response {
        self.sync_real_scan();
        self.tick();
        self.sync_nano_health();
        self.sync_xray_live_status();
        let error = if request.protocol_version != PROTOCOL_VERSION {
            Some("PROTOCOL_VERSION_MISMATCH")
        } else if request.request_id.is_empty()
            || request.timestamp.is_empty()
            || request.error_code.is_some()
        {
            Some("INVALID_ENVELOPE")
        } else if request.sequence <= self.request_sequence {
            Some("STALE_REQUEST")
        } else {
            self.request_sequence = request.sequence;
            self.validate_payload_type(&request.command, &request.payload)
                .and_then(|_| self.command(&request.command, request.payload))
                .err()
        };
        self.sequence += 1;
        if let Some(code) = error {
            self.log("WARN", "system", &format!("Command rejected: {code}"));
        }
        Response {
            protocol_version: PROTOCOL_VERSION,
            request_id: request.request_id,
            command: request.command,
            payload: self.snapshot(),
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
        self.real_scan.is_some() || matches!(self.phase, Phase::Running | Phase::Paused | Phase::Finishing | Phase::Stopping)
    }

    fn sync_real_scan(&mut self) {
        let Some(handle) = self.real_scan.as_mut() else { return; };
        let progress = handle.progress();
        let outcome = handle.try_finish();
        if progress.revision != self.real_scan_revision {
            self.real_scan_revision = progress.revision;
            self.current = progress.captured;
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
                self.cached_xray_health = Some(health);
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
                self.fault_latched = true;
                self.preflight = false;
                self.homed = false;
                self.phase = Phase::Fault;
                if let Some(health) = self.cached_xray_health.as_mut() {
                    health.connected = false;
                    health.beam_off_confirmed = false;
                    health.last_error = Some(error.clone());
                }
                if let Some(nano) = self.nano.as_ref() { let _ = nano.stop(); }
                self.log("ERR", "system", &error);
            }
            Ok(outcome) => {
                self.beam_on = outcome.xray.health().beam_on;
                self.cached_camera_health = Some(outcome.camera.health());
                self.cached_xray_health = Some(outcome.xray.health());
                self.camera = Some(outcome.camera);
                self.xray = Some(outcome.xray);
                match outcome.result {
                    Ok(ScanCompletion::Completed) if !self.fault_latched => {
                        self.phase = Phase::Completed;
                        self.last_error = None;
                    }
                    Ok(ScanCompletion::Stopped) if !self.fault_latched => {
                        self.invalidate(Phase::Stopped);
                        self.last_error = None;
                        self.log("INFO", "system", "Scan ended; saved projections retained. Repeat Preflight and HOME before a new scan.");
                    }
                    result => {
                        if let Err(error) = result { self.last_error = Some(error); }
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
    }

    fn nano_error(&mut self, context: &str, error: impl std::fmt::Display) -> &'static str {
        let message = format!("{context}: {error}");
        self.last_error = Some(message.clone());
        self.connected = false;
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
        if self.preview || self.real_scan.is_some() {
            return;
        }
        let poll_interval = if self.beam_on {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(2)
        };
        if self.last_xray_poll.elapsed() < poll_interval {
            return;
        }
        let Some(xray) = self.xray.as_mut() else {
            return;
        };
        self.last_xray_poll = Instant::now();
        let was_beam_on = self.beam_on;
        let result = if was_beam_on {
            xray.refresh_emission_status()
        } else {
            xray.refresh_status()
        };
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
                let health = xray.health();
                self.beam_on = health.beam_on;
                self.usb_auto_shut_down = health
                    .usb_auto_shutdown
                    .unwrap_or(self.usb_auto_shut_down);
                self.cached_xray_health = Some(health);
                if was_beam_on {
                    self.fault_latched = true;
                    self.invalidate(Phase::Fault);
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
    pub fn shutdown(&mut self) {
        let _ = self.stop_real_scan();
        if self.real_scan.is_some() {
            let deadline = Instant::now() + Duration::from_secs(65);
            while self.real_scan.is_some() && Instant::now() < deadline {
                self.sync_real_scan();
                thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = self.force_xray_off("Engine shutdown");
        if let Some(mut xray) = self.xray.take() {
            let _ = xray.disconnect();
        }
        if let Some(nano) = self.nano.take() {
            if let Ok(mut nano) = Arc::try_unwrap(nano) {
                nano.disconnect();
            }
        }
        if let Some(mut camera) = self.camera.take() {
            camera.disconnect();
        }
        self.beam_on = false;
        eprintln!("ct-engine shutdown complete · Moxtek OFF and warning OFF attempted");
    }

    fn sync_nano_health(&mut self) {
        let Some(nano) = self.nano.as_ref() else {
            return;
        };
        let health = nano.health();
        if !matches!(health.state, NanoConnectionState::Connected) {
            let detail = health
                .last_error
                .unwrap_or_else(|| format!("Nano state {:?}", health.state));
            self.last_error = Some(detail.clone());
            self.connected = false;
            self.fault_latched = true;
            self.invalidate(Phase::Fault);
        }
    }

    fn command(&mut self, command: &str, payload: Value) -> Result<(), &'static str> {
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
            let xray_off = self.force_xray_off("Disconnect");
            if let Some(mut xray) = self.xray.take() {
                if let Err(error) = xray.disconnect() {
                    self.log("ERR", "xray", &format!("Disconnect cleanup could not reconfirm Moxtek OFF: {error}"));
                }
            }
            if let Some(nano) = self.nano.take() {
                if let Ok(mut nano) = Arc::try_unwrap(nano) {
                    nano.disconnect();
                }
            }
            if let Some(mut camera) = self.camera.take() {
                camera.disconnect();
            }
            self.cached_camera_health = None;
            self.cached_xray_health = None;
            self.set_kv_confirmed = false;
            self.set_ua_confirmed = false;
            self.usb_auto_shut_down = true;
            self.usb_auto_shut_down_confirmed = false;
            self.connected = false;
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
                        self.invalidate(Phase::Idle);
                        self.fault_latched = false;
                        self.last_error = None;
                        self.log(
                            "INFO",
                            "nano",
                            &format!(
                                "{} v{} build {} connected · state={} · HOME not executed",
                                identity.device, identity.version, identity.build, status.state
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
                self.invalidate(Phase::Idle);
                self.log("INFO", "system", "Scan parameters updated · safety checks invalidated");
            }
            "update_scan_setup" => {
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                let setup: ScanSetupInput = serde_json::from_value(
                    payload.get("setup").cloned().ok_or("INVALID_PARAMETERS")?,
                )
                .map_err(|_| "INVALID_PARAMETERS")?;
                let (parameters, max_xray_sec) =
                    setup.apply(&self.parameters, self.max_xray_sec)?;
                if setup.exposure_ms.is_some() {
                    let health = self.camera.as_ref().map(DigiCamControlAdapter::health)
                        .or_else(|| self.cached_camera_health.clone());
                    if let Some(health) = health.filter(|health| health.connected) {
                        if let Some((min, max)) = health.exposure_min_ms.zip(health.exposure_max_ms) {
                            if !(min..=max).contains(&parameters.exposure_ms) {
                                return Err("EXPOSURE_OUTSIDE_CAMERA_RANGE");
                            }
                        }
                    }
                }
                self.parameters = parameters;
                self.max_xray_sec = max_xray_sec;
                self.current = 0;
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
                    if !(1..=600).contains(&self.max_xray_sec) {
                        return Err("INVALID_PARAMETERS");
                    }
                    if !self
                        .camera
                        .as_ref()
                        .is_some_and(|camera| camera.health().connected)
                    {
                        let mut camera = DigiCamControlAdapter::discover()
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
                    if nano_status.state == "FAULT" {
                        nano.clear_fault()
                            .map_err(|error| self.nano_error("Nano CLEAR_FAULT failed", error))?;
                        self.log("WARN", "nano", "Previous Nano fault cleared · reference remains invalid until HOME");
                    }
                    let status = nano
                        .rearm()
                        .map_err(|error| self.nano_error("Nano REARM failed", error))?;
                    self.fault_latched = false;
                    self.preflight = true;
                    self.homed = false;
                    self.phase = Phase::ReadyForHome;
                    self.last_error = None;
                    self.log(
                        "PASS",
                        "preflight",
                        &format!(
                            "8/8 real checks passed · Nano={} · D7100 host-only · Moxtek OFF/unlocked {:.1} kV / {:.1} µA · {:.1} C",
                            status.state,
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
                    self.current = 0;
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
                    if self.fault_latched {
                        return Err("FAULT_RECOVERY_REQUIRED");
                    }
                    if !self.usb_auto_shut_down_confirmed || self.usb_auto_shut_down {
                        return Err("USB_AUTO_SHUTDOWN_RELEASE_REQUIRED");
                    }
                    self.parameters.validate()?;
                    self.validate_setpoint(self.set_kv, self.set_ua)?;
                    let nano = self.nano.as_ref().ok_or("NANO_NOT_CONNECTED")?.clone();
                    let camera = self.camera.take().ok_or("CAMERA_NOT_CONNECTED")?;
                    let xray = match self.xray.take() {
                        Some(xray) => xray,
                        None => {
                            self.camera = Some(camera);
                            return Err("XRAY_NOT_CONNECTED");
                        }
                    };
                    self.cached_camera_health = Some(camera.health());
                    self.cached_xray_health = Some(xray.health());
                    self.current = 0;
                    self.scan_angle_deg = Some(0.0);
                    self.real_frames.clear();
                    self.real_scan_revision = 0;
                    self.real_scan_message_count = 0;
                    self.phase = Phase::Running;
                    self.beam_on = false;
                    self.last_error = None;
                    self.real_scan = Some(RealScanHandle::start(
                        nano,
                        camera,
                        xray,
                        RealScanConfig {
                            parameters: self.parameters.clone(),
                            max_xray_sec: self.max_xray_sec,
                            voltage_kv: self.set_kv,
                            current_ua: self.set_ua,
                        },
                    ));
                    self.log("ACTION", "operator", "Real scan worker started · output remains OFF until first READY_TO_CAPTURE");
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
                if !self.preview { return Err("RESTORE_UNAVAILABLE"); }
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
                self.current = (self.parameters.projection_count / 3).max(1);
                self.phase = Phase::Paused;
                self.log("WARN", "system", "Preview checkpoint restored · no real task file was read");
            }
            "retry_device" => {
                let device = payload.get("device").and_then(Value::as_str).ok_or("INVALID_DEVICE")?;
                if !matches!(device, "turntable" | "camera" | "xray") {
                    return Err("INVALID_DEVICE");
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
                            let mut camera = DigiCamControlAdapter::discover()
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
                            if self.xray.is_some() {
                                return Err("XRAY_ALREADY_CONNECTED");
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
                self.cached_xray_health = None;
                self.set_kv_confirmed = false;
                self.set_ua_confirmed = false;
                self.usb_auto_shut_down = true;
                self.usb_auto_shut_down_confirmed = false;
                self.preflight = false;
                self.homed = false;
                self.phase = Phase::Idle;
                result.map_err(|error| {
                    let message = format!("Moxtek disconnect failed: {error}");
                    self.last_error = Some(message.clone());
                    self.log("ERR", "xray", &message);
                    "XRAY_DISCONNECT_FAILED"
                })?;
                self.last_error = None;
                self.log("PASS", "xray", "Moxtek OFF confirmed and manually disconnected");
            }
            "camera_test_capture" => {
                if self.preview {
                    return Err("REAL_CAMERA_REQUIRED");
                }
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                self.parameters.validate()?;
                let save_path = Path::new(self.parameters.save_path.trim());
                let task_id = self.parameters.task_id.clone();
                let camera = self.camera.as_mut().ok_or("CAMERA_NOT_CONNECTED")?;
                let path = match camera.capture_test(save_path, &task_id) {
                    Ok(path) => path,
                    Err(error) => {
                        let message = format!("D7100 test capture failed: {error}");
                        self.last_error = Some(message.clone());
                        self.log("ERR", "camera", &message);
                        return Err("CAMERA_CAPTURE_FAILED");
                    }
                };
                self.log(
                    "PASS",
                    "camera",
                    &format!("D7100 test capture saved · {}", path.display()),
                );
                self.cached_camera_health = self.camera.as_ref().map(DigiCamControlAdapter::health);
                self.last_error = None;
            }
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
                    self.force_xray_off("Moxtek voltage setpoint")?;
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
                    self.force_xray_off("Moxtek current setpoint")?;
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

    fn log(&mut self, level: &str, source: &str, message: &str) {
        self.logs.insert(0, json!({
            "id": format!("{}-{}", self.sequence, timestamp()),
            "timestamp": timestamp(),
            "level": level,
            "source": source,
            "message": message,
        }));
        self.logs.truncate(MAX_LOGS);
    }

    fn tick(&mut self) {
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
            .map(DigiCamControlAdapter::health)
            .or_else(|| self.cached_camera_health.clone());
        let camera_connected = self.real_scan.is_some() || camera_health
            .as_ref()
            .is_some_and(|health| health.connected);
        let xray_health = self
            .xray
            .as_ref()
            .map(MoxtekAdapter::health)
            .or_else(|| self.cached_xray_health.clone());
        let xray_connected = self.real_scan.is_some() || xray_health
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
        } else if live_xray.is_some_and(|health| health.beam_on) {
            "on"
        } else if live_xray.is_some_and(|health| health.beam_off_confirmed) && !self.beam_on {
            "off"
        } else {
            "unknown"
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
        let device_tone = if self.phase == Phase::Fault || nano_faulted { "danger" } else if self.connected { "accent" } else { "muted" };
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
                "source": if matches!(source, "system" | "xray" | "nano" | "camera" | "preflight" | "operator") { source } else { "system" },
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
                {"id":"camera","name":"Camera","word":if camera_connected {"ONLINE"} else if self.preview {"PREVIEW"} else {"OFFLINE"},"tone":if camera_connected {"accent"} else if self.preview {device_tone} else {"muted"},"spec":if let Some(health) = camera_health.as_ref() {format!("D7100 · SERIAL {} · {}", health.serial.as_deref().unwrap_or("unknown"), health.transfer_policy.as_deref().unwrap_or("transfer unverified"))} else if self.preview {"D7100 · NO REAL CAMERA CONNECTION".to_owned()} else {"D7100 · CONNECT VIA DIGICAMCONTROL".to_owned()}}
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
                {"key":"CAMERA","text":if camera_connected {"D7100 ONLINE"} else if self.preview {"PREVIEW ONLY"} else {"OFFLINE"},"tone":if camera_connected {"accent"} else {"muted"}},
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
                "right":if self.preview {"ct-engine · JSONL v1 · PREVIEW"} else if nano_connected {"ct-engine · JSONL v1 · NANO ONLINE"} else {"ct-engine · JSONL v1 · LOCKED"},
                "dotTone":if self.phase == Phase::Fault {"danger"} else if self.phase == Phase::Running {"warn"} else if self.phase == Phase::Paused {"accent"} else {"muted"}
            },
            "dock":{
                "home":self.connected && self.preflight && !self.busy() && !self.fault_latched,
                "play":self.connected && !self.fault_latched && ((self.phase == Phase::Running || self.phase == Phase::Paused) || (!self.busy() && self.preflight && self.homed && (self.preview || (self.usb_auto_shut_down_confirmed && !self.usb_auto_shut_down)))),
                "restore":self.preview && self.connected && !self.busy() && self.preflight && self.homed && !self.fault_latched && !matches!(self.phase, Phase::Fault | Phase::Stopped),
                "stop":matches!(self.phase, Phase::Running | Phase::Paused | Phase::Finishing),
                "playMode":if self.phase == Phase::Running {"pause"} else if self.phase == Phase::Paused {"resume"} else if matches!(self.phase, Phase::Fault | Phase::Finishing | Phase::Stopping) {"disabled"} else {"start"},
                "homeReason":if !self.connected {"Connect the Nano first"} else if self.fault_latched || self.phase == Phase::Fault {"Run Preflight to revalidate device safety"} else if !self.preflight {"Complete scan setup and run Preflight first"} else if self.busy() {"HOME is unavailable while a scan is active"} else {""},
                "playReason":if self.phase == Phase::Running {"Pause after the active transaction closes"} else if self.phase == Phase::Paused {"Resume the paused scan"} else if matches!(self.phase, Phase::Finishing | Phase::Stopping) {"Wait for scan cleanup"} else if !self.connected {"Connect the Nano first"} else if self.fault_latched || self.phase == Phase::Fault {"Run Preflight to recover"} else if !self.preflight {"Run Preflight first"} else if !self.homed {"Run HOME first"} else if !self.preview && (!self.usb_auto_shut_down_confirmed || self.usb_auto_shut_down) {"Disable USB Auto Shut Down manually in the X-ray panel"} else {""}
            },
            "scanSetup":{
                "savePath":self.parameters.save_path,"taskId":self.parameters.task_id,"projectionCount":total,
                "angleStepDeg":self.parameters.angle_step_deg,"exposureMs":self.parameters.exposure_ms,"maxXraySec":self.max_xray_sec
            },
            "consoleLogs":console_logs,"frames":frames,"checkpointAvailable":self.current > 0
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
        let camera_health = self.camera.as_ref().map(DigiCamControlAdapter::health)
            .or_else(|| self.cached_camera_health.clone());
        let camera_connected = self.real_scan.is_some() || camera_health
            .as_ref()
            .is_some_and(|health| health.connected);
        let xray_health = self.xray.as_ref().map(MoxtekAdapter::health)
            .or_else(|| self.cached_xray_health.clone());
        let xray_connected = self.real_scan.is_some() || xray_health
            .as_ref()
            .is_some_and(|health| health.connected);
        let mode_label = if self.preview {
            "DEVELOPER PREVIEW · NO REAL HARDWARE".to_owned()
        } else if nano_connected && xray_connected {
            "PRODUCTION · NANO + D7100 + MOXTEK".to_owned()
        } else if nano_faulted {
            "PRODUCTION · NANO FAULT · CLEAR + PREFLIGHT REQUIRED".to_owned()
        } else if nano_connected {
            "PRODUCTION · NANO ONLINE · CAMERA/X-RAY REQUIRED".to_owned()
        } else if xray_connected {
            "PRODUCTION · X-RAY SAFE CONFIG · NANO REQUIRED".to_owned()
        } else {
            "PRODUCTION LOCKED · NANO DISCONNECTED".to_owned()
        };
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
            "mode":mode,"modeLabel":mode_label,"connectionState":connection_state,"adapterLabel":"ct-engine · Nano CH340 · JSONL stdio",
            "phase":phase,"phaseLabel":label,"preflightPassed":self.preflight,"homed":self.homed,
            "requiresPreflight":!self.preflight,"requiresHome":!self.homed,
            "safety":{
                "xrayAvailable":xray_connected && self.preflight && self.homed && !self.fault_latched,"xrayEnabled":self.beam_on,
                "interlockOk":xray_health.as_ref().and_then(|health| health.locked).is_some_and(|locked| !locked),
                "lockReason":if self.preview {"Developer preview has no real X-ray hardware"} else if xray_connected {"Moxtek communication and setpoints verified; beam enable remains locked"} else {"Stage 1 Nano only · camera and X-ray are locked OFF"}
            },
            "devices":[
                {"id":"turntable","label":"Precision Turntable","state":device_state,"detail":nano_detail},
                {"id":"camera","label":"Nikon D7100","state":if camera_connected {"connected"} else if self.preview {device_state} else {"offline"},"detail":if let Some(health) = camera_health.as_ref() {format!("digiCamControl · serial {} · {}", health.serial.as_deref().unwrap_or("unknown"), health.transfer_policy.as_deref().unwrap_or("transfer unverified"))} else if self.preview {mode_label.clone()} else {"Use Camera Retry to connect through digiCamControl".to_owned()}},
                {"id":"xray","label":"Moxtek 12 W","state":if xray_connected {"connected"} else {"locked"},"detail":if let Some(health) = xray_health.as_ref() {format!("Safe-OFF configuration session · port {} · beam OFF confirmed", health.port.as_deref().unwrap_or("unknown"))} else {"Fail-closed · connect confirms OFF before allowing setpoint tests".to_owned()}}
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
            protocol_version: 1,
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
        let mut engine = Engine::new(false);
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
            protocol_version: 1,
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
            protocol_version: 1,
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
        incompatible.protocol_version = 2;
        incompatible.sequence = 2;
        assert_eq!(engine.handle(incompatible).error_code.as_deref(), Some("PROTOCOL_VERSION_MISMATCH"));
    }
}
