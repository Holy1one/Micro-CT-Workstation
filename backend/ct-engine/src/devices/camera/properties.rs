//! Descriptor-backed Nikon photographic controls. Codes/labels derive from the
//! MIT D7100 -> D600Base -> D800 -> NikonBase chain; no catalog value is support
//! evidence until the selected camera's descriptor admits it.
use super::nikon::{shutter_ms, NIKON_PROPERTIES};
use super::ptp::{accepts, encode_value, SET_PROPERTY};
use super::types::*;
use std::collections::BTreeMap;
pub(crate) fn live_control_type_allowed(key:&str,kind:u16)->bool{
    match key {"cameraZoom"=>matches!(kind,1..=8),"focusMode"=>kind==2,_=>false}
}
pub(crate) fn clear_live_view_controls(health:&mut CameraHealth,reason:&str){
    health.live_view_controls.session_id=None;
    for property in health.live_view_controls.properties.values_mut(){property.writable=false;property.effective=false;property.disabled_reason=Some(reason.into());for option in &mut property.options{option.enabled=false;option.reason=Some(reason.into());}}
    health.capabilities.focus_step=false;
    health.capabilities.autofocus=false;
}
pub(crate) fn refresh_live_view_controls(health:&mut CameraHealth){
    let current=health.connected&&!health.isolated&&health.live_view.active&&health.live_view_controls.session_id.is_some()&&health.live_view_controls.session_id==health.live_view.session_id;
    let writable=health.device_info.as_ref().is_some_and(|info|info.operations.contains(&SET_PROPERTY));
    for (key,property) in &mut health.live_view_controls.properties{
        let physical_mf=key=="focusMode"&&property.actual.as_ref().and_then(CameraValue::integer)==Some(3);
        let unknown_focus=key=="focusMode"&&!matches!(property.actual.as_ref().and_then(CameraValue::integer),Some(0|2|3|4));
        property.effective=current&&property.supported&&live_control_type_allowed(key,property.data_type)&&property.readable&&property.confirmed&&property.error.is_none()&&!physical_mf&&!unknown_focus;
        property.writable=property.effective&&property.actual.is_some()&&property.native_writable&&writable&&(!property.supported_values.is_empty()||property.range.is_some());
        property.disabled_reason=if !current{Some("当前Live View会话已停止/未确认".into())}else if !property.supported{property.disabled_reason.clone()}else if physical_mf{Some("LV物理MF开关锁定；不能用目标值绕过".into())}else if unknown_focus{Some("LV焦点模式原生值语义未知，禁止修改".into())}else if !property.confirmed||property.error.is_some(){Some("LV实际读回未确认或与目标不一致；请停止后重新启动Live View建立新控制会话".into())}else if !property.writable{Some("设备LV描述只读或缺少可编辑枚举/范围".into())}else{None};
        let known:&[(i64,&str)]=if key=="focusMode"{&[(0,"AF-S"),(2,"AF-F"),(3,"物理MF（只读）"),(4,"软件MF")]}else{&[(0,"完整画面"),(1,"25%"),(2,"33%"),(3,"50%"),(4,"66%"),(5,"100%"),(6,"200%") ]};
        let mut candidates=known.iter().map(|(v,label)|(CameraValue::Integer(*v),(*label).to_owned())).collect::<Vec<_>>();
        for value in &property.supported_values{if !candidates.iter().any(|(v,_)|v==value){candidates.push((value.clone(),format!("原生值 {:?}",value)));}}
        property.options=candidates.into_iter().map(|(value,label)|{let supported=property.supported&&live_control_type_allowed(key,property.data_type)&&value_supported(property,&value);let allowed=key!="focusMode"||matches!(value.integer(),Some(0|2|4));let enabled=supported&&property.writable&&allowed;
            CameraPropertyOption{value,label,supported,enabled,reason:if enabled{None}else{Some(property.disabled_reason.clone().unwrap_or_else(||if !supported{"当前LV枚举/范围未支持该值".into()}else{"该LV模式只读或语义不支持修改".into()}))}}
        }).collect();
    }
}

pub(crate) fn unknown_property(key: &str, label: &str, code: u16, reason: &str) -> CameraProperty {
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
        disabled_reason: Some(reason.into()),
        options: vec![],
        supported_values: vec![],
        range: None,
        actual: None,
        target: None,
        confirmed: false,
        error: None,
    }
}

