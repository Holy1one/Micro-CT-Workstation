//! Necessary Nikon D7100 protocol adapted from digiCamControl's MIT Nikon
//! classes, Copyright (C) 2014 Duka Istvan. See THIRD-PARTY-NOTICES.md.
//! WPD transport is implemented independently against Microsoft's interface.

use super::ptp::*;
use super::types::*;
use std::collections::BTreeMap;
use std::thread;
use std::time::Duration;

pub(crate) const CAPTURE_RAM: u16 = 0x90c0;
pub(crate) const AF_DRIVE: u16 = 0x90c1;
pub(crate) const GET_EVENT: u16 = 0x90c7;
pub(crate) const DEVICE_READY: u16 = 0x90c8;
pub(crate) const START_LV: u16 = 0x9201;
pub(crate) const STOP_LV: u16 = 0x9202;
pub(crate) const GET_LV: u16 = 0x9203;
pub(crate) const FOCUS_DRIVE: u16 = 0x9204;
pub(crate) const OBJECT_RAM: u16 = 0xc101;
pub(crate) const CAPTURE_COMPLETE_RAM: u16 = 0xc102;
pub(crate) const LV_STATUS: u16 = 0xd1a2;
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
    "exposureDelay",
    "longExposureNoiseReduction",
];
pub(crate) const NIKON_PROPERTIES: &[(&str, &str, u16)] = &[
    ("exposureMode", "Exposure mode", 0x500e),
    ("autoIso", "Auto ISO", 0xd054),
    ("rawFormat", "Image format (4 = NEF only)", 0x5004),
    ("driveMode", "Release mode (1 = single)", 0x5013),
    ("focusMode", "Focus mode (4 = manual)", 0xd161),
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
];

pub(crate) struct NativeCamera {
    pub transport: Box<dyn CameraTransport>,
    pub health: CameraHealth,
    pub info: Option<DeviceInfo>,
    pub d7100: bool,
    pub capture_pending: bool,
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
        self.transport.open(id, context)?;
        let info = parse_device_info(
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
        self.d7100 = info.vendor_id == 0x0a
            && info.manufacturer.to_ascii_lowercase().contains("nikon")
            && matches!(
                info.model.trim().to_ascii_uppercase().as_str(),
                "D7100" | "NIKON D7100"
            );
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
        self.health.live_view = LiveViewStatus::default();
        self.health.connected = true;
        self.health.isolated = false;
        self.health.cleanup_confirmed = true;
        self.health.transfer_policy = self
            .d7100
            .then(|| "Nikon host RAM; no card fallback".into());
        let has = |code| info.operations.contains(&code);
        let host_capture = self.d7100
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
        self.health.capabilities = CameraCapabilities {
            properties: has(GET_PROPERTY_DESC) && has(GET_PROPERTY),
            host_capture,
            live_view: self.d7100
                && [START_LV, STOP_LV, GET_LV, GET_PROPERTY]
                    .iter()
                    .all(|code| has(*code))
                && info.properties.contains(&LV_STATUS),
            autofocus: self.d7100 && has(AF_DRIVE),
            focus_step: self.d7100 && has(FOCUS_DRIVE),
            scan_nef: host_capture,
            hardware_certified: false,
            disabled_reasons: BTreeMap::new(),
        };
        self.health.capabilities.disabled_reasons.insert("hardwareCertification".into(), "BLOCKED: D7100 event semantics and real capture require authorized hardware acceptance".into());
        if !host_capture {
            self.health.capabilities.disabled_reasons.insert("capture".into(), "Host-only RAM capture and its correlated completion events are not proven for this identity/capability set".into());
        }
        if !self.health.capabilities.live_view {
            self.health.capabilities.disabled_reasons.insert(
                "liveView".into(),
                "D7100 Live View operation/property set not advertised".into(),
            );
        }
        self.info = Some(info);
        self.read_properties(context)?;
        if self.d7100 && self.info.as_ref().unwrap().properties.contains(&LV_STATUS) {
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
                return Err(CameraError::Safety("camera already has Live View or unknown Live View status; close it before connection".into()));
            }
        }
        self.health.phase = "idle".into();
        self.health.last_seen_at = Some(chrono::Utc::now().to_rfc3339());
        self.health.last_error = None;
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
    pub fn read_properties(&mut self, context: &CallContext) -> Result<(), CameraError> {
        self.ensure_connected()?;
        if !self.health.capabilities.properties {
            return Err(CameraError::Unsupported(
                "standard property operations not advertised".into(),
            ));
        }
        let list = if self.d7100 {
            NIKON_PROPERTIES
        } else {
            STANDARD_PROPERTIES
        };
        for &(key, label, code) in list {
            context.check()?;
            if !self.info.as_ref().unwrap().properties.contains(&code) {
                continue;
            }
            let old_target = self
                .health
                .properties
                .get(key)
                .and_then(|p| p.target.clone());
            let result = self
                .transport
                .execute(
                    GET_PROPERTY_DESC,
                    &[code as u32],
                    DataPhase::Read { max: 128 * 1024 },
                    context,
                )?
                .data()
                .and_then(|b| parse_property(&b, key, label, code));
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
                    match self
                        .transport
                        .execute(
                            GET_PROPERTY,
                            &[code as u32],
                            DataPhase::Read { max: 4096 },
                            context,
                        )?
                        .data()
                        .and_then(|b| parse_value(&b, p.data_type))
                    {
                        Ok(actual) => {
                            p.actual = Some(actual);
                            p.confirmed = true;
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
                    self.health.properties.insert(
                        key.into(),
                        CameraProperty {
                            key: key.into(),
                            label: label.into(),
                            code,
                            data_type: 0,
                            readable: false,
                            writable: false,
                            supported_values: vec![],
                            range: None,
                            actual: None,
                            target: old_target,
                            confirmed: false,
                            error: Some(e.to_string()),
                        },
                    );
                }
            }
        }
        let times: Vec<f64> = self
            .health
            .properties
            .get("shutter")
            .into_iter()
            .flat_map(|p| &p.supported_values)
            .filter_map(|v| shutter_ms(v, self.d7100))
            .filter(|v| valid_exposure_ms(*v))
            .collect();
        self.health.exposure_min_ms = times.iter().copied().reduce(f64::min);
        self.health.exposure_max_ms = times.iter().copied().reduce(f64::max);
        Ok(())
    }
    pub fn apply(
        &mut self,
        values: &CameraProperties,
        context: &CallContext,
    ) -> Result<PropertyApplyResult, CameraError> {
        self.ensure_connected()?;
        validate_properties(&self.health, values)?;
        let mut applied = vec![];
        let mut failed = BTreeMap::new();
        // Exposure mode/auto modes precede shutter and lens-dependent properties.
        let order: Vec<String> = NIKON_PROPERTIES
            .iter()
            .map(|p| p.0.to_string())
            .filter(|key| values.contains_key(key))
            .collect();
        for key in order {
            context.check()?;
            let value = &values[&key];
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
        }
        // Unattempted fields remain actual evidence, with their requested draft target.
        for (key, value) in values {
            self.health.properties.get_mut(key).unwrap().target = Some(value.clone());
        }
        let complete = applied.len() == values.len() && failed.is_empty();
        if !complete {
            self.health.last_error = Some("parameter application partially failed; actual readback retained; preflight invalidated".into());
        }
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
        parse_nikon_events(
            &self
                .transport
                .execute(GET_EVENT, &[], DataPhase::Read { max: 8192 }, context)?
                .data()?,
        )
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
        }
        Err(CameraError::Safety(
            "Nikon event queue did not become empty".into(),
        ))
    }
}

