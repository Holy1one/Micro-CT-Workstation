//! The sole desktop owner of scan state. V1 has no hardware transports.
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u32 = 1;
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
            task_id: "CT-2026-001".into(),
            save_path: "MicroCT/runs/CT-2026-001".into(),
            projection_count: 120,
            angle_step_deg: 3.0,
            exposure_ms: 180,
        }
    }
}

impl Parameters {
    fn validate(&self) -> Result<(), &'static str> {
        if self.task_id.trim().is_empty() || self.save_path.trim().is_empty() {
            return Err("INVALID_PARAMETERS");
        }
        if !(4..=3600).contains(&self.projection_count)
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    ReadyForHome,
    Ready,
    Running,
    Paused,
    Stopped,
    Completed,
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
    current: u32,
    sequence: u64,
    request_sequence: u64,
    last_tick: Instant,
    logs: Vec<Value>,
}

impl Engine {
    pub fn new(preview: bool) -> Self {
        let mut engine = Self {
            preview,
            connected: false,
            preflight: false,
            homed: false,
            phase: Phase::Idle,
            parameters: Parameters::default(),
            current: 0,
            sequence: 0,
            request_sequence: 0,
            last_tick: Instant::now(),
            logs: vec![],
        };
        engine.log("info", "系统", "ct-engine started; waiting for an explicit connection");
        engine.log("warning", "射线", "V1 X-ray output is interlocked; no real device driver is loaded");
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
            self.command(&request.command, request.payload).err()
        };
        self.sequence += 1;
        if let Some(code) = error {
            self.log("warning", "系统", &format!("Command rejected: {code}"));
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

    fn busy(&self) -> bool {
        matches!(self.phase, Phase::Running | Phase::Paused)
    }

    fn invalidate(&mut self) {
        self.preflight = false;
        self.homed = false;
        self.phase = Phase::Stopped;
    }

    fn command(&mut self, command: &str, payload: Value) -> Result<(), &'static str> {
        if command == "snapshot" {
            return Ok(());
        }
        if command == "stop" || command == "disconnect" {
            self.invalidate();
            if command == "disconnect" {
                self.connected = false;
            }
            self.log("warning", "系统", "Workflow stopped; preflight and home must be repeated");
            return Ok(());
        }
        if !self.preview {
            return Err("PRODUCTION_LOCKED");
        }
        match command {
            "connect" => {
                if payload["adapter"] != "developer_preview" {
                    return Err("HARDWARE_NOT_IMPLEMENTED");
                }
                if self.connected || self.busy() {
                    return Err("ALREADY_CONNECTED");
                }
                self.connected = true;
                self.preflight = false;
                self.homed = false;
                self.phase = Phase::Idle;
                self.log("success", "系统", "Developer preview connected; no serial, camera or X-ray access");
            }
            "set_parameters" => {
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                let parameters: Parameters = serde_json::from_value(payload["parameters"].clone())
                    .map_err(|_| "INVALID_PARAMETERS")?;
                parameters.validate()?;
                self.parameters = parameters;
                self.current = 0;
                self.preflight = false;
                self.homed = false;
                self.phase = Phase::Idle;
                self.log("info", "系统", "Scan parameters applied; repeat preflight and home");
            }
            "preflight" => {
                self.require_connected()?;
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                self.parameters.validate()?;
                self.preflight = true;
                self.homed = false;
                self.phase = Phase::ReadyForHome;
                self.log("success", "系统", "Preview preflight completed; real interlocks remain unverified");
            }
            "home" => {
                self.require_connected()?;
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                if !self.preflight {
                    return Err("PREFLIGHT_REQUIRED");
                }
                self.homed = true;
                self.current = 0;
                self.phase = Phase::Ready;
                self.log("success", "转台", "Preview homing complete · 0.00 degrees");
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
                self.current = 0;
                self.phase = Phase::Running;
                self.last_tick = Instant::now();
                self.log("info", "系统", "No-output workflow preview started; no projection file will be created");
            }
            "pause" => {
                if self.phase != Phase::Running {
                    return Err("NOT_RUNNING");
                }
                self.phase = Phase::Paused;
                self.log("warning", "系统", "Preview paused");
            }
            "resume" => {
                self.require_connected()?;
                if self.phase != Phase::Paused || !self.preflight || !self.homed {
                    return Err("NOT_RESUMABLE");
                }
                self.phase = Phase::Running;
                self.last_tick = Instant::now();
                self.log("info", "系统", "Preview resumed");
            }
            "restore_previous" => {
                self.require_connected()?;
                if self.busy() {
                    return Err("SCAN_ACTIVE");
                }
                self.preflight = true;
                self.homed = true;
                self.current = (self.parameters.projection_count / 3).max(1);
                self.phase = Phase::Paused;
                self.log("warning", "系统", "Developer preview progress restored; no real task file was read");
            }
            _ => return Err("UNKNOWN_COMMAND"),
        }
        Ok(())
    }