fn actual(values: &BTreeMap<String, CameraProperty>, key: &str) -> Option<i64> {
    values
        .get(key)
        .filter(|p| p.supported && p.readable && p.confirmed && p.error.is_none())
        .and_then(|p| p.actual.as_ref())
        .and_then(CameraValue::integer)
}

pub(crate) fn dependency_reason(
    key: &str,
    values: &BTreeMap<String, CameraProperty>,
) -> Option<String> {
    let check = |parent: &str, permitted: &[i64], reason: &str| match actual(values, parent) {
        Some(value) if permitted.contains(&value) => None,
        Some(_) => Some(reason.into()),
        None => Some(format!("{parent}实际值未确认，暂不可修改此参数")),
    };
    // D7100 manual pp.81,89,115,133,321-323. A native writable flag
    // does not make a setting effective in AUTO/scene/effects modes.
    if matches!(key, "autoIso" | "autoIsoHighLimit" | "whiteBalance"
        | "whiteBalanceFluorescent" | "whiteBalanceTemperature" | "pictureControl"
        | "meteringMode" | "centerWeightedArea" | "activeDLighting"
        | "hdrMode" | "hdrExposureDifference" | "hdrSmoothing"
        | "bracketing" | "flashCompensation") {
        if let Some(reason) = check("exposureMode", &[1, 2, 3, 4],
            "This control requires confirmed P/S/A/M mode; select the mode on the camera or Apply it, then refresh settings") {
            return Some(reason);
        }
    }
    match key {
        "bracketing"=>values.get(key).filter(|p|p.code!=0xd0c0||p.data_type!=2).map(|_|"D7100包围属性原生代码/UINT8类型未确认，禁止修改或推断OFF".into()),
        "shutter" => check(
            "exposureMode",
            &[1, 4],
            "当前模式由相机决定快门；M/S模式可手动设置",
        ),
        "aperture" => check(
            "exposureMode",
            &[1, 3],
            "当前模式由相机决定光圈；M/A模式可手动设置",
        ),
        "rawBitDepth" | "rawCompression" => check(
            "rawFormat",
            &[4, 5, 6, 7],
            "当前图像格式不包含NEF，RAW参数不生效",
        ),
        "imageSize" => check(
            "rawFormat",
            &[0, 1, 2, 5, 6, 7],
            "NEF使用固定RAW尺寸；此项只控制JPEG图像尺寸",
        ),
        "whiteBalanceFluorescent" => check("whiteBalance", &[5], "请先选择荧光灯白平衡并Apply"),
        "whiteBalanceTemperature" => check("whiteBalance", &[0x8012], "请先选择色温白平衡并Apply"),
        "autoIsoHighLimit" => check("autoIso", &[1], "自动ISO关闭，自动ISO上限不生效"),
        "centerWeightedArea" => check("meteringMode", &[2], "请先选择中央重点测光并Apply"),
        "exposureCompensation" => match actual(values, "exposureMode") {
            Some(0x8010 | 0x8011 | 0x8012 | 0x8013 | 0x8014 | 0x8016 | 0x8017 | 0x8018) =>
                Some("Exposure compensation is unavailable in this automatic/scene mode".into()),
            None => Some("Exposure mode is unconfirmed; refresh settings before adjusting exposure compensation".into()),
            _ => None, // Night-vision/effects exceptions remain descriptor-backed.
        },
        "hdrExposureDifference" | "hdrSmoothing" => {
            check("rawFormat", &[0, 1, 2], "NEF格式不可使用HDR")
                .or_else(|| check("hdrMode", &[1, 2], "HDR关闭，HDR子参数不生效"))
        }
        "focusMode" if actual(values, "focusMode") == Some(3) => {
            Some("物理MF开关锁定；请在镜头或机身切换后重新读取".into())
        }
        "flashCompensation" | "flashSyncSpeed" => hdr_off_reason(values),
        _ => None,
    }
}

