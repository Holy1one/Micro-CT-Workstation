//! D7100 SD-card capture: one exposure per projection, one associated card object,
//! and no image bytes over USB.
//!
//! During a scan the NEF must stay on the camera card: this module triggers the
//! standard `InitiateCapture` opcode, associates exactly one `ObjectAdded` object
//! and one `CaptureComplete` event with that exposure, and returns a receipt that
//! names the object and its storage. It never downloads the image, never issues a
//! partial-object or thumbnail read, never writes a local file and never produces
//! a content hash — the later authorized USB import owns those steps.
//!
//! Everything here is fail closed. A missing capability, an unconfirmed card
//! destination, a duplicate or unassociated event, an object from an unplanned
//! storage, a removed store, a RAM capture event, a changed session or any
//! trailing event refuses the operation instead of guessing that an exposure was
//! saved. Real D7100 event ordering and store semantics still require the
//! hardware acceptance described in docs/decisions/0004.

use super::nikon::*;
use super::properties::require_card_destination;
use super::ptp::*;
use super::storage::StorageId;
use super::types::*;
use serde::Serialize;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Bounded post-confirmation drain: the association boundary needs an empty event
/// queue, and a device that keeps producing events must not spin forever.
const BOUNDARY_DRAIN_ATTEMPTS: usize = 8;
/// Event poll interval while waiting for this exposure's object and completion.
const CARD_EVENT_POLL: Duration = Duration::from_millis(10);

/// Object receipt for one card-resident exposure. It deliberately carries no
/// local path and no local hash: the image never reaches the PC in this phase.
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

/// One card-capture transaction's plan. Only a single or overflow (two storage)
/// destination is expressible; Backup and RAW/JPEG split are not.
#[derive(Clone, Debug, PartialEq)]
pub struct CardCapturePlan {
    /// Writable storages in the camera's advertised order.
    pub storages: Vec<StorageId>,
    /// Overflow continuation across the second storage is allowed. Requires the
    /// negotiated `card_overflow` capability and at least two storages.
    pub allow_overflow: bool,
    /// This phase accepts exactly one object per exposure.
    pub expected_objects: u32,
    /// Frozen timed exposure used only to bound the pending-capture cleanup wait.
    pub exposure_ms: f64,
}

impl CardCapturePlan {
    pub(crate) fn validate(&self, capabilities: &CameraCapabilities) -> Result<(), CameraError> {
        if self.expected_objects != 1 {
            return Err(invalid("card capture accepts exactly one object per exposure"));
        }
        if self.storages.is_empty() {
            return Err(invalid("card capture requires at least one writable storage"));
        }
        if self.storages.len() > 2 {
            return Err(invalid(
                "card capture supports only a single card or a two-card overflow",
            ));
        }
        if !self.exposure_ms.is_finite() || self.exposure_ms <= 0.0 {
            return Err(invalid("card capture requires a finite positive exposure"));
        }
        if self.allow_overflow {
            if self.storages.len() < 2 {
                return Err(invalid(
                    "card overflow needs two planned storages",
                ));
            }
            if !capabilities.card_overflow {
                return Err(CameraError::Unsupported(
                    "card overflow is not advertised for this identity".into(),
                ));
            }
        }
        Ok(())
    }
}

/// What the device layer must do after the pure association accepted an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardEventAction {
    Accepted,
    /// A property-changed notification; the device layer owns its confirmation.
    PropertyChanged(u32),
}

/// Pure association state for one card exposure. It is deliberately independent
/// of the transport so every refusal rule is testable without a device.
#[derive(Clone)]
pub(crate) struct CardAssociation {
    session_id: String,
    operation_id: String,
    plan: CardCapturePlan,
    object: Option<CardObjectReceipt>,
    store_full_seen: bool,
    completion_seen: bool,
}

