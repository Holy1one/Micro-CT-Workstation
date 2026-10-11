//! Nikon SDRAM capture, correlated events and complete atomic host-file commit.
//! The associated RAM object event is this transaction's "image available"
//! evidence and the only thing that starts ObjectInfo/transfer; the SDRAM
//! completion is optional. Timing and file appearance never substitute for the
//! associated object, and a `0x4006`/`0xD1B4` property notification is a
//! diagnostic only. Independent capture validates one host NEF output; it does
//! not infer a physical exposure count from one object/C102. Scans additionally
//! require profile evidence for a single fixed physical acquisition.

use super::nikon::*;
use super::ptp::*;
use super::types::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{mpsc::Sender, Mutex};
use std::time::{Duration, Instant};

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
/// The bounded object-arrival window elapsed with no object associated with this
/// operation. It stays a timeout-class refusal (isolate, never commit) so the
/// existing fail-closed handling is unchanged; elapsed time never substitutes for
/// the associated object evidence.
fn capture_window_error(camera: &mut NativeCamera, message: impl Into<String>) -> CameraError {
    let error = CameraError::Timeout(message.into());
    if let Some(batch) = camera.health.last_capture_event_diagnostic.as_mut() {
        batch.response.error = Some(error.to_string());
    }
    error
}
/// Records the independent "one exposure observed" diagnostic: the body published
/// a Nikon `PropertyChanged (0x4006)` notification for the continuous-shooting
/// counter `0xD1B4`. The reference library treats `0x4006` purely as "refresh this
/// property" (`Nikon/NikonBase.cs:2326-2328`) and defines no `0xD1B4`, so this is
/// reported next to the associated RAM object and never as a substitute for it.
fn note_exposure_observation(
    camera: &mut NativeCamera,
    parameter: u32,
    exposure_observed: &mut bool,
) {
    if parameter != CONTINUOUS_SHOOTING_COUNT as u32 {
        return;
    }
    *exposure_observed = true;
    if let Some(batch) = camera.health.last_capture_event_diagnostic.as_mut() {
        batch.exposure_observed = true;
    }
}
fn confirm_frozen_capture(
    camera: &mut NativeCamera,
    expected: &CameraProperties,
    context: &CallContext,
    scan_policy: bool,
) -> Result<(), CameraError> {
    camera.read_properties(context)?;
    if capture_readback(&camera.health,scan_policy)? != *expected {
        return Err(invalid(
            "runtime notification changed frozen capture parameters; commit refused",
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
fn capture_readback(health:&CameraHealth,scan_policy:bool)->Result<CameraProperties,CameraError>{
    if scan_policy { profile_readback(health) } else { independent_readback(health) }
}

fn accept_automatic_output_notification(camera:&mut NativeCamera,parameter:u32,expected:&CameraProperties,context:&CallContext,scan_policy:bool)->Result<bool,CameraError>{
    if scan_policy{return Ok(false);}
    let eligible=camera.health.properties.iter().any(|(key,p)|p.code as u32==parameter&&!expected.contains_key(key)&&automatic_exposure_property(&camera.health,key));
    if !eligible{return Ok(false);}
    confirm_frozen_capture(camera,expected,context,false)?;Ok(true)
}

fn download_current_ram(
    camera: &mut NativeCamera,
    handle: u32,
    session: &str,
    operation_id: &str,
    sender: &Option<Sender<CameraCaptureEvent>>,
    context: &CallContext,
) -> Result<Vec<u8>, CameraError> {
    if !camera.d7100 || !camera.health.properties.get("rawFormat")
        .is_some_and(|property| property.confirmed && property.actual.as_ref().and_then(CameraValue::integer) == Some(4)) {
        return Err(invalid("D7100 RAM ObjectInfo requires confirmed NEF-only format"));
    }
    let metadata = camera.read_data(GET_OBJECT_INFO, &[handle], 64 * 1024, context)?;
    let info = parse_d7100_ram_object_info(&metadata).map_err(|error| {
        // Metadata only: never include image bytes in the capture diagnostic.
        let shown = metadata.len().min(4096);
        let hex: String = metadata[..shown]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        capture_event_error(camera, format!(
            "GetObjectInfo parse failed: opcode=0x{GET_OBJECT_INFO:04X}, handle=0x{handle:08X}, metadataLength={}, metadataShown={shown}, metadataTruncated={}, metadataHex={hex}; {error}",
            metadata.len(), metadata.len() > shown
        ))
    })?;
    emit(
        sender,
        CameraCaptureEvent::TransferStarted {
            session_id: session.into(),
            operation_id: operation_id.into(),
            size: info.size as u64,
        },
        context,
    )?;
    let bytes = camera.read_data(GET_OBJECT, &[handle], info.size, context)?;
    context.check()?;
    if bytes.len() != info.size {
        return Err(invalid("NEF length differs from camera object length"));
    }
    validate_nef(&bytes)?;
    Ok(bytes)
}
#[cfg(test)]
pub(crate) fn capture(camera:&mut NativeCamera,destination:&Path,operation_id:&str,sender:&Option<Sender<CameraCaptureEvent>>,context:&CallContext,commit_gate:&Mutex<()>)->Result<CapturedImage,CameraError>{ capture_with_policy(camera,destination,operation_id,sender,context,commit_gate,true) }
pub(crate) fn capture_with_policy(
    camera: &mut NativeCamera,
    destination: &Path,
    operation_id: &str,
    sender: &Option<Sender<CameraCaptureEvent>>,
    context: &CallContext,
    commit_gate: &Mutex<()>,
    scan_policy: bool,
) -> Result<CapturedImage, CameraError> {
    camera.ensure_connected()?;
    if !camera.health.capabilities.host_capture || !camera.d7100 {
        return Err(CameraError::Unsupported(
            "selected camera has no proven Nikon D7100 host RAM capture".into(),
        ));
    }
    if camera.health.live_view.active {
        return Err(CameraError::Safety(
            "capture prohibited while Live View is active; stop it explicitly".into(),
        ));
    }
    if operation_id.is_empty() || operation_id.len() > 256 || operation_id.contains('\0') {
        return Err(invalid("invalid capture operation ID"));
    }
    if destination
        .extension()
        .is_none_or(|x| !x.eq_ignore_ascii_case("nef"))
        || destination.file_name().is_none()
    {
        return Err(invalid("capture destination must name a NEF file"));
    }
    if destination.exists() {
        return Err(invalid(
            "capture destination already exists; overwrite forbidden",
        ));
    }
    // Refresh all actual camera parameters before capture. The native RAM
    // command does not initiate AF; MF/single/timed NEF also removes ambiguity.
    let expected_profile = capture_readback(&camera.health,scan_policy)?;
    camera.read_properties(context)?;
    if capture_readback(&camera.health,scan_policy)? != expected_profile {
        return Err(invalid(
            "camera parameters changed since the confirmed capture profile",
        ));
    }
    let lv = camera
        .transport
        .execute(
            GET_PROPERTY,
            &[LV_STATUS as u32],
            DataPhase::Read { max: 16 },
            context,
        )?
        .data()?;
    if lv != [0] {
        return Err(invalid("capture requires current confirmed Live View OFF"));
    }
    camera.drain_events(context)?;
    confirm_frozen_capture(camera,&expected_profile,context,scan_policy)?;
    let session = camera
        .health
        .session_id
        .clone()
        .ok_or_else(|| invalid("capture session missing"))?;
    camera.health.last_capture_event_diagnostic = None;
    camera.capture_pending = true;
    let exposure_ms = if scan_policy { shutter_property_ms(&camera.health.properties["shutter"])? } else { 60_000.0 };
    camera.pending_capture = Some(PendingCapture {
        session_id: session.clone(),
        operation_id: operation_id.into(),
        exposure_ms,
        queue_drained: true,
        accepted: false,
    });
    // The destination is already frozen and re-confirmed above: host RAM requires
    // the confirmed `recordingMedia = 1` readback, so this is the RAM path only.
    // The trigger is the Nikon-private SDRAM opcode `0x90C0
    // InitiateCaptureRecInSdram` with parameter `0xFFFFFFFF`, exactly as the
    // reference library's host/SDRAM path does (digiCamControl
    // `CameraControl.Devices/Nikon/NikonBase.cs:61` constant, `:1871` non-Live-View
    // branch, `:1908` Live-View branch). Its card path sends the standard `0x100E
    // InitiateCapture` instead (`BaseMTPCamera.cs:82`), which this module never
    // sends — `card_capture.rs` owns that opcode. The private code is reachable
    // only through the already-negotiated host-capture capability above
    // (`negotiated_capabilities` requires the advertised `0x90C0` operation), so
    // no unadvertised private operation is ever probed; without that advertisement
    // the earlier capability check fails closed and nothing is sent.
    camera
        .transport
        .execute(CAPTURE_RAM, &[0xffffffff], DataPhase::None, context)?
        .success()?;
    camera.pending_capture.as_mut().unwrap().accepted = true;
    // Bounded, second-scale window for the object that this accepted exposure must
    // produce, and it stays inside the camera owner's `CAPTURE_TIMEOUT`
    // (`actor.rs`, where the compile-time assertion also lives). Reaching it
    // without an associated object refuses the file (timeout-class, fail closed);
    // it never removes or shortens the owner deadline.
    let object_arrival_deadline =
        (Instant::now() + super::actor::OBJECT_ARRIVAL_WINDOW).min(context.deadline);
    let mut object = None;
    let mut downloaded = None;
    let mut exposure_ended = false;
    let mut exposure_observed = false;
    let mut batch_sequence = 0;
    let mut event_batches = Vec::new();
    loop {
        if object.is_none() && Instant::now() >= object_arrival_deadline {
            return Err(capture_window_error(
                camera,
                "host RAM object evidence did not arrive inside the bounded object-arrival window; no object is associated with this operation; file commit refused; session isolated",
            ));
        }
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
            CameraEventBatchDiagnostic::record(
                &mut event_batches,
                batch_sequence,
                result.as_ref().map_or(0, Vec::len),
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
                event_count: result.as_ref().map_or(0, Vec::len),
                capture_complete_count: result.as_ref().map_or(0, |events| {
                    events
                        .iter()
                        .filter(|event| event.code == CAPTURE_COMPLETE_RAM)
                        .count()
                }),
                exposure_ended_observed: exposure_ended,
                exposure_observed,
                event_batches: event_batches.clone(),
                response: camera
                    .health
                    .last_event_diagnostic
                    .clone()
                    .unwrap_or_default(),
            });
        }
        let events = result?;
        // Preserve reliable physical termination before any later rejection in
        // this batch. A profile notification cannot erase the same batch's one
        // associated RAM completion. Duplicate completion still has no credit.
        let completions = events
            .iter()
            .filter(|event| event.code == CAPTURE_COMPLETE_RAM)
            .count();
        if completions > 1 || completions > 0 && exposure_ended {
            return Err(capture_event_error(
                camera,
                "duplicate SDRAM capture complete event; file commit refused",
            ));
        }
        if completions == 1 {
            context.check()?;
            if camera.health.session_id.as_deref() != Some(session.as_str()) {
                return Err(capture_event_error(
                    camera,
                    "RAM completion arrived for a changed camera session",
                ));
            }
            exposure_ended = true;
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
                OBJECT_RAM => {
                    if object.is_some() || event.parameter == 0 {
                        return Err(capture_event_error(camera,format!("duplicate/invalid SDRAM object event: code=0x{:04X}, parameter=0x{:08X}",event.code,event.parameter)));
                    }
                    object = Some(event.parameter);
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
                CAPTURE_COMPLETE_RAM => (), // Processed once before profile/event checks.
                0x4002 | 0x400d | 0x400a => {
                    return Err(capture_event_error(camera,format!("unassociated/card capture or store-full event during host RAM capture: code=0x{:04X}, parameter=0x{:08X}",event.code,event.parameter)))
                }
                0x4006 => {
                    note_exposure_observation(camera, event.parameter, &mut exposure_observed);
                    if accept_automatic_output_notification(camera,event.parameter,&expected_profile,context,scan_policy)?{continue;}
                    match camera.accept_runtime_notification(event.parameter, context) {
                        Ok(true) => {
                            if let Err(error)=confirm_frozen_capture(camera,&expected_profile,context,scan_policy) {
                                return Err(capture_event_error(camera,error.to_string()));
                            }
                            continue;
                        }
                        Ok(false) => (),
                        Err(error) => return Err(capture_event_error(camera, error.to_string())),
                    }
                    let property=camera.health.properties.iter().find(|(_,property)|property.code as u32==event.parameter).map(|(key,_)|key.clone()).unwrap_or_else(|| "none".into());
                    return Err(capture_event_error(camera,format!("camera property changed during capture: code=0x4006, parameter=0x{:08X}, frozenProperty={property}; frozen profile invalid; file commit refused",event.parameter)))
                }
                0xc104 => (), // Nikon obsolete-event marker has no capture evidence.
                _ => {
                    return Err(capture_event_error(camera,format!("unexpected camera event during deterministic capture: code=0x{:04X}, parameter=0x{:08X}; file commit refused",event.code,event.parameter)))
                }
            }
        }
        // The reference library's "image available" trigger is the object event
        // (`0x4002`/`0xC101`): it immediately calls `GetObjectInfo(0x1008)` and then
        // transfers with `GetObject(0x1009)` (`CameraControl.Devices/Nikon/
        // NikonBase.cs:2330-2367`) and never waits for the SDRAM completion, which
        // only reaches `OnCaptureCompleted` for bracketing/CLI consumers
        // (`:2369-2375`). So an object associated with this running operation is
        // sufficient evidence to retrieve and commit it, and the completion stays
        // optional. The object may never be inferred from elapsed time, from the
        // `0x4006`/`0xD1B4` diagnostic above, or from a card event.
        if downloaded.is_none() {
            if let Some(handle) = object {
                downloaded = Some(download_current_ram(
                    camera,
                    handle,
                    &session,
                    operation_id,
                    sender,
                    context,
                )?);
                // This operation's own object proves the exposure that produced it
                // has physically ended, so no unresolved pending exposure remains
                // for cleanup. A later `0xC102` is absorbed at the boundary below.
                camera.capture_pending = false;
                camera.pending_capture = None;
            }
        }
        if downloaded.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Download only the handle received for this single in-flight RAM capture.
    // Repeated handles across fully drained operations are allowed because the
    // Nikon RAM handle can be reused; an unresolved operation poisons the session.
    let handle = object.unwrap();
    let bytes = downloaded.unwrap();
    context.check()?;
    // DeviceReady and an empty event queue finish this operation's association
    // boundary. A late/duplicate event never becomes a later frame's evidence.
    //
    // The SDRAM completion is optional here and may arrive early, late or never
    // (`Nikon/NikonBase.cs:2369-2375` consumes it only for `OnCaptureCompleted`,
    // and `:2330-2367` transfers without it), so exactly one completion for this
    // unchanged session is absorbed as this operation's own exposure-end
    // observation. A duplicate completion, a completion for a changed session, or
    // any other extra event still refuses the boundary instead of being credited
    // to this file or to a later one.
    camera.device_ready(context)?;
    let mut boundary_empty = false;
    for _ in 0..8 {
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
                .filter(|event| event.code == CAPTURE_COMPLETE_RAM)
                .count(),
            exposure_ended_observed: exposure_ended,
            exposure_observed,
            event_batches: event_batches.clone(),
            response: camera
                .health
                .last_event_diagnostic
                .clone()
                .unwrap_or_default(),
        });
        for event in events {
            if event.code == 0x4006 {
                note_exposure_observation(camera, event.parameter, &mut exposure_observed);
                if accept_automatic_output_notification(camera,event.parameter,&expected_profile,context,scan_policy)?{continue;}
                match camera.accept_runtime_notification(event.parameter, context) {
                    Ok(true) => {
                        if let Err(error) =
                            confirm_frozen_capture(camera, &expected_profile, context,scan_policy)
                        {
                            return Err(capture_event_error(camera, error.to_string()));
                        }
                        continue;
                    }
                    Ok(false) => (),
                    Err(error) => return Err(capture_event_error(camera, error.to_string())),
                }
            }
            if event.code == CAPTURE_COMPLETE_RAM {
                if exposure_ended || camera.health.session_id.as_deref() != Some(session.as_str()) {
                    return Err(capture_event_error(camera, format!(
                        "duplicate/foreign SDRAM capture complete event after transfer: code=0x{:04X}, parameter=0x{:08X}; association boundary unresolved",
                        event.code, event.parameter
                    )));
                }
                exposure_ended = true;
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
                continue;
            }
            return Err(capture_event_error(camera, format!(
                "extra capture event after transfer: code=0x{:04X}, parameter=0x{:08X}; association boundary unresolved",
                event.code, event.parameter
            )));
        }
    }
    if !boundary_empty {
        return Err(invalid(
            "runtime notification queue did not become empty after transfer; association boundary unresolved",
        ));
    }
    confirm_frozen_capture(camera,&expected_profile,context,scan_policy)?;
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("capture destination has no parent"))?;
    fs::create_dir_all(parent)?;
    let name = destination.file_name().unwrap().to_string_lossy();
    let staging = parent.join(format!(
        ".{name}.{}.part",
        super::live_view::random_session()?
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?;
    let result = (|| {
        let mut hasher = Sha256::new();
        for chunk in bytes.chunks(64 * 1024) {
            context.check()?;
            file.write_all(chunk)?;
            hasher.update(chunk);
        }
        file.sync_all()?;
        context.check()?;
        drop(file);
        let digest = format!("{:x}", hasher.finalize());
        let image = CapturedImage {
            path: destination.into(),
            operation_id: operation_id.into(),
            session_id: session.clone(),
            object_handle: handle,
            size: bytes.len() as u64,
            format: "NEF".into(),
            sha256: digest,
            // This flag marks the committed transaction, not the optional SDRAM
            // completion: the file exists only because an object associated with
            // this operation was retrieved with complete length/identity evidence.
            // Whether the optional `0xC102` was also observed is reported
            // separately by `exposureEndedObserved` in the capture diagnostic.
            exposure_ended: true,
        };
        // Serialize cancellation acceptance with the final atomic rename only.
        // All transfer, decoding, hashing and disk flush remain outside this gate.
        let gate = commit_gate.lock().unwrap();
        context.check()?;
        atomic_commit_no_replace(&staging, destination)?;
        drop(gate);
        emit(
            sender,
            CameraCaptureEvent::FileComplete {
                image: image.clone(),
            },
            context,
        )?;
        Ok(image)
    })();
    if staging.exists() {
        let _ = fs::remove_file(&staging);
    }
    result
}

#[cfg(windows)]
fn atomic_commit_no_replace(staging: &Path, destination: &Path) -> Result<(), CameraError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let source: Vec<u16> = staging.as_os_str().encode_wide().chain([0]).collect();
    let target: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|e| CameraError::Io(e.to_string()))
    }
}
#[cfg(not(windows))]
fn atomic_commit_no_replace(staging: &Path, destination: &Path) -> Result<(), CameraError> {
    fs::hard_link(staging, destination)?;
    fs::remove_file(staging)?;
    Ok(())
}

