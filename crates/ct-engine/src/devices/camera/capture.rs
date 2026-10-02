//! Nikon SDRAM capture, correlated events and complete atomic host-file commit.
//! Timing and file appearance never substitute for an exposure-end event.

use super::nikon::*;
use super::ptp::*;
use super::types::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{mpsc::Sender, Mutex};
use std::time::Duration;

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
pub(crate) fn capture(
    camera: &mut NativeCamera,
    destination: &Path,
    operation_id: &str,
    sender: &Option<Sender<CameraCaptureEvent>>,
    context: &CallContext,
    commit_gate: &Mutex<()>,
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
    let expected_profile = profile_readback(&camera.health)?;
    camera.read_properties(context)?;
    if profile_readback(&camera.health)? != expected_profile {
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
    let session = camera
        .health
        .session_id
        .clone()
        .ok_or_else(|| invalid("capture session missing"))?;
    camera.capture_pending = true;
    camera
        .transport
        .execute(CAPTURE_RAM, &[0xffffffff], DataPhase::None, context)?
        .success()?;
    let mut object = None;
    let mut exposure_ended = false;
    loop {
        context.check()?;
        let events = camera.events(context)?;
        for event in events {
            match event.code {
                OBJECT_RAM => {
                    if object.is_some() || event.parameter == 0 {
                        return Err(invalid("duplicate/invalid SDRAM object event"));
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
                CAPTURE_COMPLETE_RAM => {
                    if exposure_ended {
                        return Err(invalid("duplicate SDRAM capture complete event"));
                    }
                    exposure_ended = true;
                    camera.capture_pending = false;
                    emit(
                        sender,
                        CameraCaptureEvent::ExposureEnded {
                            session_id: session.clone(),
                            operation_id: operation_id.into(),
                        },
                        context,
                    )?;
                }
                0x4002 | 0x400d | 0x400a => {
                    return Err(invalid(
                        "unassociated/card capture or store-full event during host RAM capture",
                    ))
                }
                0x4006 => {
                    return Err(invalid(
                        "camera property changed during capture; frozen profile invalid",
                    ))
                }
                0xc104 => (), // Nikon obsolete-event marker has no capture evidence.
                _ => {
                    return Err(invalid(
                        "unexpected camera event during deterministic capture",
                    ))
                }
            }
        }
        if exposure_ended && object.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Download only the handle received for this single in-flight RAM capture.
    // Repeated handles across fully drained operations are allowed because the
    // Nikon RAM handle can be reused; an unresolved operation poisons the session.
    let handle = object.unwrap();
    let info = parse_object_info(
        &camera
            .transport
            .execute(
                GET_OBJECT_INFO,
                &[handle],
                DataPhase::Read { max: 64 * 1024 },
                context,
            )?
            .data()?,
    )?;
    let _ = (&info.filename, info.format);
    emit(
        sender,
        CameraCaptureEvent::TransferStarted {
            session_id: session.clone(),
            operation_id: operation_id.into(),
            size: info.size as u64,
        },
        context,
    )?;
    let bytes = camera
        .transport
        .execute(
            GET_OBJECT,
            &[handle],
            DataPhase::Read { max: info.size },
            context,
        )?
        .data()?;
    context.check()?;
    if bytes.len() != info.size {
        return Err(invalid("NEF length differs from camera object length"));
    }
    validate_nef(&bytes)?;
    context.check()?;
    // DeviceReady and an empty event queue finish this operation's association
    // boundary. A late/duplicate event never becomes a later frame's evidence.
    camera.device_ready(context)?;
    if !camera.events(context)?.is_empty() {
        return Err(invalid(
            "extra capture event after transfer; association boundary unresolved",
        ));
    }
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