impl CardAssociation {
    pub(crate) fn new(session_id: String, operation_id: &str, plan: CardCapturePlan) -> Self {
        Self {
            session_id,
            operation_id: operation_id.to_owned(),
            plan,
            object: None,
            store_full_seen: false,
            completion_seen: false,
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.completion_seen && self.object.is_some()
    }

    /// Feeds one event. `resolve` reads the object metadata for an `ObjectAdded`
    /// handle; it is the only device access this state machine needs, and it must
    /// return card ObjectInfo (never image bytes).
    pub(crate) fn observe(
        &mut self,
        event: &NikonEvent,
        current_session: &str,
        resolve: &mut dyn FnMut(u32) -> Result<CardObjectInfo, CameraError>,
    ) -> Result<CardEventAction, CameraError> {
        if self.is_complete() {
            return Err(invalid(&format!(
                "extra camera event after the card association boundary: code=0x{:04X}, parameter=0x{:08X}",
                event.code, event.parameter
            )));
        }
        match event.code {
            OBJECT_ADDED => {
                if self.object.is_some() || event.parameter == 0 {
                    return Err(invalid(&format!(
                        "duplicate or invalid card object event: code=0x{:04X}, parameter=0x{:08X}",
                        event.code, event.parameter
                    )));
                }
                let info = resolve(event.parameter)?;
                let storage = StorageId(info.storage_id);
                if !self.plan.storages.contains(&storage) {
                    return Err(invalid(&format!(
                        "card object arrived from an unplanned storage 0x{:08X}",
                        info.storage_id
                    )));
                }
                let first = self.plan.storages[0];
                let storage_changed = storage != first;
                if storage_changed {
                    if !self.plan.allow_overflow {
                        return Err(invalid(
                            "card object arrived from another storage without an overflow plan",
                        ));
                    }
                    if !self.store_full_seen {
                        return Err(invalid(
                            "card object arrived from another storage without a store-full transition",
                        ));
                    }
                }
                self.object = Some(CardObjectReceipt {
                    session_id: self.session_id.clone(),
                    operation_id: self.operation_id.clone(),
                    storage_id: info.storage_id,
                    storage_changed,
                    handle: event.parameter,
                    parent: info.parent,
                    filename: info.filename,
                    format: info.format,
                    size: info.size as u64,
                    capture_date: info.capture_date,
                });
                Ok(CardEventAction::Accepted)
            }
            CAPTURE_COMPLETE_CARD => {
                if self.completion_seen {
                    return Err(invalid(
                        "duplicate card capture complete event; receipt refused",
                    ));
                }
                if current_session != self.session_id {
                    return Err(invalid(
                        "card completion arrived for a changed camera session",
                    ));
                }
                self.completion_seen = true;
                Ok(CardEventAction::Accepted)
            }
            STORE_FULL => {
                if !self.plan.allow_overflow {
                    return Err(invalid(
                        "store-full notification without an overflow plan",
                    ));
                }
                if self.store_full_seen {
                    return Err(invalid("duplicate store-full notification"));
                }
                // The notification never proves that this exposure was saved; the
                // object confirmation still has to arrive from a planned storage.
                self.store_full_seen = true;
                Ok(CardEventAction::Accepted)
            }
            STORE_REMOVED => Err(invalid(
                "card storage was removed during a card capture",
            )),
            OBJECT_RAM | CAPTURE_COMPLETE_RAM => Err(invalid(&format!(
                "RAM capture evidence during a card capture: code=0x{:04X}, parameter=0x{:08X}",
                event.code, event.parameter
            ))),
            PROPERTY_CHANGED => Ok(CardEventAction::PropertyChanged(event.parameter)),
            // Nikon's obsolete-event marker carries no capture evidence.
            0xc104 => Ok(CardEventAction::Accepted),
            _ => Err(invalid(&format!(
                "unexpected camera event during card capture: code=0x{:04X}, parameter=0x{:08X}",
                event.code, event.parameter
            ))),
        }
    }

    pub(crate) fn finish(self) -> Result<CardCaptureOutcome, CameraError> {
        let receipt = self
            .object
            .ok_or_else(|| invalid("card capture ended without a confirmed object"))?;
        if !self.completion_seen {
            return Err(invalid(
                "card capture ended without the associated completion event",
            ));
        }
        Ok(CardCaptureOutcome {
            receipt,
            exposure_ended: true,
        })
    }
}

fn card_error(camera: &mut NativeCamera, message: impl Into<String>) -> CameraError {
    let error = CameraError::Safety(message.into());
    if let Some(batch) = camera.health.last_capture_event_diagnostic.as_mut() {
        batch.response.error = Some(error.to_string());
    }
    error
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

/// Confirms the frozen scan profile and confirmed Live View OFF before a trigger.
fn confirm_frozen_card_capture(
    camera: &mut NativeCamera,
    expected: &CameraProperties,
    context: &CallContext,
) -> Result<(), CameraError> {
    camera.read_properties(context)?;
    if profile_readback(&camera.health)? != *expected {
        return Err(invalid(
            "runtime notification changed frozen capture parameters; card receipt refused",
        ));
    }
    let lv = camera.read_data(GET_PROPERTY, &[LV_STATUS as u32], 16, context)?;
    if lv != [0] {
        return Err(invalid(
            "runtime notification requires fresh confirmed Live View OFF",
        ));
    }
    camera.confirm_runtime_diagnostic(context)?;
    Ok(())
}

/// Triggers one SD-card exposure and returns its object receipt.
///
/// The only device data operation in this path is `GetObjectInfo` metadata. The
/// image is never transferred, so cancellation can never depend on a download and
/// a failed association can never leave a half-written local file.
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
    plan.validate(&camera.health.capabilities)?;

    // Freeze the acquisition, prove Live View OFF and start from a drained queue so
    // an old object or completion can never become this exposure's evidence.
    let expected_profile = profile_readback(&camera.health)?;
    camera.read_properties(context)?;
    if profile_readback(&camera.health)? != expected_profile {
        return Err(invalid(
            "camera parameters changed since the confirmed capture profile",
        ));
    }
    let lv = camera.read_data(GET_PROPERTY, &[LV_STATUS as u32], 16, context)?;
    if lv != [0] {
        return Err(invalid("card capture requires current confirmed Live View OFF"));
    }
    camera.drain_events(context)?;
    let session = camera
        .health
        .session_id
        .clone()
        .ok_or_else(|| invalid("card capture session missing"))?;

    camera.health.last_capture_event_diagnostic = None;
    camera.capture_pending = true;
    camera.pending_capture = Some(PendingCapture {
        session_id: session.clone(),
        operation_id: operation_id.into(),
        exposure_ms: plan.exposure_ms,
        queue_drained: true,
        accepted: false,
    });
    // The single proven card trigger. No Nikon private capture code is used here.
    camera
        .transport
        .execute(INITIATE_CAPTURE, &[0xffff_ffff], DataPhase::None, context)?
        .success()?;
    camera.pending_capture.as_mut().unwrap().accepted = true;

    let mut association = CardAssociation::new(session.clone(), operation_id, plan.clone());
    let mut batch_sequence = 0u64;
    let mut event_batches = Vec::new();
    let mut exposure_ended_recorded = false;
    loop {
        context.check()?;
        let events = camera.events(context)?;
        if !events.is_empty() {
            batch_sequence += 1;
            CameraEventBatchDiagnostic::record(
                &mut event_batches,
                batch_sequence,
                events.len(),
                camera
                    .health
                    .last_event_diagnostic
                    .clone()
                    .unwrap_or_default(),
            );
            camera.health.last_capture_event_diagnostic = Some(CameraCaptureEventDiagnostic {
                session_id: session.clone(),
                operation_id: operation_id.into(),
                batch_sequence,
                event_count: events.len(),
                capture_complete_count: events
                    .iter()
                    .filter(|event| event.code == CAPTURE_COMPLETE_CARD)
                    .count(),
                exposure_ended_observed: exposure_ended_recorded,
                event_batches: event_batches.clone(),
                response: camera
                    .health
                    .last_event_diagnostic
                    .clone()
                    .unwrap_or_default(),
            });
        }
        for event in events {
            let current_session = camera.health.session_id.clone().unwrap_or_default();
            let observed = association.observe(&event, &current_session, &mut |handle| {
                let metadata = camera.read_data(GET_OBJECT_INFO, &[handle], 64 * 1024, context)?;
                parse_card_object_info(&metadata)
            });
            let action = match observed {
                Ok(action) => action,
                Err(error) => {
                    let message = error.to_string();
                    return Err(card_error(camera, message));
                }
            };
            match action {
                CardEventAction::Accepted => {
                    if event.code == OBJECT_ADDED {
                        emit(
                            sender,
                            CameraCaptureEvent::ObjectAdded {
                                session_id: session.clone(),
                                operation_id: operation_id.into(),
                                handle: event.parameter,
                            },
                            context,
                        )?;
                    } else if event.code == CAPTURE_COMPLETE_CARD {
                        exposure_ended_recorded = true;
                        emit(
                            sender,
                            CameraCaptureEvent::ExposureEnded {
                                session_id: session.clone(),
                                operation_id: operation_id.into(),
                            },
                            context,
                        )?;
                    }
                }
                CardEventAction::PropertyChanged(parameter) => {
                    // A card scan uses the frozen acquisition policy: an accepted
                    // runtime diagnostic must re-confirm the frozen profile, and any
                    // other property change refuses the receipt.
                    match camera.accept_runtime_notification(parameter, context) {
                        Ok(true) => {
                            if let Err(error) =
                                confirm_frozen_card_capture(camera, &expected_profile, context)
                            {
                                return Err(card_error(camera, error.to_string()));
                            }
                        }
                        Ok(false) => {
                            return Err(card_error(
                                camera,
                                format!(
                                    "camera property changed during card capture: code=0x4006, parameter=0x{parameter:08X}; frozen profile invalid; receipt refused"
                                ),
                            ))
                        }
                        Err(error) => {
                            return Err(card_error(camera, error.to_string()));
                        }
                    }
                }
            }
            if let Some(batch) = camera.health.last_capture_event_diagnostic.as_mut() {
                batch.exposure_ended_observed = exposure_ended_recorded;
            }
        }
        if association.is_complete() {
            break;
        }
        std::thread::sleep(CARD_EVENT_POLL);
    }

    // DeviceReady plus an empty queue finish this exposure's association boundary.
    camera.device_ready(context)?;
    let mut boundary_empty = false;
    for _ in 0..BOUNDARY_DRAIN_ATTEMPTS {
        let events = camera.events(context)?;
        if events.is_empty() {
            boundary_empty = true;
            break;
        }
        batch_sequence += 1;
        CameraEventBatchDiagnostic::record(
            &mut event_batches,
            batch_sequence,
            events.len(),
            camera
                .health
                .last_event_diagnostic
                .clone()
                .unwrap_or_default(),
        );
        camera.health.last_capture_event_diagnostic = Some(CameraCaptureEventDiagnostic {
            session_id: session.clone(),
            operation_id: operation_id.into(),
            batch_sequence,
            event_count: events.len(),
            capture_complete_count: events
                .iter()
                .filter(|event| event.code == CAPTURE_COMPLETE_CARD)
                .count(),
            exposure_ended_observed: exposure_ended_recorded,
            event_batches: event_batches.clone(),
            response: camera
                .health
                .last_event_diagnostic
                .clone()
                .unwrap_or_default(),
        });
        for event in events {
            if event.code == PROPERTY_CHANGED {
                match camera.accept_runtime_notification(event.parameter, context) {
                    Ok(true) => {
                        if let Err(error) =
                            confirm_frozen_card_capture(camera, &expected_profile, context)
                        {
                            return Err(card_error(camera, error.to_string()));
                        }
                        continue;
                    }
                    Ok(false) => (),
                    Err(error) => return Err(card_error(camera, error.to_string())),
                }
            }
            return Err(card_error(
                camera,
                format!(
                    "extra capture event after the card object confirmation: code=0x{:04X}, parameter=0x{:08X}; association boundary unresolved",
                    event.code, event.parameter
                ),
            ));
        }
    }
    if !boundary_empty {
        return Err(invalid(
            "runtime notification queue did not become empty after the card confirmation",
        ));
    }
    if let Err(error) = confirm_frozen_card_capture(camera, &expected_profile, context) {
        return Err(card_error(camera, error.to_string()));
    }
    camera.capture_pending = false;
    camera.pending_capture = None;
    association.finish()
}

#[cfg(test)]
mod tests {
    use super::super::ptp::{invalid, CardObjectInfo};
    use super::*;

