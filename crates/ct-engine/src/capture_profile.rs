//! Frozen acquisition evidence for schema-3 manifests. The hash covers stable
//! device identity and native property values; labels and session IDs never
//! substitute for actual readback. Legacy data is read without rewriting it.

use crate::devices::camera::{CameraIdentity, CameraProperties, CameraValue};
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
    "exposureCompensation",
    "colorSpace",
    "pictureControl",
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
    pub identity: CameraIdentity,
    pub backend: String,
    pub format: String,
    pub parameters: CameraProperties,
    pub readback: CameraProperties,
    pub parameters_hash: String,
    pub preset: Option<PresetTag>,
}

impl CaptureProfile {
    pub fn from_readback(
        identity: CameraIdentity,
        readback: CameraProperties,
        preset: Option<PresetTag>,
    ) -> Result<Self, String> {
        let mut profile = Self {
            schema_version: 1,
            backend: identity.backend.clone(),
            identity,
            format: "NEF".into(),
            parameters: readback.clone(),
            readback,
            parameters_hash: String::new(),
            preset,
        };
        profile.parameters_hash = profile.canonical_hash()?;
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
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
        for key in PROFILE_REQUIRED_KEYS {
            if !self.readback.contains_key(*key) {
                return Err(format!("Capture profile lacks confirmed {key}"));
            }
        }
        if self.exposure_ms().is_none() {
            return Err("Capture profile requires a deterministic native timed shutter".into());
        }
        for (key, expected) in [
            ("exposureMode", 1),
            ("rawFormat", 4),
            ("autoIso", 0),
            ("focusMode", 4),
            ("driveMode", 1),
            ("exposureDelay", 3),
        ] {
            if self.readback.get(key).and_then(CameraValue::integer) != Some(expected) {
                return Err(format!(
                    "Scan capture requires {key}={expected} and its actual readback"
                ));
            }
        }
        // Camera processing must not silently add a second exposure to a scan.
        if self
            .readback
            .get("longExposureNoiseReduction")
            .is_some_and(|value| value.integer() != Some(0))
        {
            return Err("Scan capture requires long-exposure noise reduction OFF".into());
        }
        Ok(())
    }
    pub fn exposure_ms(&self) -> Option<f64> {
        shutter_exposure_ms(self.readback.get("shutter")?)
    }
    pub fn matches(&self, actual: &Self) -> Result<(), String> {
        self.validate()?;
        actual.validate()?;
        if self.parameters_hash != actual.parameters_hash
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
        }
        let bytes = serde_json::to_vec(&Canonical {
            identity: &self.identity,
            backend: &self.backend,
            format: &self.format,
            parameters: &self.parameters,
            readback: &self.readback,
        })
        .map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

pub fn shutter_exposure_ms(value: &CameraValue) -> Option<f64> {
    let code = u32::try_from(value.integer()?).ok()?;
    if code == 0 || code == u32::MAX {
        return None;
    }
    let numerator = code >> 16;
    let denominator = code & 0xffff;
    if numerator == 0 || denominator == 0 {
        return None;
    }
    let milliseconds = f64::from(numerator) * 1000.0 / f64::from(denominator);
    crate::devices::camera::valid_exposure_ms(milliseconds).then_some(milliseconds)
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
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), CameraValue::Integer(value)))
    .collect();
    CaptureProfile::from_readback(identity, readback, None).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_shutter_keeps_fractional_values_and_rejects_bulb() {
        assert_eq!(
            shutter_exposure_ms(&CameraValue::Integer(0x00011f40)),
            Some(0.125)
        );
        assert_eq!(
            shutter_exposure_ms(&CameraValue::Integer(0x00010005)),
            Some(200.0)
        );
        assert_eq!(
            shutter_exposure_ms(&CameraValue::Integer(u32::MAX.into())),
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
    fn unknown_automatic_or_incomplete_evidence_cannot_enter_manifest() {
        let profile = test_profile();
        let mut readback = profile.readback.clone();
        readback.remove("crop");
        assert!(CaptureProfile::from_readback(profile.identity.clone(), readback, None).is_err());
        let mut readback = profile.readback.clone();
        readback.insert("autoIso".into(), CameraValue::Integer(1));
        assert!(CaptureProfile::from_readback(profile.identity.clone(), readback, None).is_err());
        let mut readback = profile.readback;
        readback.insert("focusPosition".into(), CameraValue::Integer(10));
        assert!(CaptureProfile::from_readback(profile.identity, readback, None).is_err());
    }
}
