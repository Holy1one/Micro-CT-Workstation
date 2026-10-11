//! D7100 SD-card capture: one standard PTP exposure per projection, exactly one
//! card object associated with it, and no image bytes across USB.
//!
//! During a card scan the NEF never reaches the PC. The only device reads this
//! module performs are the frozen property readback, a bounded `GetObjectInfo`
//! metadata dataset (at most 64 KiB) and the standard event queue: there is no
//! `GetObject`, no partial object and no thumbnail read, no local file, no
//! synthesized `CapturedFile`/`FileComplete` and no hash. A receipt that names
//! the card object is the whole result; the image stays on the card for a later
//! authorized USB import.
//!
//! The association itself is pure state ([`CardAssociation`]): the transport
//! loop only feeds it parsed evidence, and every rule refuses rather than
//! guesses. An unproven, duplicated, late or unplanned answer never becomes a
//! receipt, and a refusal never clears the session or releases the association
//! as if the exposure had been proved. Real D7100 event ordering, `StoreFull`
//! semantics and card-resident object identity remain BLOCKED until separately
//! authorized hardware acceptance.

use super::nikon::*;
use super::properties::require_card_destination;
use super::ptp::*;
use super::storage::StorageId;
use super::types::*;
use serde::Serialize;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

/// Validates a file name the import derives from an acquisition record before it is
/// joined to the task directory. Only a plain relative name is accepted: no path
/// separator, no parent traversal, no drive or UNC prefix, no control character, no
/// reserved Windows device name and no trailing dot or space. The raw card file name
/// stays provenance and is never used as a target path.
pub(crate) fn safe_import_file_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 255 {
        return Err("IMPORT_TARGET_NAME_INVALID".into());
    }
    if name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
        || name.ends_with('.')
        || name.ends_with(' ')
        || name == "."
        || name == ".."
    {
        return Err("IMPORT_TARGET_NAME_INVALID".into());
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_lowercase();
    const RESERVED: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if RESERVED.contains(&stem.as_str()) {
        return Err("IMPORT_TARGET_NAME_RESERVED".into());
    }
    Ok(())
}

/// Bound for the metadata-only `GetObjectInfo` read. The card dataset is a
/// 52-byte header plus two bounded strings; this is a ceiling, never an image.
const CARD_OBJECT_INFO_MAX: usize = 64 * 1024;
/// How much of a rejected metadata dataset may be shown in the diagnostic.
const CARD_OBJECT_INFO_SHOWN: usize = 4 * 1024;
/// Bounded idle poll inside the association loop, mirroring the RAM loop.
const CARD_POLL_MS: u64 = 10;
/// Event polls allowed at the association boundary before it stays unresolved.
const CARD_BOUNDARY_POLLS: usize = 8;

/// One confirmed card object associated with exactly one triggered exposure.
/// It carries no local path and no local hash: the image bytes never crossed USB
/// in this phase, so no file identity can be claimed here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardObjectReceipt {
    pub session_id: String,
    pub operation_id: String,
    pub storage_id: u32,
    pub storage_changed: bool,
    pub handle: u32,
    pub parent: u32,
    pub filename: String,
    pub format: u16,
    pub size: u64,
    pub capture_date: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardCaptureOutcome {
    pub receipt: CardObjectReceipt,
    pub exposure_ended: bool,
}

/// The caller's frozen card destination decision: which advertised storages this
/// exposure may land on, and whether the overflow continuation is allowed. The
/// exposure itself is not a plan input: it is read from the frozen shutter
/// readback, so no caller can substitute a timing guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardCapturePlan {
    pub storages: Vec<StorageId>,
    pub allow_overflow: bool,
    pub expected_objects: u32,
}

/// One confirmed card object: the parsed metadata plus whether it arrived on the
/// continuation storage rather than the primary one.
#[derive(Clone, Debug)]
struct ConfirmedCardObject {
    handle: u32,
    info: CardObjectInfo,
    storage_changed: bool,
}

/// Pure association state for exactly one card exposure: at most one object, at
/// most one overflow transition and at most one completion, all of them bound to
/// the session frozen before the trigger. It performs no I/O and invents no
/// value, so every refusal rule is testable without a device.
#[derive(Clone, Debug)]
struct CardAssociation {
    session: String,
    operation_id: String,
    storages: Vec<StorageId>,
    allow_overflow: bool,
    object: Option<ConfirmedCardObject>,
    store_full: bool,
    exposure_ended: bool,
}

impl CardAssociation {
    fn new(
        session: &str,
        operation_id: &str,
        storages: &[StorageId],
        allow_overflow: bool,
    ) -> Self {
        Self {
            session: session.into(),
            operation_id: operation_id.into(),
            storages: storages.to_vec(),
            allow_overflow,
            object: None,
            store_full: false,
            exposure_ended: false,
        }
    }

    /// Admission for an `ObjectAdded` handle before its metadata is read: the
    /// handle must be real and no object may already be associated.
    fn begin_object(&self, handle: u32) -> Result<(), String> {
        if handle == 0 {
            return Err("card ObjectAdded reports handle 0; no object can be associated".into());
        }
        if self.object.is_some() {
            return Err(format!(
                "duplicate card ObjectAdded event for handle 0x{handle:08X}; exactly one object per exposure"
            ));
        }
        Ok(())
    }

    /// Admits the parsed metadata of the handle `begin_object` accepted. The
    /// object must come from a planned storage, and a storage other than the
    /// primary one additionally needs an allowed, already observed `StoreFull`
    /// continuation. A duplicate, an unplanned storage or an undocumented slot
    /// change is refused instead of becoming a receipt.
    fn confirm_object(&mut self, handle: u32, info: CardObjectInfo) -> Result<(), String> {
        if self.object.is_some() {
            return Err(format!(
                "duplicate card ObjectInfo for handle 0x{handle:08X}; exactly one object per exposure"
            ));
        }
        let storage = StorageId(info.storage_id);
        if !self.storages.contains(&storage) {
            return Err(format!(
                "card object 0x{handle:08X} arrived on storage 0x{:08X} that this operation did not plan",
                info.storage_id
            ));
        }
        let storage_changed = storage != self.storages[0];
        if storage_changed && !(self.allow_overflow && self.store_full) {
            return Err(format!(
                "card object 0x{handle:08X} changed to storage 0x{:08X} without a planned StoreFull transition",
                info.storage_id
            ));
        }
        self.object = Some(ConfirmedCardObject {
            handle,
            info,
            storage_changed,
        });
        Ok(())
    }