    fn plan(storages: &[u32], allow_overflow: bool) -> CardCapturePlan {
        CardCapturePlan {
            storages: storages.iter().copied().map(StorageId).collect(),
            allow_overflow,
            expected_objects: 1,
            exposure_ms: 1000.0,
        }
    }

    fn capabilities(card_overflow: bool) -> CameraCapabilities {
        CameraCapabilities {
            card_capture: true,
            card_overflow,
            ..CameraCapabilities::default()
        }
    }

    fn info(storage: u32, handle: u32) -> CardObjectInfo {
        CardObjectInfo {
            storage_id: storage,
            parent: 0x0100_0001,
            format: 0x3000,
            protection: 0,
            size: 1024,
            filename: format!("DSC_{handle:04}.NEF"),
            capture_date: "20261010T120000".into(),
        }
    }

    fn resolve_ok(storage: u32) -> impl FnMut(u32) -> Result<CardObjectInfo, CameraError> {
        move |handle| Ok(info(storage, handle))
    }

    fn event(code: u16, parameter: u32) -> NikonEvent {
        NikonEvent { code, parameter }
    }

    #[test]
    fn card_plan_accepts_one_or_two_storages_and_refuses_the_rest() {
        assert!(plan(&[1], false).validate(&capabilities(false)).is_ok());
        assert!(plan(&[1, 2], false).validate(&capabilities(false)).is_ok());
        assert!(plan(&[1, 2], true).validate(&capabilities(true)).is_ok());
        // Overflow needs both the advertised capability and a second storage.
        assert!(plan(&[1, 2], true).validate(&capabilities(false)).is_err());
        assert!(plan(&[1], true).validate(&capabilities(true)).is_err());
        // Backup / RAW+JPEG split planning is not expressible.
        assert!(plan(&[1, 2, 3], false).validate(&capabilities(true)).is_err());
        assert!(plan(&[], false).validate(&capabilities(false)).is_err());
        let mut many_objects = plan(&[1], false);
        many_objects.expected_objects = 2;
        assert!(many_objects.validate(&capabilities(false)).is_err());
        let mut no_exposure = plan(&[1], false);
        no_exposure.exposure_ms = 0.0;
        assert!(no_exposure.validate(&capabilities(false)).is_err());
        no_exposure.exposure_ms = f64::NAN;
        assert!(no_exposure.validate(&capabilities(false)).is_err());
    }