/// Validate TIFF directories and every out-of-line dataset and raw strip/tile
/// extent before accepting a Nikon NEF. This checks completeness and identity;
/// scientific decoding still belongs to the existing reconstruction pipeline.
pub(crate) fn validate_nef(bytes: &[u8]) -> Result<(), CameraError> {
    if !(8..=MAX_OBJECT_BYTES).contains(&bytes.len()) {
        return Err(invalid("NEF size outside bound"));
    }
    let little = match &bytes[..4] {
        b"II\x2a\0" => true,
        b"MM\0\x2a" => false,
        _ => return Err(invalid("NEF is not TIFF")),
    };
    let short = |at: usize| -> Result<u16, CameraError> {
        let b: [u8; 2] = bytes
            .get(at..at.checked_add(2).ok_or_else(|| invalid("TIFF overflow"))?)
            .ok_or_else(|| invalid("truncated TIFF"))?
            .try_into()
            .unwrap();
        Ok(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let long = |at: usize| -> Result<u32, CameraError> {
        let b: [u8; 4] = bytes
            .get(at..at.checked_add(4).ok_or_else(|| invalid("TIFF overflow"))?)
            .ok_or_else(|| invalid("truncated TIFF"))?
            .try_into()
            .unwrap();
        Ok(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let mut pending = vec![long(4)? as usize];
    let mut visited = HashSet::new();
    let mut nikon = false;
    let mut d7100 = false;
    let mut raw = false;
    while let Some(offset) = pending.pop() {
        if offset == 0 {
            continue;
        }
        if offset < 8 || visited.len() >= 64 || !visited.insert(offset) {
            return Err(invalid("invalid/cyclic/excessive TIFF IFD"));
        }
        let count = short(offset)? as usize;
        if count > 1024 {
            return Err(invalid("TIFF IFD count exceeds limit"));
        }
        let end = offset
            .checked_add(2 + count * 12)
            .ok_or_else(|| invalid("TIFF IFD overflow"))?;
        let mut strip_offsets = vec![];
        let mut strip_lengths = vec![];
        let mut photometric = None;
        let mut width = None;
        let mut height = None;
        for i in 0..count {
            let at = offset + 2 + i * 12;
            let tag = short(at)?;
            let kind = short(at + 2)?;
            let n = long(at + 4)? as usize;
            let size = match kind {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 => 8,
                _ => return Err(invalid("invalid TIFF field type")),
            };
            let length = n
                .checked_mul(size)
                .ok_or_else(|| invalid("TIFF data length overflow"))?;
            let start = if length > 4 {
                long(at + 8)? as usize
            } else {
                at + 8
            };
            let finish = start
                .checked_add(length)
                .ok_or_else(|| invalid("TIFF extent overflow"))?;
            let data = bytes
                .get(start..finish)
                .ok_or_else(|| invalid("TIFF field extends beyond complete file"))?;
            if tag == 271 {
                nikon = data.starts_with(b"NIKON") || data.starts_with(b"Nikon");
            }
            if tag == 272 {
                d7100 = data.starts_with(b"NIKON D7100") || data.starts_with(b"D7100");
            }
            if matches!(tag, 256 | 257 | 262 | 273 | 279 | 324 | 325 | 330 | 34665) {
                if !matches!(kind, 3 | 4 | 13) || n > 65536 {
                    return Err(invalid("TIFF offset/dimension datatype invalid"));
                }
                let values: Vec<usize> = (0..n)
                    .map(|j| {
                        if kind == 3 {
                            short(start + j * 2).map(|n| n as usize)
                        } else {
                            long(start + j * 4).map(|n| n as usize)
                        }
                    })
                    .collect::<Result<_, _>>()?;
                match tag {
                    256 => width = values.first().copied(),
                    257 => height = values.first().copied(),
                    262 => photometric = values.first().copied(),
                    273 | 324 => strip_offsets = values,
                    279 | 325 => strip_lengths = values,
                    330 | 34665 => pending.extend(values),
                    _ => (),
                }
            }
        }
        if !strip_offsets.is_empty() || !strip_lengths.is_empty() {
            if strip_offsets.len() != strip_lengths.len() {
                return Err(invalid("TIFF raw strip count mismatch"));
            }
            for (at, len) in strip_offsets.iter().zip(&strip_lengths) {
                if *at < 8 || *len == 0 || at.checked_add(*len).is_none_or(|n| n > bytes.len()) {
                    return Err(invalid("truncated TIFF raw strip"));
                }
            }
            if photometric == Some(32803)
                && width.is_some_and(|n| n > 0 && n <= 10000)
                && height.is_some_and(|n| n > 0 && n <= 10000)
            {
                raw = true;
            }
        }
        pending.push(long(end)? as usize);
    }
    if !nikon || !d7100 || !raw {
        return Err(invalid(
            "NEF lacks Nikon D7100 identity or bounded CFA raw payload",
        ));
    }
    Ok(())
}
