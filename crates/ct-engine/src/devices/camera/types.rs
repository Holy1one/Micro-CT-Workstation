//! Serializable camera evidence. Native property codes are values, never UI labels.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const EXPOSURE_MIN_MS: f64 = 0.125;
pub const EXPOSURE_MAX_MS: f64 = 30_000.0;
pub fn valid_exposure_ms(value: f64) -> bool {
    value.is_finite() && (EXPOSURE_MIN_MS..=EXPOSURE_MAX_MS).contains(&value)
}

/// Shared admission for domain payloads and the camera owner's public proxy.
pub fn validate_focus_step(value: i64) -> Result<i32, CameraError> {
    let step = i32::try_from(value).map_err(|_| CameraError::Safety("focus step does not fit i32".into()))?;
    if step == 0 || !(-500..=500).contains(&step) {
        return Err(CameraError::Safety("focus step must be a nonzero signed native step within +/-500".into()));
    }
    Ok(step)
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

/// Read-only GetDeviceInfo evidence. The reported namespace is preserved rather
/// than overwritten by a selected model adapter. This contains no image data.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraDeviceInfo {
    pub standard_version: u16,
    pub vendor_extension_id: u32,
    pub vendor_extension_version: u16,
    pub vendor_extension_description: String,
    pub functional_mode: u16,
    pub device_version: String,
    pub operations: Vec<u16>,
    pub events: Vec<u16>,
    pub properties: Vec<u16>,
    pub capture_formats: Vec<u16>,
    pub image_formats: Vec<u16>,
    pub dataset_size: usize,
    pub dataset_sha256: String,
    pub adapter_selection: String,
    #[serde(default)]
    pub effective_properties: Vec<u16>,
    #[serde(default)]
    pub vendor_property_codes: Option<CameraVendorPropertyCodes>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraVendorPropertyCodes {
    pub operation_code: u16,
    pub response_code: u16,
    pub properties: Vec<u16>,
    pub dataset_size: usize,
    pub dataset_sha256: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraEventDiagnostic {
    pub operation_code: u16,
    pub response_code: Option<u16>,
    pub dataset_size: usize,
    pub dataset_sha256: String,
    pub data_prefix_hex: String,
    pub events: Vec<CameraEventRecord>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraEventRecord {
    pub code: u16,
    pub parameter: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraEventBatchDiagnostic {
    pub sequence: u64,
    pub event_count: usize,
    #[serde(flatten)]
    pub response: CameraEventDiagnostic,
}
impl CameraEventBatchDiagnostic {
    pub(crate) fn record(
        batches: &mut Vec<Self>,
        sequence: u64,
        event_count: usize,
        response: CameraEventDiagnostic,
    ) {
        if batches.len() == 16 {
            batches.remove(0);
        }
        batches.push(Self {
            sequence,
            event_count,
            response,
        });
    }
}

/// Last nonempty batch consumed by this capture, retained independently from
/// subsequent idle/cancel/close polling. It is not evidence of a committed file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraCaptureEventDiagnostic {
    pub session_id: String,
    pub operation_id: String,
    pub batch_sequence: u64,
    pub event_count: usize,
    pub capture_complete_count: usize,
    pub exposure_ended_observed: bool,
    #[serde(default)]
    pub event_batches: Vec<CameraEventBatchDiagnostic>,
    #[serde(flatten)]
    pub response: CameraEventDiagnostic,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraCleanupDiagnostic {
    pub session_id: Option<String>,
    pub operation_id: Option<String>,
    pub pending_at_start: bool,
    pub exposure_ended_observed: bool,
    pub poll_count: u64,
    pub wait_limit_ms: u64,
    pub elapsed_ms: u64,
    pub io_cancel_confirmed: bool,
    pub close_confirmed: bool,
    pub error: Option<String>,
    pub last_event_diagnostic: Option<CameraEventDiagnostic>,
    #[serde(default)]
    pub event_batches: Vec<CameraEventBatchDiagnostic>,
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
    #[serde(default)]
    pub supported: bool,
    /// Raw PTP GetSet evidence, retained separately from contextual write gates.
    #[serde(default)]
    pub native_writable: bool,
    #[serde(default)]
    pub effective: bool,
    #[serde(default)]
    pub disabled_reason: Option<String>,
    #[serde(default)]
    pub options: Vec<CameraPropertyOption>,
    pub supported_values: Vec<CameraValue>,
    pub range: Option<[i64; 3]>,
    pub actual: Option<CameraValue>,
    pub target: Option<CameraValue>,
    pub confirmed: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraPropertyOption {
    pub value: CameraValue,
    pub label: String,
    pub supported: bool,
    pub enabled: bool,
    pub reason: Option<String>,
}

/// Raw device diagnostics have no proven clear/fault meaning. Confirmation
/// applies to transport readback, not to physical or radiation safety.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraRuntimeSemanticStatus {
    #[default]
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraRuntimeProperty {
    #[serde(flatten)]
    pub property: CameraProperty,
    #[serde(default)]
    pub semantic_status: CameraRuntimeSemanticStatus,
}
impl From<CameraProperty> for CameraRuntimeProperty {
    fn from(property: CameraProperty) -> Self {
        Self {
            property,
            semantic_status: CameraRuntimeSemanticStatus::Unknown,
        }
    }
}
impl std::ops::Deref for CameraRuntimeProperty {
    type Target = CameraProperty;
    fn deref(&self) -> &Self::Target {
        &self.property
    }
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
/// Ephemeral owner-described LV controls, separate from acquisition/presets.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct CameraLiveViewControls {
    pub session_id:Option<String>,
    pub properties:BTreeMap<String,CameraProperty>,
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

/// Physical feature applicability is distinct from PTP observability. No
/// native D7100 multiple-exposure mode property has verified semantics here.
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct MultipleExposureEvidence {
    pub physical_applicable:bool,pub observable:bool,pub status:String,pub source:String,pub reason:Option<String>,
}
impl Default for MultipleExposureEvidence {
    fn default()->Self{Self{physical_applicable:false,observable:false,status:"unknown".into(),source:"none".into(),reason:Some("No current physical acquisition evidence".into())}}
}
impl MultipleExposureEvidence {
    pub(crate) fn d7100_unknown()->Self{Self{physical_applicable:true,observable:false,status:"unknown".into(),source:"none".into(),reason:Some("D7100 multiple exposure is physically supported but its current OFF/inactive state is not observable through the verified native property catalog; one NEF/C102 does not prove one physical exposure".into())}}
    pub fn proves_inactive(&self)->bool{
        if !self.physical_applicable||!self.observable||self.status!="off"{return false;}
        #[cfg(test)]
        if self.source=="SCRIPTED TEST ONLY"{return true;}
        // Neither a native property nor a transport exclusion is implemented.
        // Future proof sources require explicit validation, never a caller label.
        false
    }
    #[cfg(test)]
    pub(crate) fn scripted_off()->Self{Self{physical_applicable:true,observable:true,status:"off".into(),source:"SCRIPTED TEST ONLY".into(),reason:None}}
}

/// An operator attestation that the camera body's multiple-exposure control is
/// OFF for one identified session. It is never machine evidence: the native
/// model state stays UNKNOWN/unobservable. The record binds the attested device
/// identity, the exact camera session and a fingerprint of the confirmed
/// parameter readback, so a reconnect, restart, identity change or parameter
/// change invalidates it and the operator must attest again.
pub const OPERATOR_MULTIPLE_EXPOSURE_POLICY_VERSION: u32 = 1;
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct OperatorMultipleExposureConfirmation {
    pub confirmed:bool,
    pub confirmed_at:String,
    pub operator:String,
    pub device_id:String,
    pub session_id:String,
    pub evidence_fingerprint:String,
    pub policy_version:u32,
}
impl OperatorMultipleExposureConfirmation {
    /// Self-binding only: the live session/parameter match is decided by
    /// `crate::capture_profile::operator_multiple_exposure_is_valid` against a
    /// freshly read `CameraHealth`. A stored record is never trusted alone.
    pub fn binds_to_identity(&self, identity:&CameraIdentity)->bool {
        self.confirmed
            && self.policy_version==OPERATOR_MULTIPLE_EXPOSURE_POLICY_VERSION
            && !self.device_id.is_empty()
            && self.device_id==identity.device_id
            && self.session_id.len()==32
            && self.evidence_fingerprint.len()==64
            && self.confirmed_at.contains('T')
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
    #[serde(default)]
    pub device_info: Option<CameraDeviceInfo>,
    #[serde(default)]
    pub last_event_diagnostic: Option<CameraEventDiagnostic>,
    /// Original batch requesting capability refresh, retained across empty idle polls.
    #[serde(default)]
    pub last_device_info_changed_diagnostic: Option<CameraEventDiagnostic>,
    #[serde(default)]
    pub last_capture_event_diagnostic: Option<CameraCaptureEventDiagnostic>,
    #[serde(default)]
    pub last_cleanup_diagnostic: Option<CameraCleanupDiagnostic>,
    pub discovered: Vec<CameraIdentity>,
    pub session_id: Option<String>,
    pub capabilities: CameraCapabilities,
    pub last_seen_at: Option<String>,
    pub properties: BTreeMap<String, CameraProperty>,
    /// Device runtime status is not a frozen acquisition parameter.
    #[serde(default)]
    pub runtime_properties: BTreeMap<String, CameraRuntimeProperty>,
    #[serde(default)]
    pub multiple_exposure_evidence:MultipleExposureEvidence,
    /// Stored operator attestation, raw. Consumers must re-check it against a
    /// fresh session/fingerprint before treating it as current evidence.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub operator_multiple_exposure:Option<OperatorMultipleExposureConfirmation>,
    pub live_view: LiveViewStatus,
    #[serde(default)]
    pub live_view_controls:CameraLiveViewControls,
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
/// Associated RAM completion does not establish the number of physical exposures.
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
    /// Transient device-level busy (WPD `ERROR_BUSY` 0x800700AA / PTP device busy):
    /// the device is still finishing a previous operation. Distinct from
    /// [`CameraError::Busy`], which describes a business state such as "already
    /// connected" or a full queue and must never be retried.
    DeviceBusy(String),
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
            Self::DeviceBusy(s) => write!(f, "camera device busy: {s}"),
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
