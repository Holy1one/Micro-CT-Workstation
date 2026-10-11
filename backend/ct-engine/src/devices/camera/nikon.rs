//! Necessary Nikon D7100 protocol adapted from digiCamControl's MIT Nikon
//! classes, Copyright (C) 2014 Duka Istvan. See THIRD-PARTY-NOTICES.md.
//! WPD transport is implemented independently against Microsoft's interface.

use super::properties::*;
use super::ptp::*;
use super::types::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::thread;
use std::time::Duration;
use std::time::Instant;

pub(crate) const CAPTURE_RAM: u16 = 0x90c0;
/// Standard PTP card-capture operations. These are the only card path opcodes this
/// build sends; no Nikon private capture code is used for card capture.
pub(crate) const INITIATE_CAPTURE: u16 = 0x100e;
pub(crate) const GET_STORAGE_IDS: u16 = 0x1004;
pub(crate) const GET_STORAGE_INFO: u16 = 0x1005;
pub(crate) const GET_OBJECT_HANDLES: u16 = 0x1007;
/// Standard PTP events used to associate one card exposure with one object.
pub(crate) const OBJECT_ADDED: u16 = 0x4002;
pub(crate) const PROPERTY_CHANGED: u16 = 0x4006;
pub(crate) const STORE_REMOVED: u16 = 0x4005;
pub(crate) const STORE_FULL: u16 = 0x400a;
pub(crate) const CAPTURE_COMPLETE_CARD: u16 = 0x400d;
pub(crate) const AF_DRIVE: u16 = 0x90c1;
pub(crate) const GET_EVENT: u16 = 0x90c7;
pub(crate) const DEVICE_READY: u16 = 0x90c8;
pub(crate) const GET_VENDOR_PROPERTY_CODES: u16 = 0x90ca;
pub(crate) const START_LV: u16 = 0x9201;
pub(crate) const STOP_LV: u16 = 0x9202;
pub(crate) const GET_LV: u16 = 0x9203;
pub(crate) const FOCUS_DRIVE: u16 = 0x9204;
pub(crate) const OBJECT_RAM: u16 = 0xc101;
pub(crate) const CAPTURE_COMPLETE_RAM: u16 = 0xc102;
pub(crate) const DEVICE_INFO_CHANGED: u16 = 0x4008;
pub(crate) const LV_STATUS: u16 = 0xd1a2;
pub(crate) const LV_PROHIBIT: u16 = 0xd1a4;
pub(crate) const LV_ZOOM:u16=0xd1a3;
pub(crate) const LV_FOCUS_MODE:u16=0xd061;
pub(crate) const LENS_SORT:u16=0xd0e1;
pub(crate) const LV_SELECTOR:u16=0xd1a6;
pub(crate) const ACTIVE_SLOT:u16=0xd1f2;
pub(crate) const LIVE_VIEW_PROPERTIES:&[(&str,&str,u16)]=&[
    ("cameraZoom","Camera Live View zoom",LV_ZOOM),
    ("focusMode","Live View focus mode",LV_FOCUS_MODE),
];
pub(crate) const RECORDING_MEDIA: u16 = 0xd10b;
pub(crate) const BURST_NUMBER: u16 = 0x5018;
pub(crate) const BRACKETING_ENABLE:u16=0xd0c0;
pub(crate) const INFO_DISPLAY_ERROR_STATUS: u16 = 0xd1b2;
/// Continuous shooting count. The real D7100 reported one completed exposure on
/// this property (`0x4006 PropertyChanged, parameter 0xD1B4`) for a host RAM
/// capture whose capture-complete event never arrived
/// (`docs/validation/card-sd-migration-hardware-2026-10-11.md` section 7). The
/// reference library defines neither this code nor any exposure meaning for a
/// `0x4006` notification, so it stays a diagnostic.
pub(crate) const CONTINUOUS_SHOOTING_COUNT: u16 = 0xd1b4;
const RUNTIME_ERROR_KEY: &str = "infoDisplayErrorStatus";
const LV_PROHIBIT_KEY: &str = "liveViewProhibitCondition";
// Exact documented display/counter IDs. Indicator scalar type is proven by
// the selected device descriptor when a fixed external type is unavailable.
pub(crate) const RUNTIME_DIAGNOSTICS: &[(&str, &str, u16, Option<u16>)] = &[
    ("lensSort","Lens mount/CPU category (native diagnostic)",LENS_SORT,None),
    ("liveViewSelector","Photo/movie Live View selector (native diagnostic)",LV_SELECTOR,None),
    ("activeSlot","Active storage slot (raw diagnostic)",ACTIVE_SLOT,None),
    (
        "batteryLevel",
        "Battery level (standard read-only diagnostic)",
        0x5001,
        Some(2),
    ),
    (
        LV_PROHIBIT_KEY,
        "Live View prohibit conditions (raw diagnostic)",
        LV_PROHIBIT,
        Some(6),
    ),
    (
        "exposureDisplayStatus",
        "Exposure display / low-light indication (raw diagnostic)",
        0xd1b0,
        Some(2),
    ),
    (
        "exposureIndicateStatus",
        "Exposure meter indication (raw diagnostic)",
        0xd1b1,
        Some(1),
    ),
    (
        RUNTIME_ERROR_KEY,
        "Information display status (raw diagnostic)",
        INFO_DISPLAY_ERROR_STATUS,
        Some(2),
    ),
    (
        "exposureIndicateLightup",
        "Exposure indicator illumination (raw diagnostic)",
        0xd1b3,
        None,
    ),
    (
        "continuousShootingCount",
        "Continuous shooting count (read-only counter; not BurstNumber)",
        CONTINUOUS_SHOOTING_COUNT,
        Some(2),
    ),
];
pub const PROFILE_REQUIRED_KEYS: &[&str] = &[
    "shutter",
    "iso",
    "aperture",
    "exposureMode",
    "autoIso",
    "rawFormat",
    "rawBitDepth",
    "rawCompression",
    "crop",
    "focusMode",
    "driveMode",
    "burstNumber",
    "recordingMedia",
    "exposureDelay",
    "longExposureNoiseReduction",
];
pub(crate) const NIKON_PROPERTIES: &[(&str, &str, u16)] = &[
    ("bracketing", "Automatic bracketing", BRACKETING_ENABLE),
    ("exposureMode", "Exposure mode", 0x500e),
    ("autoIso", "Auto ISO", 0xd054),
    ("rawFormat", "Image format (4 = NEF only)", 0x5004),
    ("driveMode", "Release mode (1 = single)", 0x5013),
    (
        "burstNumber",
        "Burst number (1 = single image)",
        BURST_NUMBER,
    ),
    (
        "recordingMedia",
        "Recording media (1 = SDRAM)",
        RECORDING_MEDIA,
    ),
    (
        "focusMode",
        "Focus mode (3 = physical MF, 4 = software MF)",
        0xd161,
    ),
    ("exposureDelay", "Exposure delay (3 = off)", 0xd06a),
    ("rawBitDepth", "RAW bit depth (0 = 12, 1 = 14)", 0xd149),
    ("rawCompression", "RAW compression", 0xd016),
    ("crop", "Capture area (0 = DX, 1 = 1.3x)", 0xd030),
    ("shutter", "Shutter (exact native rational)", 0xd100),
    ("iso", "ISO", 0x500f),
    ("aperture", "Aperture (f-number x 100)", 0x5007),
    ("whiteBalance", "White balance", 0x5005),
    ("exposureCompensation", "Exposure compensation", 0x5010),
    (
        "longExposureNoiseReduction",
        "Long exposure noise reduction",
        0xd06b,
    ),
    ("colorSpace", "Color space", 0xd032),
    ("pictureControl", "Picture control", 0xd200),
    ("meteringMode", "曝光测光", 0x500b),
    ("imageSize", "JPEG图像尺寸", 0x5003),
    ("highIsoNoiseReduction", "高ISO降噪", 0xd070),
    ("whiteBalanceFluorescent", "荧光灯类型", 0xd14f),
    ("whiteBalanceTemperature", "白平衡色温", 0xd01e),
    ("autoIsoHighLimit", "自动ISO上限", 0xd183),
    ("centerWeightedArea", "中央重点测光区域", 0xd059),
    ("flashMode", "闪光模式", 0x500c),
    ("flashCompensation", "闪光补偿", 0xd124),
    ("flashSyncSpeed", "闪光同步速度", 0xd074),
    ("activeDLighting", "动态D-Lighting", 0xd14e),
    ("hdrMode", "HDR模式", 0xd130),
    ("hdrExposureDifference", "HDR曝光差值", 0xd131),
    ("hdrSmoothing", "HDR强度", 0xd132),
];
const STANDARD_PROPERTIES: &[(&str, &str, u16)] = &[
    ("shutter", "Exposure time (0.1 ms units)", 0x500d),
    ("iso", "ISO", 0x500f),
    ("aperture", "Aperture (f-number x 100)", 0x5007),
    ("exposureMode", "Exposure program", 0x500e),
    ("rawFormat", "Image format", 0x5004),
    ("whiteBalance", "White balance", 0x5005),
    ("exposureCompensation", "Exposure compensation", 0x5010),
    ("driveMode", "Still capture mode", 0x5013),
    ("meteringMode", "Exposure metering mode", 0x500b),
    ("imageSize", "Image size", 0x5003),
    ("flashMode", "Flash mode", 0x500c),
];