    fn require_connected(&self) -> Result<(), &'static str> {
        if self.connected {
            Ok(())
        } else {
            Err("NOT_CONNECTED")
        }
    }

    fn log(&mut self, level: &str, source: &str, message: &str) {
        self.logs.insert(0, json!({"id": format!("{}-{}", self.sequence, timestamp()), "timestamp": timestamp(), "level": level, "source": source, "message": message}));
        self.logs.truncate(100);
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
        self.current = self
            .current
            .saturating_add(steps)
            .min(self.parameters.projection_count);
        if self.current == self.parameters.projection_count {
            self.phase = Phase::Completed;
            self.log("success", "系统", "Developer preview completed; real projection file count is zero");
        }
    }

    pub fn snapshot(&self) -> Value {
        let (phase, label) = self.phase.value();
        let state = if !self.connected {
            "offline"
        } else if self.phase == Phase::Running {
            "busy"
        } else if self.homed {
            "ready"
        } else {
            "connected"
        };
        let detail = if self.connected {
            "Developer preview · no real hardware"
        } else {
            "Waiting for connection"
        };
        json!({
            "mode": if self.preview { "developer_preview" } else { "production_locked" },
            "modeLabel": if self.preview { "Developer Preview / No Real Hardware" } else { "Production / V1 Devices Locked" },
            "connectionState": if self.connected { "connected" } else { "disconnected" },
            "adapterLabel": "ct-engine · JSONL stdio",
            "phase": phase, "phaseLabel": label,
            "preflightPassed": self.preflight, "homed": self.homed,
            "requiresPreflight": !self.preflight, "requiresHome": !self.homed,
            "safety": {"xrayAvailable": false, "xrayEnabled": false, "interlockOk": false, "lockReason": "The V1 X-ray channel is not implemented; real hardware remains locked"},
            "devices": [
                {"id": "turntable", "label": "Precision Turntable", "state": state, "detail": detail},
                {"id": "camera", "label": "Nikon D7100", "state": state, "detail": detail},
                {"id": "xray", "label": "Moxtek 12 W", "state": "locked", "detail": "V1 interlocked · no output"}
            ],
            "parameters": self.parameters,
            "progress": {"current": self.current, "total": self.parameters.projection_count,
                "percent": (100.0 * f64::from(self.current) / f64::from(self.parameters.projection_count)).round(),
                "angleDeg": if self.current == 0 {0.0} else {f64::from(self.current - 1) * self.parameters.angle_step_deg},
                "etaSeconds": if self.busy() {Some(((self.parameters.projection_count - self.current) as f64 * 0.85).ceil() as u64)} else {None}},
            "imageCount": 0, "previewFrameCount": self.current,
            "logs": self.logs, "lastError": null, "updatedAt": timestamp()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(engine: &mut Engine, command: &str, payload: Value) -> Response {
        engine.handle(Request {
            protocol_version: 1,
            request_id: format!("test-{}", engine.request_sequence + 1),
            command: command.into(),
            payload,
            timestamp: timestamp(),
            sequence: engine.request_sequence + 1,
            error_code: None,
        })
    }
    fn ready(engine: &mut Engine) {
        assert!(
            send(engine, "connect", json!({"adapter":"developer_preview"}))
                .error_code
                .is_none()
        );
        assert!(send(engine, "preflight", json!({})).error_code.is_none());
        assert!(send(engine, "home", json!({})).error_code.is_none());
    }
    #[test]
    fn production_has_no_preview_fallback() {
        let mut engine = Engine::new(false);
        assert_eq!(
            send(
                &mut engine,
                "connect",
                json!({"adapter":"developer_preview"})
            )
            .error_code
            .as_deref(),
            Some("PRODUCTION_LOCKED")
        );
        assert_eq!(engine.snapshot()["safety"]["xrayEnabled"], false);
        assert!(send(&mut engine, "stop", json!({})).error_code.is_none());
    }
    #[test]
    fn start_requires_preflight_and_home() {
        let mut engine = Engine::new(true);
        send(
            &mut engine,
            "connect",
            json!({"adapter":"developer_preview"}),
        );
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("PREFLIGHT_REQUIRED")
        );
        send(&mut engine, "preflight", json!({}));
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("HOME_REQUIRED")
        );
    }
    #[test]
    fn active_scan_cannot_be_replaced_and_stop_invalidates() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        send(&mut engine, "start_scan", json!({}));
        assert_eq!(
            send(&mut engine, "preflight", json!({}))
                .error_code
                .as_deref(),
            Some("SCAN_ACTIVE")
        );
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("SCAN_ACTIVE")
        );
        send(&mut engine, "pause", json!({}));
        assert!(send(&mut engine, "resume", json!({})).error_code.is_none());
        send(&mut engine, "stop", json!({}));
        assert!(!engine.preflight && !engine.homed);
        assert_eq!(
            send(&mut engine, "start_scan", json!({}))
                .error_code
                .as_deref(),
            Some("PREFLIGHT_REQUIRED")
        );
    }
    #[test]
    fn disconnect_invalidates_and_never_generates_images() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        send(&mut engine, "start_scan", json!({}));
        engine.last_tick -= Duration::from_secs(1);
        engine.tick();
        assert_eq!(engine.current, 1);
        assert_eq!(engine.snapshot()["imageCount"], 0);
        send(&mut engine, "disconnect", json!({}));
        assert!(!engine.connected && !engine.preflight && !engine.homed);
    }
    #[test]
    fn preview_restore_loads_a_paused_checkpoint() {
        let mut engine = Engine::new(true);
        send(
            &mut engine,
            "connect",
            json!({"adapter":"developer_preview"}),
        );
        assert!(send(&mut engine, "restore_previous", json!({}))
            .error_code
            .is_none());
        assert_eq!(engine.phase, Phase::Paused);
        assert!(engine.preflight && engine.homed && engine.current > 0);
    }
    #[test]
    fn preview_can_complete_without_hardware() {
        let mut engine = Engine::new(true);
        ready(&mut engine);
        send(&mut engine, "start_scan", json!({}));
        engine.last_tick -= Duration::from_secs(103);
        engine.tick();
        assert_eq!(engine.phase, Phase::Completed);
        assert_eq!(engine.current, 120);
        assert_eq!(engine.snapshot()["safety"]["interlockOk"], false);
    }
    #[test]
    fn invalid_parameters_are_transactional() {
        let mut engine = Engine::new(true);
        let before = engine.parameters.clone();
        let mut bad = before.clone();
        bad.angle_step_deg = 4.0;
        assert_eq!(
            send(&mut engine, "set_parameters", json!({"parameters": bad}))
                .error_code
                .as_deref(),
            Some("INVALID_PARAMETERS")
        );
        assert_eq!(engine.parameters.angle_step_deg, before.angle_step_deg);
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
        assert_eq!(
            engine.handle(request()).error_code.as_deref(),
            Some("STALE_REQUEST")
        );
        let mut incompatible = request();
        incompatible.protocol_version = 2;
        assert_eq!(
            engine.handle(incompatible).error_code.as_deref(),
            Some("PROTOCOL_VERSION_MISMATCH")
        );
    }
}
