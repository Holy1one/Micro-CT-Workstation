//! Desktop scan-state owner and protocol-v1 command processor.
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_LOGS: usize = 100;
const MAX_POWER_W: f64 = 12.0;

pub fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
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
    pub exposure_ms: u32,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            task_id: String::new(),
            save_path: String::new(),
            projection_count: 0,
            angle_step_deg: 0.0,
            exposure_ms: 0,
        }
    }
}

impl Parameters {
    fn validate(&self) -> Result<(), &'static str> {
        if self.task_id.trim().is_empty() || self.save_path.trim().is_empty() {
            return Err("INVALID_PARAMETERS");
        }
        if !(1..=360).contains(&self.projection_count)
            || !self.angle_step_deg.is_finite()
            || self.angle_step_deg <= 0.0
            || self.angle_step_deg > 360.0
            || (self.angle_step_deg * f64::from(self.projection_count) - 360.0).abs() > 0.01
            || !(1..=10000).contains(&self.exposure_ms)
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
    exposure_ms: Option<u32>,
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
            if !(1..=360).contains(&projection_count) {
                return Err("INVALID_PARAMETERS");
            }
            parameters.projection_count = projection_count;
            parameters.angle_step_deg = 360.0 / f64::from(projection_count);
        }
        if let Some(exposure_ms) = self.exposure_ms {
            if !(1..=10_000).contains(&exposure_ms) {
                return Err("INVALID_PARAMETERS");
            }
            parameters.exposure_ms = exposure_ms;
        }
        if let Some(value) = self.max_xray_sec {
            if !(1..=359_999).contains(&value) {
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
    logs: Vec<Value>,
    beam_on: bool,
    xray_latched: bool,
    timer_on: bool,
    usb_auto_shut_down: bool,
    set_kv: f64,
    set_ua: f64,
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
            max_xray_sec: 0,
            current: 0,
            sequence: 0,
            request_sequence: 0,
            last_tick: Instant::now(),
            logs: Vec::new(),
            beam_on: false,
            xray_latched: false,
            timer_on: false,
            usb_auto_shut_down: false,
            set_kv: 80.0,
            set_ua: 100.0,
        };
        engine.log("INFO", "system", "ct-engine started; waiting for an explicit connection");
        engine.log("WARN", "xray", "NO REAL HARDWARE · output remains fail-closed");
        engine
    }

    pub fn handle(&mut self, request: Request) -> Response {
        self.tick();
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
        matches!(self.phase, Phase::Running | Phase::Paused)
    }

    fn invalidate(&mut self, phase: Phase) {
        self.beam_on = false;
        self.preflight = false;
        self.homed = false;
        self.phase = phase;
    }

    fn command(&mut self, command: &str, payload: Value) -> Result<(), &'static str> {
        if command == "snapshot" {
            return Ok(());
        }
        if command == "stop" {
            self.xray_latched = true;
            self.invalidate(Phase::Fault);
            self.log("ERR", "system", "E-STOP latched · output disabled · repeat preflight and HOME");
            return Ok(());
        }
        if command == "disconnect" {
            self.connected = false;
            self.invalidate(Phase::Idle);
            self.log("WARN", "system", "Engine disconnected · safety conditions invalidated");
            return Ok(());
        }
        if !self.preview {
            return Err("PRODUCTION_LOCKED");
        }
        match command {
            "connect" => {
                if payload.get("adapter").and_then(Value::as_str) != Some("developer_preview") {
                    return Err("HARDWARE_NOT_IMPLEMENTED");
                }
                if self.connected || self.busy() {
                    return Err("ALREADY_CONNECTED");
                }
                self.invalidate(Phase::Idle);
                self.connected = true;
                self.xray_latched = false;
                self.log("INFO", "system", "DEVELOPER PREVIEW connected · NO REAL HARDWARE");
            }
            "set_parameters" => {
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
                self.parameters = parameters;
                self.max_xray_sec = max_xray_sec;
                self.current = 0;
                self.invalidate(Phase::Idle);
                self.log("INFO", "system", "Scan setup updated · repeat preflight and HOME");
            }
            "preflight" => {
                self.require_connected()?;
                if self.xray_latched || self.phase == Phase::Fault {
                    return Err("ESTOP_LATCHED");
                }
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                self.parameters.validate()?;
                if !(1..=359_999).contains(&self.max_xray_sec) {
                    return Err("INVALID_PARAMETERS");
                }
                self.preflight = true;
                self.homed = false;
                self.phase = Phase::ReadyForHome;
                self.log("PASS", "preflight", "8/8 preview checks passed · real interlocks unverified");
            }
            "home" => {
                self.require_connected()?;
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                if !self.preflight {
                    return Err("PREFLIGHT_REQUIRED");
                }
                if self.xray_latched {
                    return Err("ESTOP_LATCHED");
                }
                self.homed = true;
                self.current = 0;
                self.phase = Phase::Ready;
                self.log("INFO", "nano", "Preview HOME complete · 0.00°");
            }
            "start_scan" => {
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
                if self.xray_latched {
                    return Err("ESTOP_LATCHED");
                }
                self.current = 0;
                self.phase = Phase::Running;
                self.beam_on = true;
                self.last_tick = Instant::now();
                self.log("ACTION", "operator", "Developer preview scan started · no device output");
            }
            "pause" => {
                if self.phase != Phase::Running {
                    return Err("NOT_RUNNING");
                }
                self.phase = Phase::Paused;
                self.beam_on = false;
                self.log("WARN", "system", "Preview paused · output visualization disabled");
            }
            "resume" => {
                self.require_connected()?;
                if self.xray_latched || self.phase == Phase::Fault {
                    return Err("ESTOP_LATCHED");
                }
                if self.phase != Phase::Paused || !self.preflight || !self.homed {
                    return Err("NOT_RESUMABLE");
                }
                self.phase = Phase::Running;
                self.beam_on = true;
                self.last_tick = Instant::now();
                self.log("INFO", "system", "Preview resumed");
            }
            "restore_previous" => {
                self.require_connected()?;
                if self.xray_latched || self.phase == Phase::Fault {
                    return Err("ESTOP_LATCHED");
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
            "estop_release" => {
                if self.phase != Phase::Fault {
                    return Err("NOT_FAULTED");
                }
                self.xray_latched = false;
                self.invalidate(Phase::Stopped);
                self.log("WARN", "system", "E-STOP released · preflight and HOME are required");
            }
            "retry_device" => {
                self.require_connected()?;
                let device = payload.get("device").and_then(Value::as_str).ok_or("INVALID_DEVICE")?;
                if !matches!(device, "turntable" | "camera" | "xray") {
                    return Err("INVALID_DEVICE");
                }
                self.log("INFO", "system", &format!("{device} preview link check complete"));
            }
            "xray_toggle" => {
                self.require_connected()?;
                if self.phase == Phase::Fault || self.xray_latched {
                    return Err("ESTOP_LATCHED");
                }
                self.beam_on = !self.beam_on;
                self.log("WARN", "xray", if self.beam_on { "Preview beam visualization enabled · NO REAL HARDWARE" } else { "Preview beam visualization disabled" });
            }
            "timer_toggle" => {
                if self.phase == Phase::Running {
                    return Err("SCAN_ACTIVE");
                }
                self.timer_on = !self.timer_on;
                self.log("INFO", "xray", if self.timer_on { "Preview timer enabled" } else { "Preview timer disabled" });
            }
            "usb_auto_shut_down_toggle" => {
                self.usb_auto_shut_down = !self.usb_auto_shut_down;
                self.log("INFO", "xray", "Preview USB auto-shutdown setting updated");
            }
            "send_voltage" => {
                let kv = payload.get("kv").and_then(Value::as_f64).ok_or("INVALID_VOLTAGE")?;
                self.validate_setpoint(kv, self.set_ua)?;
                self.set_kv = kv;
                self.log("INFO", "xray", &format!("Preview voltage set to {kv:.1} kV"));
            }
            "send_current" => {
                let ua = payload.get("ua").and_then(Value::as_f64).ok_or("INVALID_CURRENT")?;
                self.validate_setpoint(self.set_kv, ua)?;
                self.set_ua = ua;
                self.log("INFO", "xray", &format!("Preview current set to {ua:.1} µA"));
            }
            _ => return Err("UNKNOWN_COMMAND"),
        }
        Ok(())
    }

    fn validate_setpoint(&self, kv: f64, ua: f64) -> Result<(), &'static str> {
        if !kv.is_finite() || !ua.is_finite() || !(20.0..=160.0).contains(&kv) || !(1.0..=500.0).contains(&ua) {
            return Err("INVALID_XRAY_SETPOINT");
        }
        if kv * ua / 1000.0 > MAX_POWER_W {
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
        if self.phase != Phase::Running {
            return;
        }
        let steps = (self.last_tick.elapsed().as_millis() / 850) as u32;
        if steps == 0 {
            return;
        }
        self.last_tick += Duration::from_millis(u64::from(steps) * 850);
        self.current = self.current.saturating_add(steps).min(self.parameters.projection_count);
        if self.current == self.parameters.projection_count {
            self.phase = Phase::Completed;
            self.beam_on = false;
            self.log("PASS", "system", "Developer preview completed · no projection files created");
        }
    }

    fn angle_deg(&self) -> f64 {
        if self.current == 0 { 0.0 } else { f64::from(self.current - 1) * self.parameters.angle_step_deg }
    }

    fn data_state(&self) -> &'static str {
        match self.phase {
            Phase::Running => "scanning",
            Phase::Paused => "paused",
            Phase::Fault => "fault",
            _ => "ready",
        }
    }

    fn phase_word(&self) -> &'static str {
        match self.phase {
            Phase::Running => "SCANNING",
            Phase::Paused => "PAUSED",
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
            && (1..=359_999).contains(&self.max_xray_sec);
        let is_preview = self.preview;
        let identity = if is_preview { "DEVELOPER PREVIEW · NO REAL HARDWARE" } else { "PRODUCTION LOCKED · NO HARDWARE ADAPTER" };
        let device_tone = if self.phase == Phase::Fault { "danger" } else if self.connected { "accent" } else { "muted" };
        let xray_text = if self.phase == Phase::Fault || self.xray_latched { "LATCHED OFF" } else if self.beam_on { "PREVIEW BEAM" } else { "OUTPUT UNAVAILABLE" };
        let safety_text = if self.phase == Phase::Fault {
            "SAFETY · FAULT · OUTPUT LATCHED OFF"
        } else if self.beam_on {
            "SAFETY · PREVIEW BEAM · NO REAL HARDWARE"
        } else {
            "SAFETY · REAL OUTPUT UNAVAILABLE"
        };
        let eta = if self.phase == Phase::Paused {
            "held".to_owned()
        } else if self.phase == Phase::Running {
            let seconds = (total.saturating_sub(captured) as f64 * 0.85).ceil() as u64;
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        } else {
            "—".to_owned()
        };
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
        let frames: Vec<Value> = (1..=captured).map(|index| json!({
            "index": index,
            "angleDeg": f64::from(index - 1) * self.parameters.angle_step_deg,
            "exposureMs": self.parameters.exposure_ms,
            "fileName": format!("preview-{index:04}.frame"),
        })).collect();
        json!({
            "dataState": data_state,
            "phaseWord": self.phase_word(),
            "phaseTone": if self.phase == Phase::Fault { "danger" } else if self.phase == Phase::Running { "warn" } else { "accent" },
            "devices": [
                {"id":"xray","name":"X-Ray Source","word": if self.xray_latched {"LOCKED"} else {"PREVIEW"},"tone":device_tone,"spec":format!("12 W · {identity}")},
                {"id":"turntable","name":"Turntable-Nano","word":if self.phase == Phase::Running {"MOVING"} else {"PREVIEW"},"tone":device_tone,"spec":format!("POS {angle:.2}° · 60:1 · 8 µSTEP · PREVIEW")},
                {"id":"camera","name":"Camera","word":"PREVIEW","tone":device_tone,"spec":"D7100 · NO REAL CAMERA CONNECTION"}
            ],
            "onlineSummary": if self.preview { "PREVIEW · 0 REAL DEVICES" } else { "LOCKED · 0 REAL DEVICES" },
            "preflight": {
                "word": if self.preflight {"PASSED"} else {"WAITING"},
                "percent": if self.preflight {100} else {0},
                "tone": if self.preflight {"pass"} else {"warn"},
                "subline": if self.preflight {"8/8 preview checks · real interlocks unverified"} else {"0/8 checks · no real hardware"}
            },
            "floats": [
                {"key":"X-RAY","text":xray_text,"tone":if self.phase == Phase::Fault || self.beam_on {"danger"} else {"muted"}},
                {"key":"CAMERA","text":"PREVIEW ONLY","tone":"muted"},
                {"key":"SAMPLE","text":format!("{angle:.2}°"),"tone":if self.phase == Phase::Fault {"danger"} else {"accent"}}
            ],
            "safetyBar":{"text":safety_text,"tone":if self.phase == Phase::Fault {"dangerBold"} else if self.beam_on {"danger"} else {"muted"}},
            "scene":{"angleDeg":angle,"rotated":angle.abs() > 0.005},
            "xray":{
                "setKv":self.set_kv,"setUa":self.set_ua,"monKv":self.set_kv,"monUa":self.set_ua,
                "powerW":self.set_kv * self.set_ua / 1000.0,"tempC":24.0,"beamOn":self.beam_on && self.preview,
                "latched":self.xray_latched || !self.preview,"onSec":10,"offSec":20,"timerOn":self.timer_on,
                "usbAutoShutDown":self.usb_auto_shut_down
            },
            "progress":{
                "captured":captured,"total":total,"percent":percent,"angleDeg":angle,"etaText":eta,
                "barTone":if self.phase == Phase::Fault {"danger"} else if self.phase == Phase::Paused {"warn"} else {"accent"},
                "barLabel":format!("{captured} / {total} · {percent:.0}%")
            },
            "summary":{
                "savePath":self.parameters.save_path,
                "acquisition":if configured {format!("{total} views · {:.2}° · {} ms", self.parameters.angle_step_deg, self.parameters.exposure_ms)} else {"Scan setup not configured".to_owned()},
                "output":if self.preview {format!("PREVIEW SET · {:.1} kV · {:.1} µA",self.set_kv,self.set_ua)} else {"LOCKED · NO REAL OUTPUT".to_owned()}
            },
            "statusbar":{
                "left":if configured {format!("{identity} · {} · {captured} / {total} · {angle:.2}°",self.phase_word())} else {format!("{identity} · SETUP REQUIRED")},
                "right":if self.preview {"ct-engine · JSONL v1 · PREVIEW"} else {"ct-engine · JSONL v1 · LOCKED"},
                "dotTone":if self.phase == Phase::Fault {"danger"} else if self.phase == Phase::Running {"warn"} else if self.phase == Phase::Paused {"accent"} else {"muted"}
            },
            "dock":{
                "home":self.connected && self.preflight && !self.busy() && !self.xray_latched,
                "play":self.connected && !self.xray_latched && ((self.phase == Phase::Running || self.phase == Phase::Paused) || (self.preflight && self.homed && self.phase != Phase::Fault)),
                "restore":self.connected && !self.busy() && self.preflight && self.homed && !self.xray_latched && !matches!(self.phase, Phase::Fault | Phase::Stopped),
                "estop":true,
                "playMode":if self.phase == Phase::Running {"pause"} else if self.phase == Phase::Paused {"resume"} else if self.phase == Phase::Fault {"disabled"} else {"start"}
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
        let mode_label = if self.preview { "DEVELOPER PREVIEW · NO REAL HARDWARE" } else { "PRODUCTION LOCKED · NO HARDWARE ADAPTER" };
        let connection_state = if self.connected { "connected" } else { "disconnected" };
        let device_state = if !self.preview { "locked" } else if !self.connected { "offline" } else if self.phase == Phase::Running { "busy" } else { "connected" };
        json!({
            "mode":mode,"modeLabel":mode_label,"connectionState":connection_state,"adapterLabel":"ct-engine · JSONL stdio",
            "phase":phase,"phaseLabel":label,"preflightPassed":self.preflight,"homed":self.homed,
            "requiresPreflight":!self.preflight,"requiresHome":!self.homed,
            "safety":{
                "xrayAvailable":false,"xrayEnabled":false,"interlockOk":false,
                "lockReason":if self.preview {"Developer preview has no real X-ray hardware"} else {"Production is locked until real adapters are implemented"}
            },
            "devices":[
                {"id":"turntable","label":"Precision Turntable","state":device_state,"detail":mode_label},
                {"id":"camera","label":"Nikon D7100","state":device_state,"detail":mode_label},
                {"id":"xray","label":"Moxtek 12 W","state":"locked","detail":"Fail-closed · no real output"}
            ],
            "parameters":self.parameters,
            "progress":{
                "current":self.current,"total":self.parameters.projection_count,
                "percent":if self.parameters.projection_count == 0 {0.0} else {(100.0 * f64::from(self.current) / f64::from(self.parameters.projection_count)).round()},
                "angleDeg":self.angle_deg(),
                "etaSeconds":if self.phase == Phase::Running {Some(((self.parameters.projection_count - self.current) as f64 * 0.85).ceil() as u64)} else {None}
            },
            "imageCount":0,"logs":self.logs,"lastError":Value::Null,"updatedAt":timestamp(),
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
        assert!(send(&mut engine, "stop", json!({})).error_code.is_none());
        assert!(send(&mut engine, "disconnect", json!({})).error_code.is_none());
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
        assert_eq!(engine.max_xray_sec, 0);
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
    fn stop_and_estop_release_require_full_recovery() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        send(&mut engine, "start_scan", json!({}));
        send(&mut engine, "stop", json!({}));
        assert_eq!(engine.phase, Phase::Fault);
        assert!(!engine.beam_on && !engine.preflight && !engine.homed);
        assert_eq!(engine.snapshot()["workstation"]["dock"]["restore"], false);
        assert_eq!(
            send(&mut engine, "preflight", json!({}))
                .error_code
                .as_deref(),
            Some("ESTOP_LATCHED")
        );
        assert_eq!(
            send(&mut engine, "restore_previous", json!({}))
                .error_code
                .as_deref(),
            Some("ESTOP_LATCHED")
        );
        assert_eq!(
            send(&mut engine, "resume", json!({})).error_code.as_deref(),
            Some("ESTOP_LATCHED")
        );
        assert_eq!(engine.phase, Phase::Fault);
        assert!(!engine.beam_on && !engine.preflight && !engine.homed);

        assert!(send(&mut engine, "estop_release", json!({}))
            .error_code
            .is_none());
        assert_eq!(engine.phase, Phase::Stopped);
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
    }

    #[test]
    fn preview_commands_are_implemented() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        for (command, extra) in [
            ("retry_device", json!({"device":"camera"})),
            ("timer_toggle", json!({})),
            ("usb_auto_shut_down_toggle", json!({})),
            ("send_voltage", json!({"kv":70.0})),
            ("send_current", json!({"ua":120.0})),
            ("xray_toggle", json!({})),
        ] {
            assert!(send(&mut engine, command, extra).error_code.is_none(), "{command}");
        }
    }

    #[test]
    fn xray_power_limit_is_transactional() {
        let mut engine = Engine::new(true);
        let before = engine.set_ua;
        assert_eq!(send(&mut engine, "send_current", json!({"ua":500.0})).error_code.as_deref(), Some("XRAY_POWER_LIMIT"));
        assert_eq!(engine.set_ua, before);
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
        assert_eq!(engine.parameters.exposure_ms, 200);
        assert_eq!(engine.max_xray_sec, 60);
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
        assert_eq!(engine.phase, Phase::Completed);
        assert_eq!(engine.snapshot()["imageCount"], 0);
        assert_eq!(engine.snapshot()["workstation"]["frames"].as_array().unwrap().len(), 5);
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