    #[test]
    fn one_object_and_one_completion_make_a_receipt_for_the_planned_storage() {
        let mut association = CardAssociation::new("session-1".into(), "op-1", plan(&[7], false));
        let mut resolve = resolve_ok(7);
        assert_eq!(
            association.observe(&event(OBJECT_ADDED, 0x0000_00aa), "session-1", &mut resolve),
            Ok(CardEventAction::Accepted)
        );
        assert!(!association.is_complete());
        assert_eq!(
            association.observe(&event(CAPTURE_COMPLETE_CARD, 0), "session-1", &mut resolve),
            Ok(CardEventAction::Accepted)
        );
        assert!(association.is_complete());
        let outcome = association.finish().unwrap();
        assert!(outcome.exposure_ended);
        assert_eq!(outcome.receipt.storage_id, 7);
        assert!(!outcome.receipt.storage_changed);
        assert_eq!(outcome.receipt.handle, 0x0000_00aa);
        assert_eq!(outcome.receipt.parent, 0x0100_0001);
        assert_eq!(outcome.receipt.filename, "DSC_0170.NEF");
        assert_eq!(outcome.receipt.size, 1024);
        assert_eq!(outcome.receipt.capture_date, "20261010T120000");
        assert_eq!(outcome.receipt.session_id, "session-1");
        assert_eq!(outcome.receipt.operation_id, "op-1");
    }