/// Unadvertised HDR is not machine OFF. Confirmed NEF incompatibility is
/// sufficient only when the HDR descriptor was genuinely unadvertised.
fn hdr_off_reason(properties: &BTreeMap<String, CameraProperty>) -> Option<String> {
    match actual(properties, "hdrMode") {
        Some(0) => None,
        Some(1 | 2) => Some("HDR is on; turn it Off, Apply and verify before using this setting".into()),
        _ if properties.get("hdrMode").is_some_and(|p| !p.supported && !p.readable && p.error.is_none())
            && matches!(actual(properties, "rawFormat"), Some(4..=7)) => None,
        _ => Some("HDR readback is unconfirmed; refresh settings before using this control".into()),
    }
}

pub(crate) fn option_reason(
    key: &str,
    value: &CameraValue,
    code: u16,
    properties: &BTreeMap<String, CameraProperty>,
) -> Option<String> {
    if key == "focusMode" && code == 0xd161 && value.integer() == Some(3) {
        return Some("物理MF由机身/镜头开关选择，只读显示；软件目标请使用受支持的软件MF".into());
    }
    if key == "shutter" && shutter_ms(value, code).is_none() {
        return Some("Bulb/Time/闪光同步特殊值未实现有界主机曝光，暂不支持".into());
    }
    if key == "hdrMode" && value.integer() != Some(0) {
        return match actual(properties, "rawFormat") {
            Some(0 | 1 | 2) => None,
            Some(_) => Some("HDR不能与NEF/NEF+JPEG格式同时启用；OFF仍可用".into()),
            None => Some("图像格式实际值未确认，不能启用HDR".into()),
        }.or_else(|| match actual(properties, "bracketing") {
            Some(0) => None,
            Some(_) => Some("Bracketing is on; turn it Off, Apply and verify before enabling HDR".into()),
            None => Some("Bracketing readback is unconfirmed; refresh settings before enabling HDR".into()),
        });
    }
    if key == "rawFormat" && matches!(value.integer(), Some(4..=7)) {
        if properties.get("hdrMode").is_some_and(|p| !p.supported && !p.readable && p.error.is_none()) {
            return None; // No private probe or invented HDR state.
        }
        return hdr_off_reason(properties);
    }
    // OFF stays available to leave an incompatible combination. NEF does not
    // disable every kind of bracketing: only WB bracketing is incompatible.
    if key == "bracketing" && value.integer() != Some(0)
        || key == "flashMode" && value.integer() != Some(2) {
        if let Some(reason) = hdr_off_reason(properties) { return Some(reason); }
    }
    if key == "flashMode" && matches!(value.integer(), Some(0x8011 | 0x8013)) {
        return match actual(properties, "exposureMode") {
            Some(2 | 3) => None,
            Some(1 | 4) => Some("Slow sync and red-eye slow sync require P/A mode; they are unavailable in S/M".into()),
            Some(_) => None, // Scene-specific choices remain descriptor-backed.
            None => Some("Exposure mode is unconfirmed; refresh settings before choosing slow sync".into()),
        };
    }
    None
}

pub(crate) fn value_supported(property: &CameraProperty, value: &CameraValue) -> bool {
    let mut native = property.clone();
    native.writable = true;
    property.supported && accepts(&native, value)
}

