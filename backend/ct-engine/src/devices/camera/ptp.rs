//! Bounded PTP datasets shared by real WPD and scripted offline transports.

use super::types::{CameraDeviceInfo, CameraError, CameraIdentity, CameraProperty, CameraValue};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub const OK: u16 = 0x2001;
pub const DEVICE_BUSY: u16 = 0x2019;
pub const GET_DEVICE_INFO: u16 = 0x1001;
pub const GET_OBJECT_INFO: u16 = 0x1008;
pub const GET_OBJECT: u16 = 0x1009;
pub const GET_PROPERTY_DESC: u16 = 0x1014;
pub const GET_PROPERTY: u16 = 0x1015;
pub const SET_PROPERTY: u16 = 0x1016;
pub const MAX_OBJECT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct CallContext {
    pub deadline: Instant,
    pub epoch: u64,
    pub generation: Arc<AtomicU64>,
}
impl CallContext {
    pub fn check(&self) -> Result<(), CameraError> {
        if self.generation.load(Ordering::Acquire) != self.epoch {
            return Err(CameraError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(CameraError::Timeout(
                "shared operation deadline exceeded".into(),
            ));
        }
        Ok(())
    }
}

/// Names the failing transaction of a multi-step device sequence, so a timeout on
/// a shared deadline identifies which command hung instead of only reporting the
/// generic deadline error.
pub(crate) fn step_error(step: &str, error: CameraError) -> CameraError {
    CameraError::Command(format!("{step}: {error}"))
}

#[derive(Debug)]
pub(crate) struct PtpResponse {
    pub code: u16,
    pub data: Vec<u8>,
}
impl PtpResponse {
    pub fn data(self) -> Result<Vec<u8>, CameraError> {
        if self.code == OK {
            Ok(self.data)
        } else {
            Err(CameraError::Command(format!(
                "PTP response 0x{:04x}",
                self.code
            )))
        }
    }
    pub fn success(self) -> Result<(), CameraError> {
        self.data().map(|_| ())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum DataPhase {
    None,
    Read { max: usize },
    Write(Vec<u8>),
}

/// This trait intentionally has no Send bound. Its factory crosses threads;
/// the COM-bearing implementation is created, used, and dropped on one thread.
pub(crate) trait CameraTransport {
    fn discover(&mut self, context: &CallContext) -> Result<Vec<CameraIdentity>, CameraError>;
    fn open(&mut self, id: &str, context: &CallContext) -> Result<(), CameraError>;
    fn close(&mut self) -> Result<(), CameraError>;
    fn cancel(&mut self) -> Result<(), CameraError>;
    fn execute(
        &mut self,
        code: u16,
        params: &[u32],
        phase: DataPhase,
        context: &CallContext,
    ) -> Result<PtpResponse, CameraError>;
}

pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], CameraError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| invalid("dataset length overflow"))?;
        let result = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated PTP dataset"))?;
        self.offset = end;
        Ok(result)
    }
    pub fn u8(&mut self) -> Result<u8, CameraError> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, CameraError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, CameraError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn string(&mut self) -> Result<String, CameraError> {
        let count = self.u8()? as usize;
        if count == 0 {
            return Ok(String::new());
        }
        let mut units: Vec<u16> = self
            .take(count * 2)?
            .chunks_exact(2)
            .map(|x| u16::from_le_bytes([x[0], x[1]]))
            .collect();
        if units.pop() != Some(0) || units.contains(&0) {
            return Err(invalid("invalid PTP string terminator"));
        }
        String::from_utf16(&units).map_err(|_| invalid("invalid PTP UTF-16"))
    }
    pub fn array16(&mut self) -> Result<Vec<u16>, CameraError> {
        let count = self.u32()? as usize;
        if count > 4096 {
            return Err(invalid("PTP array exceeds limit"));
        }
        (0..count).map(|_| self.u16()).collect()
    }
    pub fn finish(self) -> Result<(), CameraError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid("unexpected trailing PTP bytes"))
        }
    }
    pub fn value(&mut self, kind: u16) -> Result<CameraValue, CameraError> {
        let integer = match kind {
            1 => self.u8()? as i8 as i64,
            2 => self.u8()? as i64,
            3 => self.u16()? as i16 as i64,
            4 => self.u16()? as i64,
            5 => self.u32()? as i32 as i64,
            6 => self.u32()? as i64,
            7 => i64::from_le_bytes(self.take(8)?.try_into().unwrap()),
            8 => i64::try_from(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
                .map_err(|_| invalid("UINT64 outside JSON signed exact range"))?,
            0xffff => return Ok(CameraValue::Text(self.string()?)),
            _ => {
                return Err(CameraError::Unsupported(format!(
                    "PTP property datatype 0x{kind:04x}"
                )))
            }
        };
        Ok(CameraValue::Integer(integer))
    }
}
pub(crate) fn invalid(s: &str) -> CameraError {
    CameraError::Safety(s.into())
}