    #[test]
    fn duplicate_or_invalid_object_events_are_refused() {
        let mut association = CardAssociation::new("s".into(), "op", plan(&[7], false));
        let mut resolve = resolve_ok(7);
        assert!(association
            .observe(&event(OBJECT_ADDED, 0), "s", &mut resolve)
            .is_err());
        assert_eq!(
            association.observe(&event(OBJECT_ADDED, 5), "s", &mut resolve),
            Ok(CardEventAction::Accepted)
        );
        assert!(association
            .observe(&event(OBJECT_ADDED, 6), "s", &mut resolve)
            .is_err());
    }

    #[test]
    fn unplanned_storage_and_unproven_overflow_are_refused() {
        let mut single = CardAssociation::new("s".into(), "op", plan(&[7], false));
        let mut other = resolve_ok(8);
        assert!(single
            .observe(&event(OBJECT_ADDED, 5), "s", &mut other)
            .is_err());

        // Planned overflow still needs the store-full transition before the second
        // storage may carry the object.
        let mut overflow = CardAssociation::new("s".into(), "op", plan(&[7, 8], true));
        assert!(overflow
            .observe(&event(OBJECT_ADDED, 5), "s", &mut other)
            .is_err());
        assert_eq!(
            overflow.observe(&event(STORE_FULL, 0), "s", &mut other),
            Ok(CardEventAction::Accepted)
        );
        assert_eq!(
            overflow.observe(&event(OBJECT_ADDED, 5), "s", &mut other),
            Ok(CardEventAction::Accepted)
        );
        assert_eq!(
            overflow.observe(&event(CAPTURE_COMPLETE_CARD, 0), "s", &mut other),
            Ok(CardEventAction::Accepted)
        );
        let outcome = overflow.finish().unwrap();
        assert_eq!(outcome.receipt.storage_id, 8);
        assert!(outcome.receipt.storage_changed);
    }