    /// The `StoreFull` continuation. It is only evidence of a storage change and
    /// never substitutes for the object confirmation.
    fn accept_store_full(&mut self) -> Result<(), String> {
        if !self.allow_overflow {
            return Err(
                "card StoreFull arrived although this operation planned no overflow continuation"
                    .into(),
            );
        }
        if self.store_full {
            return Err("duplicate card StoreFull transition; storage continuity unresolved".into());
        }
        self.store_full = true;
        Ok(())
    }

    /// Associates this batch's completion count. `Ok(true)` means the completion
    /// was accepted now. A duplicate, or a completion whose camera session no
    /// longer equals the operation's session, is refused without credit.
    fn accept_completion(
        &mut self,
        completions: usize,
        health_session: Option<&str>,
    ) -> Result<bool, String> {
        if completions > 1 || completions > 0 && self.exposure_ended {
            return Err("duplicate card CaptureComplete event; receipt refused".into());
        }
        if completions == 0 {
            return Ok(false);
        }
        if health_session != Some(self.session.as_str()) {
            return Err("card completion arrived for a changed camera session".into());
        }
        self.exposure_ended = true;
        Ok(true)
    }

    fn is_complete(&self) -> bool {
        self.object.is_some() && self.exposure_ended
    }

    fn receipt(&self) -> Option<CardObjectReceipt> {
        let object = self.object.as_ref()?;
        Some(CardObjectReceipt {
            session_id: self.session.clone(),
            operation_id: self.operation_id.clone(),
            storage_id: object.info.storage_id,
            storage_changed: object.storage_changed,
            handle: object.handle,
            parent: object.info.parent,
            filename: object.info.filename.clone(),
            format: object.info.format,
            size: object.info.size as u64,
            capture_date: object.info.capture_date.clone(),
        })
    }
}

fn emit(
    sender: &Option<Sender<CameraCaptureEvent>>,
    event: CameraCaptureEvent,
    context: &CallContext,
) -> Result<(), CameraError> {
    context.check()?;
    if let Some(sender) = sender {
        let _ = sender.send(event);
    }
    Ok(())
}

fn capture_event_error(camera: &mut NativeCamera, message: impl Into<String>) -> CameraError {
    let error = CameraError::Safety(message.into());
    if let Some(batch) = camera.health.last_capture_event_diagnostic.as_mut() {
        batch.response.error = Some(error.to_string());
    }
    error
}

/// The frozen profile of a card operation.
///
/// `profile_readback` is the RAM scan policy and cannot describe this phase: it
/// requires the confirmed SDRAM destination (`recordingMedia = 1`) through
/// `capture_profile::scan_camera_issues`, while a card operation requires the
/// confirmed card destination (`recordingMedia = 0`). This readback freezes the
/// same required keys with the same confirmed/target agreement rule, the same
/// native code/type checks, manual focus and a decodable shutter, differing only
/// in the destination the operation itself requires. It reads only values the
/// camera actually reported; nothing is substituted or defaulted.
fn card_profile_readback(health: &CameraHealth) -> Result<CameraProperties, CameraError> {
    if !health.connected || health.isolated {
        return Err(CameraError::Safety(
            "confirmed card capture session required".into(),
        ));
    }
    let mut values = CameraProperties::new();
    for key in PROFILE_REQUIRED_KEYS {
        let property = health
            .properties
            .get(*key)
            .ok_or_else(|| CameraError::Safety(format!("card capture profile lacks {key}")))?;
        if !property.confirmed || property.error.is_some() {
            return Err(CameraError::Safety(format!(
                "card capture profile readback {key} is unknown or failed"
            )));
        }
        if property
            .target
            .as_ref()
            .is_some_and(|target| Some(target) != property.actual.as_ref())
        {
            return Err(CameraError::Safety(format!(
                "card capture profile readback {key} does not match its applied target"
            )));
        }
        values.insert(
            (*key).into(),
            property.actual.clone().ok_or_else(|| {
                CameraError::Safety(format!("card capture profile readback {key} is unknown"))
            })?,
        );
    }
    for (key, property) in &health.properties {
        if !property.readable {
            continue;
        }
        if !property.confirmed || property.actual.is_none() {
            return Err(CameraError::Safety(format!(
                "supported capture parameter {key} is unknown"
            )));
        }
        if property
            .target
            .as_ref()
            .is_some_and(|target| Some(target) != property.actual.as_ref())
        {
            return Err(CameraError::Safety(format!(
                "supported capture parameter {key} differs from its applied target"
            )));
        }
        values.insert(key.clone(), property.actual.clone().unwrap());
    }
    // The card destination replaces the RAM scan policy's SDRAM value; every
    // other fixed acquisition value is identical to the RAM scan profile.
    for (key, required) in [
        ("exposureMode", 1),
        ("autoIso", 0),
        ("rawFormat", 4),
        ("driveMode", 1),
        ("burstNumber", 1),
        ("recordingMedia", 0),
        ("exposureDelay", 3),
        ("longExposureNoiseReduction", 0),
    ] {
        if values.get(key) != Some(&CameraValue::Integer(required)) {
            return Err(CameraError::Safety(format!(
                "card capture requires {key}={required}"
            )));
        }
    }
    if health.properties.get("hdrMode").is_some_and(|property| {
        (property.supported || property.error.is_some())
            && (!property.confirmed || property.actual != Some(CameraValue::Integer(0)))
    }) {
        return Err(CameraError::Safety(
            "card capture requires a declared HDR property confirmed OFF".into(),
        ));
    }
    if !values
        .get("focusMode")
        .and_then(CameraValue::integer)
        .is_some_and(|mode| matches!(mode, 3 | 4))
    {
        return Err(CameraError::Safety(
            "card capture requires physical MF (3) or software MF (4); AF modes are refused".into(),
        ));
    }
    for (key, code, kind) in [
        ("recordingMedia", RECORDING_MEDIA, 2),
        ("burstNumber", BURST_NUMBER, 4),
    ] {
        let property = health
            .properties
            .get(key)
            .ok_or_else(|| CameraError::Safety(format!("card capture profile lacks {key}")))?;
        if property.code != code || property.data_type != kind {
            return Err(invalid(&format!(
                "card capture profile {key} native code/type is unsupported"
            )));
        }
    }
    shutter_property_ms(
        health
            .properties
            .get("shutter")
            .ok_or_else(|| CameraError::Safety("card capture shutter description missing".into()))?,
    )?;
    Ok(values)
}