#[derive(Debug)]
pub(crate) struct DeviceInfo {
    pub manufacturer: String,
    pub model: String,
    pub serial: String,
    pub operations: Vec<u16>,
    pub properties: Vec<u16>,
    pub events: Vec<u16>,
    pub vendor_id: u32,
    pub diagnostic: CameraDeviceInfo,
}
pub(crate) fn parse_device_info(bytes: &[u8]) -> Result<DeviceInfo, CameraError> {
    let mut c = Cursor::new(bytes);
    let standard_version = c.u16()?;
    let vendor_id = c.u32()?;
    let vendor_extension_version = c.u16()?;
    let vendor_extension_description = c.string()?;
    let functional_mode = c.u16()?;
    let operations = c.array16()?;
    let events = c.array16()?;
    let properties = c.array16()?;
    let capture_formats = c.array16()?;
    let image_formats = c.array16()?;
    let manufacturer = c.string()?;
    let model = c.string()?;
    let device_version = c.string()?;
    let serial = c.string()?;
    c.finish()?;
    if manufacturer.is_empty() || model.is_empty() {
        return Err(invalid("missing PTP camera identity"));
    }
    Ok(DeviceInfo {
        manufacturer,
        model,
        serial,
        diagnostic: CameraDeviceInfo {
            standard_version,
            vendor_extension_id: vendor_id,
            vendor_extension_version,
            vendor_extension_description,
            functional_mode,
            device_version,
            operations: operations.clone(),
            events: events.clone(),
            properties: properties.clone(),
            capture_formats,
            image_formats,
            dataset_size: bytes.len(),
            dataset_sha256: format!("{:x}", Sha256::digest(bytes)),
            adapter_selection: String::new(),
            effective_properties: properties.clone(),
            vendor_property_codes: None,
        },
        operations,
        properties,
        events,
        vendor_id,
    })
}