pub fn validate_properties(
    health: &CameraHealth,
    values: &CameraProperties,
) -> Result<(), CameraError> {
    if !health.connected || health.isolated {
        return Err(CameraError::Unavailable(
            "no selected camera session".into(),
        ));
    }
    if values.is_empty() || values.len() > 32 {
        return Err(CameraError::Safety(
            "camera Apply requires 1..32 described properties".into(),
        ));
    }
    for (key, value) in values {
        let p = health.properties.get(key).ok_or_else(|| {
            CameraError::Unsupported(format!("property {key} is not described by this camera"))
        })?;
        if !accepts(p, value) {
            return Err(CameraError::Unsupported(format!(
                "property {key} is not writable or value is outside the native enumeration/range"
            )));
        }
    }
    Ok(())
}
pub fn profile_readback(health: &CameraHealth) -> Result<CameraProperties, CameraError> {
    if !health.connected || health.isolated || !health.capabilities.scan_nef {
        return Err(CameraError::Safety(
            "D7100 native NEF session required".into(),
        ));
    }
    let mut values = BTreeMap::new();
    for key in PROFILE_REQUIRED_KEYS {
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
        ("focusMode", 4),
        ("exposureDelay", 3),
        ("longExposureNoiseReduction", 0),
    ] {
        if values.get(key) != Some(&CameraValue::Integer(required)) {
            return Err(CameraError::Safety(format!(
                "scan requires {key}={required}"
            )));
        }
    }
    if !values
        .get("shutter")
        .and_then(|v| shutter_ms(v, true))
        .is_some_and(valid_exposure_ms)
    {
        return Err(CameraError::Safety(
            "scan requires an exact supported timed shutter".into(),
        ));
    }
    Ok(values)
}
pub fn shutter_ms(value: &CameraValue, nikon_rational: bool) -> Option<f64> {
    let code = u32::try_from(value.integer()?).ok()?;
    if nikon_rational {
        let numerator = code >> 16;
        let denominator = code & 0xffff;
        if code >= 0xfffffffd || numerator == 0 || denominator == 0 {
            return None;
        }
        Some(numerator as f64 * 1000.0 / denominator as f64)
    } else {
        (code > 0 && code != u32::MAX).then_some(code as f64 / 10.0)
    }
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
    let nikon = health.backend == "windows_wpd_nikon_d7100";
    p.supported_values
        .iter()
        .find(|value| shutter_ms(value, nikon) == Some(milliseconds))
        .cloned()
        .ok_or_else(|| {
            CameraError::Unsupported(
                "requested exposure has no exact native shutter enum; rounding is forbidden".into(),
            )
        })
}