pub(crate) fn refresh_property_controls(health: &mut CameraHealth) {
    let snapshot = health.properties.clone();
    let nikon = health.backend == "windows_wpd_nikon_d7100";
    let setter_advertised = health
        .device_info
        .as_ref()
        .is_some_and(|info| info.operations.contains(&SET_PROPERTY));
    for (key, property) in &mut health.properties {
        let dependency = if nikon || matches!(key.as_str(), "shutter" | "aperture") {
            dependency_reason(key, &snapshot)
        } else {
            None
        };
        property.effective = property.supported
            && property.readable
            && property.confirmed
            && dependency.is_none()
            && health.connected
            && !health.isolated
            && !health.live_view.active;
        let descriptor_editable = property.native_writable
            && (!property.supported_values.is_empty() || property.range.is_some());
        property.writable = property.supported
            && property.confirmed
            && property.actual.is_some()
            && descriptor_editable
            && dependency.is_none()
            && setter_advertised
            && health.connected
            && !health.isolated
            && !health.live_view.active;
        property.disabled_reason = if !property.supported {
            property
                .disabled_reason
                .clone()
                .or_else(|| Some("当前相机未提供可验证的属性描述".into()))
        } else if !health.connected || health.isolated {
            Some("相机会话不可用，需重新连接并读取".into())
        } else if health.live_view.active {
            Some("Live View开启期间禁止参数Apply；请先明确停止预览".into())
        } else if !property.confirmed || property.actual.is_none() {
            Some("属性实际读回未确认，需成功重新读取后修改".into())
        } else if let Some(reason) = dependency {
            Some(reason)
        } else if !setter_advertised {
            Some("相机未声明属性写入操作".into())
        } else if !property.native_writable {
            Some("相机当前属性描述为只读；需检查机身模式、镜头或控制状态".into())
        } else if !descriptor_editable {
            Some("相机未描述可安全编辑的枚举或范围".into())
        } else {
            None
        };
        if key == "iso" && nikon {
            property.label = if actual(&snapshot, "autoIso") == Some(1) {
                "ISO（自动ISO基准值，非实测曝光ISO）".into()
            } else {
                "ISO".into()
            };
        }
        let mut candidates = if nikon { known_options(key) } else { vec![] };
        for value in &property.supported_values {
            if !candidates.iter().any(|(known, _)| known == value) {
                candidates.push((value.clone(), value_label(key, value, property.code)));
            }
        }
        property.options = candidates
            .into_iter()
            .map(|(value, label)| {
                let supported = value_supported(property, &value);
                let reason = if !supported {
                    Some("当前设备枚举/范围未声明此选项".into())
                } else if !property.writable {
                    property.disabled_reason.clone()
                } else {
                    if nikon || key == "shutter" {
                        option_reason(key, &value, property.code, &snapshot)
                    } else {
                        None
                    }
                };
                CameraPropertyOption {
                    value,
                    label,
                    supported,
                    enabled: supported && reason.is_none(),
                    reason,
                }
            })
            .collect();
    }
    if let Some(property)=health.properties.get_mut("bracketing") {
        if property.code!=0xd0c0||property.data_type!=2 {
            property.effective=false;property.writable=false;property.disabled_reason=Some("D7100包围属性原生代码/UINT8类型未确认".into());
            for option in &mut property.options{option.label=format!("原生值 {:?}（包围类型未验证）",option.value);option.enabled=false;option.reason=property.disabled_reason.clone();}
        }
    }
    if health.capabilities.card_capture {
        if let Err(error)=super::card_capture::card_capture_readiness(health){
            health.capabilities.disabled_reasons.insert("cardCaptureReady".into(),error.to_string());
        }else{health.capabilities.disabled_reasons.remove("cardCaptureReady");}
    }else{health.capabilities.disabled_reasons.remove("cardCaptureReady");}
    if health.capabilities.host_capture {
        // One shared admission decision for both the published `captureReady`
        // reason and the domain-entry pre-enqueue gate, so the two can never
        // disagree about whether a host NEF capture is ready.
        let readiness = capture_readiness(health);
        if let Err(error) = readiness {
            health
                .capabilities
                .disabled_reasons
                .insert("captureReady".into(), error.to_string());
        } else {
            health.capabilities.disabled_reasons.remove("captureReady");
        }
    } else {
        health.capabilities.disabled_reasons.remove("captureReady");
    }
}