pub(crate) fn parse_property(
    bytes: &[u8],
    key: &str,
    label: &str,
    expected_code: u16,
) -> Result<CameraProperty, CameraError> {
    let mut c = Cursor::new(bytes);
    let code = c.u16()?;
    if code != expected_code {
        return Err(invalid("property description code does not match request"));
    }
    let data_type = c.u16()?;
    let writable = match c.u8()? {
        0 => false,
        1 => true,
        _ => return Err(invalid("invalid PTP GetSet flag")),
    };
    let _factory = c.value(data_type)?;
    let actual = c.value(data_type)?;
    let mut supported_values = vec![];
    let mut range = None;
    match c.u8()? {
        0 => (),
        1 => {
            let minimum = c
                .value(data_type)?
                .integer()
                .ok_or_else(|| invalid("string range"))?;
            let maximum = c
                .value(data_type)?
                .integer()
                .ok_or_else(|| invalid("string range"))?;
            let step = c
                .value(data_type)?
                .integer()
                .ok_or_else(|| invalid("string range"))?;
            if minimum > maximum || step <= 0 {
                return Err(invalid("invalid PTP property range"));
            }
            range = Some([minimum, maximum, step]);
        }
        2 => {
            let count = c.u16()? as usize;
            if count > 4096 {
                return Err(invalid("PTP property enum exceeds limit"));
            }
            for _ in 0..count {
                supported_values.push(c.value(data_type)?);
            }
        }
        _ => return Err(invalid("invalid property form flag")),
    }
    c.finish()?;
    Ok(CameraProperty {
        key: key.into(),
        label: label.into(),
        code,
        data_type,
        readable: true,
        writable,
        supported: true,
        native_writable: writable,
        effective: true,
        disabled_reason: None,
        options: vec![],
        supported_values,
        range,
        actual: Some(actual),
        target: None,
        confirmed: true,
        error: None,
    })
}
pub(crate) fn parse_value(bytes: &[u8], kind: u16) -> Result<CameraValue, CameraError> {
    let mut c = Cursor::new(bytes);
    let value = c.value(kind)?;
    c.finish()?;
    Ok(value)
}
pub(crate) fn encode_value(value: &CameraValue, kind: u16) -> Result<Vec<u8>, CameraError> {
    if let CameraValue::Text(text) = value {
        if kind != 0xffff || text.contains('\0') {
            return Err(invalid("property value/type mismatch"));
        }
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        if units.len() > 255 {
            return Err(invalid("property string too long"));
        }
        let mut result = vec![units.len() as u8];
        result.extend(units.into_iter().flat_map(u16::to_le_bytes));
        return Ok(result);
    }
    let n = value.integer().unwrap();
    macro_rules! number {
        ($t:ty) => {
            <$t>::try_from(n)
                .map(|v| v.to_le_bytes().to_vec())
                .map_err(|_| invalid("property number outside declared type"))
        };
    }
    match kind {
        1 => number!(i8),
        2 => number!(u8),
        3 => number!(i16),
        4 => number!(u16),
        5 => number!(i32),
        6 => number!(u32),
        7 => number!(i64),
        8 => number!(u64),
        _ => Err(invalid("property value/type mismatch")),
    }
}
pub(crate) fn accepts(property: &CameraProperty, value: &CameraValue) -> bool {
    if !property.writable || encode_value(value, property.data_type).is_err() {
        return false;
    }
    if !property.supported_values.is_empty() {
        return property.supported_values.contains(value);
    }
    if let Some([min, max, step]) = property.range {
        return value
            .integer()
            .is_some_and(|n| n >= min && n <= max && (n - min) % step == 0);
    }
    // An unconstrained property description does not prove a safe editable domain.
    false
}

#[derive(Debug)]
pub(crate) struct ObjectInfo {
    pub format: u16,
    pub size: usize,
    pub filename: String,
}
pub(crate) fn parse_object_info(bytes: &[u8]) -> Result<ObjectInfo, CameraError> {
    parse_ram_object_info(bytes, false)
}
/// Only the identity-verified D7100 NEF RAM path may use this compatibility.
pub(crate) fn parse_d7100_ram_object_info(bytes: &[u8]) -> Result<ObjectInfo, CameraError> {
    parse_ram_object_info(bytes, true)
}
fn d7100_object_date(c: &mut Cursor<'_>) -> Result<String, CameraError> {
    let count = c.u8()? as usize;
    if count == 0 {
        return Ok(String::new());
    }
    let units: Vec<u16> = c.take(count * 2)?.chunks_exact(2)
        .map(|x| u16::from_le_bytes([x[0], x[1]])).collect();
    let end = units.iter().position(|unit| *unit == 0)
        .ok_or_else(|| invalid("invalid D7100 ObjectInfo date terminator"))?;
    if units[end..].iter().any(|unit| *unit != 0) {
        return Err(invalid("nonzero D7100 ObjectInfo date padding"));
    }
    String::from_utf16(&units[..end]).map_err(|_| invalid("invalid PTP UTF-16"))
}
fn parse_ram_object_info(bytes: &[u8], d7100: bool) -> Result<ObjectInfo, CameraError> {
    let mut c = Cursor::new(bytes);
    let _storage = c.u32()?;
    let format = c.u16()?;
    let _protection = c.u16()?;
    let size = c.u32()? as usize;
    c.take(40)?;
    let filename = c.string()?;
    let _capture_date = if d7100 { d7100_object_date(&mut c)? } else { c.string()? };
    let _modified = if d7100 { d7100_object_date(&mut c)? } else { c.string()? };
    let _keywords = c.string()?;
    c.finish()?;
    if !(8..=MAX_OBJECT_BYTES).contains(&size)
        || !filename.to_ascii_lowercase().ends_with(".nef")
        || !(matches!(format, 0x3000 | 0xb101) || d7100 && format == 0x3800)
    {
        return Err(invalid("RAM object is not a bounded Nikon NEF"));
    }
    Ok(ObjectInfo {
        format,
        size,
        filename,
    })
}