    #[test]
    fn store_full_and_store_removed_follow_the_plan() {
        let mut no_plan = CardAssociation::new("s".into(), "op", plan(&[7], false));
        let mut resolve = resolve_ok(7);
        assert!(no_plan
            .observe(&event(STORE_FULL, 0), "s", &mut resolve)
            .is_err());
        assert!(no_plan
            .observe(&event(STORE_REMOVED, 0), "s", &mut resolve)
            .is_err());

        let mut overflow = CardAssociation::new("s".into(), "op", plan(&[7, 8], true));
        let mut other = resolve_ok(8);
        assert_eq!(
            overflow.observe(&event(STORE_FULL, 0), "s", &mut other),
            Ok(CardEventAction::Accepted)
        );
        // A second store-full transition is not additional evidence.
        assert!(overflow
            .observe(&event(STORE_FULL, 0), "s", &mut other)
            .is_err());
    }

    #[test]
    fn ram_evidence_unknown_events_and_a_changed_session_are_refused() {
        let mut association = CardAssociation::new("s".into(), "op", plan(&[7], false));
        let mut resolve = resolve_ok(7);
        assert!(association
            .observe(&event(OBJECT_RAM, 5), "s", &mut resolve)
            .is_err());
        assert!(association
            .observe(&event(CAPTURE_COMPLETE_RAM, 0), "s", &mut resolve)
            .is_err());
        assert_eq!(
            association.observe(&event(0xc104, 0), "s", &mut resolve),
            Ok(CardEventAction::Accepted)
        );
        assert_eq!(
            association.observe(&event(PROPERTY_CHANGED, 0xd10b), "s", &mut resolve),
            Ok(CardEventAction::PropertyChanged(0xd10b))
        );
        assert!(association
            .observe(&event(0x4003, 5), "s", &mut resolve)
            .is_err());

        // A completion from a replaced camera session never closes this exposure.
        let mut changed = CardAssociation::new("s".into(), "op", plan(&[7], false));
        assert!(changed
            .observe(&event(CAPTURE_COMPLETE_CARD, 0), "other-session", &mut resolve)
            .is_err());
        let mut duplicate = CardAssociation::new("s".into(), "op", plan(&[7], false));
        assert_eq!(
            duplicate.observe(&event(CAPTURE_COMPLETE_CARD, 0), "s", &mut resolve),
            Ok(CardEventAction::Accepted)
        );
        assert!(duplicate
            .observe(&event(CAPTURE_COMPLETE_CARD, 0), "s", &mut resolve)
            .is_err());
    }