/// Native descriptors remain mandatory even for a dependency-changing draft.
/// A confirmed no-op never needs a write permission or an active dependency.
/// Why a parameter write was refused before any hardware write happened. The
/// domain entry surfaces [`PropertyRejection::code`] so a frontend can explain a
/// rejection instead of collapsing every cause into one opaque error, while
/// [`PropertyRejection::message`] keeps the operator-facing diagnostic text
/// unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyRejection {
    SessionUnavailable,
    Count,
    LiveViewForbidden,
    Undescribed { key: String },
    BracketingDescription,
    FocusModeLocked,
    WriteOperationUnsupported,
    UnsupportedValue { key: String },
    Dependency { key: String, reason: String },
}
impl PropertyRejection {
    pub fn code(&self) -> &'static str {
        match self {
            Self::SessionUnavailable => "CAMERA_PARAMETER_SESSION_UNAVAILABLE",
            Self::Count => "INVALID_CAMERA_PARAMETER_COUNT",
            Self::LiveViewForbidden => "LIVE_VIEW_APPLY_FORBIDDEN",
            Self::Undescribed { .. } | Self::BracketingDescription => "CAMERA_PROPERTY_UNDESCRIBED",
            Self::FocusModeLocked => "CAMERA_FOCUS_MODE_LOCKED",
            Self::WriteOperationUnsupported => "CAMERA_PROPERTY_WRITE_UNSUPPORTED",
            Self::UnsupportedValue { .. } => "CAMERA_PROPERTY_UNSUPPORTED_VALUE",
            Self::Dependency { .. } => "CAMERA_PROPERTY_DEPENDENCY_UNMET",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::SessionUnavailable => "no selected camera session".into(),
            Self::Count => "camera Apply requires 1..64 described properties".into(),
            Self::LiveViewForbidden => {
                "Live View开启期间禁止普通摄影参数Apply；请先明确停止预览".into()
            }
            Self::Undescribed { key } => format!("属性{key}没有可验证描述"),
            Self::BracketingDescription => {
                "D7100包围属性必须经当前广告及UINT8描述验证后明确Apply".into()
            }
            Self::FocusModeLocked => "物理MF开关锁定，禁止用草稿目标绕过硬件模式".into(),
            Self::WriteOperationUnsupported => "Live View开启或相机未声明属性写入操作".into(),
            Self::UnsupportedValue { key } => format!(
                "属性{key}不可写或选项未获当前设备描述支持；模式变化请先Apply后重新读取"
            ),
            Self::Dependency { key, reason } => format!("属性{key}：{reason}"),
        }
    }
    /// Keep the previous `CameraError` taxonomy and wording for internal callers
    /// that still reason in `CameraError` terms.
    pub fn into_camera_error(self) -> CameraError {
        match self {
            Self::SessionUnavailable => CameraError::Unavailable(self.message()),
            Self::Count => CameraError::Safety(self.message()),
            _ => CameraError::Unsupported(self.message()),
        }
    }
}

/// Full pre-write admission decision, including the session, Live View and count
/// gates. The domain entry classifies a rejection from this result.
pub fn property_rejection(
    health: &CameraHealth,
    values: &CameraProperties,
) -> Result<(), PropertyRejection> {
    if !health.connected || health.isolated {
        return Err(PropertyRejection::SessionUnavailable);
    }
    if health.live_view.active {
        return Err(PropertyRejection::LiveViewForbidden);
    }
    if values.is_empty() || values.len() > 64 {
        return Err(PropertyRejection::Count);
    }
    validate_control_values(health, values)
}

/// The same preconditions the capture worker applies before a host NEF trigger
/// that can be evaluated without a device round-trip: the policy readback, Live
/// View OFF, and a confirmed read-only `infoDisplayErrorStatus` runtime
/// diagnostic. The worker additionally performs a *fresh* `recordingMedia`
/// readback and a fresh LV_STATUS read (see `confirm_runtime_diagnostic`), which
/// by definition cannot be proven before enqueue; those remain worker-only and a
/// change detected there still fails the operation fail-closed.
/// Strict, fresh-read evidence that the camera SD card is this capture's destination.
/// D7100 `recordingMedia` 0 selects the card and 1 selects SDRAM; this gate accepts
/// only a confirmed 0 and never falls back to RAM. Missing, unreadable, unwritable,
/// unsupported or drifted evidence fails closed.
pub(crate) fn require_card_destination(health: &CameraHealth) -> Result<(), CameraError> {
    if !health.connected || health.isolated {
        return Err(CameraError::Unsupported("camera is not connected".into()));
    }
    if !health.capabilities.card_capture {
        return Err(CameraError::Unsupported(
            "card capture operations and association events are not advertised".into(),
        ));
    }
    if health.live_view.active {
        return Err(CameraError::Unsupported(
            "Live View must be stopped before a card capture".into(),
        ));
    }
    let property = health
        .properties
        .get("recordingMedia")
        .ok_or_else(|| CameraError::Unsupported("recording media property was not read".into()))?;
    if !property.readable || !property.writable || !property.confirmed || property.error.is_some() {
        return Err(CameraError::Unsupported(
            "recording media destination is not confirmed readback".into(),
        ));
    }
    // D7100 can describe D10B as UINT8 range [0, 2, 1] with no enum list.
    // Use the same descriptor admission as Apply: both forms must prove 0.
    if !value_supported(property, &CameraValue::Integer(0)) {
        return Err(CameraError::Unsupported(
            "this body does not advertise the card destination".into(),
        ));
    }
    if property.actual != Some(CameraValue::Integer(0)) {
        return Err(CameraError::Unsupported(
            "recording media is not set to the card destination".into(),
        ));
    }
    Ok(())
}