/// Strict card-resident ObjectInfo (0x1008) metadata for SD-card capture.
///
/// The 52-byte header is the declared PTP order shared with the RAM layout.
/// `parse_ram_object_info` skips the whole 40-byte numeric block; this parser
/// reads ParentObject at its PIMA 15740 offset 38 inside that block, so
/// `parent` is reported evidence rather than a guessed constant. It shares none
/// of the D7100 RAM compatibility tolerance: a reserved storage id, a
/// non-NEF/unsafe filename, an unknown object format, an unbounded size,
/// truncation or trailing bytes are all rejected. The D7100 UTF-16 date form is
/// reused from `d7100_object_date`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardObjectInfo {
    pub storage_id: u32,
    pub parent: u32,
    pub format: u16,
    pub protection: u16,
    pub size: usize,
    pub filename: String,
    pub capture_date: String,
}
pub(crate) fn parse_card_object_info(bytes: &[u8]) -> Result<CardObjectInfo, CameraError> {
    let mut c = Cursor::new(bytes);
    let storage_id = c.u32()?;
    let format = c.u16()?;
    let protection = c.u16()?;
    let size = c.u32()? as usize;
    c.take(26)?; // ThumbFormat, ThumbCompressedSize, thumbnail/image extents, ImageBitDepth
    let parent = c.u32()?;
    c.take(10)?; // AssociationType, AssociationDesc, SequenceNumber
    let filename = c.string()?;
    let capture_date = d7100_object_date(&mut c)?;
    let _modified = d7100_object_date(&mut c)?;
    let _keywords = c.string()?;
    c.finish()?;
    if storage_id == 0 || storage_id == u32::MAX {
        return Err(invalid("card object reports a reserved storage id"));
    }
    if !(8..=MAX_OBJECT_BYTES).contains(&size) {
        return Err(invalid("card object size is outside the bounded NEF range"));
    }
    if !matches!(format, 0x3000 | 0xb101 | 0x3800) {
        return Err(invalid("card object format is not a declared NEF format"));
    }
    let name_chars = filename.chars().count();
    if name_chars == 0
        || name_chars > 255
        || !filename.to_ascii_lowercase().ends_with(".nef")
        || filename
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
    {
        return Err(invalid("card object filename is not a plain NEF name"));
    }
    Ok(CardObjectInfo {
        storage_id,
        parent,
        format,
        protection,
        size,
        filename,
        capture_date,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NikonEvent {
    pub code: u16,
    pub parameter: u32,
}
pub(crate) fn parse_nikon_events(bytes: &[u8]) -> Result<Vec<NikonEvent>, CameraError> {
    let mut c = Cursor::new(bytes);
    let count = c.u16()? as usize;
    if count > 1024 {
        return Err(invalid("Nikon event count exceeds limit"));
    }
    let mut events = Vec::with_capacity(count);
    for _ in 0..count {
        events.push(NikonEvent {
            code: c.u16()?,
            parameter: c.u32()?,
        });
    }
    c.finish()?;
    Ok(events)
}

/// Nikon GetVendorPropCodes: UINT32 count followed by exactly that many UINT16
/// codes (MIT BaseMTPCamera.LoadDeviceInfo). No padding/trailing data accepted.
pub(crate) fn parse_vendor_property_codes(bytes: &[u8]) -> Result<Vec<u16>, CameraError> {
    let mut cursor = Cursor::new(bytes);
    let codes = cursor.array16()?;
    cursor.finish()?;
    if codes
        .iter()
        .any(|code| !matches!(*code, 0x5000..=0x5fff | 0xd000..=0xdfff))
    {
        return Err(invalid(
            "Nikon vendor property list contains a non-property code",
        ));
    }
    Ok(codes)
}
