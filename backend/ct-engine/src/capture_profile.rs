//! Frozen acquisition evidence for schema-3 manifests. The hash covers stable
//! device identity and native property values; labels and session IDs never
//! substitute for actual readback. Legacy data is read without rewriting it.

use crate::devices::camera::{CameraHealth, CameraIdentity, CameraProperties, CameraValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
const HOST_CAPTURE_REQUIRED_KEYS: &[&str] = &["recordingMedia", "burstNumber"];
pub const PRESET_ALLOWED_KEYS: &[&str] = &[
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
    "whiteBalance",
    "bracketing",
    "exposureCompensation",
    "colorSpace",
    "pictureControl",
    "recordingMedia",
    "burstNumber",
    "meteringMode",
    "imageSize",
    "highIsoNoiseReduction",
    "whiteBalanceFluorescent",
    "whiteBalanceTemperature",
    "autoIsoHighLimit",
    "centerWeightedArea",
    "flashMode",
    "flashCompensation",
    "flashSyncSpeed",
    "activeDLighting",
    "hdrMode",
    "hdrExposureDifference",
    "hdrSmoothing",
];

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresetTag {
    pub id: String,
    pub name: String,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureProfile {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shutter_property_code: Option<u16>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub fixed_policy_support: Option<std::collections::BTreeMap<String,bool>>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub fixed_acquisition_evidence:Option<FixedAcquisitionEvidence>,
    pub identity: CameraIdentity,
    pub backend: String,
    pub format: String,
    pub parameters: CameraProperties,
    pub readback: CameraProperties,
    pub parameters_hash: String,
    pub preset: Option<PresetTag>,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct FixedAcquisitionEvidence {
    pub bracketing_property_code:u16,pub active_d_lighting_property_code:u16,pub hdr_proof:String,
    // Read-only serialization compatibility for profiles written before v6.
    // Retired fields never participate in current admission or runtime health.
    #[serde(default,rename="multipleExposure",skip_serializing_if="Option::is_none")]
    retired_sensor_policy:Option<RetainedSensorPolicy>,
    #[serde(default,rename="multipleExposureOperator",skip_serializing_if="Option::is_none")]
    retired_operator_record:Option<serde_json::Value>,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct RetainedSensorPolicy {
    physical_applicable:bool,observable:bool,status:String,source:String,reason:Option<String>,
}

impl CaptureProfile {
    pub fn from_camera_readback(
        health: &CameraHealth,
        readback: CameraProperties,
        preset: Option<PresetTag>,
    ) -> Result<Self, String> {
        let identity = health.identity.clone().ok_or("Camera identity is unknown")?;
        let code = health.properties.get("shutter").ok_or("Shutter property is unknown")?.code;
        let issues=scan_camera_issues(health);
        if !issues.is_empty(){return Err(serde_json::to_string(&issues).unwrap_or_else(|_|"Camera fixed policy blocked".into()));}
        let mut profile=Self::build_profile(identity, readback, preset, code)?;
        profile.fixed_policy_support=Some(["whiteBalance","activeDLighting","hdrMode"].into_iter().map(|key|(key.into(),health.properties.get(key).is_some_and(|p|p.supported))).collect());
        profile.parameters_hash=profile.canonical_hash()?;profile.validate()?;Ok(profile)
    }
    pub fn from_readback(
        identity: CameraIdentity,
        readback: CameraProperties,
        preset: Option<PresetTag>,
        shutter_property_code: u16,
    ) -> Result<Self, String> {
        let profile=Self::build_profile(identity,readback,preset,shutter_property_code)?;profile.validate()?;Ok(profile)
    }
    fn build_profile(identity:CameraIdentity,readback:CameraProperties,preset:Option<PresetTag>,shutter_property_code:u16)->Result<Self,String>{
        let mut profile = Self {
            schema_version: 6,
            shutter_property_code: Some(shutter_property_code),
            fixed_policy_support:Some(["whiteBalance","activeDLighting","hdrMode"].into_iter().map(|key|(key.into(),readback.contains_key(key))).collect()),
            fixed_acquisition_evidence:Some(FixedAcquisitionEvidence{bracketing_property_code:0xd0c0,active_d_lighting_property_code:0xd14e,hdr_proof:if readback.contains_key("hdrMode"){"native_off"}else{"nef_exclusion"}.into(),retired_sensor_policy:None,retired_operator_record:None}),
            backend: identity.backend.clone(),
            identity,
            format: "NEF".into(),
            parameters: readback.clone(),
            readback,
            parameters_hash: String::new(),
            preset,
        };
        profile.parameters_hash = profile.canonical_hash()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.schema_version, 1 | 2 | 3 | 4 | 5 | 6)
            || (self.schema_version == 1 && self.shutter_property_code.is_some())
            || (self.schema_version >= 2 && !matches!(self.shutter_property_code, Some(0x500d | 0xd100)))
            || self.identity.device_id.is_empty()
            || self.identity.model.trim().to_uppercase() != "NIKON D7100"
                && self.identity.model.trim().to_uppercase() != "D7100"
            || self.backend.is_empty()
            || self.backend != self.identity.backend
            || self.format != "NEF"
            || self.parameters != self.readback
            || self.parameters_hash != self.canonical_hash()?
        {
            return Err(
                "Capture profile identity, backend, format, readback or hash is invalid".into(),
            );
        }
        validate_parameter_keys(&self.readback)?;
        if self.schema_version<5&&self.readback.contains_key("bracketing"){return Err("Legacy profile cannot contain later bracketing policy fields".into());}
        for key in PROFILE_REQUIRED_KEYS {
            if !self.readback.contains_key(*key) {
                return Err(format!("Capture profile lacks confirmed {key}"));
            }
        }
        if self.schema_version >= 3 {
            for key in HOST_CAPTURE_REQUIRED_KEYS {
                if self.readback.get(*key).and_then(CameraValue::integer) != Some(1) {
                    return Err(format!("Host-only single capture requires {key}=1 and its readback"));
                }
            }
        }
        if self.exposure_ms().is_none() {
            return Err("Capture profile requires a deterministic native timed shutter".into());
        }
        for (key, expected) in [
            ("exposureMode", 1),
            ("rawFormat", 4),
            ("autoIso", 0),
            ("driveMode", 1),
            ("exposureDelay", 3),
        ] {
            if self.readback.get(key).and_then(CameraValue::integer) != Some(expected) {
                return Err(format!(
                    "Scan capture requires {key}={expected} and its actual readback"
                ));
            }
        }
        if !matches!(self.readback.get("focusMode").and_then(CameraValue::integer), Some(3 | 4)) {
            return Err("Scan capture requires hardware or software manual focus (3 or 4)".into());
        }
        // Camera processing must not silently add a second exposure to a scan.
        if self
            .readback
            .get("longExposureNoiseReduction")
            .is_some_and(|value| value.integer() != Some(0))
        {
            return Err("Scan capture requires long-exposure noise reduction OFF".into());
        }
        if self.schema_version >= 4 {
            let support=self.fixed_policy_support.as_ref().ok_or("Fixed-policy supported-feature evidence missing")?;
            if support.len()!=3||support.get("whiteBalance")!=Some(&true){return Err("Fixed white-balance support evidence missing".into());}
            for key in ["activeDLighting","hdrMode"]{if !support.contains_key(key)||support[key]&&!self.readback.contains_key(key){return Err(format!("Fixed-policy evidence missing for {key}"));}}
            validate_fixed_values(&self.readback)?;
        }
        if self.schema_version>=5 {
            let evidence=self.fixed_acquisition_evidence.as_ref().ok_or("Fixed physical-acquisition evidence missing")?;
            if evidence.bracketing_property_code!=0xd0c0||evidence.active_d_lighting_property_code!=0xd14e||self.readback.get("bracketing").and_then(CameraValue::integer)!=Some(0)||self.readback.get("activeDLighting").and_then(CameraValue::integer)!=Some(4)||self.fixed_policy_support.as_ref().and_then(|s|s.get("activeDLighting"))!=Some(&true){return Err("Fixed scan requires observable bracketing OFF0 and D7100 ADL OFF4".into());}
            if self.schema_version>=6&&(evidence.retired_sensor_policy.is_some()||evidence.retired_operator_record.is_some()) {
                return Err("Current capture profile contains retired policy metadata".into());
            }
            if self.schema_version==5&&evidence.retired_sensor_policy.is_none() {
                return Err("Retained profile lacks its original hash metadata".into());
            }
            match evidence.hdr_proof.as_str(){"native_off" if self.readback.get("hdrMode").and_then(CameraValue::integer)==Some(0)=>{},"nef_exclusion" if !self.readback.contains_key("hdrMode")&&self.readback.get("rawFormat").and_then(CameraValue::integer)==Some(4)=>{},_=>return Err("HDR evidence contradicts native OFF or verified NEF exclusion".into())}
        }
        Ok(())
    }
    pub fn require_current_policy(&self) -> Result<(), String> {
        self.validate()?;
        if self.schema_version != 6 { return Err("CAPTURE_POLICY_LEGACY_READ_ONLY: retained profile lacks current fixed-policy evidence".into()); }
        Ok(())
    }
    pub fn exposure_ms(&self) -> Option<f64> {
        // Schema 1 defined shutter values as Nikon packed fractions. Schema 2+
        // freezes the native property code; device model never selects encoding.
        let code = if self.schema_version == 1 { 0xd100 } else { self.shutter_property_code? };
        crate::devices::camera::shutter_ms(self.readback.get("shutter")?, code)
            .filter(|ms| crate::devices::camera::valid_exposure_ms(*ms))
    }
    pub fn matches(&self, actual: &Self) -> Result<(), String> {
        self.require_current_policy()?;
        actual.require_current_policy()?;
        if self.parameters_hash != actual.parameters_hash
            || self.schema_version != actual.schema_version
            || self.shutter_property_code != actual.shutter_property_code
            || self.identity != actual.identity
            || self.backend != actual.backend
            || self.format != actual.format
            || self.parameters != actual.parameters
            || self.readback != actual.readback
        {
            return Err("CAPTURE_PROFILE_MISMATCH: actual camera evidence differs from the frozen acquisition".into());
        }
        Ok(())
    }
    fn canonical_evidence(&self)->Option<CanonicalEvidence<'_>>{
        let evidence=self.fixed_acquisition_evidence.as_ref()?;
        Some(CanonicalEvidence{
            bracketing_property_code:evidence.bracketing_property_code,
            active_d_lighting_property_code:evidence.active_d_lighting_property_code,
            hdr_proof:&evidence.hdr_proof,
            retired_sensor_policy:if self.schema_version==5{evidence.retired_sensor_policy.as_ref()}else{None},
            retired_operator_policy:if self.schema_version==5{evidence.retired_operator_record.as_ref().and_then(|record|record.get("policyVersion")).and_then(serde_json::Value::as_u64)}else{None},
        })
    }
    fn canonical_hash(&self) -> Result<String, String> {
        // A struct and BTreeMaps guarantee stable field/key order. No floating
        // point display string, preset name, unknown focus or transient session.
        #[derive(Serialize)]
        struct Canonical<'a> {
            identity: &'a CameraIdentity,
            backend: &'a str,
            format: &'a str,
            parameters: &'a CameraProperties,
            readback: &'a CameraProperties,
            #[serde(skip_serializing_if = "Option::is_none")]
            shutter_property_code: Option<u16>,
            #[serde(skip_serializing_if = "Option::is_none")]
            fixed_policy_version: Option<u32>,
            #[serde(skip_serializing_if="Option::is_none")]
            fixed_policy_support:Option<&'a std::collections::BTreeMap<String,bool>>,
            #[serde(skip_serializing_if="Option::is_none")]
            fixed_acquisition_evidence:Option<CanonicalEvidence<'a>>,
        }
        let bytes = serde_json::to_vec(&Canonical {
            identity: &self.identity,
            backend: &self.backend,
            format: &self.format,
            parameters: &self.parameters,
            readback: &self.readback,
            shutter_property_code: self.shutter_property_code,
            fixed_policy_version:if self.schema_version>=6{Some(3)}else if self.schema_version>=5{Some(2)}else{(self.schema_version>=4).then_some(1)},
            fixed_policy_support:if self.schema_version>=4{self.fixed_policy_support.as_ref()}else{None},
            fixed_acquisition_evidence:if self.schema_version>=5{self.canonical_evidence()}else{None},
        })
        .map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
struct CanonicalEvidence<'a>{
    bracketing_property_code:u16,
    active_d_lighting_property_code:u16,
    hdr_proof:&'a str,
    #[serde(rename="multipleExposure",skip_serializing_if="Option::is_none")]
    retired_sensor_policy:Option<&'a RetainedSensorPolicy>,
    #[serde(rename="multipleExposureOperatorPolicy",skip_serializing_if="Option::is_none")]
    retired_operator_policy:Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanCameraIssue {
    pub key: String,
    pub actual: Option<CameraValue>,
    pub requirement: String,
    pub remedy: String,
}
pub fn scan_camera_issues(health: &CameraHealth) -> Vec<ScanCameraIssue> {
    let mut issues = Vec::new();
    if !health.connected || health.isolated || !health.capabilities.scan_nef {
        issues.push(ScanCameraIssue { key: "session".into(), actual: None, requirement: "Confirmed native D7100 NEF session".into(), remedy: "Connect and explicitly select the camera".into() }); return issues;
    }
    if health.live_view.active { issues.push(ScanCameraIssue { key: "liveView".into(), actual: None, requirement: "Live View OFF".into(), remedy: "Explicitly stop Live View".into() }); }
    for key in PROFILE_REQUIRED_KEYS.iter().copied().chain(HOST_CAPTURE_REQUIRED_KEYS.iter().copied()).chain(["whiteBalance", "activeDLighting", "hdrMode","bracketing"]) {
        let p = health.properties.get(key);
        if key=="hdrMode" && hdr_nef_exclusion(health){continue;}
        let actual = p.and_then(|p| p.actual.clone());
        let requirement = match key {
            "bracketing"=>"Automatic bracketing OFF (native UINT8 0)",
            "exposureMode" => "Manual exposure (M)", "autoIso" => "Auto ISO OFF",
            "focusMode" => "Hardware MF or software MF", "whiteBalance" => "Confirmed fixed white balance",
            "activeDLighting" | "hdrMode" | "longExposureNoiseReduction" => "OFF",
            "exposureDelay" => "OFF (native value 3)", "recordingMedia" => "Host RAM (1)",
            "burstNumber" | "driveMode" => "Single image (1)", "rawFormat" => "NEF only (4)",
            "shutter" => "Confirmed native timed shutter", _ => "Confirmed actual value matching Apply",
        };
        let evidence = p.is_some_and(|p| p.confirmed && p.readable && p.error.is_none() && p.actual.is_some() && p.target.as_ref().is_none_or(|target| Some(target) == p.actual.as_ref()) && match key {"recordingMedia"=>p.code==0xd10b&&p.data_type==2,"burstNumber"=>p.code==0x5018&&p.data_type==4,"bracketing"=>p.code==0xd0c0&&p.data_type==2,"activeDLighting"=>p.code==0xd14e,_=>true});
        let value = actual.as_ref().and_then(CameraValue::integer);
        let fixed = match key {
            "exposureMode" | "recordingMedia" | "burstNumber" | "driveMode" => value == Some(1),
            "bracketing" | "autoIso" | "hdrMode" | "longExposureNoiseReduction" => value == Some(0),
            "activeDLighting" => value == Some(4),
            "rawFormat" => value == Some(4), "exposureDelay" => value == Some(3),
            "focusMode" => matches!(value, Some(3 | 4)),
            "whiteBalance" => matches!(value, Some(4 | 5 | 6 | 7 | 0x8010 | 0x8011 | 0x8012 | 0x8013)),
            "shutter" => p.is_some_and(|p| crate::devices::camera::shutter_property_ms(p).is_ok()), _ => true,
        };
        if !evidence || !fixed { issues.push(ScanCameraIssue { key: key.into(), actual, requirement: requirement.into(), remedy: if p.is_some_and(|p| p.native_writable) { "Select a supported fixed value, Apply and verify, then repeat Preflight".into() } else { "Adjust the camera body or lens, then Read and repeat Preflight".into() } }); }
    }
    for (key,p) in &health.properties {
        if p.supported&&p.readable&&(p.error.is_some()||!p.confirmed||p.actual.is_none()||p.target.as_ref().is_some_and(|target|Some(target)!=p.actual.as_ref()))&&!issues.iter().any(|issue|issue.key==*key){
            issues.push(ScanCameraIssue{key:key.clone(),actual:p.actual.clone(),requirement:"Confirmed actual value matching Apply".into(),remedy:"Read current properties; explicitly Apply any intended change and repeat Preflight".into()});
        }
    }
    issues
}
fn hdr_nef_exclusion(health:&CameraHealth)->bool{
    let d7100=health.identity.as_ref().is_some_and(|i|matches!(i.model.trim().to_uppercase().as_str(),"D7100"|"NIKON D7100"));
    let no_advertised_hdr=health.device_info.as_ref().is_some_and(|info|!info.effective_properties.contains(&0xd130)&&!info.properties.contains(&0xd130));
    d7100&&no_advertised_hdr&&health.properties.get("hdrMode").is_some_and(|p|!p.supported&&!p.readable&&p.error.is_none())&&health.properties.get("rawFormat").is_some_and(|p|p.confirmed&&p.error.is_none()&&p.code==0x5004&&p.actual.as_ref().and_then(CameraValue::integer)==Some(4))
}

fn validate_fixed_values(values: &CameraProperties) -> Result<(), String> {
    if !matches!(values.get("whiteBalance").and_then(CameraValue::integer), Some(4 | 5 | 6 | 7 | 0x8010 | 0x8011 | 0x8012 | 0x8013)) {
        return Err("Scan requires confirmed fixed white balance; Auto/unknown is blocked".into());
    }
    for key in ["activeDLighting", "hdrMode"] {
        if values.get(key).is_some_and(|v| v.integer() != Some(if key=="activeDLighting"{4}else{0})) { return Err(format!("Scan requires {key} OFF")); }
    }
    Ok(())
}

pub fn validate_parameter_keys(parameters: &CameraProperties) -> Result<(), String> {
    if parameters.is_empty() || parameters.len() > PRESET_ALLOWED_KEYS.len() {
        return Err("Empty or oversized camera parameter set".into());
    }
    for (key, value) in parameters {
        if !PRESET_ALLOWED_KEYS.contains(&key.as_str()) {
            return Err(format!("Unknown or non-camera preset key: {key}"));
        }
        if let CameraValue::Text(text) = value {
            if text.is_empty() || text.len() > 256 || text.chars().any(char::is_control) {
                return Err(format!("Invalid native value for {key}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_profile() -> CaptureProfile {
    let identity = CameraIdentity {
        device_id: "offline-scripted-d7100".into(),
        manufacturer: "Nikon".into(),
        model: "D7100".into(),
        serial: Some("fake-test-only".into()),
        backend: "scripted-native-ptp".into(),
    };
    let readback = [
        ("shutter", 0x00010005),
        ("iso", 100),
        ("aperture", 560),
        ("exposureMode", 1),
        ("autoIso", 0),
        ("rawFormat", 4),
        ("rawBitDepth", 1),
        ("rawCompression", 0),
        ("crop", 0),
        ("focusMode", 4),
        ("driveMode", 1),
        ("exposureDelay", 3),
        ("longExposureNoiseReduction", 0),
        ("recordingMedia", 1),
        ("burstNumber", 1),
        ("whiteBalance", 4),
        ("bracketing",0),
        ("activeDLighting", 4),
        ("hdrMode", 0),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), CameraValue::Integer(value)))
    .collect();
    CaptureProfile::from_readback(identity, readback, None, 0xd100).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_scan_readiness_uses_confirmed_camera_parameters() {
        let camera=crate::devices::camera::scripted_camera(Default::default());
        let health=camera.health();
        assert!(scan_camera_issues(&health).is_empty(),"{:?}",scan_camera_issues(&health));
        let profile=CaptureProfile::from_camera_readback(&health,camera.profile_readback().unwrap(),None).unwrap();
        profile.require_current_policy().unwrap();
        assert_eq!(profile.schema_version,6);
        let mut invalid=health.clone();
        invalid.properties.get_mut("longExposureNoiseReduction").unwrap().actual=Some(CameraValue::Integer(1));
        assert!(scan_camera_issues(&invalid).iter().any(|issue|issue.key=="longExposureNoiseReduction"));
        assert!(crate::devices::camera::profile_readback(&invalid).is_err());
    }
    #[test]
    fn frozen_profile_matches_a_new_session_and_rejects_parameter_drift() {
        let first=crate::devices::camera::scripted_camera(Default::default());
        let second=crate::devices::camera::scripted_camera(Default::default());
        assert_ne!(first.health().session_id,second.health().session_id);
        let frozen=CaptureProfile::from_camera_readback(&first.health(),first.profile_readback().unwrap(),None).unwrap();
        let actual=CaptureProfile::from_camera_readback(&second.health(),second.profile_readback().unwrap(),None).unwrap();
        frozen.matches(&actual).unwrap();
        let mut changed=actual.readback.clone();
        changed.insert("iso".into(),CameraValue::Integer(400));
        let drift=CaptureProfile::from_readback(actual.identity,changed,None,0xd100).unwrap();
        assert!(frozen.matches(&drift).is_err());
    }
    #[test]
    fn retained_profile_five_preserves_original_hashes_and_is_read_only() {
        // Captured with the pre-change serializer; both historical record shapes.
        for text in [r#"{"schemaVersion":5,"shutterPropertyCode":53504,"fixedPolicySupport":{"activeDLighting":true,"hdrMode":true,"whiteBalance":true},"fixedAcquisitionEvidence":{"bracketingPropertyCode":53440,"activeDLightingPropertyCode":53582,"hdrProof":"native_off","multipleExposure":{"physicalApplicable":true,"observable":true,"status":"off","source":"SCRIPTED TEST ONLY","reason":null}},"identity":{"deviceId":"offline-scripted-d7100","manufacturer":"Nikon","model":"D7100","serial":"fake-test-only","backend":"scripted-native-ptp"},"backend":"scripted-native-ptp","format":"NEF","parameters":{"activeDLighting":4,"aperture":560,"autoIso":0,"bracketing":0,"burstNumber":1,"crop":0,"driveMode":1,"exposureDelay":3,"exposureMode":1,"focusMode":4,"hdrMode":0,"iso":100,"longExposureNoiseReduction":0,"rawBitDepth":1,"rawCompression":0,"rawFormat":4,"recordingMedia":1,"shutter":65541,"whiteBalance":4},"readback":{"activeDLighting":4,"aperture":560,"autoIso":0,"bracketing":0,"burstNumber":1,"crop":0,"driveMode":1,"exposureDelay":3,"exposureMode":1,"focusMode":4,"hdrMode":0,"iso":100,"longExposureNoiseReduction":0,"rawBitDepth":1,"rawCompression":0,"rawFormat":4,"recordingMedia":1,"shutter":65541,"whiteBalance":4},"parametersHash":"8506fb3099c8b00a2a226127e03df6ca596889413cdcc36d47d111a74585910a","preset":null}"#,r#"{"schemaVersion":5,"shutterPropertyCode":53504,"fixedPolicySupport":{"activeDLighting":true,"hdrMode":true,"whiteBalance":true},"fixedAcquisitionEvidence":{"bracketingPropertyCode":53440,"activeDLightingPropertyCode":53582,"hdrProof":"native_off","multipleExposure":{"physicalApplicable":true,"observable":false,"status":"unknown","source":"none","reason":"D7100 multiple exposure is physically supported but its current OFF/inactive state is not observable through the verified native property catalog; one NEF/C102 does not prove one physical exposure"},"multipleExposureOperator":{"confirmed":true,"confirmedAt":"2026-10-08T01:30:00.000Z","operator":"operator","deviceId":"fake-camera-1","sessionId":"621c415d11e671c001c029f3c722c19f","evidenceFingerprint":"7808b14a4493bd658ae0462b3312fc537a8ab20d07c2ff02f1378efe6c971029","policyVersion":1}},"identity":{"deviceId":"fake-camera-1","manufacturer":"Nikon Corporation","model":"NIKON D7100","serial":"fake-serial","backend":"windows_wpd_nikon_d7100"},"backend":"windows_wpd_nikon_d7100","format":"NEF","parameters":{"activeDLighting":4,"aperture":800,"autoIso":0,"autoIsoHighLimit":0,"bracketing":0,"burstNumber":1,"centerWeightedArea":0,"colorSpace":0,"crop":0,"driveMode":1,"exposureCompensation":0,"exposureDelay":3,"exposureMode":1,"flashCompensation":0,"flashMode":0,"flashSyncSpeed":0,"focusMode":4,"hdrExposureDifference":0,"hdrMode":0,"hdrSmoothing":0,"highIsoNoiseReduction":0,"imageSize":"6000x4000","iso":200,"longExposureNoiseReduction":0,"meteringMode":3,"pictureControl":0,"rawBitDepth":0,"rawCompression":0,"rawFormat":4,"recordingMedia":1,"shutter":65541,"whiteBalance":4,"whiteBalanceFluorescent":0,"whiteBalanceTemperature":0},"readback":{"activeDLighting":4,"aperture":800,"autoIso":0,"autoIsoHighLimit":0,"bracketing":0,"burstNumber":1,"centerWeightedArea":0,"colorSpace":0,"crop":0,"driveMode":1,"exposureCompensation":0,"exposureDelay":3,"exposureMode":1,"flashCompensation":0,"flashMode":0,"flashSyncSpeed":0,"focusMode":4,"hdrExposureDifference":0,"hdrMode":0,"hdrSmoothing":0,"highIsoNoiseReduction":0,"imageSize":"6000x4000","iso":200,"longExposureNoiseReduction":0,"meteringMode":3,"pictureControl":0,"rawBitDepth":0,"rawCompression":0,"rawFormat":4,"recordingMedia":1,"shutter":65541,"whiteBalance":4,"whiteBalanceFluorescent":0,"whiteBalanceTemperature":0},"parametersHash":"9c85e5fd13a642b6c966a09439d3ae3128b326f46b301936989a500463df1a8a","preset":null}"#] {
            let old:CaptureProfile=serde_json::from_str(text).unwrap();
            old.validate().unwrap();
            assert_eq!(old.parameters_hash,old.canonical_hash().unwrap());
            assert!(old.require_current_policy().is_err());
            let roundtrip:CaptureProfile=serde_json::from_slice(&serde_json::to_vec(&old).unwrap()).unwrap();
            roundtrip.validate().unwrap();
            assert_eq!(old,roundtrip);
            let mut tampered=old.clone();
            tampered.fixed_acquisition_evidence.as_mut().unwrap().retired_sensor_policy.as_mut().unwrap().source="tampered".into();
            assert!(tampered.validate().is_err());
        }
    }
    #[test]
    fn current_policy_is_versioned_while_previous_hashes_remain_read_only(){
        let current=test_profile();assert_eq!(current.schema_version,6);
        let mut old=current.clone();old.schema_version=3;old.fixed_policy_support=None;old.fixed_acquisition_evidence=None;old.readback.remove("bracketing");
        old.readback.insert("whiteBalance".into(),CameraValue::Integer(2));old.parameters=old.readback.clone();old.parameters_hash=old.canonical_hash().unwrap();
        let bytes=serde_json::to_vec(&old).unwrap();let restored:CaptureProfile=serde_json::from_slice(&bytes).unwrap();restored.validate().unwrap();assert_eq!(serde_json::to_vec(&restored).unwrap(),bytes);assert!(restored.require_current_policy().is_err());assert!(restored.matches(&current).is_err());assert_ne!(old.parameters_hash,current.parameters_hash);
        for (key,value) in [("whiteBalance",2),("activeDLighting",0),("hdrMode",1),("longExposureNoiseReduction",1),("exposureDelay",0)]{let mut values=current.readback.clone();values.insert(key.into(),CameraValue::Integer(value));assert!(CaptureProfile::from_readback(current.identity.clone(),values,None,0xd100).is_err(),"{key}");}
    }
    #[test]
    fn legacy_profile_four_preserves_its_original_canonical_hash_and_remains_read_only(){
        let mut old=test_profile();old.schema_version=4;old.fixed_acquisition_evidence=None;old.readback.remove("bracketing");old.parameters=old.readback.clone();old.parameters_hash="706f9928ced0f83708ad126837edae8f480b8e25747014ec6aace5af8fd7da82".into();
        old.validate().unwrap();assert_eq!(old.canonical_hash().unwrap(),"706f9928ced0f83708ad126837edae8f480b8e25747014ec6aace5af8fd7da82");let bytes=serde_json::to_vec(&old).unwrap();let decoded:CaptureProfile=serde_json::from_slice(&bytes).unwrap();decoded.validate().unwrap();assert_eq!(serde_json::to_vec(&decoded).unwrap(),bytes);assert!(decoded.require_current_policy().is_err());
    }
    #[test]
    fn native_shutter_keeps_fractional_values_and_rejects_bulb() {
        assert_eq!(
            crate::devices::camera::shutter_ms(&CameraValue::Integer(0x00011f40), 0xd100),
            Some(0.125)
        );
        assert_eq!(
            crate::devices::camera::shutter_ms(&CameraValue::Integer(0x00010005), 0xd100),
            Some(200.0)
        );
        assert_eq!(
            crate::devices::camera::shutter_ms(&CameraValue::Integer(u32::MAX.into()), 0xd100),
            None
        );
    }
    #[test]
    fn restore_compares_actual_values_and_hash_not_preset_name() {
        let frozen = test_profile();
        let mut actual = frozen.clone();
        actual.preset = Some(PresetTag {
            id: "renamed".into(),
            name: "same values".into(),
            source: "operator".into(),
        });
        frozen.matches(&actual).unwrap();
        actual
            .readback
            .insert("iso".into(), CameraValue::Integer(200));
        assert!(frozen.matches(&actual).is_err());
    }
    #[test]
    fn native_shutter_encoding_is_frozen_and_legacy_profile_remains_readable() {
        let original = test_profile();
        let mut values = original.readback.clone();
        values.insert("shutter".into(), CameraValue::Integer(20000));
        let standard = CaptureProfile::from_readback(original.identity.clone(), values.clone(), None, 0x500d).unwrap();
        assert_eq!(standard.exposure_ms(), Some(2000.0));
        assert!(CaptureProfile::from_readback(original.identity.clone(), values, None, 0).is_err());
        let mut tampered = standard.clone();
        tampered.shutter_property_code = Some(0xd100);
        assert!(standard.matches(&tampered).is_err());
        let mut legacy = original.clone();
        legacy.schema_version = 1;
        legacy.fixed_acquisition_evidence=None;legacy.readback.remove("bracketing");legacy.parameters.remove("bracketing");
        legacy.shutter_property_code = None;
        for key in HOST_CAPTURE_REQUIRED_KEYS { legacy.readback.remove(*key); legacy.parameters.remove(*key); }
        legacy.parameters_hash = legacy.canonical_hash().unwrap();
        legacy.validate().unwrap();
        let encoded = serde_json::to_value(&legacy).unwrap();
        assert!(encoded.get("shutterPropertyCode").is_none());
        let restored: CaptureProfile = serde_json::from_value(encoded).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.exposure_ms(), Some(200.0));
        assert!(restored.matches(&original).is_err());
        let mut previous = legacy.clone();
        previous.schema_version = 2;
        previous.shutter_property_code = Some(0xd100);
        previous.parameters_hash = previous.canonical_hash().unwrap();
        previous.validate().unwrap();
        assert!(previous.matches(&original).is_err());
    }
    #[test]
    fn host_capture_profile_requires_ram_and_exactly_one_photo() {
        let profile = test_profile();
        for key in HOST_CAPTURE_REQUIRED_KEYS {
            for value in [0, 2] {
                let mut values = profile.readback.clone();
                values.insert((*key).into(), CameraValue::Integer(value));
                assert!(CaptureProfile::from_readback(profile.identity.clone(), values, None, 0xd100).is_err());
            }
        }
    }
    #[test]
    fn hardware_manual_focus_is_valid_but_all_autofocus_modes_are_blocked() {
        let profile = test_profile();
        for mode in 0..=4 {
            let mut values = profile.readback.clone();
            values.insert("focusMode".into(), CameraValue::Integer(mode));
            let result = CaptureProfile::from_readback(profile.identity.clone(), values, None, 0xd100);
            assert_eq!(result.is_ok(), mode == 3 || mode == 4);
            if let Ok(frozen) = result { assert_eq!(frozen.readback["focusMode"], CameraValue::Integer(mode)); }
        }
    }
    #[test]
    fn unknown_automatic_or_incomplete_evidence_cannot_enter_manifest() {
        let profile = test_profile();
        let mut readback = profile.readback.clone();
        readback.remove("crop");
        assert!(CaptureProfile::from_readback(profile.identity.clone(), readback, None, 0xd100).is_err());
        let mut readback = profile.readback.clone();
        readback.insert("autoIso".into(), CameraValue::Integer(1));
        assert!(CaptureProfile::from_readback(profile.identity.clone(), readback, None, 0xd100).is_err());
        let mut readback = profile.readback;
        readback.insert("focusPosition".into(), CameraValue::Integer(10));
        assert!(CaptureProfile::from_readback(profile.identity, readback, None, 0xd100).is_err());
    }
}
