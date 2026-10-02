//! Serializable camera evidence. Native property codes are values, never UI labels.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const EXPOSURE_MIN_MS: f64 = 0.125;
pub const EXPOSURE_MAX_MS: f64 = 30_000.0;
pub fn valid_exposure_ms(value: f64) -> bool {
    value.is_finite() && (EXPOSURE_MIN_MS..=EXPOSURE_MAX_MS).contains(&value)
}

/// Untagged numeric/string value encoded using the property's described PTP type.
/// JSON numbers must be integers; exposure uses the exact native rational code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CameraValue {
    Integer(i64),
    Text(String),
}
impl CameraValue {
    pub fn integer(&self) -> Option<i64> {
        if let Self::Integer(n) = self {
            Some(*n)
        } else {
            None
        }
    }
}
pub type CameraProperties = BTreeMap<String, CameraValue>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraIdentity {
    pub device_id: String,
    pub manufacturer: String,
    pub model: String,
    pub serial: Option<String>,
    pub backend: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraProperty {
    pub key: String,
    pub label: String,
    pub code: u16,
    pub data_type: u16,
    pub readable: bool,
    pub writable: bool,
    pub supported_values: Vec<CameraValue>,
    pub range: Option<[i64; 3]>,
    pub actual: Option<CameraValue>,
    pub target: Option<CameraValue>,
    pub confirmed: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraCapabilities {
    pub properties: bool,
    pub host_capture: bool,
    pub live_view: bool,
    pub autofocus: bool,
    pub focus_step: bool,
    pub scan_nef: bool,
    pub hardware_certified: bool,
    pub disabled_reasons: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveViewStatus {
    pub active: bool,
    pub session_id: Option<String>,
    pub sequence: u64,
    pub last_frame_at: Option<String>,
    pub fresh: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps_limit: u8,
    pub error: Option<String>,
}
impl Default for LiveViewStatus {
    fn default() -> Self {
        Self {
            active: false,
            session_id: None,
            sequence: 0,
            last_frame_at: None,
            fresh: false,
            width: None,
            height: None,
            fps_limit: 10,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraHealth {
    pub connected: bool,
    pub backend: String,
    pub serial: Option<String>,
    pub transfer_policy: Option<String>,
    pub last_capture: Option<String>,
    pub last_error: Option<String>,
    pub exposure_min_ms: Option<f64>,
    pub exposure_max_ms: Option<f64>,
    pub identity: Option<CameraIdentity>,
    pub discovered: Vec<CameraIdentity>,
    pub session_id: Option<String>,
    pub capabilities: CameraCapabilities,
    pub last_seen_at: Option<String>,
    pub properties: BTreeMap<String, CameraProperty>,
    pub live_view: LiveViewStatus,
    pub isolated: bool,
    pub cleanup_confirmed: bool,
    pub busy: bool,
    pub operation_id: Option<String>,
    pub phase: String,
    pub focus_position: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyApplyResult {
    pub complete: bool,
    pub applied: Vec<String>,
    pub failed: BTreeMap<String, String>,
    pub properties: BTreeMap<String, CameraProperty>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedImage {
    pub path: PathBuf,
    pub operation_id: String,
    pub session_id: String,
    pub object_handle: u32,
    pub size: u64,
    pub format: String,
    pub sha256: String,
    pub exposure_ended: bool,
}

/// ExposureEnded comes only from the Nikon SDRAM capture-complete event in the
/// single in-flight, freshly drained session. FileComplete is separate evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CameraCaptureEvent {
    ExposureEnded {
        session_id: String,
        operation_id: String,
    },
    ObjectAdded {
        session_id: String,
        operation_id: String,
        handle: u32,
    },
    TransferStarted {
        session_id: String,
        operation_id: String,
        size: u64,
    },
    FileComplete {
        image: CapturedImage,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CameraError {
    Cancelled,
    Unavailable(String),
    Command(String),
    Safety(String),
    Io(String),
    Timeout(String),
    Busy(String),
    Unsupported(String),
    Isolated(String),
}
impl std::fmt::Display for CameraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "camera operation cancelled; cleanup evidence required"),
            Self::Unavailable(s) => write!(f, "camera unavailable: {s}"),
            Self::Command(s) => write!(f, "camera command failed: {s}"),
            Self::Safety(s) => write!(f, "camera safety check failed: {s}"),
            Self::Io(s) => write!(f, "camera I/O failed: {s}"),
            Self::Timeout(s) => write!(f, "camera timeout: {s}"),
            Self::Busy(s) => write!(f, "camera busy: {s}"),
            Self::Unsupported(s) => write!(f, "camera capability unavailable: {s}"),
            Self::Isolated(s) => write!(f, "camera session isolated: {s}"),
        }
    }
}
impl std::error::Error for CameraError {}
impl From<std::io::Error> for CameraError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