/// Re-reads the device and refuses a receipt when the frozen card profile, the
/// card destination or Live View OFF cannot be confirmed again. This mirrors the
/// RAM `confirm_frozen_capture` without its SDRAM-only runtime diagnostic, which
/// by definition cannot describe a card destination.
fn confirm_frozen_card_profile(
    camera: &mut NativeCamera,
    expected: &CameraProperties,
    context: &CallContext,
) -> Result<(), CameraError> {
    camera.read_properties(context)?;
    if card_profile_readback(&camera.health)? != *expected {
        return Err(invalid(
            "runtime notification changed frozen card capture parameters; receipt refused",
        ));
    }
    let live_view = camera.read_data(GET_PROPERTY, &[LV_STATUS as u32], 16, context)?;
    if live_view != [0] {
        return Err(invalid(
            "runtime notification requires fresh confirmed Live View OFF",
        ));
    }
    require_card_destination(&camera.health)?;
    Ok(())
}

/// Accepts one `0x4006` property notification exactly as the RAM scan path does:
/// card capture is a fixed-scan operation, so an automatic-exposure output
/// notification is never admitted (`accept_automatic_output_notification`
/// returns false for `scan_policy == true`) and only the runtime-diagnostic path
/// can accept it, after the frozen card profile is confirmed again. Anything
/// else is refused by the caller with the property name.
///
/// The card destination owns its runtime confirmation: the shared SDRAM
/// confirmation requires `recordingMedia=1` and would therefore reject every card
/// session. Only the advertised read-only runtime diagnostic is accepted here,
/// through the destination-aware confirmation with the card media value, and the
/// frozen card profile is confirmed again afterwards. Every other property change
/// stays refused with its native code.
fn accept_property_notification(
    camera: &mut NativeCamera,
    expected: &CameraProperties,
    event: &NikonEvent,
    context: &CallContext,
) -> Result<bool, CameraError> {
    if event.parameter != INFO_DISPLAY_ERROR_STATUS as u32 {
        return Ok(false);
    }
    camera
        .confirm_runtime_diagnostic_for(context, 0)
        .map_err(|error| capture_event_error(camera, error.to_string()))?;
    confirm_frozen_card_profile(camera, expected, context)
        .map(|_| true)
        .map_err(|error| capture_event_error(camera, error.to_string()))
}

/// Records one consumed event batch in the same capture diagnostics the RAM path
/// keeps, so a refused card operation leaves its refusal chargeable to a batch.
#[allow(clippy::too_many_arguments)]
fn record_event_batch(
    camera: &mut NativeCamera,
    session: &str,
    operation_id: &str,
    batch_sequence: u64,
    event_count: usize,
    completion_count: usize,
    exposure_ended: bool,
    event_batches: &mut Vec<CameraEventBatchDiagnostic>,
) {
    CameraEventBatchDiagnostic::record(
        event_batches,
        batch_sequence,
        event_count,
        camera.health.last_event_diagnostic.clone().unwrap_or_default(),
    );
    camera.health.last_capture_event_diagnostic = Some(CameraCaptureEventDiagnostic {
        session_id: session.into(),
        operation_id: operation_id.into(),
        batch_sequence,
        event_count,
        capture_complete_count: completion_count,
        exposure_ended_observed: exposure_ended,
        // The continuous-shooting-counter diagnostic describes the host-RAM path
        // this card operation never uses; the card association is proved by the
        // standard `ObjectAdded`/`CaptureComplete` pair below and never by a
        // `0x4006` property notification.
        exposure_observed: false,
        // The post-commit drain window belongs to the host-RAM path only: a card
        // operation never arms it and never absorbs a late RAM completion.
        late_completion_absorbed_count: 0,
        late_completion_parameter: None,
        event_batches: event_batches.clone(),
        response: camera
            .health
            .last_event_diagnostic
            .clone()
            .unwrap_or_default(),
    });
}

fn completion_count(events: &[NikonEvent]) -> usize {
    events
        .iter()
        .filter(|event| event.code == CAPTURE_COMPLETE_CARD)
        .count()
}