pub(crate) struct NativeCamera {
    pub transport: Box<dyn CameraTransport>,
    pub health: CameraHealth,
    pub info: Option<DeviceInfo>,
    pub d7100: bool,
    pub capture_pending: bool,
    pub pending_capture: Option<PendingCapture>,
    /// Bounded post-commit drain window of the host-RAM operation that committed
    /// and registered its file last. It exists only to recognise that operation's
    /// optional late `0xC102` RAM completion while the camera is idle instead of
    /// retiring the session for an unassociated event. It is never capture credit:
    /// an absorbed event can neither complete, commit nor resurrect an operation.
    pub post_commit_drain: Option<PostCommitDrainWindow>,
    /// Once Close is attempted this interface is retired, including Close failures.
    pub transport_open: bool,
}
pub(crate) struct PendingCapture {
    pub session_id: String,
    pub operation_id: String,
    pub exposure_ms: f64,
    pub queue_drained: bool,
    pub accepted: bool,
}
/// Armed on the owner thread only after a host-RAM capture committed its file; it
/// names that one operation and the bound inside which its optional late `0xC102`
/// completion may still be recognised while idle.
pub(crate) struct PostCommitDrainWindow {
    pub session_id: String,
    pub operation_id: String,
    pub deadline: Instant,
    /// Late completions already absorbed for this one operation. A committed
    /// exposure has exactly one optional completion, so a second in-window
    /// `0xC102` stays fail-closed just like the duplicate rule inside the capture.
    pub absorbed: usize,
}
impl NativeCamera {
    pub fn new(transport: Box<dyn CameraTransport>) -> Self {
        Self {
            transport,
            health: CameraHealth {
                backend: "windows_wpd_ptp".into(),
                cleanup_confirmed: true,
                phase: "disconnected".into(),
                ..CameraHealth::default()
            },
            info: None,
            d7100: false,
            capture_pending: false,
            pending_capture: None,
            post_commit_drain: None,
            transport_open: false,
        }
    }
    pub fn connect(
        &mut self,
        id: &str,
        session: &str,
        context: &CallContext,
    ) -> Result<(), CameraError> {
        context.check()?;
        if id.is_empty() {
            return Err(CameraError::Safety(
                "explicit camera device ID required".into(),
            ));
        }
        if self.health.connected {
            return Err(CameraError::Busy(
                "disconnect the active camera before selecting another".into(),
            ));
        }
        retry_while_busy(context, || self.transport.open(id, context))?;
        self.transport_open = true;
        let mut info = parse_device_info(
            &self
                .transport
                .execute(
                    GET_DEVICE_INFO,
                    &[],
                    DataPhase::Read { max: 128 * 1024 },
                    context,
                )?
                .data()?,
        )?;
        let selection = d7100_selection(&info, id);
        self.d7100 = selection.is_some();
        info.diagnostic.adapter_selection = selection
            .unwrap_or("Standard PTP only: Nikon D7100 identity/extension binding not established")
            .into();
        self.health.device_info = Some(info.diagnostic.clone());
        if self.d7100 && info.diagnostic.functional_mode != 0 {
            return Err(invalid("Nikon FunctionalMode is not Standard Mode; automatic wake or alternate-mode control is unsupported"));
        }
        if self.d7100 && info.operations.contains(&GET_VENDOR_PROPERTY_CODES) {
            // Read-only discovery is allowed only after exact identity binding
            // and the operation's own GetDeviceInfo advertisement.
            let response = self.transport.execute(
                GET_VENDOR_PROPERTY_CODES,
                &[],
                DataPhase::Read { max: 4 + 2 * 4096 },
                context,
            )?;
            let parsed = if response.code == OK {
                parse_vendor_property_codes(&response.data)
            } else {
                Err(CameraError::Command(format!(
                    "GetVendorPropCodes response 0x{:04x}",
                    response.code
                )))
            };
            info.diagnostic.vendor_property_codes = Some(CameraVendorPropertyCodes {
                operation_code: GET_VENDOR_PROPERTY_CODES,
                response_code: response.code,
                properties: parsed.as_ref().cloned().unwrap_or_default(),
                dataset_size: response.data.len(),
                dataset_sha256: format!("{:x}", Sha256::digest(&response.data)),
                error: parsed.as_ref().err().map(ToString::to_string),
            });
            self.health.device_info = Some(info.diagnostic.clone());
            info.properties.extend(parsed?);
            info.properties.sort_unstable();
            info.properties.dedup();
            info.diagnostic.effective_properties = info.properties.clone();
            self.health.device_info = Some(info.diagnostic.clone());
        }
        self.health.identity = Some(CameraIdentity {
            device_id: id.into(),
            manufacturer: info.manufacturer.clone(),
            model: info.model.clone(),
            serial: (!info.serial.is_empty()).then(|| info.serial.clone()),
            backend: if self.d7100 {
                "windows_wpd_nikon_d7100"
            } else {
                "windows_wpd_standard_ptp"
            }
            .into(),
        });
        self.health.backend = self.health.identity.as_ref().unwrap().backend.clone();
        self.health.serial = self.health.identity.as_ref().unwrap().serial.clone();
        self.health.session_id = Some(session.into());
        self.health.properties.clear();
        self.health.runtime_properties.clear();
        self.health.live_view = LiveViewStatus::default();
        self.health.live_view_controls=CameraLiveViewControls::default();
        self.health.last_event_diagnostic = None;
        self.health.last_device_info_changed_diagnostic = None;
        self.health.last_capture_event_diagnostic = None;
        self.health.last_cleanup_diagnostic = None;
        self.health.connected = true;
        self.health.isolated = false;
        self.health.cleanup_confirmed = true;
        self.health.transfer_policy = self
            .d7100
            .then(|| "Nikon host RAM; no card fallback".into());
        self.health.capabilities = negotiated_capabilities(&info, self.d7100);
        self.info = Some(info);
        self.read_properties(context)?;
        if self.d7100 && self.info.as_ref().unwrap().properties.contains(&LV_STATUS) {
            let state = self.live_view_status(context)?;
            if state != [0] {
                // End-Live-View is a device-level command: it does not require this
                // process to have started Live View. A body left in Live View by an
                // interrupted session (or by another host) is therefore recovered in
                // software, instead of refusing to connect and asking the operator to
                // press the LV button or power-cycle the camera.
                self.force_live_view_off(context)?;
                let after = self.live_view_status(context)?;
                if after != [0] {
                    return Err(CameraError::Safety(format!(
                        "camera Live View could not be cleared in software (LV_STATUS={after:?}); check the camera body"
                    )));
                }
                self.health.last_error = Some(
                    "camera was left in Live View; the engine cleared it with EndLiveView before connecting"
                        .into(),
                );
            }
        }
        self.health.phase = "idle".into();
        self.health.last_seen_at = Some(chrono::Utc::now().to_rfc3339());
        if self.health.last_error.as_deref() != Some("camera was left in Live View; the engine cleared it with EndLiveView before connecting") {
            self.health.last_error = None;
        }
        Ok(())
    }
    fn live_view_status(&mut self, context: &CallContext) -> Result<Vec<u8>, CameraError> {
        retry_while_busy(context, || {
            self.transport
                .execute(
                    GET_PROPERTY,
                    &[LV_STATUS as u32],
                    DataPhase::Read { max: 16 },
                    context,
                )?
                .data()
        })
    }
    /// Device-level end-Live-View used when the body is already in Live View and
    /// no managed session exists. Mirrors the validated reference sequence
    /// (digiCamControl `NikonBase.StopLiveView`: `DeviceReady(); EndLiveView;
    /// DeviceReady();`), so recovery never depends on the operator touching the
    /// camera.
    pub fn force_live_view_off(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.device_ready(context)
            .map_err(|error| step_error("DeviceReady before forced end-Live-View", error))?;
        self.transport
            .execute(STOP_LV, &[], DataPhase::None, context)
            .and_then(PtpResponse::success)
            .map_err(|error| step_error("forced EndLiveView 0x9202", error))?;
        self.device_ready(context)
            .map_err(|error| step_error("DeviceReady after forced end-Live-View", error))?;
        Ok(())
    }
    pub fn ensure_connected(&self) -> Result<(), CameraError> {
        if !self.health.connected || self.health.isolated {
            Err(CameraError::Unavailable(
                "no usable selected camera session".into(),
            ))
        } else {
            Ok(())
        }
    }
    /// PTP DeviceInfoChanged requests a fresh capability dataset, not a new
    /// capture or a new identity. No Open, SetProperty, AF or capture is sent.
    pub fn refresh_device_info(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.refresh_device_info_scoped(context,false)
    }
    fn refresh_device_info_scoped(&mut self,context:&CallContext,live_read_only:bool)->Result<(),CameraError>{
        self.ensure_connected()?;
        if self.capture_pending || (self.health.live_view.active && !live_read_only) {
            return Err(invalid(
                "DeviceInfoChanged cannot refresh an active acquisition session",
            ));
        }
        let identity = self
            .health
            .identity
            .clone()
            .ok_or_else(|| invalid("selected camera identity missing"))?;
        let mut info = parse_device_info(
            &self
                .transport
                .execute(
                    GET_DEVICE_INFO,
                    &[],
                    DataPhase::Read { max: 128 * 1024 },
                    context,
                )?
                .data()?,
        )?;
        let selection = d7100_selection(&info, &identity.device_id);
        info.diagnostic.adapter_selection = selection
            .unwrap_or("Standard PTP only: Nikon D7100 identity/extension binding not established")
            .into();
        self.health.device_info = Some(info.diagnostic.clone());
        let serial = (!info.serial.is_empty()).then(|| info.serial.clone());
        // Even a lost serial cannot be accepted as the old device. Validate
        // before any newly advertised Nikon vendor discovery operation.
        if info.manufacturer != identity.manufacturer
            || info.model != identity.model
            || serial != identity.serial
            || selection.is_some() != self.d7100
        {
            return Err(invalid(
                "DeviceInfoChanged identity/model/serial or Nikon binding changed; session retired",
            ));
        }
        if self.d7100 && info.diagnostic.functional_mode != 0 {
            return Err(invalid("DeviceInfoChanged FunctionalMode is sleep/reserved/vendor-defined; automatic wake and acquisition refused"));
        }
        if self.d7100 && info.operations.contains(&GET_VENDOR_PROPERTY_CODES) {
            let response = self.transport.execute(
                GET_VENDOR_PROPERTY_CODES,
                &[],
                DataPhase::Read { max: 4 + 2 * 4096 },
                context,
            )?;
            let parsed = if response.code == OK {
                parse_vendor_property_codes(&response.data)
            } else {
                Err(CameraError::Command(format!(
                    "GetVendorPropCodes response 0x{:04x}",
                    response.code
                )))
            };
            info.diagnostic.vendor_property_codes = Some(CameraVendorPropertyCodes {
                operation_code: GET_VENDOR_PROPERTY_CODES,
                response_code: response.code,
                properties: parsed.as_ref().cloned().unwrap_or_default(),
                dataset_size: response.data.len(),
                dataset_sha256: format!("{:x}", Sha256::digest(&response.data)),
                error: parsed.as_ref().err().map(ToString::to_string),
            });
            self.health.device_info = Some(info.diagnostic.clone());
            info.properties.extend(parsed?);
            info.properties.sort_unstable();
            info.properties.dedup();
            info.diagnostic.effective_properties = info.properties.clone();
        }
        self.health.device_info = Some(info.diagnostic.clone());
        self.health.capabilities = negotiated_capabilities(&info, self.d7100);
        self.info = Some(info);
        if live_read_only {context.check()?;return Ok(());}
        self.health.runtime_properties.clear();
        self.read_properties(context)?;
        if self.d7100 {
            let info = self.info.as_ref().unwrap();
            if info.operations.contains(&DEVICE_READY) {
                self.device_ready(context)?;
            }
            if !self.info.as_ref().unwrap().properties.contains(&LV_STATUS)
                || !self
                    .info
                    .as_ref()
                    .unwrap()
                    .operations
                    .contains(&GET_PROPERTY)
            {
                return Err(invalid(
                    "DeviceInfoChanged cannot confirm advertised Live View state",
                ));
            }
            let state = self
                .transport
                .execute(
                    GET_PROPERTY,
                    &[LV_STATUS as u32],
                    DataPhase::Read { max: 16 },
                    context,
                )?
                .data()?;
            if state != [0] {
                return Err(invalid(
                    "DeviceInfoChanged Live View is ON or unknown; session retired",
                ));
            }
        }
        context.check()?;
        Ok(())
    }
    pub fn read_properties(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.ensure_connected()?;
        if !self.health.capabilities.properties {
            return Err(CameraError::Unsupported(
                "standard property operations not advertised".into(),
            ));
        }
        let mut list = if self.d7100 {
            NIKON_PROPERTIES.to_vec()
        } else {
            STANDARD_PROPERTIES.to_vec()
        };
        // Some D7100 MTP sessions advertise the standard ExposureTime but not
        // the private rational property. Preserve the advertised native code.
        if self.d7100
            && !self.info.as_ref().unwrap().properties.contains(&0xd100)
            && self.info.as_ref().unwrap().properties.contains(&0x500d)
        {
            if let Some(shutter) = list.iter_mut().find(|p| p.0 == "shutter") {
                *shutter = ("shutter", "Exposure time (0.1 ms units)", 0x500d);
            }
        }
        for &(key, label, code) in &list {
            context.check()?;
            if !self.info.as_ref().unwrap().properties.contains(&code) {
                self.health.properties.insert(
                    key.into(),
                    unknown_property(key, label, code, "相机未声明此属性；未发送读取或写入操作"),
                );
                continue;
            }
            let old_target = self
                .health
                .properties
                .get(key)
                .and_then(|p| p.target.clone());
            let result = self.read_data(GET_PROPERTY_DESC, &[code as u32], 128 * 1024, context);
            if let Err(error) = &result {
                if matches!(
                    error,
                    CameraError::Cancelled
                        | CameraError::Timeout(_)
                        | CameraError::Unavailable(_)
                        | CameraError::Io(_)
                ) {
                    return Err(error.clone());
                }
            }
            let result = result.and_then(|b| parse_property(&b, key, label, code));
            match result {
                Ok(mut p) => {
                    p.target = old_target;
                    p.writable = p.writable
                        && self
                            .info
                            .as_ref()
                            .unwrap()
                            .operations
                            .contains(&SET_PROPERTY)
                        && (!p.supported_values.is_empty() || p.range.is_some());
                    // Read the current value explicitly, independently of the enumeration.
                    let readback = self.read_data(GET_PROPERTY, &[code as u32], 4096, context);
                    if let Err(error) = &readback {
                        if matches!(
                            error,
                            CameraError::Cancelled
                                | CameraError::Timeout(_)
                                | CameraError::Unavailable(_)
                                | CameraError::Io(_)
                        ) {
                            return Err(error.clone());
                        }
                    }
                    match readback.and_then(|b| parse_value(&b, p.data_type)) {
                        Ok(actual) => {
                            p.error = p.target.as_ref().filter(|target|**target!=actual)
                                .map(|_|"property actual readback differs from requested native target; preflight invalidated".into());
                            p.actual = Some(actual);
                            p.confirmed = true;
                            // Nikon's physical MF switch is not software writable
                            // (MIT NikonBase/D600Base disable this selector at 3).
                            if key == "focusMode"
                                && p.actual.as_ref().and_then(CameraValue::integer) == Some(3)
                            {
                                p.writable = false;
                            }
                        }
                        Err(e) => {
                            p.actual = None;
                            p.confirmed = false;
                            p.error = Some(e.to_string());
                        }
                    }
                    self.health.properties.insert(key.into(), p);
                }
                Err(e) => {
                    self.health.properties.insert(key.into(), {
                        let mut p = unknown_property(key, label, code, &e.to_string());
                        p.target = old_target;
                        p.error = Some(e.to_string());
                        p
                    });
                }
            }
        }
        if self.d7100 {
            let mode = self
                .health
                .properties
                .get("focusMode")
                .filter(|p| p.confirmed)
                .and_then(|p| p.actual.as_ref())
                .and_then(CameraValue::integer);
            let drive_eligible = matches!(mode, Some(0 | 1 | 2));
            let operations = &self.info.as_ref().unwrap().operations;
            self.health.capabilities.autofocus = drive_eligible
                && operations.contains(&AF_DRIVE)
                && operations.contains(&DEVICE_READY);
            self.health.capabilities.focus_step = false;
            for key in ["autofocus", "focusStep"] {
                if !drive_eligible {
                    self.health.capabilities.disabled_reasons.insert(key.into(),"Physical MF or unknown focus mode: current lens motor drive eligibility is unverified".into());
                } else {
                    self.health.capabilities.disabled_reasons.remove(key);
                }
            }
        }
        let times: Vec<f64> = self
            .health
            .properties
            .get("shutter")
            .into_iter()
            .filter(|p| p.data_type == 0x0006)
            .flat_map(|p| {
                p.supported_values
                    .iter()
                    .filter_map(|v| shutter_ms(v, p.code))
            })
            .filter(|v| valid_exposure_ms(*v))
            .collect();
        self.health.exposure_min_ms = times.iter().copied().reduce(f64::min);
        self.health.exposure_max_ms = times.iter().copied().reduce(f64::max);
        if self.d7100
            && self
                .info
                .as_ref()
                .unwrap()
                .properties
                .contains(&INFO_DISPLAY_ERROR_STATUS)
        {
            self.read_runtime_status(context)?;
        }
        if self.d7100
            && self
                .info
                .as_ref()
                .unwrap()
                .properties
                .contains(&LV_PROHIBIT)
        {
            self.read_live_view_prohibit(context)?;
        }
        for &(_, _, code, _) in RUNTIME_DIAGNOSTICS {
            if !matches!(code, LV_PROHIBIT | INFO_DISPLAY_ERROR_STATUS)
                && self.d7100
                && self.info.as_ref().unwrap().properties.contains(&code)
            {
                if matches!(code,LENS_SORT|LV_SELECTOR|ACTIVE_SLOT){self.read_optional_runtime(code,context)?;}else{self.read_catalog_runtime(code, context)?;}
            }else if matches!(code,LENS_SORT|LV_SELECTOR|ACTIVE_SLOT){
                let (key,label,_,_)=RUNTIME_DIAGNOSTICS.iter().find(|row|row.2==code).unwrap();
                self.health.runtime_properties.insert((*key).into(),unknown_property(key,label,code,"本次设备未广告此诊断；未发送读取或写入").into());
            }
        }
        refresh_property_controls(&mut self.health);
        self.refresh_live_view_permissions();
        Ok(())
    }
    /// Explicit preflight preparation precedes the RAM trigger. Restored/frozen
    /// capture paths only validate these values and never call this writer.
    pub fn prepare_host_capture(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.ensure_connected()?;
        if !self.d7100 || !self.health.capabilities.host_capture || self.health.live_view.active {
            return Err(invalid("D7100 host capture with Live View OFF required"));
        }
        self.read_properties(context)?;
        let mut changes = CameraProperties::new();
        for (key, kind) in [("recordingMedia", 2), ("burstNumber", 4)] {
            let property = self
                .health
                .properties
                .get(key)
                .ok_or_else(|| invalid(&format!("host capture lacks {key} descriptor")))?;
            if property.data_type != kind
                || !property.readable
                || !property.confirmed
                || property
                    .actual
                    .as_ref()
                    .and_then(CameraValue::integer)
                    .is_none()
            {
                return Err(invalid(&format!(
                    "host capture {key} type/actual readback is unknown"
                )));
            }
            if property.actual != Some(CameraValue::Integer(1)) {
                changes.insert(key.into(), CameraValue::Integer(1));
            }
        }
        if !changes.is_empty() && !self.apply(&changes, context)?.complete {
            return Err(invalid(
                "SDRAM/single-image preparation did not complete write/readback",
            ));
        }
        for key in ["recordingMedia", "burstNumber"] {
            self.health.properties.get_mut(key).unwrap().target = Some(CameraValue::Integer(1));
        }
        self.read_properties(context)?;
        profile_readback(&self.health)?;
        self.confirm_runtime_diagnostic(context)?;
        Ok(())
    }
    pub fn read_runtime_status(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.read_runtime_property(
            RUNTIME_ERROR_KEY,
            "Information display status (raw diagnostic; meaning unknown)",
            INFO_DISPLAY_ERROR_STATUS,
            context,
        )
    }
    pub fn read_live_view_prohibit(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.read_runtime_property(LV_PROHIBIT_KEY,
            "Live View prohibit conditions (raw diagnostic; not capture completion or global health)", LV_PROHIBIT, context)?;
        let property = &self.health.runtime_properties[LV_PROHIBIT_KEY];
        if property.data_type != 6
            || !property.readable
            || property.writable
            || !property.confirmed
            || property.error.is_some()
            || property
                .actual
                .as_ref()
                .and_then(CameraValue::integer)
                .is_none()
        {
            return Err(invalid(
                "D7100 D1A4 requires advertised read-only UINT32 and confirmed raw readback",
            ));
        }
        Ok(())
    }
    pub fn read_catalog_runtime(
        &mut self,
        code: u16,
        context: &CallContext,
    ) -> Result<bool, CameraError> {
        let Some(&(key, label, _, kind)) = RUNTIME_DIAGNOSTICS.iter().find(|row| row.2 == code)
        else {
            return Ok(false);
        };
        self.read_runtime_property(key, label, code, context)?;
        let property = &self.health.runtime_properties[key];
        if !property.readable
            || property.writable
            || !property.confirmed
            || property.error.is_some()
            || !matches!(property.data_type, 1..=8)
            || kind.is_some_and(|kind| property.data_type != kind)
            || property
                .actual
                .as_ref()
                .and_then(CameraValue::integer)
                .is_none()
        {
            return Err(invalid("documented runtime diagnostic requires advertised read-only scalar and confirmed native type/value"));
        }
        Ok(true)
    }
    fn check_live_session(&self,session:&str,context:&CallContext,guard:&dyn Fn()->Result<(),CameraError>)->Result<(),CameraError>{
        context.check()?;guard()?;self.ensure_connected()?;
        if !self.d7100 || !self.health.live_view.active || self.health.live_view.session_id.as_deref()!=Some(session){
            return Err(CameraError::Unsupported("当前Live View会话不匹配或已停止".into()));
        }Ok(())
    }
    fn confirm_live_view_active(&mut self,session:&str,context:&CallContext,guard:&dyn Fn()->Result<(),CameraError>)->Result<(),CameraError>{
        self.check_live_session(session,context,guard)?;
        if !self.info.as_ref().is_some_and(|info|info.operations.contains(&GET_PROPERTY)&&info.properties.contains(&LV_STATUS)){
            return Err(CameraError::Unavailable("LV状态读取资格已失效；当前会话隔离".into()));
        }
        let state=self.read_data(GET_PROPERTY,&[LV_STATUS as u32],16,context)?;
        self.check_live_session(session,context,guard)?;
        if state==[1]{return Ok(());}
        if state==[0]{
            self.health.live_view.active=false;self.health.live_view.fresh=false;self.health.live_view.session_id=None;
            clear_live_view_controls(&mut self.health,"相机读回Live View已停止；旧控制会话失效");
            return Err(CameraError::Unsupported("相机读回Live View已停止；请重新启动建立新会话".into()));
        }
        Err(CameraError::Unavailable("LV状态读回未知；当前会话隔离".into()))
    }
    fn read_live_property(&mut self,key:&str,label:&str,code:u16,context:&CallContext)->Result<CameraProperty,CameraError>{
        if !self.info.as_ref().is_some_and(|info|info.operations.contains(&GET_PROPERTY_DESC)&&info.operations.contains(&GET_PROPERTY)&&info.properties.contains(&code)){
            return Ok(unknown_property(key,label,code,"当前设备未广告此LV属性或读取操作；未试探属性"));
        }
        let mut property=parse_property(&self.read_data(GET_PROPERTY_DESC,&[code as u32],128*1024,context)?,key,label,code)?;
        if !live_control_type_allowed(key,property.data_type)
            || property.supported_values.iter().any(|value|!matches!(value.integer(),Some(0..=255)))
            || property.range.is_some_and(|range|range.iter().any(|value|!(0..=255).contains(value))){
            return Ok(unknown_property(key,label,code,"Live View control requires a bounded integer descriptor; focusMode requires UINT8"));
        }
        let value=parse_value(&self.read_data(GET_PROPERTY,&[code as u32],4096,context)?,property.data_type)?;
        if !matches!(value.integer(),Some(0..=255)){return Ok(unknown_property(key,label,code,"Live View control readback is outside 0..255"));}
        property.actual=Some(value);property.confirmed=true;property.error=None;
        Ok(property)
    }
    pub fn read_live_view_controls(&mut self,session:&str,context:&CallContext,guard:&dyn Fn()->Result<(),CameraError>)->Result<CameraLiveViewControls,CameraError>{
        self.check_live_session(session,context,guard)?;
        self.refresh_device_info_scoped(context,true).map_err(|error|if matches!(error,CameraError::Safety(_)){CameraError::Unavailable(format!("LV设备身份/能力重读失败，当前会话隔离：{error}"))}else{error})?;
        self.check_live_session(session,context,guard)?;
        self.confirm_live_view_active(session,context,guard)?;
        self.health.live_view_controls.session_id=Some(session.into());
        for &(key,label,code) in LIVE_VIEW_PROPERTIES{
            self.check_live_session(session,context,guard)?;
            let target=self.health.live_view_controls.properties.get(key).and_then(|p|p.target.clone());
            let mut property=match self.read_live_property(key,label,code,context){
                Ok(property)=>property,
                Err(error)if matches!(error,CameraError::Command(_)|CameraError::Unsupported(_)|CameraError::Safety(_))=>{
                    let mut p=unknown_property(key,label,code,&error.to_string());p.error=Some(error.to_string());p
                },
                Err(error)=>return Err(error),
            };
            property.target=target;
            if property.target.as_ref().is_some_and(|target|property.actual.as_ref()!=Some(target)){
                property.error=Some("LV实际读回与本次目标不一致；请停止后重新启动Live View建立新控制会话".into());
            }
            self.health.live_view_controls.properties.insert(key.into(),property);
        }
        for code in [LENS_SORT,LV_SELECTOR]{
            self.check_live_session(session,context,guard)?;
            if self.info.as_ref().unwrap().properties.contains(&code){self.read_optional_runtime(code,context)?;}
            else{let (key,label,_,_)=RUNTIME_DIAGNOSTICS.iter().find(|row|row.2==code).unwrap();self.health.runtime_properties.insert((*key).into(),unknown_property(key,label,code,"本次设备未广告此诊断；不试读").into());}
        }
        self.check_live_session(session,context,guard)?;
        self.refresh_live_view_permissions();
        Ok(self.health.live_view_controls.clone())
    }
    pub fn configure_live_view(&mut self,session:&str,values:&CameraProperties,context:&CallContext,guard:&dyn Fn()->Result<(),CameraError>)->Result<PropertyApplyResult,CameraError>{
        if values.is_empty()||values.len()>2||values.iter().any(|(key,value)|!LIVE_VIEW_PROPERTIES.iter().any(|p|p.0==key)||!matches!(value.integer(),Some(0..=255))){
            return Err(CameraError::Unsupported("LV专用命令只接受cameraZoom/focusMode，至少一项".into()));
        }
        self.read_live_view_controls(session,context,guard)?;
        let mut applied=Vec::new();let mut failed=BTreeMap::new();
        for key in ["focusMode","cameraZoom"].into_iter().filter(|key|values.contains_key(*key)){
            self.check_live_session(session,context,guard)?;
            let (_,label,code)=*LIVE_VIEW_PROPERTIES.iter().find(|p|p.0==key).unwrap();
            // Re-describe each item immediately before SET, retaining the fresh native byte width.
            let mut property=self.read_live_property(key,label,code,context)?;
            let value=&values[key];
            self.health.live_view_controls.properties.insert(key.into(),property.clone());self.refresh_live_view_permissions();
            if key=="focusMode"&&!matches!(value.integer(),Some(0|2|4)){
                failed.insert(key.into(),"LV焦点模式只允许有据的AF-S/AF-F/软件MF目标；物理MF只读".into());break;
            }
            if property.confirmed&&property.error.is_none()&&property.actual.as_ref()==Some(value)&&live_control_type_allowed(key,property.data_type)&&value_supported(&property,value){
                self.health.live_view_controls.properties.get_mut(key).unwrap().target=Some(value.clone());applied.push(key.into());continue;
            }
            property=self.health.live_view_controls.properties[key].clone();
            if !property.writable||!accepts(&property,value)||(key=="focusMode"&&!matches!(value.integer(),Some(0|2|4))) {
                failed.insert(key.into(),property.disabled_reason.clone().unwrap_or_else(||"LV值未获实际描述/当前资格支持".into()));break;
            }
            let bytes=encode_value(value,property.data_type)?;
            self.confirm_live_view_active(session,context,guard)?;
            let p=self.health.live_view_controls.properties.get_mut(key).unwrap();p.target=Some(value.clone());p.confirmed=false;p.writable=false;
            let write=self.transport.execute(SET_PROPERTY,&[code as u32],DataPhase::Write(bytes),context).and_then(PtpResponse::success);
            if let Err(error)=&write{if matches!(error,CameraError::Cancelled|CameraError::Timeout(_)|CameraError::Io(_)|CameraError::Unavailable(_)){return Err(error.clone());}}
            self.check_live_session(session,context,guard)?;
            let readback=self.read_live_property(key,label,code,context);
            match readback{
                Ok(mut actual)=>{
                    actual.target=Some(value.clone());
                    if write.is_ok()&&actual.confirmed&&actual.data_type==property.data_type&&actual.actual.as_ref()==Some(value){actual.error=None;applied.push(key.into());}
                    else{let error=write.err().map(|e|e.to_string()).unwrap_or_else(||if actual.data_type!=property.data_type{"Live View descriptor type changed during write; target is unconfirmed".into()}else{"LV实际读回与目标不一致；请停止后重新启动Live View建立新控制会话".into()});actual.error=Some(error.clone());failed.insert(key.into(),error);}
                    self.health.live_view_controls.properties.insert(key.into(),actual);
                },
                Err(error)=>{let p=self.health.live_view_controls.properties.get_mut(key).unwrap();p.actual=None;p.confirmed=false;p.error=Some(error.to_string());if matches!(error,CameraError::Cancelled|CameraError::Timeout(_)|CameraError::Io(_)|CameraError::Unavailable(_)){return Err(error);}failed.insert(key.into(),error.to_string());}
            }
            self.check_live_session(session,context,guard)?;self.refresh_live_view_permissions();
            if !failed.is_empty(){break;}
        }
        Ok(PropertyApplyResult{complete:failed.is_empty(),applied,failed,properties:self.health.live_view_controls.properties.clone()})
    }
    pub fn refresh_live_view_permissions(&mut self){
        refresh_live_view_controls(&mut self.health);
        let runtime=|key:&str|self.health.runtime_properties.get(key).filter(|p|p.readable&&p.confirmed&&p.error.is_none()).and_then(|p|p.actual.as_ref()).and_then(CameraValue::integer);
        let lens=runtime("lensSort")==Some(1);let selector=runtime("liveViewSelector");
        let operations=self.info.as_ref().map(|info|&info.operations);
        let advertised=|op|operations.is_some_and(|ops|ops.contains(&op)&&ops.contains(&DEVICE_READY));
        if self.health.live_view.active{
            let mode=self.health.live_view_controls.properties.get("focusMode").filter(|p|p.supported&&p.data_type==2&&p.confirmed&&p.error.is_none()&&p.actual.as_ref().is_some_and(|value|value_supported(p,value))).and_then(|p|p.actual.as_ref()).and_then(CameraValue::integer);
            let current=self.health.live_view_controls.session_id==self.health.live_view.session_id;
            self.health.capabilities.autofocus=current&&lens&&selector==Some(0)&&mode==Some(0)&&advertised(AF_DRIVE);
            self.health.capabilities.focus_step=current&&lens&&selector==Some(0)&&matches!(mode,Some(0|4))&&advertised(FOCUS_DRIVE);
            for (key,enabled) in [("autofocus",self.health.capabilities.autofocus),("focusStep",self.health.capabilities.focus_step)]{
                if enabled{self.health.capabilities.disabled_reasons.remove(key);}else{self.health.capabilities.disabled_reasons.insert(key.into(),"当前LV镜头/照片选择器/D061资格未确认，或物理MF/AF-F禁止该驱动".into());}
            }
        }else{
            self.health.capabilities.focus_step=false;self.health.capabilities.disabled_reasons.insert("focusStep".into(),"镜头步进需要当前Live View及鲜读D061/镜头资格".into());
            let normal_mode=self.health.properties.get("focusMode").filter(|p|p.confirmed&&p.error.is_none()).and_then(|p|p.actual.as_ref()).and_then(CameraValue::integer);
            self.health.capabilities.autofocus=lens&&matches!(normal_mode,Some(0|1|2))&&advertised(AF_DRIVE);
            if !self.health.capabilities.autofocus{self.health.capabilities.disabled_reasons.insert("autofocus".into(),"正常焦点模式或镜头驱动资格未确认".into());}
        }
    }
    fn read_optional_runtime(&mut self,code:u16,context:&CallContext)->Result<(),CameraError>{
        match self.read_catalog_runtime(code,context){
            Ok(_)=>Ok(()),
            Err(error)if matches!(error,CameraError::Command(_)|CameraError::Unsupported(_)|CameraError::Safety(_))=>{
                if let Some((key,_,_,_))=RUNTIME_DIAGNOSTICS.iter().find(|row|row.2==code){
                    if let Some(property)=self.health.runtime_properties.get_mut(*key){property.property.error=Some(error.to_string());property.property.writable=false;property.property.effective=false;}
                }
                Ok(())
            },
            Err(error)=>Err(error),
        }
    }
    pub fn read_data(
        &mut self,
        opcode: u16,
        params: &[u32],
        max: usize,
        context: &CallContext,
    ) -> Result<Vec<u8>, CameraError> {
        loop {
            context.check()?;
            let response =
                self.transport
                    .execute(opcode, params, DataPhase::Read { max }, context)?;
            if response.code == DEVICE_BUSY {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            return response.data();
        }
    }
    fn read_runtime_property(
        &mut self,
        key: &str,
        label: &str,
        code: u16,
        context: &CallContext,
    ) -> Result<(), CameraError> {
        if !self.d7100
            || !self
                .info
                .as_ref()
                .is_some_and(|info| info.properties.contains(&code))
        {
            return Err(invalid(
                "D7100 runtime diagnostic property is not advertised",
            ));
        }
        let result = (|| {
            let bytes = self.read_data(GET_PROPERTY_DESC, &[code as u32], 128 * 1024, context)?;
            let mut property = parse_property(&bytes, key, label, code)?;
            let bytes = self.read_data(GET_PROPERTY, &[code as u32], 4096, context)?;
            property.actual = Some(parse_value(&bytes, property.data_type)?);
            property.confirmed = true;
            // Only the newly exposed optional diagnostics have a display-only
            // policy. Mandatory runtime checks must retain native writability
            // so an unexpected writable descriptor still prevents capture.
            if matches!(code, LENS_SORT | LV_SELECTOR | ACTIVE_SLOT) {
                property.writable = false;
                property.effective = false;
                property.disabled_reason = Some("只读运行诊断；不进入摄影Apply或预设".into());
            }
            Ok::<_, CameraError>(property)
        })();
        match result {
            Ok(property) => {
                self.health
                    .runtime_properties
                    .insert(key.into(), property.into());
                Ok(())
            }
            Err(error) => {
                self.health.runtime_properties.insert(
                    key.into(),
                    CameraProperty {
                        key: key.into(),
                        label: label.into(),
                        code,
                        data_type: 0,
                        readable: false,
                        writable: false,
                        supported: false,
                        native_writable: false,
                        effective: false,
                        disabled_reason: Some(error.to_string()),
                        options: vec![],
                        supported_values: vec![],
                        range: None,
                        actual: None,
                        target: None,
                        confirmed: false,
                        error: Some(error.to_string()),
                    }
                    .into(),
                );
                Err(error)
            }
        }
    }
    /// Validate transport evidence only. No raw value has established clear or
    /// fault semantics, and this status cannot authorize radiation output.
    pub fn confirm_runtime_diagnostic(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.confirm_runtime_diagnostic_for(context, 1)
    }
    /// Destination-aware runtime confirmation. The host RAM path requires
    /// `recordingMedia=1` and the card path requires `0`; the descriptor, fresh
    /// readback and `0xD1B2` evidence rules are identical, so a card session can
    /// confirm its own runtime status instead of being refused by an SDRAM-only
    /// assumption.
    pub fn confirm_runtime_diagnostic_for(
        &mut self,
        context: &CallContext,
        expected_media: i64,
    ) -> Result<(), CameraError> {
        let media = self
            .health
            .properties
            .get("recordingMedia")
            .ok_or_else(|| invalid("runtime status requires confirmed recording media"))?;
        if media.code != RECORDING_MEDIA
            || media.data_type != 2
            || !media.confirmed
            || media.error.is_some()
            || media.actual != Some(CameraValue::Integer(expected_media))
        {
            return Err(invalid(&format!(
                "runtime status requires confirmed recording media={expected_media}"
            )));
        }
        let readback = self
            .read_data(GET_PROPERTY, &[RECORDING_MEDIA as u32], 16, context)
            .and_then(|bytes| parse_value(&bytes, 2));
        let media = self.health.properties.get_mut("recordingMedia").unwrap();
        match readback {
            Ok(actual) => {
                media.confirmed = true;
                media.actual = Some(actual.clone());
                if actual != CameraValue::Integer(expected_media) {
                    media.error = Some(format!(
                        "recording media changed from confirmed value {expected_media}"
                    ));
                    return Err(invalid(
                        "runtime notification cannot confirm changed recording media",
                    ));
                }
            }
            Err(error) => {
                media.confirmed = false;
                media.actual = None;
                media.error = Some(error.to_string());
                return Err(error);
            }
        }
        self.read_runtime_status(context)?;
        let status = &self.health.runtime_properties[RUNTIME_ERROR_KEY];
        if status.data_type != 2
            || !status.readable
            || status.writable
            || !status.confirmed
            || status.error.is_some()
            || status
                .actual
                .as_ref()
                .and_then(CameraValue::integer)
                .is_none()
        {
            return Err(invalid(
                "D7100 0xD1B2 diagnostic must be advertised read-only UINT8 with confirmed raw readback; semantic meaning remains unknown",
            ));
        }
        Ok(())
    }
    pub fn accept_runtime_notification(
        &mut self,
        parameter: u32,
        context: &CallContext,
    ) -> Result<bool, CameraError> {
        if parameter == INFO_DISPLAY_ERROR_STATUS as u32 {
            self.confirm_runtime_diagnostic(context)?;
            return Ok(true);
        }
        let Ok(code) = u16::try_from(parameter) else {
            return Ok(false);
        };
        self.read_catalog_runtime(code, context)
    }
    pub fn apply(
        &mut self,
        values: &CameraProperties,
        context: &CallContext,
    ) -> Result<PropertyApplyResult, CameraError> {
        self.ensure_connected()?;
        self.read_properties(context)?;
        validate_properties(&self.health, values)?;
        let mut applied = vec![];
        let mut failed = BTreeMap::new();
        // Exposure mode/auto modes precede shutter and lens-dependent properties.
        let order = application_order(values);
        for key in order {
            context.check()?;
            let value = &values[&key];
            let property = &self.health.properties[&key];
            if property.confirmed
                && property.error.is_none()
                && property.actual.as_ref() == Some(value)
            {
                let property = self.health.properties.get_mut(&key).unwrap();
                property.target = Some(value.clone());
                property.error = None;
                applied.push(key.clone());
                continue;
            }
            // Every child is admitted again against the descriptors/actual
            // returned after its parent write, never against a predicted enum.
            if let Err(error) = validate_control_values(
                &self.health,
                &BTreeMap::from([(key.clone(), value.clone())]),
            ) {
                // Render through CameraError so the operator-facing payload keeps
                // exactly the wording it had before the rejection was classified.
                failed.insert(key.clone(), error.into_camera_error().to_string());
                break;
            }
            let p = self.health.properties.get_mut(&key).unwrap();
            p.target = Some(value.clone());
            p.confirmed = false;
            let code = p.code;
            let kind = p.data_type;
            let result = self
                .transport
                .execute(
                    SET_PROPERTY,
                    &[code as u32],
                    DataPhase::Write(encode_value(value, kind)?),
                    context,
                )
                .and_then(PtpResponse::success);
            let readback = self
                .transport
                .execute(
                    GET_PROPERTY,
                    &[code as u32],
                    DataPhase::Read { max: 4096 },
                    context,
                )
                .and_then(PtpResponse::data)
                .and_then(|b| parse_value(&b, kind));
            let p = self.health.properties.get_mut(&key).unwrap();
            match readback {
                Ok(actual) => {
                    p.actual = Some(actual.clone());
                    p.confirmed = true;
                    if result.is_ok() && actual == *value {
                        p.error = None;
                        applied.push(key.clone());
                    } else {
                        let e = result.err().map(|e| e.to_string()).unwrap_or_else(|| {
                            "property readback does not match requested native value".into()
                        });
                        p.error = Some(e.clone());
                        failed.insert(key.clone(), e);
                    }
                }
                Err(e) => {
                    p.actual = None;
                    p.confirmed = false;
                    p.error = Some(e.to_string());
                    failed.insert(key.clone(), e.to_string());
                }
            }
            if !failed.is_empty() {
                break;
            }
            if changes_dependencies(&key) {
                self.read_properties(context)?;
            }
        }
        // Unattempted fields remain actual evidence, with their requested draft target.
        for (key, value) in values {
            let property = self.health.properties.get_mut(key).unwrap();
            property.target = Some(value.clone());
            if let Some(reason) = failed.get(key) {
                property.error = Some(reason.clone());
            }
        }
        let complete = applied.len() == values.len() && failed.is_empty();
        if !complete {
            self.health.last_error = Some("parameter application partially failed; actual readback retained; preflight invalidated".into());
        }
        refresh_property_controls(&mut self.health);
        Ok(PropertyApplyResult {
            complete,
            applied,
            failed,
            properties: self.health.properties.clone(),
        })
    }
    pub fn device_ready(&mut self, context: &CallContext) -> Result<(), CameraError> {
        loop {
            context.check()?;
            match self
                .transport
                .execute(DEVICE_READY, &[], DataPhase::None, context)?
                .code
            {
                OK => return Ok(()),
                DEVICE_BUSY => thread::sleep(Duration::from_millis(10)),
                code => {
                    return Err(CameraError::Command(format!(
                        "Nikon DeviceReady 0x{code:04x}"
                    )))
                }
            }
        }
    }
    pub fn events(&mut self, context: &CallContext) -> Result<Vec<NikonEvent>, CameraError> {
        let response = (|| loop {
            context.check()?;
            let response =
                self.transport
                    .execute(GET_EVENT, &[], DataPhase::Read { max: 8192 }, context)?;
            if response.code == DEVICE_BUSY {
                self.health.last_event_diagnostic = Some(CameraEventDiagnostic {
                    operation_code: GET_EVENT,
                    response_code: Some(response.code),
                    dataset_size: response.data.len(),
                    dataset_sha256: format!("{:x}", Sha256::digest(&response.data)),
                    error: Some("Nikon GetEvent DeviceBusy; bounded retry pending".into()),
                    ..Default::default()
                });
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            return Ok::<_, CameraError>(response);
        })();
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                self.health.last_event_diagnostic = Some(CameraEventDiagnostic {
                    operation_code: GET_EVENT,
                    error: Some(error.to_string()),
                    ..Default::default()
                });
                return Err(error);
            }
        };
        let parsed = if response.code == OK {
            parse_nikon_events(&response.data)
        } else {
            Err(CameraError::Command(format!(
                "Nikon GetEvent response 0x{:04x}",
                response.code
            )))
        };
        self.health.last_event_diagnostic = Some(CameraEventDiagnostic {
            operation_code: GET_EVENT,
            response_code: Some(response.code),
            dataset_size: response.data.len(),
            dataset_sha256: format!("{:x}", Sha256::digest(&response.data)),
            data_prefix_hex: response
                .data
                .iter()
                .take(64)
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            events: parsed
                .as_ref()
                .map(|events| {
                    events
                        .iter()
                        .take(16)
                        .map(|event| CameraEventRecord {
                            code: event.code,
                            parameter: event.parameter,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            error: parsed.as_ref().err().map(ToString::to_string),
        });
        if parsed
            .as_ref()
            .is_ok_and(|events| events.iter().any(|event| event.code == DEVICE_INFO_CHANGED))
        {
            self.health.last_device_info_changed_diagnostic =
                self.health.last_event_diagnostic.clone();
        }
        parsed
    }
    pub fn drain_events(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.device_ready(context)?;
        // Old capture/object events are rejected, never rebound to the next operation.
        for _ in 0..8 {
            let events = self.events(context)?;
            if events.is_empty() {
                return Ok(());
            }
            if events
                .iter()
                .any(|e| matches!(e.code, OBJECT_RAM | CAPTURE_COMPLETE_RAM | 0x4002 | 0x400d))
            {
                return Err(CameraError::Safety(
                    "unassociated capture event pending before new operation".into(),
                ));
            }
            if events.iter().any(|event| event.code == DEVICE_INFO_CHANGED) {
                return Err(invalid("DeviceInfoChanged pending at capture boundary; fresh idle capability validation required"));
            }
        }
        Err(CameraError::Safety(
            "Nikon event queue did not become empty".into(),
        ))
    }
}

fn negotiated_capabilities(info: &DeviceInfo, d7100: bool) -> CameraCapabilities {
    let has = |code| info.operations.contains(&code);
    let host_capture = d7100
        && [
            CAPTURE_RAM,
            GET_EVENT,
            DEVICE_READY,
            GET_OBJECT_INFO,
            GET_OBJECT,
            GET_PROPERTY,
        ]
        .iter()
        .all(|code| has(*code))
        && info.events.contains(&OBJECT_RAM)
        && info.events.contains(&CAPTURE_COMPLETE_RAM);
    // Card capture is negotiated separately from host RAM capture: it needs the
    // standard InitiateCapture/storage/object operations plus both association
    // events. A missing advertisement keeps card capture disabled instead of
    // probing an unproven opcode.
    let card_capture = d7100
        && [
            INITIATE_CAPTURE,
            GET_EVENT,
            DEVICE_READY,
            GET_OBJECT_INFO,
            GET_STORAGE_IDS,
            GET_STORAGE_INFO,
            GET_OBJECT_HANDLES,
        ]
        .iter()
        .all(|code| has(*code))
        && info.events.contains(&OBJECT_ADDED)
        && info.events.contains(&CAPTURE_COMPLETE_CARD);
    // Overflow continuation additionally needs the storage notifications that
    // describe a slot change; without them the task stays single-card.
    let card_overflow =
        card_capture && info.events.contains(&STORE_FULL) && info.events.contains(&STORE_REMOVED);
    let mut capabilities = CameraCapabilities {
        properties: has(GET_PROPERTY_DESC) && has(GET_PROPERTY),
        host_capture,
        card_capture,
        card_overflow,
        live_view: d7100
            && [START_LV, STOP_LV, GET_LV, GET_PROPERTY, DEVICE_READY]
                .iter()
                .all(|code| has(*code))
            && info.properties.contains(&LV_STATUS),
        autofocus: d7100 && has(AF_DRIVE) && has(DEVICE_READY),
        focus_step: d7100 && has(FOCUS_DRIVE) && has(DEVICE_READY),
        scan_nef: host_capture,
        hardware_certified: false,
        disabled_reasons: BTreeMap::new(),
    };
    capabilities.disabled_reasons.insert(
        "hardwareCertification".into(),
        "BLOCKED: D7100 event semantics and real capture require authorized hardware acceptance"
            .into(),
    );
    if !host_capture {
        capabilities.disabled_reasons.insert("capture".into(),
        "Host-only RAM capture and its correlated completion events are not proven for this identity/capability set".into());
    }
    if !card_capture {
        capabilities.disabled_reasons.insert("cardCapture".into(),
        "Card capture and its ObjectAdded/CaptureComplete association are not advertised for this identity/capability set".into());
    }
    if card_capture && !card_overflow {
        capabilities.disabled_reasons.insert("cardOverflow".into(),
        "StoreFull/StoreRemoved notifications are not advertised; only one verified writable card is offered".into());
    }
    if !capabilities.live_view {
        capabilities.disabled_reasons.insert(
            "liveView".into(),
            "D7100 Live View operation/property set not advertised".into(),
        );
    }
    capabilities
}

/// VendorExtensionID selects a PTP/MTP namespace, not the USB manufacturer.
/// Exact Nikon/model identity is mandatory. MTP namespaces additionally need
/// the known D7100 USB binding or an explicit Nikon extension declaration.
pub(crate) fn d7100_selection(info: &DeviceInfo, device_id: &str) -> Option<&'static str> {
    let manufacturer = info.manufacturer.trim();
    if !["Nikon Corporation", "Nikon"]
        .iter()
        .any(|name| manufacturer.eq_ignore_ascii_case(name))
        || !matches!(
            info.model.trim().to_ascii_uppercase().as_str(),
            "D7100" | "NIKON D7100"
        )
    {
        return None;
    }
    if info.vendor_id == 0x0a {
        return Some("Exact Nikon D7100 identity and Nikon PTP extension 0x0000000A");
    }
    if !matches!(info.vendor_id, 0x06 | 0xffffffff) {
        return None;
    }
    let usb_binding = device_id.to_ascii_uppercase().contains("VID_04B0&PID_0430");
    let nikon_extension = info
        .diagnostic
        .vendor_extension_description
        .split(';')
        .any(|part| {
            part.trim()
                .split_once(':')
                .is_some_and(|(domain, version)| {
                    domain.trim().eq_ignore_ascii_case("nikon.com") && !version.trim().is_empty()
                })
        });
    if usb_binding {
        Some("Exact Nikon D7100 identity; MTP namespace; Nikon USB VID_04B0/PID_0430 binding")
    } else if nikon_extension {
        Some("Exact Nikon D7100 identity; MTP namespace with explicit nikon.com extension")
    } else {
        None
    }
}

/// Retry a device acquisition step while the device reports itself busy, mirroring
/// the validated reference implementation (`DeviceReady` busy loop with a short
/// sleep; it treats both the PTP busy code and `0x800700AA` as retryable). Without
/// this a device still finishing a previous operation makes the workstation fail
/// to re-acquire the camera and forces an operator restart. Bounded by the
/// caller's context deadline.
fn retry_while_busy<T>(
    context: &CallContext,
    mut operation: impl FnMut() -> Result<T, CameraError>,
) -> Result<T, CameraError> {
    loop {
        match operation() {
            Err(CameraError::DeviceBusy(message)) => {
                if Instant::now() >= context.deadline {
                    return Err(CameraError::Timeout(format!("device stayed busy: {message}")));
                }
                thread::sleep(Duration::from_millis(25));
            }
            other => return other,
        }
    }
}

pub fn validate_properties(
    health: &CameraHealth,
    values: &CameraProperties,
) -> Result<(), CameraError> {
    // Ordinary photography Apply is prohibited throughout LV, even if every
    // requested value is already confirmed, and the shared admission decision is
    // classified so the domain entry can report why a write was refused before it
    // ever reaches the device.
    property_rejection(health, values).map_err(PropertyRejection::into_camera_error)
}
pub fn profile_readback(health: &CameraHealth) -> Result<CameraProperties, CameraError> {
    let issues = crate::capture_profile::scan_camera_issues(health);
    if !issues.is_empty() { return Err(CameraError::Safety(serde_json::to_string(&issues).unwrap_or_else(|_| "Fixed camera policy blocked".into()))); }
    acquisition_readback(health, true)
}
pub fn independent_readback(health: &CameraHealth) -> Result<CameraProperties, CameraError> { acquisition_readback(health, false) }
pub(crate) fn automatic_exposure_property(health:&CameraHealth,key:&str)->bool {
    let actual=|key:&str|health.properties.get(key).filter(|p|p.confirmed&&p.error.is_none()).and_then(|p|p.actual.as_ref()).and_then(CameraValue::integer);
    match key {"shutter"=>matches!(actual("exposureMode"),Some(2|3)),"aperture"=>matches!(actual("exposureMode"),Some(2|4)),"iso"=>actual("autoIso")==Some(1),_=>false}
}
fn acquisition_readback(health: &CameraHealth, scan_policy: bool) -> Result<CameraProperties, CameraError> {
    if !health.connected || health.isolated || !health.capabilities.scan_nef {
        return Err(CameraError::Safety(
            "D7100 native NEF session required".into(),
        ));
    }
    if !scan_policy {
        let actual=|key:&str|health.properties.get(key).filter(|p|p.confirmed&&p.error.is_none()).and_then(|p|p.actual.as_ref()).and_then(CameraValue::integer);
        if !matches!(actual("exposureMode"),Some(1|2|3|4))||!matches!(actual("autoIso"),Some(0|1)){return Err(invalid("Independent exposure mode or Auto ISO is unknown/unsupported"));}
    }
    let mut values = BTreeMap::new();
    for key in PROFILE_REQUIRED_KEYS {
        if !scan_policy && automatic_exposure_property(health,key) { continue; }
        let p = health
            .properties
            .get(*key)
            .ok_or_else(|| CameraError::Safety(format!("capture profile lacks {key}")))?;
        if !p.confirmed || p.error.is_some() {
            return Err(CameraError::Safety(format!(
                "capture profile readback {key} is unknown or failed"
            )));
        }
        if p.target
            .as_ref()
            .is_some_and(|target| Some(target) != p.actual.as_ref())
        {
            return Err(CameraError::Safety(format!(
                "capture profile readback {key} does not match its applied target"
            )));
        }
        values.insert(
            (*key).into(),
            p.actual.clone().ok_or_else(|| {
                CameraError::Safety(format!("capture profile readback {key} is unknown"))
            })?,
        );
    }
    for (key, p) in &health.properties {
        if p.readable {
            if !scan_policy && automatic_exposure_property(health,key) { continue; }
            if !p.confirmed || p.actual.is_none() {
                return Err(CameraError::Safety(format!(
                    "supported capture parameter {key} is unknown"
                )));
            }
            if p.target
                .as_ref()
                .is_some_and(|target| Some(target) != p.actual.as_ref())
            {
                return Err(CameraError::Safety(format!(
                    "supported capture parameter {key} differs from its applied target"
                )));
            }
            values.insert(key.clone(), p.actual.clone().unwrap());
        }
    }
    for (key, required) in [
        ("exposureMode", 1),
        ("autoIso", 0),
        ("rawFormat", 4),
        ("driveMode", 1),
        ("burstNumber", 1),
        ("recordingMedia", 1),
        ("exposureDelay", 3),
        ("longExposureNoiseReduction", 0),
    ] {
        if !scan_policy && matches!(key,"exposureMode"|"autoIso"|"exposureDelay") { continue; }
        if values.get(key) != Some(&CameraValue::Integer(required)) {
            return Err(CameraError::Safety(format!(
                "scan requires {key}={required}"
            )));
        }
    }
    if health.properties.get("hdrMode").is_some_and(|p| {
        (p.supported || p.error.is_some())
            && (!p.confirmed || p.actual != Some(CameraValue::Integer(0)))
    }) {
        return Err(CameraError::Safety(
            "NEF主机采集要求已声明的HDR属性确认关闭；不得用RAW设置覆盖矛盾/未知HDR状态".into(),
        ));
    }
    if scan_policy && !values
        .get("focusMode")
        .and_then(CameraValue::integer)
        .is_some_and(|mode| matches!(mode, 3 | 4))
    {
        return Err(CameraError::Safety(
            "scan requires physical MF (3) or software MF (4); AF modes are refused".into(),
        ));
    }
    for (key, code, kind) in [
        ("recordingMedia", RECORDING_MEDIA, 2),
        ("burstNumber", BURST_NUMBER, 4),
    ] {
        let property = &health.properties[key];
        if property.code != code || property.data_type != kind {
            return Err(invalid(&format!(
                "capture profile {key} native code/type is unsupported"
            )));
        }
    }
    if scan_policy || !automatic_exposure_property(health,"shutter") { shutter_property_ms(
        health
            .properties
            .get("shutter")
            .ok_or_else(|| CameraError::Safety("scan shutter description missing".into()))?,
    )?; }
    Ok(values)
}
/// Decode the native value by its described PTP property code, never by model.
pub fn shutter_ms(value: &CameraValue, property_code: u16) -> Option<f64> {
    let code = u32::try_from(value.integer()?).ok()?;
    if property_code == 0xd100 {
        let numerator = code >> 16;
        let denominator = code & 0xffff;
        if code >= 0xfffffffd || numerator == 0 || denominator == 0 {
            return None;
        }
        Some(numerator as f64 * 1000.0 / denominator as f64)
    } else if property_code == 0x500d {
        (code > 0 && code != u32::MAX).then_some(code as f64 / 10.0)
    } else {
        None
    }
}
pub fn shutter_property_ms(property: &CameraProperty) -> Result<f64, CameraError> {
    if !property.readable || !property.confirmed || property.actual.is_none() {
        return Err(CameraError::Safety(
            "shutter actual readback is unknown".into(),
        ));
    }
    // Both ExposureTime (0x500D) and Nikon ShutterSpeed (0xD100) are UINT32.
    if property.data_type != 0x0006 || !matches!(property.code, 0x500d | 0xd100) {
        return Err(CameraError::Unsupported(
            "shutter property code/type is not supported".into(),
        ));
    }
    shutter_ms(property.actual.as_ref().unwrap(), property.code)
        .filter(|ms| valid_exposure_ms(*ms))
        .ok_or_else(|| CameraError::Safety("shutter is not a supported timed native value".into()))
}
pub fn exact_shutter_value(
    health: &CameraHealth,
    milliseconds: f64,
) -> Result<CameraValue, CameraError> {
    if !valid_exposure_ms(milliseconds) {
        return Err(CameraError::Safety(
            "timed exposure outside 0.125..30000 ms".into(),
        ));
    }
    let p = health
        .properties
        .get("shutter")
        .ok_or_else(|| CameraError::Unsupported("shutter is not described".into()))?;
    if p.data_type != 0x0006 || !matches!(p.code, 0x500d | 0xd100) {
        return Err(CameraError::Unsupported(
            "shutter property code/type is not supported".into(),
        ));
    }
    p.supported_values
        .iter()
        .find(|value| shutter_ms(value, p.code) == Some(milliseconds))
        .cloned()
        .ok_or_else(|| {
            CameraError::Unsupported(
                "requested exposure has no exact native shutter enum; rounding is forbidden".into(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD_OPS: [u16; 7] = [
        INITIATE_CAPTURE,
        GET_EVENT,
        DEVICE_READY,
        GET_OBJECT_INFO,
        GET_STORAGE_IDS,
        GET_STORAGE_INFO,
        GET_OBJECT_HANDLES,
    ];
    const HOST_OPS: [u16; 7] = [
        CAPTURE_RAM,
        GET_EVENT,
        DEVICE_READY,
        GET_OBJECT_INFO,
        GET_OBJECT,
        GET_PROPERTY,
        GET_PROPERTY_DESC,
    ];

    fn device_info(operations: &[u16], events: &[u16]) -> DeviceInfo {
        DeviceInfo {
            manufacturer: "Nikon Corporation".into(),
            model: "NIKON D7100".into(),
            serial: "8322769".into(),
            operations: operations.to_vec(),
            properties: vec![RECORDING_MEDIA],
            events: events.to_vec(),
            vendor_id: 0,
            diagnostic: CameraDeviceInfo::default(),
        }
    }

    #[test]
    fn card_capture_is_negotiated_separately_from_host_capture() {
        // The RAM host-capture advertisement alone must not enable card capture.
        let host_only = negotiated_capabilities(
            &device_info(&HOST_OPS, &[OBJECT_RAM, CAPTURE_COMPLETE_RAM]),
            true,
        );
        assert!(host_only.host_capture && !host_only.card_capture && !host_only.card_overflow);
        assert!(host_only.disabled_reasons.contains_key("cardCapture"));

        // Either association event missing keeps card capture closed.
        assert!(!negotiated_capabilities(&device_info(&CARD_OPS, &[OBJECT_ADDED]), true).card_capture);
        assert!(
            !negotiated_capabilities(&device_info(&CARD_OPS, &[CAPTURE_COMPLETE_CARD]), true)
                .card_capture
        );
        // One required storage operation missing also keeps it closed.
        let missing = [
            INITIATE_CAPTURE,
            GET_EVENT,
            DEVICE_READY,
            GET_OBJECT_INFO,
            GET_STORAGE_IDS,
            GET_OBJECT_HANDLES,
        ];
        assert!(
            !negotiated_capabilities(&device_info(&missing, &[OBJECT_ADDED, CAPTURE_COMPLETE_CARD]), true)
                .card_capture
        );
        // A non-D7100 identity never negotiates the D7100 card contract.
        assert!(
            !negotiated_capabilities(
                &device_info(&CARD_OPS, &[OBJECT_ADDED, CAPTURE_COMPLETE_CARD, STORE_FULL, STORE_REMOVED]),
                false
            )
            .card_capture
        );
    }

    #[test]
    fn card_overflow_needs_the_storage_notifications_and_stays_optional() {
        let single = negotiated_capabilities(
            &device_info(&CARD_OPS, &[OBJECT_ADDED, CAPTURE_COMPLETE_CARD]),
            true,
        );
        assert!(single.card_capture && !single.card_overflow && !single.host_capture);
        assert!(single.disabled_reasons.contains_key("cardOverflow"));

        let overflow = negotiated_capabilities(
            &device_info(
                &CARD_OPS,
                &[OBJECT_ADDED, CAPTURE_COMPLETE_CARD, STORE_FULL, STORE_REMOVED],
            ),
            true,
        );
        assert!(overflow.card_capture && overflow.card_overflow);
        assert!(!overflow.disabled_reasons.contains_key("cardOverflow"));
    }

    fn health_with_media(actual: Option<i64>, writable: bool, confirmed: bool) -> CameraHealth {
        let mut health = CameraHealth::default();
        health.connected = true;
        health.capabilities.card_capture = true;
        let mut property = unknown_property("recordingMedia", "Recording media", RECORDING_MEDIA, "");
        property.readable = true;
        property.writable = writable;
        property.confirmed = confirmed;
        property.supported_values = vec![CameraValue::Integer(0), CameraValue::Integer(1)];
        property.actual = actual.map(CameraValue::Integer);
        health.properties.insert("recordingMedia".into(), property);
        health
    }

    #[test]
    fn card_destination_requires_a_confirmed_card_readback_and_never_falls_back_to_ram() {
        assert!(require_card_destination(&health_with_media(Some(0), true, true)).is_ok());
        // SDRAM (1), unconfirmed or unwritable evidence and a missing value fail closed.
        assert!(require_card_destination(&health_with_media(Some(1), true, true)).is_err());
        assert!(require_card_destination(&health_with_media(Some(0), false, true)).is_err());
        assert!(require_card_destination(&health_with_media(Some(0), true, false)).is_err());
        assert!(require_card_destination(&health_with_media(None, true, true)).is_err());
        let mut not_advertised = health_with_media(Some(0), true, true);
        not_advertised.capabilities.card_capture = false;
        assert!(require_card_destination(&not_advertised).is_err());
        let mut live_view = health_with_media(Some(0), true, true);
        live_view.live_view.active = true;
        assert!(require_card_destination(&live_view).is_err());
        let mut offline = health_with_media(Some(0), true, true);
        offline.connected = false;
        assert!(require_card_destination(&offline).is_err());
        // A body that does not advertise the card value is refused even when the
        // current readback happens to be zero.
        let mut unsupported = health_with_media(Some(0), true, true);
        unsupported
            .properties
            .get_mut("recordingMedia")
            .unwrap()
            .supported_values = vec![CameraValue::Integer(1)];
        assert!(require_card_destination(&unsupported).is_err());
        // No recording-media evidence at all is not a card destination.
        let mut missing = health_with_media(Some(0), true, true);
        missing.properties.clear();
        assert!(require_card_destination(&missing).is_err());
    }
}