    #[test]
    fn the_boundary_closes_after_both_proofs_and_an_incomplete_operation_has_no_receipt() {
        let mut association = CardAssociation::new("s".into(), "op", plan(&[7], false));
        let mut resolve = resolve_ok(7);
        assert!(association.clone().finish().is_err());
        assert_eq!(
            association.observe(&event(OBJECT_ADDED, 5), "s", &mut resolve),
            Ok(CardEventAction::Accepted)
        );
        // Object only: still no receipt.
        assert!(association.clone().finish().is_err());
        assert_eq!(
            association.observe(&event(CAPTURE_COMPLETE_CARD, 0), "s", &mut resolve),
            Ok(CardEventAction::Accepted)
        );
        // Any trailing event after both proofs is extra unassociated evidence.
        assert!(association
            .observe(&event(OBJECT_ADDED, 6), "s", &mut resolve)
            .is_err());
        assert!(association
            .observe(&event(CAPTURE_COMPLETE_CARD, 0), "s", &mut resolve)
            .is_err());
        assert!(association.finish().is_ok());
    }

    #[test]
    fn the_card_transaction_has_no_image_transfer_path() {
        // Structural guard: the only data opcode this module may use is
        // GET_OBJECT_INFO metadata. The patterns are assembled at compile time so
        // this assertion does not match its own literals.
        let source = include_str!("card_capture.rs");
        assert!(!source.contains(concat!("GET_OBJECT", ",")), "card capture must not download objects");
        assert!(!source.contains(concat!("GET_PARTIAL", "_OBJECT")), "no partial-object read");
        assert!(!source.contains(concat!("Get", "Thumb")), "no thumbnail read");
        assert!(source.contains("GET_OBJECT_INFO"));
        // No local file or digest is produced for a card-resident image.
        assert!(!source.contains(concat!("sha", "2")));
        assert!(!source.contains(concat!("std::", "fs")));
    }

    #[test]
    fn an_unresolvable_object_metadata_read_refuses_the_receipt() {
        let mut association = CardAssociation::new("s".into(), "op", plan(&[7], false));
        let mut resolve = |_handle| Err(invalid("GetObjectInfo parse failed"));
        assert!(association
            .observe(&event(OBJECT_ADDED, 5), "s", &mut resolve)
            .is_err());
        assert!(!association.is_complete());
    }

    #[test]
    fn validate_rejects_an_object_that_resolves_to_a_foreign_storage() {
        let mut association = CardAssociation::new("s".into(), "op", plan(&[7, 8], true));
        let mut resolve = resolve_ok(9);
        assert!(association
            .observe(&event(OBJECT_ADDED, 5), "s", &mut resolve)
            .is_err());
    }
}