/// Triggers exactly one standard PTP exposure and associates exactly one card
/// object with it. No image byte is requested: the only device read carrying
/// object data is the bounded `GetObjectInfo` metadata dataset.
pub(crate) fn capture_to_card(
    camera: &mut NativeCamera,
    plan: &CardCapturePlan,
    operation_id: &str,
    sender: &Option<Sender<CameraCaptureEvent>>,
    context: &CallContext,
) -> Result<CardCaptureOutcome, CameraError> {
    camera.ensure_connected()?;
    require_card_destination(&camera.health)?;
    if operation_id.is_empty() || operation_id.len() > 256 || operation_id.contains('\0') {
        return Err(invalid("invalid capture operation ID"));
    }
    if plan.expected_objects != 1 {
        return Err(invalid(
            "card capture associates exactly one object per exposure",
        ));
    }
    if plan.storages.is_empty() || plan.storages.len() > 2 {
        return Err(invalid("card capture requires one or two advertised storages"));
    }
    if plan.storages.len() < 2 && plan.allow_overflow {
        return Err(invalid(
            "card overflow continuation requires a second planned storage",
        ));
    }
    if plan.allow_overflow && !camera.health.capabilities.card_overflow {
        return Err(CameraError::Unsupported(
            "card overflow continuation events are not advertised".into(),
        ));
    }
    // Freeze the scan profile, then prove it survives a fresh device read. Two
    // planned storages without the continuation flag are accepted: the object
    // then has to come from the primary storage, which the association enforces
    // instead of trusting the listing order.
    let expected_profile = card_profile_readback(&camera.health)?;
    camera.read_properties(context)?;
    if card_profile_readback(&camera.health)? != expected_profile {
        return Err(invalid(
            "camera parameters changed since the confirmed card capture profile",
        ));
    }
    require_card_destination(&camera.health)?;
    let live_view = camera
        .transport
        .execute(
            GET_PROPERTY,
            &[LV_STATUS as u32],
            DataPhase::Read { max: 16 },
            context,
        )?
        .data()?;
    if live_view != [0] {
        return Err(invalid("card capture requires current confirmed Live View OFF"));
    }
    camera.drain_events(context)?;
    confirm_frozen_card_profile(camera, &expected_profile, context)?;
    let session = camera
        .health
        .session_id
        .clone()
        .ok_or_else(|| invalid("capture session missing"))?;
    camera.health.last_capture_event_diagnostic = None;
    camera.capture_pending = true;
    let exposure_ms = shutter_property_ms(
        camera
            .health
            .properties
            .get("shutter")
            .ok_or_else(|| invalid("card capture shutter property missing"))?,
    )?;
    camera.pending_capture = Some(PendingCapture {
        session_id: session.clone(),
        operation_id: operation_id.into(),
        exposure_ms,
        queue_drained: true,
        accepted: false,
    });
    // The standard PTP still-capture operation, with parameter `0xFFFFFFFF` and
    // zero image payload. The reference library's card path sends exactly this
    // opcode (`CameraControl.Devices/BaseMTPCamera.cs:82` constant, used at
    // `Nikon/NikonBase.cs:1872`); a Nikon private capture code is never sent on
    // the card path, and an unproven opcode is never probed.
    camera
        .transport
        .execute(INITIATE_CAPTURE, &[0xffffffff], DataPhase::None, context)?
        .success()?;
    camera.pending_capture.as_mut().unwrap().accepted = true;
    let mut association = CardAssociation::new(
        &session,
        operation_id,
        &plan.storages,
        plan.allow_overflow,
    );
    let mut batch_sequence = 0u64;
    let mut event_batches = Vec::new();
    loop {
        context.check()?;
        let result = camera.events(context);
        let nonempty = result.as_ref().is_ok_and(|events| !events.is_empty())
            || result.is_err()
                && camera
                    .health
                    .last_event_diagnostic
                    .as_ref()
                    .is_some_and(|diagnostic| diagnostic.dataset_size > 0);
        if nonempty {
            batch_sequence += 1;
            record_event_batch(
                camera,
                &session,
                operation_id,
                batch_sequence,
                result.as_ref().map_or(0, Vec::len),
                result.as_ref().map_or(0, |events| completion_count(events)),
                association.exposure_ended,
                &mut event_batches,
            );
        }
        let events = result?;
        // Preserve reliable physical termination before any later rejection in
        // this batch, exactly as the RAM loop does: the association's own
        // duplicate and session rules decide whether the completion counts.
        let session_now = camera.health.session_id.clone();
        if association
            .accept_completion(completion_count(&events), session_now.as_deref())
            .map_err(|rule| capture_event_error(camera, rule))?
        {
            camera.capture_pending = false;
            camera.pending_capture = None;
            if let Some(batch) = camera.health.last_capture_event_diagnostic.as_mut() {
                batch.exposure_ended_observed = true;
            }
            emit(
                sender,
                CameraCaptureEvent::ExposureEnded {
                    session_id: session.clone(),
                    operation_id: operation_id.into(),
                },
                context,
            )?;
        }
        for event in events {
            match event.code {
                OBJECT_ADDED => {
                    association
                        .begin_object(event.parameter)
                        .map_err(|rule| capture_event_error(camera, rule))?;
                    // Metadata only: never an image, never a local copy.
                    let metadata = camera.read_data(
                        GET_OBJECT_INFO,
                        &[event.parameter],
                        CARD_OBJECT_INFO_MAX,
                        context,
                    )?;
                    let info = parse_card_object_info(&metadata).map_err(|error| {
                        let shown = metadata.len().min(CARD_OBJECT_INFO_SHOWN);
                        let hex: String = metadata[..shown]
                            .iter()
                            .map(|byte| format!("{byte:02X}"))
                            .collect();
                        capture_event_error(
                            camera,
                            format!(
                                "GetObjectInfo parse failed: opcode=0x{GET_OBJECT_INFO:04X}, handle=0x{:08X}, metadataLength={}, metadataShown={shown}, metadataTruncated={}, metadataHex={hex}; {error}",
                                event.parameter,
                                metadata.len(),
                                metadata.len() > shown
                            ),
                        )
                    })?;
                    association
                        .confirm_object(event.parameter, info)
                        .map_err(|rule| capture_event_error(camera, rule))?;
                    emit(
                        sender,
                        CameraCaptureEvent::ObjectAdded {
                            session_id: session.clone(),
                            operation_id: operation_id.into(),
                            handle: event.parameter,
                        },
                        context,
                    )?;
                }
                CAPTURE_COMPLETE_CARD => (), // Associated once, before this loop.
                STORE_FULL => {
                    association
                        .accept_store_full()
                        .map_err(|rule| capture_event_error(camera, rule))?;
                }
                STORE_REMOVED => {
                    return Err(capture_event_error(camera, format!(
                        "storage removal during card capture: code=0x{:04X}, parameter=0x{:08X}; no receipt and no association credit",
                        event.code, event.parameter
                    )))
                }
                OBJECT_RAM | CAPTURE_COMPLETE_RAM => {
                    return Err(capture_event_error(camera, format!(
                        "host RAM capture evidence during a card operation: code=0x{:04X}, parameter=0x{:08X}; unassociated evidence refused",
                        event.code, event.parameter
                    )))
                }
                PROPERTY_CHANGED => {
                    if accept_property_notification(camera, &expected_profile, &event, context)? {
                        continue;
                    }
                    let property = camera
                        .health
                        .properties
                        .iter()
                        .find(|(_, property)| property.code as u32 == event.parameter)
                        .map(|(key, _)| key.clone())
                        .unwrap_or_else(|| "none".into());
                    return Err(capture_event_error(camera, format!(
                        "camera property changed during card capture: code=0x4006, parameter=0x{:08X}, frozenProperty={property}; frozen profile invalid; receipt refused",
                        event.parameter
                    )));
                }
                0xc104 => (), // Nikon obsolete-event marker has no capture evidence.
                _ => {
                    return Err(capture_event_error(camera, format!(
                        "unexpected camera event during card capture: code=0x{:04X}, parameter=0x{:08X}; receipt refused",
                        event.code, event.parameter
                    )))
                }
            }
        }
        if association.is_complete() {
            break;
        }
        thread::sleep(Duration::from_millis(CARD_POLL_MS));
    }
    // DeviceReady and an empty event queue finish this operation's association
    // boundary. A late, duplicate or mismatched event belongs to no frame of this
    // operation and refuses the receipt instead of extending the association.
    camera.device_ready(context)?;
    let mut boundary_empty = false;
    for _ in 0..CARD_BOUNDARY_POLLS {
        let events = camera.events(context)?;
        if events.is_empty() {
            boundary_empty = true;
            break;
        }
        batch_sequence += 1;
        record_event_batch(
            camera,
            &session,
            operation_id,
            batch_sequence,
            events.len(),
            completion_count(&events),
            association.exposure_ended,
            &mut event_batches,
        );
        for event in events {
            if event.code == PROPERTY_CHANGED {
                if accept_property_notification(camera, &expected_profile, &event, context)? {
                    continue;
                }
            }
            return Err(capture_event_error(camera, format!(
                "extra capture event after card receipt: code=0x{:04X}, parameter=0x{:08X}; association boundary unresolved",
                event.code, event.parameter
            )));
        }
    }
    if !boundary_empty {
        return Err(invalid(
            "runtime notification queue did not become empty after card capture; association boundary unresolved",
        ));
    }
    confirm_frozen_card_profile(camera, &expected_profile, context)?;
    let receipt = association
        .receipt()
        .ok_or_else(|| invalid("card capture completed without an associated object"))?;
    Ok(CardCaptureOutcome {
        receipt,
        exposure_ended: association.exposure_ended,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{scripted_card_camera, ScriptedCameraConfig, ScriptedCameraProbe};
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::{mpsc, Arc};
    use std::time::Instant;

    const SESSION: &str = "00112233445566778899aabbccddeeff";
    const PRIMARY: StorageId = StorageId(0x0001_0001);
    const SECOND: StorageId = StorageId(0x0001_0002);
    const UNPLANNED: StorageId = StorageId(0x0001_0003);
    const HANDLE: u32 = 0x000b_0001;
    const OTHER_HANDLE: u32 = 0x000b_0002;
    const OP: &str = "card-op";
    /// Opcodes a card operation must never issue: image data, partial image data
    /// and thumbnails. The scripted read log is asserted against these.
    const NEVER_READ: [u16; 3] = [GET_OBJECT, 0x101b, 0x100a];

    fn context(ms: u64) -> CallContext {
        CallContext {
            deadline: Instant::now() + Duration::from_millis(ms),
            epoch: 1,
            generation: Arc::new(AtomicU64::new(1)),
        }
    }

    fn card_camera(overflow: bool) -> (NativeCamera, ScriptedCameraProbe, CallContext) {
        let context = context(5_000);
        let (camera, probe) = scripted_card_camera(
            ScriptedCameraConfig {
                card_capture: true,
                card_overflow: overflow,
                ..Default::default()
            },
            SESSION,
            &context,
        );
        (camera, probe, context)
    }

    fn plan(storages: Vec<StorageId>, allow_overflow: bool) -> CardCapturePlan {
        CardCapturePlan {
            storages,
            allow_overflow,
            expected_objects: 1,
        }
    }

    fn ptp_string(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let mut bytes = vec![units.len() as u8];
        bytes.extend(units.into_iter().flat_map(u16::to_le_bytes));
        bytes
    }

    /// Card ObjectInfo dataset in the declared PTP field order: storage, format,
    /// protection, size, the 26-byte thumbnail/image block, ParentObject, the
    /// 10-byte association block, filename, capture date, modification date and
    /// keywords. The date uses the D7100 counted-UTF-16 form.
    fn card_object(storage_id: u32, size: u32, filename: &str) -> Vec<u8> {
        let mut bytes = storage_id.to_le_bytes().to_vec();
        bytes.extend(0x3800u16.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend([0u8; 26]);
        bytes.extend(0x0002_0005u32.to_le_bytes());
        bytes.extend([0u8; 10]);
        bytes.extend(ptp_string(filename));
        bytes.extend(ptp_string("20161004T224936"));
        bytes.extend(ptp_string("20161004T224936"));
        bytes.extend(ptp_string(""));
        bytes
    }

    fn object_added(handle: u32) -> NikonEvent {
        NikonEvent {
            code: OBJECT_ADDED,
            parameter: handle,
        }
    }

    fn completed() -> NikonEvent {
        NikonEvent {
            code: CAPTURE_COMPLETE_CARD,
            parameter: 0,
        }
    }

    fn trigger_count(probe: &ScriptedCameraProbe) -> usize {
        probe.operation_count(INITIATE_CAPTURE)
    }

    #[test]
    fn single_card_exposure_confirms_one_object_and_transfers_no_image_bytes() {
        let (mut camera, probe, context) = card_camera(false);
        probe.set_card_object(HANDLE, card_object(PRIMARY.0, 26_673_241, "DSC_0001.NEF"));
        probe.set_capture_events(vec![vec![object_added(HANDLE)], vec![completed()]]);
        let call_mark = probe.calls().len();
        let read_mark = probe.reads().len();
        let (tx, rx) = mpsc::channel();
        let outcome = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            OP,
            &Some(tx),
            &context,
        )
        .unwrap();
        assert!(outcome.exposure_ended);
        assert_eq!(
            outcome.receipt,
            CardObjectReceipt {
                session_id: SESSION.into(),
                operation_id: OP.into(),
                storage_id: PRIMARY.0,
                storage_changed: false,
                handle: HANDLE,
                parent: 0x0002_0005,
                filename: "DSC_0001.NEF".into(),
                format: 0x3800,
                size: 26_673_241,
                capture_date: "20161004T224936".into(),
            }
        );
        // The associated completion released the pending capture; nothing else
        // did, and the outcome carries no local image evidence at all.
        assert!(!camera.capture_pending);
        assert!(camera.pending_capture.is_none());
        // Exactly one standard still capture with parameter `0xFFFFFFFF`, never the
        // private SDRAM trigger: the reference library's card path sends
        // `InitiateCapture 0x100E` (`CameraControl.Devices/BaseMTPCamera.cs:82`,
        // used at `Nikon/NikonBase.cs:1872`) and its host/SDRAM path is the one that
        // sends `0x90C0` (`NikonBase.cs:61`).
        let calls = probe.calls()[call_mark..].to_vec();
        assert_eq!(
            calls.iter().filter(|code| **code == INITIATE_CAPTURE).count(),
            1
        );
        assert!(!calls.contains(&CAPTURE_RAM));
        assert!(calls.iter().all(|code| !NEVER_READ.contains(code)));
        // No image-payload opcode may appear on a card operation: no whole-object
        // `GetObject 0x1009` and no `0x101b` partial object (the reference never
        // sends the latter at all).
        assert!(!calls.contains(&GET_OBJECT));
        assert!(!calls.contains(&0x101b));
        let requests = probe.requests();
        let trigger: Vec<_> = requests[call_mark..]
            .iter()
            .filter(|(code, _)| *code == INITIATE_CAPTURE)
            .collect();
        assert_eq!(trigger.len(), 1);
        assert_eq!(trigger[0].1, vec![0xffffffff]);
        // Zero image payload: the only reads after the trigger are the event
        // queue, the property readback and one bounded metadata dataset for the
        // associated handle.
        let reads = probe.reads()[read_mark..].to_vec();
        // Evidence for the offline gate: the whole operation's opcodes and read
        // bounds, showing that no image-sized read was ever requested.
        println!("card operation opcodes={calls:X?} reads={reads:?}");
        assert!(reads.iter().all(|(code, max)| {
            matches!(*code, GET_EVENT | GET_PROPERTY | GET_PROPERTY_DESC)
                || *code == GET_OBJECT_INFO && *max == CARD_OBJECT_INFO_MAX
        }));
        assert_eq!(
            reads.iter().filter(|(code, _)| *code == GET_OBJECT_INFO).count(),
            1
        );
        assert!(requests[call_mark..]
            .iter()
            .any(|(code, params)| *code == GET_OBJECT_INFO && params == &[HANDLE]));
        // The coordinator sees the association and the exposure end, and never a
        // transfer or file completion: no image crossed USB.
        let events: Vec<_> = rx.try_iter().collect();
        assert!(events.iter().any(|event| matches!(
            event,
            CameraCaptureEvent::ObjectAdded { handle, .. } if *handle == HANDLE
        )));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, CameraCaptureEvent::ExposureEnded { .. }))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            CameraCaptureEvent::TransferStarted { .. } | CameraCaptureEvent::FileComplete { .. }
        )));
    }

    #[test]
    fn overflow_object_from_the_second_planned_storage_needs_a_store_full_transition() {
        let (mut camera, probe, context) = card_camera(true);
        probe.set_card_object(HANDLE, card_object(SECOND.0, 26_673_241, "DSC_0002.NEF"));
        probe.set_capture_events(vec![
            vec![NikonEvent {
                code: STORE_FULL,
                parameter: PRIMARY.0,
            }],
            vec![object_added(HANDLE)],
            vec![completed()],
        ]);
        let outcome = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY, SECOND], true),
            "card-op-overflow",
            &None,
            &context,
        )
        .unwrap();
        assert!(outcome.receipt.storage_changed);
        assert_eq!(outcome.receipt.storage_id, SECOND.0);
        assert_eq!(outcome.receipt.filename, "DSC_0002.NEF");
        assert!(outcome.exposure_ended);
    }

    #[test]
    fn second_storage_object_without_a_store_full_transition_is_refused() {
        let (mut camera, probe, context) = card_camera(true);
        probe.set_card_object(HANDLE, card_object(SECOND.0, 26_673_241, "DSC_0002.NEF"));
        probe.set_capture_events(vec![vec![object_added(HANDLE)], vec![completed()]]);
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY, SECOND], true),
            "card-op-slot-jump",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("StoreFull"), "{error}");
        // Fail closed: the refusal is chargeable to the batch, and the pending
        // capture is not released as if the exposure had been proved.
        assert!(camera.capture_pending);
        assert!(camera.pending_capture.is_some());
        assert!(camera
            .health
            .last_capture_event_diagnostic
            .as_ref()
            .and_then(|batch| batch.response.error.as_deref())
            .is_some_and(|message| message.contains("StoreFull")));
    }

    #[test]
    fn store_full_is_refused_without_the_overflow_plan_or_a_second_storage() {
        for (name, planned) in [
            (
                "two-storages-without-overflow",
                plan(vec![PRIMARY, SECOND], false),
            ),
            ("single-storage", plan(vec![PRIMARY], false)),
        ] {
            let (mut camera, probe, context) = card_camera(true);
            probe.set_capture_events(vec![vec![NikonEvent {
                code: STORE_FULL,
                parameter: PRIMARY.0,
            }]]);
            let error = capture_to_card(&mut camera, &planned, name, &None, &context).unwrap_err();
            assert!(error.to_string().contains("StoreFull"), "{name}: {error}");
            assert!(camera.capture_pending && camera.pending_capture.is_some());
        }
    }

    #[test]
    fn overflow_plan_requires_the_advertised_overflow_events() {
        // A card body that never advertised StoreFull must not accept a plan that
        // depends on it, and must not be probed with the trigger either.
        let (mut camera, probe, context) = card_camera(false);
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY, SECOND], true),
            "card-op-unadvertised-overflow",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(matches!(error, CameraError::Unsupported(_)), "{error}");
        assert!(error.to_string().contains("overflow"));
        assert_eq!(trigger_count(&probe), 0);
        assert!(!camera.capture_pending);
    }

    #[test]
    fn duplicate_or_second_object_added_is_refused() {
        for (name, batches) in [
            (
                "same-batch-duplicate",
                vec![vec![object_added(HANDLE), object_added(HANDLE)]],
            ),
            (
                "second-object-in-a-later-batch",
                vec![vec![object_added(HANDLE)], vec![object_added(OTHER_HANDLE)]],
            ),
        ] {
            let (mut camera, probe, context) = card_camera(false);
            probe.set_card_object(HANDLE, card_object(PRIMARY.0, 26_673_241, "DSC_0001.NEF"));
            probe.set_card_object(
                OTHER_HANDLE,
                card_object(PRIMARY.0, 26_673_242, "DSC_0002.NEF"),
            );
            probe.set_capture_events(batches);
            let error =
                capture_to_card(&mut camera, &plan(vec![PRIMARY], false), name, &None, &context)
                    .unwrap_err();
            assert!(
                error.to_string().contains("duplicate")
                    || error.to_string().contains("exactly one object"),
                "{name}: {error}"
            );
            assert!(camera.capture_pending && camera.pending_capture.is_some());
        }
    }

    #[test]
    fn zero_handle_object_added_is_refused_without_a_metadata_read() {
        let (mut camera, probe, context) = card_camera(false);
        probe.set_capture_events(vec![vec![object_added(0)]]);
        let read_mark = probe.reads().len();
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-zero-handle",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("handle 0"), "{error}");
        assert_eq!(
            probe.reads()[read_mark..]
                .iter()
                .filter(|(code, _)| *code == GET_OBJECT_INFO)
                .count(),
            0
        );
    }

    #[test]
    fn object_from_an_unplanned_storage_is_refused_even_after_store_full() {
        let (mut camera, probe, context) = card_camera(true);
        probe.set_card_object(HANDLE, card_object(UNPLANNED.0, 26_673_241, "DSC_0003.NEF"));
        probe.set_capture_events(vec![
            vec![NikonEvent {
                code: STORE_FULL,
                parameter: PRIMARY.0,
            }],
            vec![object_added(HANDLE)],
        ]);
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY, SECOND], true),
            "card-op-unplanned",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("did not plan"), "{error}");
        assert!(camera.capture_pending && camera.pending_capture.is_some());
    }

    #[test]
    fn store_removed_is_refused() {
        let (mut camera, probe, context) = card_camera(true);
        probe.set_capture_events(vec![vec![NikonEvent {
            code: STORE_REMOVED,
            parameter: PRIMARY.0,
        }]]);
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY, SECOND], true),
            "card-op-removed",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("storage removal"), "{error}");
        assert!(camera.capture_pending && camera.pending_capture.is_some());
    }

    #[test]
    fn ram_object_and_ram_completion_events_are_refused() {
        for code in [OBJECT_RAM, CAPTURE_COMPLETE_RAM] {
            let (mut camera, probe, context) = card_camera(false);
            probe.set_capture_events(vec![vec![NikonEvent {
                code,
                parameter: 0xffff_0001,
            }]]);
            let error = capture_to_card(
                &mut camera,
                &plan(vec![PRIMARY], false),
                "card-op-ram-evidence",
                &None,
                &context,
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("host RAM capture evidence"),
                "0x{code:04X}: {error}"
            );
            assert!(camera.capture_pending && camera.pending_capture.is_some());
        }
    }

    #[test]
    fn duplicate_capture_complete_card_is_refused() {
        let (mut camera, probe, context) = card_camera(false);
        probe.set_card_object(HANDLE, card_object(PRIMARY.0, 26_673_241, "DSC_0001.NEF"));
        probe.set_capture_events(vec![
            vec![object_added(HANDLE)],
            vec![completed(), completed()],
        ]);
        let (tx, rx) = mpsc::channel();
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-duplicate-completion",
            &Some(tx),
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate"), "{error}");
        assert!(camera.capture_pending && camera.pending_capture.is_some());
        assert!(!rx
            .try_iter()
            .any(|event| matches!(event, CameraCaptureEvent::ExposureEnded { .. })));
    }

    /// The completion gate compares the live camera session with the session
    /// frozen before the trigger. A session change mid-exposure (reconnect, owner
    /// restart) cannot be scripted through the transport without an unsound
    /// self-reference, so the production gate is exercised directly with the live
    /// session value the loop reads.
    #[test]
    fn completion_from_a_changed_session_is_refused() {
        let mut association = CardAssociation::new(SESSION, OP, &[PRIMARY], false);
        let error = association
            .accept_completion(1, Some("another-session"))
            .unwrap_err();
        assert!(error.contains("changed camera session"), "{error}");
        assert!(!association.exposure_ended);
        let error = association.accept_completion(1, None).unwrap_err();
        assert!(error.contains("changed camera session"), "{error}");
        assert!(!association.exposure_ended);
        assert!(association.accept_completion(1, Some(SESSION)).unwrap());
        assert!(association.exposure_ended);
        let error = association.accept_completion(1, Some(SESSION)).unwrap_err();
        assert!(error.contains("duplicate"), "{error}");
    }

    /// The pending capture is armed for this session and operation with the scan
    /// exposure, and an exposure that never produced evidence stays armed with no
    /// receipt; elapsed time never substitutes for the completion.
    #[test]
    fn an_unassociated_card_exposure_stays_armed_and_reports_no_receipt() {
        let context = context(150);
        let (mut camera, probe) = scripted_card_camera(
            ScriptedCameraConfig {
                card_capture: true,
                ..Default::default()
            },
            SESSION,
            &context,
        );
        probe.set_card_object(HANDLE, card_object(PRIMARY.0, 26_673_241, "DSC_0001.NEF"));
        probe.set_capture_events(vec![vec![object_added(HANDLE)]]);
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-timeout",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(matches!(error, CameraError::Timeout(_)), "{error}");
        assert!(camera.capture_pending);
        let pending = camera.pending_capture.as_ref().unwrap();
        assert_eq!(pending.session_id, SESSION);
        assert_eq!(pending.operation_id, "card-op-timeout");
        // Scripted 0x10005 rational shutter: 1/5 s = 200 ms.
        assert_eq!(pending.exposure_ms, 200.0);
        assert!(pending.queue_drained && pending.accepted);
    }

    #[test]
    fn card_capture_requires_the_advertised_card_capability() {
        // The default scripted body negotiates RAM only, so no card operation may
        // be sent; the destination is scripted so only the capability gates.
        let context = context(2_000);
        let (mut camera, probe) =
            scripted_card_camera(ScriptedCameraConfig::default(), SESSION, &context);
        probe.set_actual("recordingMedia", CameraValue::Integer(0));
        camera.read_properties(&context).unwrap();
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-unsupported",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(matches!(error, CameraError::Unsupported(_)), "{error}");
        assert!(error.to_string().contains("card capture"));
        assert_eq!(trigger_count(&probe), 0);
    }

    #[test]
    fn recording_media_other_than_the_card_is_refused() {
        let (mut camera, probe, context) = card_camera(false);
        probe.set_actual("recordingMedia", CameraValue::Integer(1));
        camera.read_properties(&context).unwrap();
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-sdram-media",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("card destination"), "{error}");
        assert_eq!(trigger_count(&probe), 0);
    }

    #[test]
    fn active_live_view_is_refused() {
        let (mut camera, probe, context) = card_camera(false);
        camera.health.live_view.active = true;
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-live-view",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Live View"), "{error}");
        assert_eq!(trigger_count(&probe), 0);
    }

    /// The cached session state is not the evidence: a fresh `GET_PROPERTY
    /// [LV_STATUS]` read from the device refuses the trigger even while the
    /// cached session still looks idle.
    #[test]
    fn a_device_that_reports_live_view_on_refuses_the_trigger() {
        let (mut camera, probe, context) = card_camera(false);
        probe.set_live_view_state(true);
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-device-live-view",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Live View OFF"), "{error}");
        assert_eq!(trigger_count(&probe), 0);
    }

    #[test]
    fn expected_objects_other_than_one_is_refused() {
        for expected in [0u32, 2] {
            let (mut camera, probe, context) = card_camera(false);
            let mut planned = plan(vec![PRIMARY], false);
            planned.expected_objects = expected;
            let error =
                capture_to_card(&mut camera, &planned, "card-op-count", &None, &context)
                    .unwrap_err();
            assert!(error.to_string().contains("exactly one object"), "{error}");
            assert_eq!(trigger_count(&probe), 0);
        }
    }

    #[test]
    fn empty_or_more_than_two_planned_storages_are_refused() {
        for (name, storages) in [
            ("no-storage", vec![]),
            ("three-storages", vec![PRIMARY, SECOND, UNPLANNED]),
        ] {
            let (mut camera, probe, context) = card_camera(true);
            let error =
                capture_to_card(&mut camera, &plan(storages, false), name, &None, &context)
                    .unwrap_err();
            assert!(
                error.to_string().contains("one or two advertised storages"),
                "{name}: {error}"
            );
            assert_eq!(trigger_count(&probe), 0);
        }
    }

    #[test]
    fn empty_nul_or_oversized_operation_id_is_refused() {
        let oversized = "x".repeat(257);
        for (name, operation_id) in [
            ("empty", ""),
            ("nul", "card\0op"),
            ("too-long", oversized.as_str()),
        ] {
            let (mut camera, probe, context) = card_camera(false);
            let error = capture_to_card(
                &mut camera,
                &plan(vec![PRIMARY], false),
                operation_id,
                &None,
                &context,
            )
            .unwrap_err();
            assert!(error.to_string().contains("operation ID"), "{name}: {error}");
            assert_eq!(trigger_count(&probe), 0);
        }
    }

    #[test]
    fn extra_event_after_the_association_boundary_withholds_the_receipt() {
        let (mut camera, probe, context) = card_camera(false);
        probe.set_card_object(HANDLE, card_object(PRIMARY.0, 26_673_241, "DSC_0001.NEF"));
        probe.set_capture_events(vec![
            vec![object_added(HANDLE)],
            vec![completed()],
            // A late second object after both proofs: it belongs to no frame of
            // this operation, so the receipt is refused.
            vec![object_added(OTHER_HANDLE)],
        ]);
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-late-event",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("association boundary unresolved"),
            "{error}"
        );
    }

    #[test]
    fn unknown_event_is_refused_with_code_and_parameter() {
        let (mut camera, probe, context) = card_camera(false);
        probe.set_capture_events(vec![vec![NikonEvent {
            code: 0x4003,
            parameter: 0x0000_00aa,
        }]]);
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-unknown-event",
            &None,
            &context,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("unexpected camera event"), "{message}");
        assert!(
            message.contains("0x4003") && message.contains("000000AA"),
            "{message}"
        );
        assert!(camera.capture_pending && camera.pending_capture.is_some());
    }

    #[test]
    fn missing_camera_session_is_refused_before_the_trigger() {
        let (mut camera, probe, context) = card_camera(false);
        camera.health.session_id = None;
        let error = capture_to_card(
            &mut camera,
            &plan(vec![PRIMARY], false),
            "card-op-no-session",
            &None,
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("session missing"), "{error}");
        assert_eq!(trigger_count(&probe), 0);
        assert!(!camera.capture_pending);
    }

    #[test]
    fn import_target_names_reject_escapes_and_platform_hazards() {
        for name in ["frame-0007.nef", "reference-0001.nef", "a.b.c"] {
            assert!(safe_import_file_name(name).is_ok(), "{name}");
        }
        for name in [
            "", ".", "..", "../escape.nef", "sub/dir.nef", "sub\\dir.nef", "C:frame.nef",
            "\\\\server\\share.nef", "trailing.", "trailing ", "con", "con.nef", "NUL.nef",
            "lpt1.nef", "with\u{0}control.nef", "line\nbreak.nef",
        ] {
            assert!(safe_import_file_name(name).is_err(), "{name} must be refused");
        }
        assert!(safe_import_file_name(&"x".repeat(256)).is_err());
        assert!(safe_import_file_name(&format!("{}.nef", "y".repeat(251))).is_ok());
    }
}