pub fn capture_readiness(health: &CameraHealth) -> Result<(), CameraError> {
    super::nikon::independent_readback(health)?;
    if health.live_view.active {
        return Err(CameraError::Safety(
            "Live View开启期间禁止拍摄；请先明确停止预览".into(),
        ));
    }
    require_capture_runtime_diagnostic(health)
}

pub(crate) fn require_capture_runtime_diagnostic(health:&CameraHealth)->Result<(),CameraError>{
    let diagnostic = health
        .runtime_properties
        .get("infoDisplayErrorStatus")
        .ok_or_else(|| CameraError::Safety("运行诊断尚未成功读取，拍摄前需重新确认".into()))?;
    if diagnostic.data_type != 2
        || diagnostic.writable
        || !diagnostic.confirmed
        || diagnostic.error.is_some()
        || diagnostic
            .actual
            .as_ref()
            .and_then(CameraValue::integer)
            .is_none()
    {
        return Err(CameraError::Safety(
            "运行诊断的类型、只读描述或实际读回未确认".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_control_values(
    health: &CameraHealth,
    values: &CameraProperties,
) -> Result<(), PropertyRejection> {
    let mut projected = health.properties.clone();
    for (key, value) in values {
        if let Some(property) = projected.get_mut(key) {
            property.actual = Some(value.clone());
            property.confirmed = true;
            property.error = None;
        }
    }
    for (key, value) in values {
        let property = health
            .properties
            .get(key)
            .ok_or_else(|| PropertyRejection::Undescribed { key: key.clone() })?;
        if key=="bracketing"&&(property.code!=0xd0c0||property.data_type!=2){return Err(PropertyRejection::BracketingDescription);}
        if property.confirmed && property.error.is_none() && property.actual.as_ref() == Some(value)
        {
            continue;
        }
        if key == "focusMode" && property.actual.as_ref().and_then(CameraValue::integer) == Some(3)
        {
            return Err(PropertyRejection::FocusModeLocked);
        }
        if health.live_view.active
            || !health
                .device_info
                .as_ref()
                .is_some_and(|info| info.operations.contains(&SET_PROPERTY))
        {
            return Err(PropertyRejection::WriteOperationUnsupported);
        }
        if !property.supported
            || !property.confirmed
            || property.actual.is_none()
            || !property.native_writable
            || !value_supported(property, value)
            || encode_value(value, property.data_type).is_err()
        {
            return Err(PropertyRejection::UnsupportedValue { key: key.clone() });
        }
        let nikon = health.backend == "windows_wpd_nikon_d7100";
        let dependency = if nikon || matches!(key.as_str(), "shutter" | "aperture") {
            dependency_reason(key, &projected)
        } else {
            None
        };
        if let Some(reason) = dependency.or_else(|| {
            if nikon || key == "shutter" {
                option_reason(key, value, property.code, &projected)
            } else {
                None
            }
        }) {
            return Err(PropertyRejection::Dependency { key: key.clone(), reason });
        }
    }
    Ok(())
}

pub(crate) fn application_order(values: &CameraProperties) -> Vec<String> {
    const PARENTS: &[&str] = &[
        "exposureMode",
        "autoIso",
        "rawFormat",
        "crop",
        "whiteBalance",
        "meteringMode",
        "hdrMode",
    ];
    // Explicit OFF requests must precede entering the mutually exclusive mode.
    // Nothing is added to the request, and every subsequent write is revalidated
    // against fresh descriptors/readback in NativeCamera::apply.
    let exits = ["hdrMode", "bracketing"].into_iter()
        .filter(|key| values.get(*key).and_then(CameraValue::integer) == Some(0))
        .collect::<Vec<_>>();
    let remaining = PARENTS
        .iter()
        .copied()
        .chain(
            NIKON_PROPERTIES
                .iter()
                .map(|entry| entry.0)
                .filter(|key| !PARENTS.contains(key)),
        )
        .filter(|key| !exits.contains(key));
    exits.iter().copied().chain(remaining)
        .filter(|key| values.contains_key(*key))
        .map(str::to_string)
        .collect()
}
pub(crate) fn changes_dependencies(key: &str) -> bool {
    matches!(
        key,
        "exposureMode"
            | "autoIso"
            | "rawFormat"
            | "crop"
            | "whiteBalance"
            | "meteringMode"
            | "hdrMode"
            | "bracketing"
            | "flashMode"
    )
}

fn value_label(key: &str, value: &CameraValue, code: u16) -> String {
    let Some(n) = value.integer() else {
        return match value {
            CameraValue::Text(text) => text.clone(),
            _ => unreachable!(),
        };
    };
    match key {
        "shutter" => shutter_ms(value, code)
            .map(|ms| {
                if ms < 1000.0 {
                    format!("1/{:.4} s", 1000.0 / ms)
                } else {
                    format!("{:.4} s", ms / 1000.0)
                }
            })
            .unwrap_or_else(|| match n {
                0xffffffff => "Bulb（暂不支持）".into(),
                0xfffffffe => "Time（暂不支持）".into(),
                _ => format!("特殊快门值 {n}（暂不支持）"),
            }),
        "aperture" => format!("f/{:.2}", n as f64 / 100.0),
        "exposureCompensation" => format!("{:+.3} EV", n as f64 / 1000.0),
        "flashCompensation" => format!("{:+.2} EV", n as f64 / 6.0),
        "iso" => match n {
            8000 => "Hi 0.3".into(),
            9000 => "Hi 0.5".into(),
            10000 => "Hi 0.7".into(),
            12800 => "Hi 1".into(),
            25600 => "Hi 2".into(),
            _ => format!("ISO {n}"),
        },
        "burstNumber" => format!("{n}张"),
        _ => format!("原生值 {n}"),
    }
}

fn known_options(key: &str) -> Vec<(CameraValue, String)> {
    let pairs: &[(i64, &str)] = match key {
        "exposureMode" => &[
            (1, "M 手动"),
            (2, "P 程序自动"),
            (3, "A 光圈优先"),
            (4, "S 快门优先"),
            (0x8010, "AUTO"),
            (0x8011, "人像"),
            (0x8012, "风景"),
            (0x8013, "微距"),
            (0x8014, "运动"),
            (0x8016, "禁用闪光AUTO"),
            (0x8017, "儿童"),
            (0x8018, "SCENE"),
            (0x8019, "EFFECTS"),
        ],
        "bracketing" | "autoIso" | "longExposureNoiseReduction" => &[(0, "关闭"), (1, "开启")],
        "rawFormat" => &[
            (0, "JPEG Basic"),
            (1, "JPEG Normal"),
            (2, "JPEG Fine"),
            (3, "TIFF RGB（仅设备声明时可用）"),
            (4, "NEF RAW"),
            (5, "NEF+JPEG Basic"),
            (6, "NEF+JPEG Normal"),
            (7, "NEF+JPEG Fine"),
        ],
        "driveMode" => &[
            (1, "单张"),
            (2, "CH 连拍高速"),
            (0x8010, "CL 连拍低速"),
            (0x8011, "自拍"),
            (0x8012, "反光板抬起"),
            (0x8016, "安静"),
            (0x8017, "遥控"),
        ],
        "focusMode" => &[
            (0, "AF-S"),
            (1, "AF-C"),
            (2, "AF-A"),
            (3, "物理MF"),
            (4, "软件MF"),
        ],
        "exposureDelay" => &[(0, "3秒"), (1, "2秒"), (2, "1秒"), (3, "关闭")],
        "rawBitDepth" => &[(0, "12 bit"), (1, "14 bit")],
        "rawCompression" => &[
            (0, "无损压缩"),
            (1, "压缩"),
            (2, "未压缩（仅设备声明时可用）"),
        ],
        "crop" => &[(0, "DX"), (1, "1.3x")],
        "whiteBalance" => &[
            (2, "自动"),
            (4, "晴天"),
            (5, "荧光灯"),
            (6, "白炽灯"),
            (7, "闪光灯"),
            (0x8010, "阴天"),
            (0x8011, "阴影"),
            (0x8012, "色温"),
            (0x8013, "手动预设"),
            (0x8016, "自然光自动（仅设备声明时可用）"),
        ],
        "colorSpace" => &[(0, "sRGB"), (1, "Adobe RGB")],
        "pictureControl" => &[
            (1, "标准"),
            (2, "中性"),
            (3, "鲜艳"),
            (4, "单色"),
            (5, "人像"),
            (6, "风景"),
        ],
        "meteringMode" => &[
            (2, "中央重点"),
            (3, "矩阵"),
            (4, "点测光"),
            (0x8010, "亮部重点（仅设备声明时可用）"),
        ],
        "highIsoNoiseReduction" => &[(0, "关闭"), (1, "低"), (2, "标准"), (3, "高")],
        "whiteBalanceFluorescent" => &[
            (0, "钠灯"),
            (1, "冷白荧光灯"),
            (2, "暖白荧光灯"),
            (3, "白色荧光灯"),
            (4, "日光白荧光灯"),
            (5, "日光荧光灯"),
            (6, "高色温水银灯"),
        ],
        "autoIsoHighLimit" => &[
            (0, "ISO 200"),
            (1, "ISO 250"),
            (2, "ISO 280"),
            (3, "ISO 320"),
            (4, "ISO 400"),
            (5, "ISO 500"),
            (6, "ISO 560"),
            (7, "ISO 640"),
            (8, "ISO 800"),
            (9, "ISO 1000"),
            (10, "ISO 1100"),
            (11, "ISO 1250"),
            (12, "ISO 1600"),
            (13, "ISO 2000"),
            (14, "ISO 2200"),
            (15, "ISO 2500"),
            (16, "ISO 3200"),
            (17, "ISO 4000"),
            (18, "ISO 4500"),
            (19, "ISO 5000"),
            (20, "ISO 6400"),
            (21, "Hi 0.3"),
            (22, "Hi 0.5"),
            (23, "Hi 0.7"),
            (24, "Hi 1"),
            (25, "Hi 2"),
        ],
        "centerWeightedArea" => &[
            (0, "6 mm"),
            (1, "8 mm"),
            (2, "10 mm"),
            (3, "13 mm"),
            (4, "全画面平均"),
        ],
        "flashMode" => &[
            (2, "禁止闪光"),
            (4, "减轻红眼"),
            (0x8010, "前帘同步"),
            (0x8011, "慢速同步"),
            (0x8012, "后帘同步"),
            (0x8013, "红眼慢速同步"),
        ],
        "flashSyncSpeed" => &[
            (0, "1/320 s 自动FP"),
            (1, "1/250 s 自动FP"),
            (2, "1/250 s"),
            (3, "1/200 s"),
            (4, "1/160 s"),
            (5, "1/125 s"),
            (6, "1/100 s"),
            (7, "1/80 s"),
            (8, "1/60 s"),
        ],
        "activeDLighting" => &[
            (0, "极高"),
            (1, "高"),
            (2, "标准"),
            (3, "低"),
            (4, "关闭"),
            (5, "自动"),
        ],
        "hdrMode" => &[(0, "关闭"), (1, "开启 单张"), (2, "开启 系列")],
        "hdrExposureDifference" => &[(0, "自动"), (1, "1 EV"), (2, "2 EV"), (3, "3 EV")],
        "hdrSmoothing" => &[
            (0, "高"),
            (1, "标准"),
            (2, "低"),
            (3, "自动"),
            (4, "极高（仅设备声明时可用）"),
        ],
        "recordingMedia" => &[(0, "相机存储卡（主机捕获不可用）"), (1, "SDRAM 主机采集")],
        _ => &[],
    };
    if key == "whiteBalanceTemperature" {
        return [
            2500, 2560, 2630, 2700, 2780, 2860, 2940, 3030, 3130, 3230, 3330, 3450, 3570, 3700,
            3850, 4000, 4170, 4350, 4550, 4760, 5000, 5260, 5560, 5880, 6250, 6670, 7140, 7690,
            8330, 9090, 10000,
        ]
        .iter()
        .enumerate()
        .map(|(n, k)| (CameraValue::Integer(n as i64), format!("{k} K")))
        .collect();
    }
    let mut options: Vec<_> = pairs
        .iter()
        .map(|(value, label)| (CameraValue::Integer(*value), (*label).to_string()))
        .collect();
    if key == "pictureControl" {
        options.extend((201..=209).map(|n| {
            (
                CameraValue::Integer(n),
                format!("自定义Picture Control {}（名称未读取）", n - 200),
            )
        }));
    }
    options
}
