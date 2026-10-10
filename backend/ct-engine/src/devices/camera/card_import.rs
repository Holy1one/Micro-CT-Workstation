//! Authorized USB import of the card objects a schema-4 card acquisition left on
//! the D7100 SD cards.
//!
//! A card scan never transfers an image: every projection and reference is one
//! card object receipt. This module is the only place that ever requests
//! `GetObject` for those objects, and it does so only for a task that the task
//! registry already retains as `acquired_pending_import`. It contacts the card
//! through the same single camera owner as the scan (the caller passes the one
//! [`super::CameraAdapter`] proxy), never through a second COM session.
//!
//! Fail-closed rules this module implements:
//!
//! - **Source binding.** The import enumerates `GetStorageIDs` →one
//!   `GetStorageInfo` per id →`bind_storage_ids`, then `GetObjectHandles` per
//!   participating storage and one bounded `GetObjectInfo` per handle through the
//!   strict card parser. A record is bound to exactly one object by *storage id +
//!   source file name + size + capture date*. Handles and storage ids are session
//!   evidence only: they are never a permanent identity, and a reconnect simply
//!   re-enumerates from scratch. A record with no candidate, several candidates,
//!   an object in the session's folder that no record claims, a changed camera
//!   identity/serial, a storage a receipt names that is no longer advertised, or
//!   metadata that changed between enumeration and download all refuse the whole
//!   import with a stable `IMPORT_*` code.
//! - **Bounded download and verification.** One `GetObjectInfo` re-read (metadata
//!   only, at most 64 KiB), then `GetObject` bounded by the object's own declared
//!   size, which must be inside `MAX_OBJECT_BYTES`. The complete NEF is validated
//!   with the existing `validate_nef`, the transfer length must equal the declared
//!   length (a truncated or over-long transfer is refused), and the bytes are
//!   written to a task-private staging file in 64 KiB chunks with a cancellation
//!   check and a streaming SHA-256 per chunk, flushed, then committed atomically
//!   **without overwriting**.
//! - **Path safety.** The save root must be absolute, the task directory and every
//!   directory the import writes into must be real directories (no symlink or
//!   Windows reparse point), and every target must canonicalize inside the task
//!   directory. A target name is derived from the record's acquisition index
//!   (`frame-NNNN.nef` under `frames/` or `references/pre-dark|pre-flat/`), never
//!   from the raw card name. An existing target file is never overwritten: it is
//!   adopted only when its length and SHA-256 already match the verified bytes,
//!   and otherwise the import refuses with `IMPORT_TARGET_CONFLICT`.
//! - **Resume.** A record already `verified` whose committed file re-verifies
//!   (same length and SHA-256) is skipped; a record whose verified file is missing
//!   is re-downloaded; a leftover partial staging file is discarded and
//!   re-fetched. Cancellation is bounded, leaves every verified file, the
//!   acquisition journal and the retained pending-import state intact, and the
//!   task stays re-importable.
//! - **Seal.** Only when every projection and reference record is verified with its
//!   local file identity does this module fill `frames`/`references`, set
//!   `dataStatus=verified` and `completed=true` and persist the schema-4 manifest
//!   atomically; the domain then promotes the retained registry record. Until that
//!   seal, `scan::completed_manifest_exists` keeps reconstruction and preview
//!   unavailable.
//!
//! The card itself is read-only here: nothing is deleted, renamed or modified on
//! the card, and the original card object metadata (handle, storage id, parent,
//! source file name) stays untouched in the manifest as provenance.
//!
//! Every conclusion in this module is offline: real D7100 USB download semantics,
//! `GetObjectHandles` parameter behaviour and card-side throughput remain hardware
//! acceptance items.

use super::capture::validate_nef;
use super::nikon::*;
use super::ptp::*;
use super::storage::{
    bind_storage_ids, parse_object_handles, parse_storage_ids, parse_storage_info, StorageId,
    StorageInfo,
};
use super::types::*;
use super::CameraAdapter;
use crate::capture_profile::CaptureDestination;
use crate::scan::{
    card_acquisition_finish_gate, schema_four_completion_consistent, CardCaptureRecord, RealFrame,
    ReferenceFrame, ReferenceFrames, ACQUISITION_ACQUIRED, DATA_IMPORT_FAILED, DATA_IMPORTING,
    DATA_PENDING_IMPORT, DATA_VERIFIED,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The five import statuses the snapshot contract pins.
pub(crate) const IMPORT_STATUS_IDLE: &str = "idle";
pub(crate) const IMPORT_STATUS_RUNNING: &str = "running";
pub(crate) const IMPORT_STATUS_CANCELLED: &str = "cancelled";
pub(crate) const IMPORT_STATUS_FAILED: &str = "failed";
pub(crate) const IMPORT_STATUS_VERIFIED: &str = "verified";

/// Stable refusal codes. A refusal names the code and, where a record is
/// involved, its acquisition index; nothing is ever silently skipped.
pub(crate) const IMPORT_OBJECT_MISSING: &str = "IMPORT_OBJECT_MISSING";
pub(crate) const IMPORT_OBJECT_AMBIGUOUS: &str = "IMPORT_OBJECT_AMBIGUOUS";
pub(crate) const IMPORT_OBJECT_UNEXPECTED: &str = "IMPORT_OBJECT_UNEXPECTED";
pub(crate) const IMPORT_OBJECT_UNREADABLE: &str = "IMPORT_OBJECT_UNREADABLE";
pub(crate) const IMPORT_OBJECT_OVERSIZE: &str = "IMPORT_OBJECT_OVERSIZE";
pub(crate) const IMPORT_OBJECT_CHANGED: &str = "IMPORT_OBJECT_CHANGED";
pub(crate) const IMPORT_IDENTITY_CHANGED: &str = "IMPORT_IDENTITY_CHANGED";
pub(crate) const IMPORT_STORAGE_CHANGED: &str = "IMPORT_STORAGE_CHANGED";
pub(crate) const IMPORT_SOURCE_INVALID: &str = "IMPORT_SOURCE_INVALID";
pub(crate) const IMPORT_SOURCE_INCOMPLETE: &str = "IMPORT_SOURCE_INCOMPLETE";
pub(crate) const IMPORT_RECORD_KIND_UNKNOWN: &str = "IMPORT_RECORD_KIND_UNKNOWN";
pub(crate) const IMPORT_ACQUISITION_UNRESOLVED: &str = "IMPORT_ACQUISITION_UNRESOLVED";
pub(crate) const IMPORT_TRANSFER_TRUNCATED: &str = "IMPORT_TRANSFER_TRUNCATED";
pub(crate) const IMPORT_NEF_INVALID: &str = "IMPORT_NEF_INVALID";
pub(crate) const IMPORT_TARGET_CONFLICT: &str = "IMPORT_TARGET_CONFLICT";
pub(crate) const IMPORT_PATH_ESCAPE: &str = "IMPORT_PATH_ESCAPE";
pub(crate) const IMPORT_COMMIT_FAILED: &str = "IMPORT_COMMIT_FAILED";
pub(crate) const IMPORT_SEAL_INCONSISTENT: &str = "IMPORT_SEAL_INCONSISTENT";

/// Ceiling for one card `GetObjectInfo` metadata dataset. It is a 52-byte header
/// plus two bounded strings, so this bound can never describe image data.
const CARD_IMPORT_INFO_MAX: usize = 64 * 1024;
/// Ceiling for one card storage dataset (`GetStorageIDs`/`GetStorageInfo`).
const CARD_IMPORT_STORAGE_MAX: usize = 64 * 1024;
/// Staging write / verification chunk. One cancellation check per chunk.
const CARD_IMPORT_CHUNK: usize = 64 * 1024;
/// The reference groups a schema-4 card task always writes, and the sealed
/// manifest must hold: 10 pre-dark and 10 pre-flat frames.
const CARD_REFERENCE_COUNT: usize = 10;
/// The fixed four-digit acquisition-index file name cannot address more.
const MAX_RECORD_INDEX: u32 = 9_999;
/// Windows `FILE_ATTRIBUTE_REPARSE_POINT`.
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// A coded refusal: the stable code plus a bounded human-readable detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardImportRefusal {
    pub code: &'static str,
    pub detail: String,
}
impl CardImportRefusal {
    pub(crate) fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}
impl std::fmt::Display for CardImportRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

/// A device-side refusal carried as a `CameraError`, so the caller sees the same
/// stable code in the operation error, the log and the snapshot.
fn device_refusal(code: &'static str, detail: impl std::fmt::Display) -> CameraError {
    CameraError::Safety(format!("{code}: {detail}"))
}

/// One bounded card object metadata set from the strict card parser, keyed by the
/// handle the current session reports. The handle is evidence for this session
/// only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardObjectCandidate {
    pub storage_id: u32,
    pub handle: u32,
    pub parent: u32,
    pub filename: String,
    pub format: u16,
    pub size: u64,
    pub capture_date: String,
}

/// Everything one fresh enumeration read from the card: identity, the advertised
/// storages (positionally bound to their ids) and one candidate per handle whose
/// metadata parsed strictly.
#[derive(Clone, Debug)]
pub(crate) struct CardImportInventory {
    pub identity: CameraIdentity,
    pub storages: Vec<StorageInfo>,
    pub candidates: Vec<CardObjectCandidate>,
    /// How many handles had their `GetObjectInfo` read, for bounded diagnostics.
    pub examined: usize,
}

/// One record bound to exactly one card object for this session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardImportObject {
    pub record_index: u32,
    pub kind: String,
    pub candidate: CardObjectCandidate,
}

/// One verified download: the bytes were streamed to the staging file and hashed
/// while writing, so this is the local file identity the manifest will record.
#[derive(Clone, Debug)]
pub(crate) struct CardImportDownload {
    pub bytes: u64,
    pub sha256: String,
    pub format: u16,
    pub handle: u32,
}

/// Bounded import progress. The worker owns the writes; the engine only copies it
/// for the snapshot and drains its messages, so a slow card or a stuck transfer
/// can never block status reads or Stop.
#[derive(Clone, Debug)]
pub(crate) struct CardImportProgress {
    pub status: &'static str,
    pub operation_id: String,
    pub error: Option<String>,
    pub stage: &'static str,
    /// Every card record of the task.
    pub records: usize,
    /// Records whose committed local file exists and was verified.
    pub imported: usize,
    /// Records durably recorded as verified in the manifest.
    pub verified: usize,
    pub acquired_projections: u32,
    pub imported_projections: u32,
    pub verified_projections: u32,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub messages: Vec<CardImportMessage>,
}
impl Default for CardImportProgress {
    fn default() -> Self {
        Self {
            status: IMPORT_STATUS_IDLE,
            operation_id: String::new(),
            error: None,
            stage: "idle",
            records: 0,
            imported: 0,
            verified: 0,
            acquired_projections: 0,
            imported_projections: 0,
            verified_projections: 0,
            processed_bytes: 0,
            total_bytes: 0,
            messages: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CardImportMessage {
    pub level: &'static str,
    pub text: String,
}

/// The two shared records one import publishes: the snapshot progress and the
/// bounded operation progress the existing operation receipt already exposes.
#[derive(Clone)]
pub(crate) struct CardImportReporter {
    pub progress: Arc<Mutex<CardImportProgress>>,
    pub operation: Arc<Mutex<crate::operations::OperationProgress>>,
}
impl CardImportReporter {
    pub(crate) fn new() -> Self {
        Self {
            progress: Arc::new(Mutex::new(CardImportProgress::default())),
            operation: Arc::new(Mutex::new(crate::operations::OperationProgress::default())),
        }
    }
    pub(crate) fn publish(&self, apply: impl FnOnce(&mut CardImportProgress)) {
        let (percent, processed, total, verified, records) = {
            let mut value = self.progress.lock().expect("import progress mutex poisoned");
            apply(&mut value);
            let percent = if value.total_bytes == 0 {
                0
            } else {
                ((value.processed_bytes.saturating_mul(100)) / value.total_bytes).min(99) as u8
            };
            (
                percent,
                value.processed_bytes,
                value.total_bytes,
                value.verified,
                value.records,
            )
        };
        let mut operation = self
            .operation
            .lock()
            .expect("import operation mutex poisoned");
        operation.processed_bytes = processed;
        operation.total_bytes = total;
        operation.percent = percent;
        operation.verified_files = verified;
        operation.total_files = records;
    }
    pub(crate) fn message(&self, level: &'static str, text: impl Into<String>) {
        self.publish(|value| {
            value.messages.push(CardImportMessage {
                level,
                text: text.into(),
            });
        });
    }
    /// Drains the messages exactly once. The engine logs them from its own tick,
    /// so the bounded worker never writes a log or a second piece of state.
    pub(crate) fn take_messages(&self) -> Vec<CardImportMessage> {
        std::mem::take(
            &mut self
                .progress
                .lock()
                .expect("import progress mutex poisoned")
                .messages,
        )
    }
}

/// Terminal failure of one import run. Cancellation is separated from a refusal
/// so the task keeps its open pending-import state after a cancel.
#[derive(Clone, Debug)]
pub(crate) enum CardImportFailure {
    Cancelled,
    Failed(String),
}
impl CardImportFailure {
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Cancelled => {
                "CARD_IMPORT_CANCELLED: the import was cancelled; every verified file, the acquisition journal and the pending-import state are retained".into()
            }
            Self::Failed(error) => error.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CardImportSummary {
    pub records: usize,
    pub verified: usize,
    pub projections: u32,
}

fn cancelled(cancel: &AtomicBool) -> Result<(), CardImportFailure> {
    if cancel.load(Ordering::SeqCst) {
        Err(CardImportFailure::Cancelled)
    } else {
        Ok(())
    }
}

fn refusal_failure(refusal: CardImportRefusal) -> CardImportFailure {
    CardImportFailure::Failed(refusal.to_string())
}

// ---------------------------------------------------------------------------
// Pure source binding
// ---------------------------------------------------------------------------

/// Binds every unfinished record to exactly one card object of this session.
///
/// The binding key is the record's storage id, source file name, size and capture
/// date. A name comparison is ASCII-case-insensitive (the card reports the
/// uppercase DSC name the receipt already holds); size and date must match
/// exactly. Handles never participate in the key.
///
/// `already_verified` are the records a previous run (or this run's resume pass)
/// already proved with a committed local file. Their card objects are legitimately
/// accounted for even when they are no longer listed, so a resume is never refused
/// for an object it has already imported.
///
/// The "unexpected object" rule is scoped to the acquisition's own folder: the
/// session parents are derived from the objects that actually matched (plus the
/// verified records' provenance), so a changed folder handle after a reconnect
/// cannot be mistaken for a missing source, while a NEF that appears in that same
/// folder and is claimed by no record refuses the import. Objects in other folders
/// (older photographs) are not this task's source and are never touched.
pub(crate) fn bind_card_objects(
    records: &[CardCaptureRecord],
    already_verified: &[CardCaptureRecord],
    inventory: &CardImportInventory,
) -> Result<Vec<CardImportObject>, CardImportRefusal> {
    let advertised: BTreeSet<u32> = inventory.storages.iter().map(|info| info.id.0).collect();
    for record in records {
        if !advertised.contains(&record.storage_id) {
            return Err(CardImportRefusal::new(
                IMPORT_STORAGE_CHANGED,
                format!(
                    "record={} kind={} names storage 0x{:08X}, which the camera no longer advertises (advertised: {:08X?})",
                    record.index, record.kind, record.storage_id, advertised
                ),
            ));
        }
    }
    let mut claimed: Vec<(u32, &CardObjectCandidate)> = Vec::with_capacity(records.len());
    for record in records {
        let matches: Vec<&CardObjectCandidate> = inventory
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.storage_id == record.storage_id
                    && candidate.filename.eq_ignore_ascii_case(&record.source_file_name)
                    && candidate.size == record.size
                    && candidate.capture_date == record.capture_date
            })
            .collect();
        if matches.is_empty() {
            let by_name = inventory
                .candidates
                .iter()
                .filter(|candidate| {
                    candidate.storage_id == record.storage_id
                        && candidate.filename.eq_ignore_ascii_case(&record.source_file_name)
                })
                .count();
            return Err(CardImportRefusal::new(
                IMPORT_OBJECT_MISSING,
                format!(
                    "record={} kind={} storage=0x{:08X} name={} size={} captureDate={} has no card object (name-only candidates on that storage: {by_name}; examined {} handles)",
                    record.index,
                    record.kind,
                    record.storage_id,
                    record.source_file_name,
                    record.size,
                    record.capture_date,
                    inventory.examined
                ),
            ));
        }
        if matches.len() > 1 {
            return Err(CardImportRefusal::new(
                IMPORT_OBJECT_AMBIGUOUS,
                format!(
                    "record={} kind={} storage=0x{:08X} name={} size={} captureDate={} matches {} card objects (handles {:08X?}); the source is not unique",
                    record.index,
                    record.kind,
                    record.storage_id,
                    record.source_file_name,
                    record.size,
                    record.capture_date,
                    matches.len(),
                    matches.iter().map(|candidate| candidate.handle).collect::<Vec<_>>()
                ),
            ));
        }
        claimed.push((record.index, matches[0]));
    }
    for (position, (index, candidate)) in claimed.iter().enumerate() {
        if let Some((other_index, _)) = claimed
            .iter()
            .skip(position + 1)
            .find(|(_, other)| other.handle == candidate.handle && other.storage_id == candidate.storage_id)
        {
            return Err(CardImportRefusal::new(
                IMPORT_OBJECT_AMBIGUOUS,
                format!(
                    "card object handle 0x{:08X} on storage 0x{:08X} is claimed by records {index} and {other_index}; exactly one record must own one object",
                    candidate.handle, candidate.storage_id
                ),
            ));
        }
    }
    let session_parents: BTreeSet<(u32, u32)> = claimed
        .iter()
        .map(|(_, candidate)| (candidate.storage_id, candidate.parent))
        .chain(
            already_verified
                .iter()
                .map(|record| (record.storage_id, record.parent)),
        )
        .collect();
    let proven: BTreeSet<(u32, String, u64, String)> = already_verified
        .iter()
        .map(|record| {
            (
                record.storage_id,
                record.source_file_name.to_ascii_lowercase(),
                record.size,
                record.capture_date.clone(),
            )
        })
        .collect();
    let mut unexpected: Vec<String> = Vec::new();
    for candidate in &inventory.candidates {
        if !advertised.contains(&candidate.storage_id) {
            continue;
        }
        if !session_parents.contains(&(candidate.storage_id, candidate.parent)) {
            continue;
        }
        if claimed
            .iter()
            .any(|(_, bound)| bound.handle == candidate.handle && bound.storage_id == candidate.storage_id)
        {
            continue;
        }
        // An object whose provenance a verified record already proved is
        // accounted for: the resume must not be refused for its own import.
        if proven.contains(&(
            candidate.storage_id,
            candidate.filename.to_ascii_lowercase(),
            candidate.size,
            candidate.capture_date.clone(),
        )) {
            continue;
        }
        unexpected.push(format!(
            "handle=0x{:08X} storage=0x{:08X} parent=0x{:08X} name={} size={} captureDate={}",
            candidate.handle,
            candidate.storage_id,
            candidate.parent,
            candidate.filename,
            candidate.size,
            candidate.capture_date
        ));
    }
    if !unexpected.is_empty() {
        return Err(CardImportRefusal::new(
            IMPORT_OBJECT_UNEXPECTED,
            format!(
                "{} card object(s) in this acquisition's folder belong to no record: {}",
                unexpected.len(),
                unexpected.join("; ")
            ),
        ));
    }
    Ok(records
        .iter()
        .zip(claimed.into_iter().map(|(_, candidate)| candidate))
        .map(|(record, candidate)| CardImportObject {
            record_index: record.index,
            kind: record.kind.clone(),
            candidate: candidate.clone(),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Device-side enumeration and download (single camera owner thread)
// ---------------------------------------------------------------------------

/// What a non-strict `GetObjectInfo` read proves about an object whose strict
/// parse failed, so an object that could never be one of this task's records is
/// skipped instead of refusing the import for an unrelated card file.
enum CardObjectScope {
    /// A declared NEF format with a declared size above the shared cap.
    OversizeNef { size: u64 },
    /// A NEF-format or `.nef`-named object that the strict parser refused: a real
    /// inconsistency in this task's source domain.
    NefLike { filename: String, format: u16 },
    /// Anything else (a JPEG, a movie, a folder, an unclassifiable dataset).
    Other,
}

/// Classifies a rejected metadata dataset with a bounded read of only the fields
/// needed for scope. It never substitutes a value for the strict parser: it only
/// decides whether a parse failure is chargeable to this task's source domain.
fn card_object_scope(bytes: &[u8]) -> CardObjectScope {
    let mut cursor = Cursor::new(bytes);
    let Ok(_storage) = cursor.u32() else {
        return CardObjectScope::Other;
    };
    let Ok(format) = cursor.u16() else {
        return CardObjectScope::Other;
    };
    let Ok(_protection) = cursor.u16() else {
        return CardObjectScope::Other;
    };
    let Ok(size) = cursor.u32() else {
        return CardObjectScope::Other;
    };
    // The name is the declared field after the 40-byte numeric block.
    let filename = bytes
        .get(52..)
        .and_then(|tail| Cursor::new(tail).string().ok())
        .unwrap_or_default();
    let nef_format = matches!(format, 0x3000 | 0xb101 | 0x3800);
    if nef_format && u64::from(size) > MAX_OBJECT_BYTES as u64 {
        return CardObjectScope::OversizeNef {
            size: u64::from(size),
        };
    }
    if nef_format || filename.to_ascii_lowercase().ends_with(".nef") {
        return CardObjectScope::NefLike { filename, format };
    }
    CardObjectScope::Other
}

/// Fresh card enumeration on the owner thread: `GetStorageIDs`, one bounded
/// `GetStorageInfo` per advertised id, positional id binding, then
/// `GetObjectHandles` for every participating storage and one bounded
/// `GetObjectInfo` per handle through the strict card parser.
///
/// Read-only metadata only: no `GetObject`, no partial object and no thumbnail is
/// requested here, and no card byte is modified.
pub(crate) fn enumerate_card_objects(
    camera: &mut NativeCamera,
    expected_storages: &[u32],
    context: &CallContext,
) -> Result<CardImportInventory, CameraError> {
    camera.ensure_connected()?;
    if !camera.d7100 {
        return Err(device_refusal(
            IMPORT_IDENTITY_CHANGED,
            "the connected camera is not the identity-verified D7100",
        ));
    }
    if !camera.health.capabilities.card_capture {
        return Err(device_refusal(
            IMPORT_IDENTITY_CHANGED,
            "the connected camera does not advertise the standard card object operations",
        ));
    }
    if camera.health.live_view.active {
        return Err(CameraError::Safety(
            "the card import requires Live View OFF".into(),
        ));
    }
    let identity = camera
        .health
        .identity
        .clone()
        .ok_or_else(|| device_refusal(IMPORT_IDENTITY_CHANGED, "camera identity missing"))?;
    let ids = parse_storage_ids(&camera.read_data(
        GET_STORAGE_IDS,
        &[],
        CARD_IMPORT_STORAGE_MAX,
        context,
    )?)?;
    let mut infos = Vec::with_capacity(ids.len());
    for id in &ids {
        infos.push(parse_storage_info(&camera.read_data(
            GET_STORAGE_INFO,
            &[id.0],
            CARD_IMPORT_STORAGE_MAX,
            context,
        )?)?);
    }
    let storages = bind_storage_ids(&ids, infos)?;
    let participating: Vec<StorageId> = ids
        .iter()
        .copied()
        .filter(|id| expected_storages.contains(&id.0))
        .collect();
    let mut candidates = Vec::new();
    let mut examined = 0usize;
    for storage in participating {
        let handles = parse_object_handles(&camera.read_data(
            GET_OBJECT_HANDLES,
            &[storage.0, 0, 0],
            CARD_IMPORT_INFO_MAX,
            context,
        )?)?;
        for handle in handles {
            context.check()?;
            let metadata = camera.read_data(
                GET_OBJECT_INFO,
                &[handle],
                CARD_IMPORT_INFO_MAX,
                context,
            )?;
            examined += 1;
            match parse_card_object_info(&metadata) {
                Ok(info) => candidates.push(CardObjectCandidate {
                    storage_id: info.storage_id,
                    handle,
                    parent: info.parent,
                    filename: info.filename,
                    format: info.format,
                    size: info.size as u64,
                    capture_date: info.capture_date,
                }),
                Err(error) => match card_object_scope(&metadata) {
                    CardObjectScope::OversizeNef { size } => {
                        return Err(device_refusal(
                            IMPORT_OBJECT_OVERSIZE,
                            format!(
                                "handle=0x{handle:08X} storage=0x{:08X} declares {size} bytes, above the {MAX_OBJECT_BYTES}-byte object bound",
                                storage.0
                            ),
                        ))
                    }
                    CardObjectScope::NefLike { filename, format } => {
                        return Err(device_refusal(
                            IMPORT_OBJECT_UNREADABLE,
                            format!(
                                "handle=0x{handle:08X} storage=0x{:08X} name={filename} format=0x{format:04X} is a card NEF whose strict metadata parse failed: {error}",
                                storage.0
                            ),
                        ))
                    }
                    // A card file outside this task's source domain (another
                    // format, a folder, an unclassifiable dataset) can never be
                    // one of the records, because every record's object parsed
                    // strictly when it was acquired.
                    CardObjectScope::Other => (),
                },
            }
        }
    }
    Ok(CardImportInventory {
        identity,
        storages,
        candidates,
        examined,
    })
}

/// One object download on the owner thread. The metadata is re-read and must
/// still equal the bound evidence, the length must equal the declared length, the
/// complete NEF is validated, and the bytes are streamed to `staging` in bounded
/// chunks with a cancellation check and a SHA-256 per chunk before `fsync`.
pub(crate) fn download_card_object(
    camera: &mut NativeCamera,
    object: &CardImportObject,
    staging: &Path,
    context: &CallContext,
    progress: Option<&Arc<Mutex<CardImportProgress>>>,
) -> Result<CardImportDownload, CameraError> {
    let candidate = &object.candidate;
    let record = object.record_index;
    let fail = |error: CameraError| {
        let _ = fs::remove_file(staging);
        error
    };
    if candidate.size == 0 || candidate.size > MAX_OBJECT_BYTES as u64 {
        return Err(device_refusal(
            IMPORT_OBJECT_OVERSIZE,
            format!(
                "record={record} handle=0x{:08X} declares {} bytes, outside the 1..={MAX_OBJECT_BYTES} object bound",
                candidate.handle, candidate.size
            ),
        ));
    }
    camera.ensure_connected()?;
    if camera.health.live_view.active {
        return Err(CameraError::Safety(
            "the card import requires Live View OFF".into(),
        ));
    }
    context.check()?;
    let metadata = camera.read_data(
        GET_OBJECT_INFO,
        &[candidate.handle],
        CARD_IMPORT_INFO_MAX,
        context,
    )?;
    let info = parse_card_object_info(&metadata).map_err(|error| {
        device_refusal(
            IMPORT_OBJECT_UNREADABLE,
            format!(
                "record={record} handle=0x{:08X} metadata re-read failed: {error}",
                candidate.handle
            ),
        )
    })?;
    if info.storage_id != candidate.storage_id
        || info.size as u64 != candidate.size
        || !info.filename.eq_ignore_ascii_case(&candidate.filename)
        || info.capture_date != candidate.capture_date
    {
        return Err(device_refusal(
            IMPORT_OBJECT_CHANGED,
            format!(
                "record={record} handle=0x{:08X} changed between enumeration and download: storage=0x{:08X} name={} size={} captureDate={}",
                candidate.handle, info.storage_id, info.filename, info.size, info.capture_date
            ),
        ));
    }
    let bytes = camera
        .read_data(GET_OBJECT, &[candidate.handle], candidate.size as usize, context)
        .map_err(|error| {
            fail(device_refusal(
                IMPORT_TRANSFER_TRUNCATED,
                format!(
                    "record={record} handle=0x{:08X} GetObject failed: {error}",
                    candidate.handle
                ),
            ))
        })?;
    if bytes.len() as u64 != candidate.size {
        return Err(fail(device_refusal(
            IMPORT_TRANSFER_TRUNCATED,
            format!(
                "record={record} handle=0x{:08X} transferred {} bytes but the object declares {}",
                candidate.handle,
                bytes.len(),
                candidate.size
            ),
        )));
    }
    context.check()?;
    validate_nef(&bytes).map_err(|error| {
        fail(device_refusal(
            IMPORT_NEF_INVALID,
            format!(
                "record={record} handle=0x{:08X} is not a complete Nikon D7100 NEF: {error}",
                candidate.handle
            ),
        ))
    })?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging)
        .map_err(|error| {
            fail(device_refusal(
                IMPORT_COMMIT_FAILED,
                format!("record={record} staging file could not be created: {error}"),
            ))
        })?;
    let mut file = file;
    let mut hasher = Sha256::new();
    let mut written = 0u64;
    // Progress is published in bounded steps rather than once per 64 KiB chunk,
    // so the snapshot's own lock never waits behind a large transfer.
    let report_step = (CARD_IMPORT_CHUNK * 32) as u64;
    let mut next_report = report_step;
    let mut reported = 0u64;
    let result = (|| -> Result<(), CameraError> {
        for chunk in bytes.chunks(CARD_IMPORT_CHUNK) {
            context.check()?;
            file.write_all(chunk)?;
            hasher.update(chunk);
            written += chunk.len() as u64;
            if written >= next_report {
                next_report = written.saturating_add(report_step);
                if let Some(progress) = progress {
                    if let Ok(mut value) = progress.lock() {
                        value.processed_bytes = value.processed_bytes.saturating_add(written - reported);
                        value.stage = "download";
                    }
                }
                reported = written;
            }
        }
        file.sync_all()?;
        context.check()?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = result {
        return Err(fail(match error {
            CameraError::Cancelled => CameraError::Cancelled,
            other => device_refusal(
                IMPORT_COMMIT_FAILED,
                format!("record={record} staging write failed: {other}"),
            ),
        }));
    }
    if written != candidate.size {
        return Err(fail(device_refusal(
            IMPORT_TRANSFER_TRUNCATED,
            format!("record={record} staging wrote {written} of {} bytes", candidate.size),
        )));
    }
    Ok(CardImportDownload {
        bytes: written,
        sha256: format!("{:x}", hasher.finalize()),
        format: info.format,
        handle: candidate.handle,
    })
}

// ---------------------------------------------------------------------------
// Path safety, staging and atomic commit
// ---------------------------------------------------------------------------

fn reject_reparse(path: &Path, code: &'static str) -> Result<(), CardImportRefusal> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CardImportRefusal::new(code, format!("{} could not be inspected: {error}", path.display()))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(CardImportRefusal::new(
            IMPORT_PATH_ESCAPE,
            format!("{} is a symbolic link", path.display()),
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(CardImportRefusal::new(
                IMPORT_PATH_ESCAPE,
                format!("{} is a Windows reparse point", path.display()),
            ));
        }
    }
    Ok(())
}

/// The absolute, real task directory. A relative save root, a missing directory
/// or any symlink/reparse point on the way refuses the import before a single
/// card byte is requested.
pub(crate) fn verified_task_root(parameters: &crate::Parameters) -> Result<PathBuf, CardImportRefusal> {
    parameters
        .validate()
        .map_err(|error| CardImportRefusal::new(IMPORT_SOURCE_INVALID, error))?;
    let save_root = Path::new(parameters.save_path.trim());
    if !save_root.is_absolute() {
        return Err(CardImportRefusal::new(
            IMPORT_PATH_ESCAPE,
            format!("save root {} is not an absolute path", save_root.display()),
        ));
    }
    reject_reparse(save_root, IMPORT_PATH_ESCAPE)?;
    let root = save_root.join(parameters.task_id.trim());
    if !root.is_dir() {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            format!("task directory {} is missing", root.display()),
        ));
    }
    reject_reparse(&root, IMPORT_PATH_ESCAPE)?;
    let canonical_root = root.canonicalize().map_err(|error| {
        CardImportRefusal::new(
            IMPORT_PATH_ESCAPE,
            format!("task directory {} could not be resolved: {error}", root.display()),
        )
    })?;
    let canonical_save = save_root.canonicalize().map_err(|error| {
        CardImportRefusal::new(
            IMPORT_PATH_ESCAPE,
            format!("save root {} could not be resolved: {error}", save_root.display()),
        )
    })?;
    if !canonical_root.starts_with(&canonical_save) {
        return Err(CardImportRefusal::new(
            IMPORT_PATH_ESCAPE,
            format!(
                "task directory {} resolves outside the save root",
                canonical_root.display()
            ),
        ));
    }
    Ok(canonical_root)
}

/// Confirms a target file stays inside the task directory. Used for the target
/// itself and for every staging file.
fn verify_inside(root: &Path, target: &Path) -> Result<(), CardImportRefusal> {
    let parent = target.parent().ok_or_else(|| {
        CardImportRefusal::new(IMPORT_PATH_ESCAPE, format!("{} has no parent", target.display()))
    })?;
    reject_reparse(parent, IMPORT_PATH_ESCAPE)?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        CardImportRefusal::new(
            IMPORT_PATH_ESCAPE,
            format!("{} could not be resolved: {error}", parent.display()),
        )
    })?;
    if !canonical_parent.starts_with(root) {
        return Err(CardImportRefusal::new(
            IMPORT_PATH_ESCAPE,
            format!("{} resolves outside {}", target.display(), root.display()),
        ));
    }
    Ok(())
}

/// The committed target of one record: `frames/frame-NNNN.nef` for a projection
/// and `references/pre-dark|pre-flat/frame-NNNN.nef` for a reference, exactly the
/// layout a host-RAM scan writes. The name comes from the acquisition index only,
/// never from the raw card name.
pub(crate) fn record_target(
    root: &Path,
    record: &CardCaptureRecord,
) -> Result<PathBuf, CardImportRefusal> {
    if record.index == 0 || record.index > MAX_RECORD_INDEX {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            format!(
                "record={} kind={} has an index outside 1..={MAX_RECORD_INDEX}",
                record.index, record.kind
            ),
        ));
    }
    let directory = match record.kind.as_str() {
        "projection" => root.join("frames"),
        "preDark" => root.join("references").join("pre-dark"),
        "preFlat" => root.join("references").join("pre-flat"),
        other => {
            return Err(CardImportRefusal::new(
                IMPORT_RECORD_KIND_UNKNOWN,
                format!("record={} reports the unknown kind {other}", record.index),
            ))
        }
    };
    fs::create_dir_all(&directory).map_err(|error| {
        CardImportRefusal::new(
            IMPORT_COMMIT_FAILED,
            format!("{} could not be created: {error}", directory.display()),
        )
    })?;
    reject_reparse(&directory, IMPORT_PATH_ESCAPE)?;
    let target = directory.join(format!("frame-{:04}.nef", record.index));
    verify_inside(root, &target)?;
    if target.exists() {
        reject_reparse(&target, IMPORT_PATH_ESCAPE)?;
    }
    Ok(target)
}

fn staging_path(target: &Path) -> Result<PathBuf, CardImportRefusal> {
    let parent = target.parent().ok_or_else(|| {
        CardImportRefusal::new(IMPORT_PATH_ESCAPE, "target has no parent directory")
    })?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CardImportRefusal::new(IMPORT_PATH_ESCAPE, "target name is not text"))?;
    let nonce = super::live_view::random_session()
        .map_err(|error| CardImportRefusal::new(IMPORT_COMMIT_FAILED, error.to_string()))?;
    Ok(parent.join(format!(".{name}.{nonce}.part")))
}

/// Removes leftover partial staging files of a previous interrupted attempt. The
/// task directory is exclusively owned by this task, so a `.<name>.<nonce>.part`
/// file can only be this import's own abandoned staging data.
fn discard_stale_staging(directory: &Path) -> Result<usize, CardImportRefusal> {
    if !directory.is_dir() {
        return Ok(0);
    }
    let entries = fs::read_dir(directory).map_err(|error| {
        CardImportRefusal::new(
            IMPORT_COMMIT_FAILED,
            format!("{} could not be listed: {error}", directory.display()),
        )
    })?;
    let mut removed = 0;
    for entry in entries {
        let entry = entry.map_err(|error| {
            CardImportRefusal::new(IMPORT_COMMIT_FAILED, format!("staging listing failed: {error}"))
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name.ends_with(".part") {
            let path = entry.path();
            if fs::symlink_metadata(&path).map(|meta| meta.file_type().is_file()).unwrap_or(false) {
                fs::remove_file(&path).map_err(|error| {
                    CardImportRefusal::new(
                        IMPORT_COMMIT_FAILED,
                        format!("stale staging {} could not be removed: {error}", path.display()),
                    )
                })?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Atomic no-overwrite commit. `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`
/// fails when the destination already exists, so a racing writer can never be
/// overwritten.
#[cfg(windows)]
fn atomic_commit_no_replace(staging: &Path, destination: &Path) -> Result<(), String> {
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
        .map_err(|error| error.to_string())
    }
}

#[cfg(not(windows))]
fn atomic_commit_no_replace(staging: &Path, destination: &Path) -> Result<(), String> {
    fs::hard_link(staging, destination).map_err(|error| error.to_string())?;
    fs::remove_file(staging).map_err(|error| error.to_string())
}

/// Length and SHA-256 of one local file, with a cancellation check per chunk.
pub(crate) fn hash_file_cancellable(
    path: &Path,
    cancel: &AtomicBool,
) -> Result<(u64, String), CardImportFailure> {
    let mut file = fs::File::open(path)
        .map_err(|error| CardImportFailure::Failed(format!("{IMPORT_SOURCE_INVALID}: {} could not be read: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0u8; CARD_IMPORT_CHUNK];
    loop {
        cancelled(cancel)?;
        let count = file.read(&mut buffer).map_err(|error| {
            CardImportFailure::Failed(format!("{IMPORT_SOURCE_INVALID}: {} could not be read: {error}", path.display()))
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        bytes += count as u64;
    }
    Ok((bytes, format!("{:x}", hasher.finalize())))
}

// ---------------------------------------------------------------------------
// Manifest load, per-record persistence and seal
// ---------------------------------------------------------------------------

struct PendingCardTask {
    manifest: Value,
    profile: crate::capture_profile::CaptureProfile,
    records: Vec<CardCaptureRecord>,
}

fn read_manifest_value(root: &Path) -> Result<Value, CardImportRefusal> {
    let path = root.join("manifest.json");
    let bytes = fs::read(&path).map_err(|error| {
        CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            format!("{} could not be read: {error}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            format!("{} is not valid JSON: {error}", path.display()),
        )
    })
}

/// Loads the schema-4 manifest of a card acquisition that is still waiting for
/// its import. Every field the seal and the registry depend on is checked here,
/// so a drifted or foreign manifest refuses before any card contact.
fn load_pending_task(
    root: &Path,
    parameters: &crate::Parameters,
) -> Result<PendingCardTask, CardImportRefusal> {
    let manifest = read_manifest_value(root)?;
    if manifest["schemaVersion"].as_u64() != Some(4) {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            "the card import requires a schema-4 manifest",
        ));
    }
    if manifest["scanId"].as_str() != Some(parameters.task_id.as_str())
        || manifest["projectionCount"].as_u64() != Some(u64::from(parameters.projection_count))
        || manifest["exposureMs"].as_f64() != Some(parameters.exposure_ms)
    {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            "the manifest does not match the requested task",
        ));
    }
    let destination = serde_json::from_value::<CaptureDestination>(
        manifest["captureDestination"].clone(),
    )
    .map_err(|_| CardImportRefusal::new(IMPORT_SOURCE_INVALID, "no card destination"))?;
    if destination != CaptureDestination::CameraCard {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            "the manifest is not a camera-card acquisition",
        ));
    }
    let profile = serde_json::from_value::<crate::capture_profile::CaptureProfile>(
        manifest["captureProfile"].clone(),
    )
    .map_err(|error| {
        CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            format!("the frozen capture profile is invalid: {error}"),
        )
    })?;
    if !profile.is_card_destination() {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            "the frozen profile lacks the card destination",
        ));
    }
    if manifest["acquisitionStatus"].as_str() != Some(ACQUISITION_ACQUIRED) {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            "the acquisition is not safely finished",
        ));
    }
    let data_status = manifest["dataStatus"].as_str().unwrap_or_default();
    if !matches!(
        data_status,
        DATA_PENDING_IMPORT | DATA_IMPORTING | DATA_IMPORT_FAILED
    ) {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            format!("dataStatus {data_status:?} is not open for import"),
        ));
    }
    if manifest["frames"]
        .as_array()
        .is_some_and(|frames| !frames.is_empty())
    {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INVALID,
            "a card acquisition waiting for its import cannot already hold local frames",
        ));
    }
    let records: Vec<CardCaptureRecord> =
        serde_json::from_value(manifest["captureRecords"].clone()).map_err(|error| {
            CardImportRefusal::new(
                IMPORT_SOURCE_INVALID,
                format!("the card records are invalid: {error}"),
            )
        })?;
    if records.iter().filter(|record| record.kind == "projection").count()
        != parameters.projection_count as usize
    {
        return Err(CardImportRefusal::new(
            IMPORT_SOURCE_INCOMPLETE,
            format!(
                "{} projection records do not cover the {} planned projections",
                records.iter().filter(|record| record.kind == "projection").count(),
                parameters.projection_count
            ),
        ));
    }
    for record in &records {
        if record.storage_id == 0
            || record.handle == 0
            || record.source_file_name.trim().is_empty()
            || record.camera_session_id.trim().is_empty()
            || record.operation_id.trim().is_empty()
            || record.size == 0
        {
            return Err(CardImportRefusal::new(
                IMPORT_SOURCE_INCOMPLETE,
                format!(
                    "record={} kind={} lacks its acquisition provenance",
                    record.index, record.kind
                ),
            ));
        }
        if record.state != DATA_VERIFIED && record.imported_file.is_some() {
            return Err(CardImportRefusal::new(
                IMPORT_SOURCE_INVALID,
                format!(
                    "record={} kind={} carries a local file identity without the verified state",
                    record.index, record.kind
                ),
            ));
        }
    }
    for kind in ["preDark", "preFlat"] {
        let count = records.iter().filter(|record| record.kind == kind).count();
        if count != CARD_REFERENCE_COUNT {
            return Err(CardImportRefusal::new(
                IMPORT_SOURCE_INCOMPLETE,
                format!("{count} {kind} records; a sealed card task needs exactly {CARD_REFERENCE_COUNT}"),
            ));
        }
    }
    let mut keys = BTreeSet::new();
    for record in &records {
        if !keys.insert((record.kind.clone(), record.index)) {
            return Err(CardImportRefusal::new(
                IMPORT_SOURCE_INVALID,
                format!("two records share kind={} index={}", record.kind, record.index),
            ));
        }
    }
    Ok(PendingCardTask {
        manifest,
        profile,
        records,
    })
}

fn persist_card_records(
    root: &Path,
    manifest: &mut Value,
    records: &[CardCaptureRecord],
    data_status: &str,
) -> Result<(), CardImportRefusal> {
    manifest["dataStatus"] = json!(data_status);
    manifest["captureRecords"] = serde_json::to_value(records).map_err(|error| {
        CardImportRefusal::new(IMPORT_SOURCE_INVALID, format!("records cannot be serialized: {error}"))
    })?;
    write_manifest(root, manifest)
}

fn write_manifest(root: &Path, manifest: &Value) -> Result<(), CardImportRefusal> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|error| {
        CardImportRefusal::new(IMPORT_COMMIT_FAILED, format!("manifest serialization failed: {error}"))
    })?;
    crate::task_registry::atomic_store(&root.join("manifest.json"), &bytes).map_err(|error| {
        CardImportRefusal::new(IMPORT_COMMIT_FAILED, format!("manifest commit failed: {error}"))
    })
}

fn verified_frames(records: &[CardCaptureRecord], kind: &str) -> Vec<RealFrame> {
    let mut frames: Vec<RealFrame> = records
        .iter()
        .filter(|record| record.kind == kind)
        .filter_map(|record| record.imported_file.clone())
        .collect();
    frames.sort_by_key(|frame| frame.index);
    frames
}

fn reference_frames(frames: &[RealFrame]) -> Vec<ReferenceFrame> {
    frames
        .iter()
        .map(|frame| ReferenceFrame {
            index: frame.index,
            exposure_ms: frame.exposure_ms,
            file_name: frame.file_name.clone(),
            path: frame.path.clone(),
            bytes: frame.bytes,
            sha256: frame.sha256.clone(),
        })
        .collect()
}

/// The seal: only a complete, verified record set becomes reconstructable input.
/// `frames` and `references` are filled from the verified local files, the data
/// state becomes `verified`, `completed` becomes true, and the schema-4
/// consistency rule is re-evaluated before the manifest is written. The written
/// manifest is read back and re-checked, so a successful seal is never claimed
/// from an in-memory value alone.
fn seal_manifest(
    root: &Path,
    manifest: &mut Value,
    profile: &crate::capture_profile::CaptureProfile,
    records: &[CardCaptureRecord],
    projection_count: u32,
) -> Result<(), CardImportRefusal> {
    if !schema_four_completion_consistent(
        Some(CaptureDestination::CameraCard),
        Some(profile),
        Some(ACQUISITION_ACQUIRED),
        Some(DATA_VERIFIED),
        records,
        projection_count,
    ) {
        return Err(CardImportRefusal::new(
            IMPORT_SEAL_INCONSISTENT,
            "the verified record set does not satisfy the schema-4 completion rule",
        ));
    }
    let projections = verified_frames(records, "projection");
    let pre_dark = verified_frames(records, "preDark");
    let pre_flat = verified_frames(records, "preFlat");
    if projections.len() != projection_count as usize
        || pre_dark.len() != CARD_REFERENCE_COUNT
        || pre_flat.len() != CARD_REFERENCE_COUNT
    {
        return Err(CardImportRefusal::new(
            IMPORT_SEAL_INCONSISTENT,
            format!(
                "verified local files: {} projections, {} pre-dark, {} pre-flat",
                projections.len(),
                pre_dark.len(),
                pre_flat.len()
            ),
        ));
    }
    let references = ReferenceFrames {
        pre_dark: reference_frames(&pre_dark),
        pre_flat: reference_frames(&pre_flat),
        post_flat: Vec::new(),
        post_dark: Vec::new(),
    };
    manifest["dataStatus"] = json!(DATA_VERIFIED);
    manifest["completed"] = json!(true);
    manifest["stage"] = json!("completed");
    manifest["frames"] = serde_json::to_value(&projections).map_err(|error| {
        CardImportRefusal::new(IMPORT_SEAL_INCONSISTENT, format!("frames cannot be serialized: {error}"))
    })?;
    manifest["references"] = serde_json::to_value(&references).map_err(|error| {
        CardImportRefusal::new(IMPORT_SEAL_INCONSISTENT, format!("references cannot be serialized: {error}"))
    })?;
    manifest["captureRecords"] = serde_json::to_value(records).map_err(|error| {
        CardImportRefusal::new(IMPORT_SEAL_INCONSISTENT, format!("records cannot be serialized: {error}"))
    })?;
    write_manifest(root, manifest)?;
    // Read the durable bytes back: the seal is only proven by the file.
    let stored = read_manifest_value(root)?;
    let stored_records: Vec<CardCaptureRecord> =
        serde_json::from_value(stored["captureRecords"].clone()).map_err(|error| {
            CardImportRefusal::new(
                IMPORT_SEAL_INCONSISTENT,
                format!("the sealed manifest records cannot be read back: {error}"),
            )
        })?;
    let stored_profile = serde_json::from_value::<crate::capture_profile::CaptureProfile>(
        stored["captureProfile"].clone(),
    )
    .map_err(|error| {
        CardImportRefusal::new(
            IMPORT_SEAL_INCONSISTENT,
            format!("the sealed manifest profile cannot be read back: {error}"),
        )
    })?;
    if stored["completed"].as_bool() != Some(true)
        || stored["dataStatus"].as_str() != Some(DATA_VERIFIED)
        || stored["stage"].as_str() != Some("completed")
        || stored["frames"].as_array().map(Vec::len) != Some(projection_count as usize)
        || stored["references"]["preDark"].as_array().map(Vec::len) != Some(CARD_REFERENCE_COUNT)
        || stored["references"]["preFlat"].as_array().map(Vec::len) != Some(CARD_REFERENCE_COUNT)
        || !schema_four_completion_consistent(
            serde_json::from_value::<CaptureDestination>(stored["captureDestination"].clone()).ok(),
            Some(&stored_profile),
            stored["acquisitionStatus"].as_str(),
            stored["dataStatus"].as_str(),
            &stored_records,
            projection_count,
        )
    {
        return Err(CardImportRefusal::new(
            IMPORT_SEAL_INCONSISTENT,
            "the durable manifest does not prove the sealed verified state",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Camera identity
// ---------------------------------------------------------------------------

/// The import must run against the same body that acquired the task: the frozen
/// profile's manufacturer, model and serial are compared with the fresh session.
fn verify_camera_identity(
    identity: &CameraIdentity,
    profile: &crate::capture_profile::CaptureProfile,
) -> Result<(), CardImportRefusal> {
    let frozen = &profile.identity;
    if !identity.model.eq_ignore_ascii_case(&frozen.model)
        || !identity.manufacturer.eq_ignore_ascii_case(&frozen.manufacturer)
    {
        return Err(CardImportRefusal::new(
            IMPORT_IDENTITY_CHANGED,
            format!(
                "the connected camera reports {} {} but the task froze {} {}",
                identity.manufacturer, identity.model, frozen.manufacturer, frozen.model
            ),
        ));
    }
    match (frozen.serial.as_deref(), identity.serial.as_deref()) {
        (Some(frozen), Some(current)) if frozen == current => Ok(()),
        (Some(frozen), Some(current)) => Err(CardImportRefusal::new(
            IMPORT_IDENTITY_CHANGED,
            format!("the connected camera serial {current} differs from the frozen serial {frozen}"),
        )),
        (Some(frozen), None) => Err(CardImportRefusal::new(
            IMPORT_IDENTITY_CHANGED,
            format!("the frozen serial {frozen} cannot be confirmed: the connected camera reports none"),
        )),
        // A frozen profile without a serial cannot prove a body either: the card
        // objects of a task must never be bound to an unproven camera.
        (None, _) => Err(CardImportRefusal::new(
            IMPORT_IDENTITY_CHANGED,
            "the frozen profile carries no camera serial; the source body cannot be proven",
        )),
    }
}

// ---------------------------------------------------------------------------
// The import run
// ---------------------------------------------------------------------------

/// Runs one authorized import of `parameters` through the single camera owner.
///
/// The manifest is loaded and validated, the already-verified files are
/// re-verified and skipped, the remaining records are enumerated, bound,
/// downloaded, verified and committed one by one (each step durably recorded in
/// the manifest), and the task is sealed only when every record is verified.
pub(crate) fn run_card_import(
    camera: &CameraAdapter,
    parameters: &crate::Parameters,
    cancel: &AtomicBool,
    reporter: &CardImportReporter,
) -> Result<CardImportSummary, CardImportFailure> {
    let root = verified_task_root(parameters).map_err(refusal_failure)?;
    let task = load_pending_task(&root, parameters).map_err(refusal_failure)?;
    let mut manifest = task.manifest;
    let mut records = task.records;
    let profile = task.profile;
    let result = run_import_inner(
        camera,
        parameters,
        cancel,
        reporter,
        &root,
        &profile,
        &mut manifest,
        &mut records,
    );
    if let Err(failure) = &result {
        // Keep the resume entry explicit: a cancelled attempt returns the data
        // state to `pendingImport` (nothing failed), a refusal records
        // `importFailed`. Every already verified record stays as it was written.
        let status = match failure {
            CardImportFailure::Cancelled => DATA_PENDING_IMPORT,
            CardImportFailure::Failed(_) => DATA_IMPORT_FAILED,
        };
        reporter.publish(|value| {
            value.stage = if status == DATA_PENDING_IMPORT {
                "cancelled"
            } else {
                "failed"
            };
            if let CardImportFailure::Cancelled = failure {
                value.status = IMPORT_STATUS_CANCELLED;
            } else {
                value.status = IMPORT_STATUS_FAILED;
                value.error = Some(failure.message());
            }
        });
        match persist_card_records(&root, &mut manifest, &records, status) {
            Ok(()) => (),
            Err(refusal) => reporter.message(
                "ERR",
                format!(
                    "the open card import state could not be persisted after {}: {refusal}",
                    failure.message()
                ),
            ),
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_import_inner(
    camera: &CameraAdapter,
    parameters: &crate::Parameters,
    cancel: &AtomicBool,
    reporter: &CardImportReporter,
    root: &Path,
    profile: &crate::capture_profile::CaptureProfile,
    manifest: &mut Value,
    records: &mut Vec<CardCaptureRecord>,
) -> Result<CardImportSummary, CardImportFailure> {
    card_acquisition_finish_gate(root).map_err(|error| {
        refusal_failure(CardImportRefusal::new(IMPORT_ACQUISITION_UNRESOLVED, error))
    })?;
    let health = camera.health();
    if !health.connected || health.isolated || !health.cleanup_confirmed {
        return Err(CardImportFailure::Failed(format!(
            "{IMPORT_IDENTITY_CHANGED}: the camera session is not usable for an import (connected={}, isolated={}, cleanupConfirmed={})",
            health.connected, health.isolated, health.cleanup_confirmed
        )));
    }
    if health.live_view.active {
        return Err(CardImportFailure::Failed(
            "IMPORT_LIVE_VIEW_ACTIVE: stop Live View before importing the card objects".into(),
        ));
    }
    let identity = health.identity.clone().ok_or_else(|| {
        refusal_failure(CardImportRefusal::new(
            IMPORT_IDENTITY_CHANGED,
            "the connected camera reports no identity",
        ))
    })?;
    verify_camera_identity(&identity, profile).map_err(refusal_failure)?;

    let mut targets: Vec<PathBuf> = Vec::with_capacity(records.len());
    for record in records.iter() {
        targets.push(record_target(root, record).map_err(refusal_failure)?);
    }
    let directories: BTreeSet<PathBuf> = targets
        .iter()
        .filter_map(|target| target.parent().map(Path::to_path_buf))
        .collect();
    for directory in &directories {
        let removed = discard_stale_staging(directory).map_err(refusal_failure)?;
        if removed > 0 {
            reporter.message(
                "WARN",
                format!(
                    "discarded {removed} partial staging file(s) in {}; every unfinished record is re-fetched",
                    directory.display()
                ),
            );
        }
    }

    let total_bytes: u64 = records.iter().map(|record| record.size).sum();
    reporter.publish(|value| {
        value.stage = "resume";
        value.total_bytes = total_bytes;
        publish_counts(value, records);
    });

    // Resume: an already verified record whose committed file still re-verifies
    // (same length and SHA-256) is never downloaded again. A verified record whose
    // file is missing is unfinished; a file that differs refuses the import
    // instead of being overwritten.
    let mut unfinished: Vec<usize> = Vec::new();
    for (position, record) in records.iter().enumerate() {
        cancelled(cancel)?;
        let target = &targets[position];
        let Some(imported) = record.imported_file.as_ref() else {
            unfinished.push(position);
            continue;
        };
        if record.state != DATA_VERIFIED {
            unfinished.push(position);
            continue;
        }
        if !target.is_file() {
            reporter.message(
                "WARN",
                format!(
                    "record={} kind={} was verified but {} is missing; it is re-downloaded",
                    record.index,
                    record.kind,
                    target.display()
                ),
            );
            unfinished.push(position);
            continue;
        }
        let (bytes, sha256) = hash_file_cancellable(target, cancel)?;
        if bytes == imported.bytes && sha256 == imported.sha256 {
            continue;
        }
        return Err(refusal_failure(CardImportRefusal::new(
            IMPORT_TARGET_CONFLICT,
            format!(
                "record={} kind={} target {} re-verifies as {bytes} bytes / {sha256} but the manifest records {} bytes / {}; an existing file is never overwritten",
                record.index, record.kind, target.display(), imported.bytes, imported.sha256
            ),
        )));
    }
    reporter.publish(|value| {
        value.stage = "resume";
        value.processed_bytes = records
            .iter()
            .filter(|record| record.imported_file.is_some() && record.state == DATA_VERIFIED)
            .map(|record| record.size)
            .sum();
        publish_counts(value, records);
    });
    let reused = records.len() - unfinished.len();
    if reused > 0 {
        reporter.message(
            "PASS",
            format!("{reused} card record(s) already verified; their committed files re-verified and were not downloaded again"),
        );
    }

    if !unfinished.is_empty() {
        let storages: Vec<u32> = unfinished
            .iter()
            .map(|position| records[*position].storage_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let inventory = camera
            .card_import_enumerate(&storages, cancel)
            .map_err(|error| map_camera_failure(error, cancel))?;
        verify_camera_identity(&inventory.identity, profile).map_err(refusal_failure)?;
        reporter.publish(|value| value.stage = "bind");
        reporter.message(
            "INFO",
            format!(
                "card enumeration: {} advertised storage(s), {} handle(s) examined, {} bounded NEF candidate(s)",
                inventory.storages.len(),
                inventory.examined,
                inventory.candidates.len()
            ),
        );
        let pending: Vec<CardCaptureRecord> = unfinished
            .iter()
            .map(|position| records[*position].clone())
            .collect();
        let proven: Vec<CardCaptureRecord> = records
            .iter()
            .filter(|record| record.state == DATA_VERIFIED && record.imported_file.is_some())
            .cloned()
            .collect();
        let bound = bind_card_objects(&pending, &proven, &inventory).map_err(refusal_failure)?;
        for object in &bound {
            reporter.message(
                "PASS",
                format!(
                    "bound record={} kind={} to storage=0x{:08X} handle=0x{:08X} name={} size={} captureDate={}",
                    object.record_index,
                    object.kind,
                    object.candidate.storage_id,
                    object.candidate.handle,
                    object.candidate.filename,
                    object.candidate.size,
                    object.candidate.capture_date
                ),
            );
        }
        for object in bound {
            cancelled(cancel)?;
            let position = records
                .iter()
                .position(|record| record.index == object.record_index && record.kind == object.kind)
                .ok_or_else(|| {
                    refusal_failure(CardImportRefusal::new(
                        IMPORT_SOURCE_INVALID,
                        format!(
                            "bound object record={} kind={} has no manifest record",
                            object.record_index, object.kind
                        ),
                    ))
                })?;
            let target = targets[position].clone();
            let staging = staging_path(&target).map_err(refusal_failure)?;
            let download = camera
                .card_import_download(&object, &staging, cancel, Some(&reporter.progress))
                .map_err(|error| map_camera_failure(error, cancel))?;
            if download.bytes != records[position].size {
                let _ = fs::remove_file(&staging);
                return Err(refusal_failure(CardImportRefusal::new(
                    IMPORT_TRANSFER_TRUNCATED,
                    format!(
                        "record={} transferred {} bytes but the acquisition recorded {}",
                        records[position].index, download.bytes, records[position].size
                    ),
                )));
            }
            let frame = RealFrame {
                index: records[position].index,
                angle_deg: records[position].angle_deg,
                exposure_ms: records[position].exposure_ms,
                file_name: target
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("frame.nef")
                    .to_owned(),
                path: target.display().to_string(),
                bytes: download.bytes,
                sha256: download.sha256.clone(),
            };
            if target.exists() {
                // A target may only be adopted when it already holds exactly the
                // verified bytes: the crash window between commit and the receipt
                // is recovered without ever overwriting a conflicting file.
                let (bytes, sha256) = hash_file_cancellable(&target, cancel)?;
                if bytes != frame.bytes || sha256 != frame.sha256 {
                    let _ = fs::remove_file(&staging);
                    return Err(refusal_failure(CardImportRefusal::new(
                        IMPORT_TARGET_CONFLICT,
                        format!(
                            "record={} target {} already exists with {bytes} bytes / {sha256} but the card object verifies as {} bytes / {}; nothing is overwritten",
                            records[position].index,
                            target.display(),
                            frame.bytes,
                            frame.sha256
                        ),
                    )));
                }
                let _ = fs::remove_file(&staging);
            } else {
                atomic_commit_no_replace(&staging, &target).map_err(|error| {
                    let _ = fs::remove_file(&staging);
                    refusal_failure(CardImportRefusal::new(
                        IMPORT_COMMIT_FAILED,
                        format!(
                            "record={} commit to {} failed: {error}",
                            records[position].index,
                            target.display()
                        ),
                    ))
                })?;
            }
            records[position].state = DATA_VERIFIED.into();
            records[position].imported_file = Some(frame.clone());
            persist_card_records(root, manifest, records, DATA_IMPORTING).map_err(refusal_failure)?;
            reporter.publish(|value| {
                value.stage = "download";
                publish_counts(value, records);
            });
            reporter.message(
                "PASS",
                format!(
                    "record={} kind={} verified {}-byte NEF format=0x{:04X} sha256={} from storage=0x{:08X} handle=0x{:08X} {}",
                    records[position].index,
                    records[position].kind,
                    frame.bytes,
                    download.format,
                    frame.sha256,
                    object.candidate.storage_id,
                    download.handle,
                    target.display()
                ),
            );
        }
    }

    cancelled(cancel)?;
    reporter.publish(|value| value.stage = "seal");
    seal_manifest(root, manifest, profile, records, parameters.projection_count)
        .map_err(refusal_failure)?;
    let verified = records
        .iter()
        .filter(|record| record.state == DATA_VERIFIED && record.imported_file.is_some())
        .count();
    reporter.publish(|value| {
        value.status = IMPORT_STATUS_VERIFIED;
        value.stage = "sealed";
        value.error = None;
        publish_counts(value, records);
    });
    reporter.message(
        "PASS",
        format!(
            "card import sealed: {verified}/{} records verified with local file identities; dataStatus=verified, completed=true",
            records.len()
        ),
    );
    Ok(CardImportSummary {
        records: records.len(),
        verified,
        projections: records.iter().filter(|record| record.kind == "projection").count() as u32,
    })
}

fn publish_counts(value: &mut CardImportProgress, records: &[CardCaptureRecord]) {
    let complete = |record: &CardCaptureRecord| {
        record.state == DATA_VERIFIED && record.imported_file.is_some()
    };
    value.records = records.len();
    value.imported = records.iter().filter(|record| record.imported_file.is_some()).count();
    value.verified = records.iter().filter(|record| complete(record)).count();
    value.processed_bytes = records
        .iter()
        .filter(|record| complete(record))
        .map(|record| record.size)
        .sum();
    value.acquired_projections = records.iter().filter(|record| record.kind == "projection").count() as u32;
    value.imported_projections = records
        .iter()
        .filter(|record| record.kind == "projection" && record.imported_file.is_some())
        .count() as u32;
    value.verified_projections = records
        .iter()
        .filter(|record| record.kind == "projection" && complete(record))
        .count() as u32;
}

fn map_camera_failure(error: CameraError, cancel: &AtomicBool) -> CardImportFailure {
    if matches!(error, CameraError::Cancelled) || cancel.load(Ordering::SeqCst) {
        CardImportFailure::Cancelled
    } else {
        CardImportFailure::Failed(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{scripted_card_adapter, ScriptedCameraConfig, ScriptedCameraProbe};
    use super::*;
    use crate::scan::{CardCaptureRecord, DATA_PENDING_IMPORT, DATA_VERIFIED};
    use std::time::{Duration, Instant};

    const STORAGE_ONE: u32 = 0x0001_0001;
    const STORAGE_TWO: u32 = 0x0001_0002;
    const PARENT_ONE: u32 = 0x50;
    const PARENT_TWO: u32 = 0x60;
    const NEF_BYTES: u64 = 256;
    const GIB: u64 = 1024 * 1024 * 1024;

    fn root(topic: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(format!(
                "{}-card-import-{topic}-{}",
                chrono::Utc::now().format("%Y%m%d-%H%M%S"),
                std::process::id()
            ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("command.txt"),
            "cargo test -p ct-engine card_import --offline\nScripted PTP transport only; no real camera, card or USB device.\n",
        )
        .unwrap();
        root
    }

    /// PTP string: UINT8 count of UTF-16 units including the NUL terminator.
    fn ptp_string(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let mut bytes = vec![units.len() as u8];
        bytes.extend(units.into_iter().flat_map(u16::to_le_bytes));
        bytes
    }

    /// D7100 ObjectInfo date string with zero padding, exactly as the strict
    /// parser expects.
    fn d7100_date(text: &str) -> Vec<u8> {
        ptp_string(text)
    }

    /// Card ObjectInfo in the declared field order the strict parser reads.
    fn card_object_info(
        storage: u32,
        format: u16,
        size: u32,
        parent: u32,
        name: &str,
        date: &str,
    ) -> Vec<u8> {
        let mut bytes = storage.to_le_bytes().to_vec();
        bytes.extend(format.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend(0x3808u16.to_le_bytes());
        bytes.extend(7791u32.to_le_bytes());
        bytes.extend(160u32.to_le_bytes());
        bytes.extend(120u32.to_le_bytes());
        bytes.extend(6000u32.to_le_bytes());
        bytes.extend(4000u32.to_le_bytes());
        bytes.extend(14u32.to_le_bytes());
        bytes.extend(parent.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(ptp_string(name));
        bytes.extend(d7100_date(date));
        bytes.extend(d7100_date(date));
        bytes.extend(ptp_string(""));
        bytes
    }

    /// A bounded synthetic TIFF CFA NEF that the production `validate_nef`
    /// accepts. The fill byte makes every object's bytes and hash distinct; it is
    /// explicitly synthetic and not a scientific Nikon sample.
    fn synthetic_nef(fill: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; NEF_BYTES as usize];
        bytes[..8].copy_from_slice(b"II\x2a\0\x08\0\0\0");
        bytes[8..10].copy_from_slice(&8u16.to_le_bytes());
        let tags = [
            (256u16, 4u16, 1u32, 8u32),
            (257, 4, 1, 6),
            (262, 3, 1, 32803),
            (271, 2, 6, 112),
            (272, 2, 12, 120),
            (273, 4, 1, 144),
            (279, 4, 1, 48),
            (258, 3, 1, 12),
        ];
        for (position, (tag, kind, count, value)) in tags.into_iter().enumerate() {
            let at = 10 + position * 12;
            bytes[at..at + 2].copy_from_slice(&tag.to_le_bytes());
            bytes[at + 2..at + 4].copy_from_slice(&kind.to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&count.to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
        }
        bytes[112..118].copy_from_slice(b"NIKON\0");
        bytes[120..132].copy_from_slice(b"NIKON D7100\0");
        bytes[144..192].fill(fill);
        bytes
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn record(
        kind: &str,
        index: u32,
        storage: u32,
        name: &str,
        size: u64,
        date: &str,
    ) -> CardCaptureRecord {
        let kind_offset = match kind {
            "projection" => 0u32,
            "preDark" => 0x100,
            _ => 0x200,
        };
        CardCaptureRecord {
            index,
            kind: kind.into(),
            angle_deg: 0.0,
            exposure_ms: 200.0,
            state: DATA_PENDING_IMPORT.into(),
            camera_session_id: "session-card".into(),
            operation_id: format!("op-{kind}-{index}"),
            storage_id: storage,
            storage_changed: storage == STORAGE_TWO,
            handle: 0x200 + kind_offset + index,
            parent: if storage == STORAGE_TWO { PARENT_TWO } else { PARENT_ONE },
            source_file_name: name.into(),
            format: 0x3800,
            size,
            capture_date: date.into(),
            imported_file: None,
        }
    }

    /// Three projections plus the fixed 10 pre-dark and 10 pre-flat references.
    /// `overflow` moves the later projections to the second storage, which is what
    /// an SD overflow continuation produces.
    fn standard_records(projections: u32, overflow: bool) -> Vec<CardCaptureRecord> {
        let mut records = Vec::new();
        let mut push = |kind: &str, index: u32, storage: u32| {
            let name = match kind {
                "projection" => format!("DSC_{:04}.NEF", 100 + index),
                "preDark" => format!("DSC_{:04}.NEF", 1000 + index),
                _ => format!("DSC_{:04}.NEF", 2000 + index),
            };
            let date = format!(
                "20261011T{:02}{index:02}00",
                match kind {
                    "projection" => 12,
                    "preDark" => 13,
                    _ => 14,
                }
            );
            records.push(record(kind, index, storage, &name, NEF_BYTES, &date));
        };
        for index in 1..=projections {
            let storage = if overflow && index > projections / 2 {
                STORAGE_TWO
            } else {
                STORAGE_ONE
            };
            push("projection", index, storage);
        }
        for index in 1..=CARD_REFERENCE_COUNT as u32 {
            push("preDark", index, STORAGE_ONE);
        }
        for index in 1..=CARD_REFERENCE_COUNT as u32 {
            push("preFlat", index, STORAGE_ONE);
        }
        records
    }

    /// One scripted card object: the handle this session reports, its declared
    /// metadata and the bytes `GetObject` returns.
    #[derive(Clone)]
    struct ScriptedObject {
        storage: u32,
        handle: u32,
        name: String,
        date: String,
        parent: u32,
        format: u16,
        bytes: Vec<u8>,
    }

    fn scripted_objects(records: &[CardCaptureRecord]) -> Vec<ScriptedObject> {
        records
            .iter()
            .enumerate()
            .map(|(position, record)| ScriptedObject {
                storage: record.storage_id,
                handle: 0x1000 + position as u32,
                name: record.source_file_name.clone(),
                date: record.capture_date.clone(),
                parent: record.parent,
                format: 0x3800,
                bytes: synthetic_nef(position as u8),
            })
            .collect()
    }

    struct Fixture {
        root: PathBuf,
        parameters: crate::Parameters,
        records: Vec<CardCaptureRecord>,
        objects: Vec<ScriptedObject>,
        profile: crate::capture_profile::CaptureProfile,
    }

    impl Fixture {
        fn new(topic: &str, projections: u32, overflow: bool, adapter: &CameraAdapter) -> Self {
            let root = root(topic);
            let parameters = crate::Parameters {
                task_id: "card-task".into(),
                save_path: root.to_string_lossy().into_owned(),
                projection_count: projections,
                angle_step_deg: 360.0 / f64::from(projections),
                exposure_ms: 200.0,
            };
            parameters.validate().unwrap();
            let records = standard_records(projections, overflow);
            let objects = scripted_objects(&records);
            // The frozen profile is built from the live scripted card session, so
            // its identity, backend, readback and canonical hash are the ones the
            // import later compares against.
            let profile = crate::capture_profile::CaptureProfile::from_card_readback(
                &adapter.health(),
                adapter.card_profile_readback().unwrap(),
                None,
                card_policy(overflow),
            )
            .unwrap();
            Self {
                root,
                parameters,
                records,
                objects,
                profile,
            }
        }

        fn task_root(&self) -> PathBuf {
            self.root.join(&self.parameters.task_id)
        }

        fn manifest(&self, data_status: &str, records: &[CardCaptureRecord]) -> Value {
            json!({
                "schemaVersion": 4,
                "captureProfile": self.profile,
                "maxXraySec": 1000,
                "scanId": self.parameters.task_id,
                "createdAt": "2026-10-11T00:00:00Z",
                "completed": false,
                "voltageKv": 20.0,
                "currentUa": 100.0,
                "projectionCount": self.parameters.projection_count,
                "exposureMs": self.parameters.exposure_ms,
                "geometry": {
                    "sodMm": 100.0, "objectToDetectorMm": 100.0, "detectorWidthMm": 40.0,
                    "centerOffsetXMm": 0.0, "centerOffsetYMm": 0.0,
                    "rotationDirection": "counterclockwise", "mirrorX": false,
                    "measurement": "measured",
                },
                "references": {"preDark": [], "preFlat": [], "postFlat": [], "postDark": []},
                "stage": "finalizing",
                "frames": [],
                "captureDestination": "cameraCard",
                "acquisitionStatus": "acquired",
                "dataStatus": data_status,
                "captureRecords": records,
            })
        }

        fn write_manifest(&self) -> Value {
            let manifest = self.manifest(DATA_PENDING_IMPORT, &self.records);
            fs::create_dir_all(self.task_root()).unwrap();
            write_manifest(&self.task_root(), &manifest).unwrap();
            manifest
        }

        fn stored_manifest(&self) -> Value {
            read_manifest_value(&self.task_root()).unwrap()
        }

        fn stored_records(&self) -> Vec<CardCaptureRecord> {
            serde_json::from_value(self.stored_manifest()["captureRecords"].clone()).unwrap()
        }

        fn script(&self, probe: &ScriptedCameraProbe) {
            self.script_objects(probe, &self.objects);
        }

        fn script_objects(&self, probe: &ScriptedCameraProbe, objects: &[ScriptedObject]) {
            let storages: BTreeSet<u32> = objects.iter().map(|object| object.storage).collect();
            let cards: Vec<(u32, u16, u64, u64, u32)> = storages
                .iter()
                .map(|storage| (*storage, 0, 64 * GIB, 64 * GIB, 5000))
                .collect();
            probe.set_storages(&cards);
            for storage in &storages {
                let handles: Vec<u32> = objects
                    .iter()
                    .filter(|object| object.storage == *storage)
                    .map(|object| object.handle)
                    .collect();
                probe.set_object_handles(*storage, &handles);
            }
            for object in objects {
                probe.set_card_object(
                    object.handle,
                    card_object_info(
                        object.storage,
                        object.format,
                        object.bytes.len() as u32,
                        object.parent,
                        &object.name,
                        &object.date,
                    ),
                );
                probe.set_card_object_bytes(object.handle, object.bytes.clone());
            }
        }

        fn target(&self, record: &CardCaptureRecord) -> PathBuf {
            record_target(&self.task_root().canonicalize().unwrap(), record).unwrap()
        }
    }

    /// The frozen schema-7 card policy of this fixture, from the pinned controller
    /// firmware contract the engine already validates.
    fn card_policy(overflow: bool) -> crate::capture_profile::CardCapturePolicy {
        let identity = crate::devices::turntable::NanoIdentity {
            device: crate::devices::turntable::EXPECTED_DEVICE.into(),
            version: crate::scan::CARD_FIRMWARE_VERSION.into(),
            protocol: 2,
            build: crate::scan::CARD_FIRMWARE_BUILD.into(),
            buzzer_installed: true,
            capabilities: vec![
                "HEARTBEAT_ACK".into(),
                crate::scan::CARD_SETTLE_CAPABILITY.into(),
            ],
        };
        crate::scan::card_capture_policy(&identity, overflow).unwrap()
    }

    fn camera(topic: &str, overflow: bool) -> (CameraAdapter, ScriptedCameraProbe) {
        let (adapter, probe) = scripted_card_adapter(ScriptedCameraConfig {
            card_capture: true,
            card_overflow: overflow,
            ..Default::default()
        });
        let _ = topic;
        (adapter, probe)
    }

    fn run(
        fixture: &Fixture,
        adapter: &CameraAdapter,
        cancel: &AtomicBool,
    ) -> (Result<CardImportSummary, CardImportFailure>, CardImportReporter) {
        let reporter = CardImportReporter::new();
        let result = run_card_import(adapter, &fixture.parameters, cancel, &reporter);
        (result, reporter)
    }

    fn failure_code(failure: &CardImportFailure) -> String {
        match failure {
            CardImportFailure::Cancelled => "CARD_IMPORT_CANCELLED".into(),
            CardImportFailure::Failed(error) => error
                .split(|character: char| character.is_whitespace() || character == ':')
                .find(|token| token.starts_with("IMPORT_") || token.starts_with("CARD_IMPORT_"))
                .unwrap_or(error.as_str())
                .to_owned(),
        }
    }

    #[test]
    fn binding_refuses_missing_ambiguous_and_unexpected_objects() {
        let records = standard_records(3, false);
        let objects: Vec<CardObjectCandidate> = records
            .iter()
            .enumerate()
            .map(|(position, record)| CardObjectCandidate {
                storage_id: record.storage_id,
                handle: 0x1000 + position as u32,
                parent: record.parent,
                filename: record.source_file_name.clone(),
                format: 0x3800,
                size: record.size,
                capture_date: record.capture_date.clone(),
            })
            .collect();
        let inventory = |candidates: Vec<CardObjectCandidate>| CardImportInventory {
            identity: CameraIdentity::default(),
            storages: vec![StorageInfo {
                id: StorageId(STORAGE_ONE),
                access: super::super::storage::StorageAccess::ReadWrite,
                capacity: super::super::storage::StorageCapacity {
                    max_bytes: 64 * GIB,
                    free_bytes: 64 * GIB,
                    free_images: 5000,
                },
                storage_type: 0,
                filesystem_type: 2,
                description: "SD card".into(),
                volume_label: String::new(),
            }],
            candidates,
            examined: records.len(),
        };
        // The complete set binds one object per record, in record order.
        let bound = bind_card_objects(&records, &[], &inventory(objects.clone())).unwrap();
        assert_eq!(bound.len(), records.len());
        assert_eq!(bound[0].record_index, 1);
        assert_eq!(bound[0].candidate.handle, 0x1000);
        // A missing object names the record and its provenance.
        let missing = bind_card_objects(&records, &[], &inventory(objects[1..].to_vec())).unwrap_err();
        assert_eq!(missing.code, IMPORT_OBJECT_MISSING);
        assert!(missing.detail.contains("record=1"), "{missing}");
        // A second object with the same provenance is ambiguous.
        let mut duplicated = objects.clone();
        duplicated.push(objects[0].clone());
        assert_eq!(
            bind_card_objects(&records, &[], &inventory(duplicated)).unwrap_err().code,
            IMPORT_OBJECT_AMBIGUOUS
        );
        // An object in the acquisition's own folder that no record claims.
        let mut unexpected = objects.clone();
        unexpected.push(CardObjectCandidate {
            storage_id: STORAGE_ONE,
            handle: 0x9999,
            parent: PARENT_ONE,
            filename: "DSC_8888.NEF".into(),
            format: 0x3800,
            size: NEF_BYTES,
            capture_date: "20261011T120000".into(),
        });
        assert_eq!(
            bind_card_objects(&records, &[], &inventory(unexpected)).unwrap_err().code,
            IMPORT_OBJECT_UNEXPECTED
        );
        // The same extra object in another folder is not this task's source.
        let mut elsewhere = objects.clone();
        elsewhere.push(CardObjectCandidate {
            storage_id: STORAGE_ONE,
            handle: 0x9998,
            parent: 0x77,
            filename: "DSC_7777.NEF".into(),
            format: 0x3800,
            size: NEF_BYTES,
            capture_date: "20261011T120000".into(),
        });
        assert_eq!(bind_card_objects(&records, &[], &inventory(elsewhere)).unwrap().len(), records.len());
        // A storage the receipts name but the camera no longer advertises.
        let mut withdrawn = records.clone();
        withdrawn[0].storage_id = STORAGE_TWO;
        assert_eq!(
            bind_card_objects(&withdrawn, &[], &inventory(objects.clone())).unwrap_err().code,
            IMPORT_STORAGE_CHANGED
        );
        // A resume binds only the unfinished records: the object of a record that
        // is already verified with its local file is accounted for, not unexpected.
        let resumed = bind_card_objects(&records[1..], &records[..1], &inventory(objects.clone())).unwrap();
        assert_eq!(resumed.len(), records.len() - 1);
        assert!(resumed
            .iter()
            .all(|object| !(object.kind == "projection" && object.record_index == 1)));
        assert_eq!(
            bind_card_objects(&records[1..], &[], &inventory(objects)).unwrap_err().code,
            IMPORT_OBJECT_UNEXPECTED
        );
    }

    #[test]
    fn single_card_import_verifies_every_record_and_seals_the_manifest() {
        let (adapter, probe) = camera("single", false);
        let fixture = Fixture::new("single", 3, false, &adapter);
        fixture.write_manifest();
        // An unrelated JPEG in the same folder is not this task's source: the
        // strict parser refuses it, the scope classifier skips it, and the import
        // is not blocked by pre-existing card content.
        let mut objects = fixture.objects.clone();
        objects.push(ScriptedObject {
            storage: STORAGE_ONE,
            handle: 0x9001,
            name: "DSC_9001.JPG".into(),
            date: "20261011T120000".into(),
            parent: PARENT_ONE,
            format: 0x3801,
            bytes: synthetic_nef(0xEE),
        });
        fixture.script_objects(&probe, &objects);

        let (result, reporter) = run(&fixture, &adapter, &AtomicBool::new(false));
        let summary = result.unwrap();
        assert_eq!(summary.records, 23);
        assert_eq!(summary.verified, 23);
        assert_eq!(summary.projections, 3);
        assert_eq!(reporter.progress.lock().unwrap().status, IMPORT_STATUS_VERIFIED);
        assert_eq!(probe.card_object_read_count(), 23);
        assert_eq!(probe.operation_count(GET_OBJECT), 23);

        // Every record has its committed local file with the exact SHA-256 of the
        // scripted card object, at the index-derived path.
        let records = fixture.stored_records();
        assert_eq!(records.len(), 23);
        for (position, record) in records.iter().enumerate() {
            assert_eq!(record.state, DATA_VERIFIED, "record {position}");
            let imported = record.imported_file.as_ref().unwrap();
            let target = fixture.target(record);
            assert_eq!(Path::new(&imported.path), target, "record {position}");
            assert_eq!(imported.file_name, format!("frame-{:04}.nef", record.index));
            let bytes = fs::read(&target).unwrap();
            assert_eq!(bytes.len() as u64, imported.bytes);
            assert_eq!(sha256_hex(&bytes), imported.sha256);
            assert_eq!(imported.sha256, sha256_hex(&fixture.objects[position].bytes));
            // The original card provenance is untouched.
            assert_eq!(record.source_file_name, fixture.objects[position].name);
            assert_eq!(record.storage_id, fixture.objects[position].storage);
            assert!(record.handle != 0 && record.camera_session_id == "session-card");
        }
        assert!(fixture.target(&records[0]).ends_with("frames/frame-0001.nef")
            || fixture.target(&records[0]).ends_with("frames\\frame-0001.nef"));
        let pre_dark = records.iter().find(|record| record.kind == "preDark").unwrap();
        assert!(fixture.target(pre_dark).to_string_lossy().contains("pre-dark"));

        // The sealed manifest is the durable proof, and the schema-4 rule accepts it.
        let manifest = fixture.stored_manifest();
        assert_eq!(manifest["dataStatus"], DATA_VERIFIED);
        assert_eq!(manifest["completed"], true);
        assert_eq!(manifest["stage"], "completed");
        assert_eq!(manifest["acquisitionStatus"], ACQUISITION_ACQUIRED);
        assert_eq!(manifest["frames"].as_array().unwrap().len(), 3);
        assert_eq!(manifest["references"]["preDark"].as_array().unwrap().len(), 10);
        assert_eq!(manifest["references"]["preFlat"].as_array().unwrap().len(), 10);
        assert!(crate::scan::completed_manifest_exists(&fixture.parameters));
        // No staging file survives a completed import.
        for directory in ["frames", "references/pre-dark", "references/pre-flat"] {
            let leftovers: Vec<String> = fs::read_dir(fixture.task_root().join(directory))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".part"))
                .collect();
            assert!(leftovers.is_empty(), "{directory}: {leftovers:?}");
        }
        // Every object read was bounded by the object's own declared size.
        for (code, max) in probe.reads() {
            if code == GET_OBJECT {
                assert_eq!(max, NEF_BYTES as usize);
            }
            if code == GET_OBJECT_INFO {
                assert_eq!(max, CARD_IMPORT_INFO_MAX);
            }
        }
        let evidence = json!({
            "topic": "single-card-import",
            "records": records.len(),
            "cardObjectReads": probe.card_object_read_count(),
            "sha256": records[0].imported_file.as_ref().unwrap().sha256,
            "frames": manifest["frames"].as_array().unwrap().len(),
        });
        fs::write(
            fixture.root.join("result.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
        println!("card import evidence: {evidence}");
    }

    #[test]
    fn overflow_import_binds_objects_across_two_storages() {
        let (adapter, probe) = camera("overflow", true);
        let fixture = Fixture::new("overflow", 4, true, &adapter);
        fixture.write_manifest();
        fixture.script(&probe);
        assert!(fixture.records.iter().any(|record| record.storage_id == STORAGE_TWO));
        let (result, _) = run(&fixture, &adapter, &false.into());
        let summary = result.unwrap();
        assert_eq!(summary.verified, 24);
        let records = fixture.stored_records();
        let second: Vec<&CardCaptureRecord> = records
            .iter()
            .filter(|record| record.storage_id == STORAGE_TWO)
            .collect();
        assert!(!second.is_empty());
        for record in second {
            assert!(record.storage_changed);
            assert_eq!(record.state, DATA_VERIFIED);
            let imported = record.imported_file.as_ref().unwrap();
            assert_eq!(sha256_hex(&fs::read(&imported.path).unwrap()), imported.sha256);
        }
        assert_eq!(probe.operation_count(GET_OBJECT), 24);
        assert!(crate::scan::completed_manifest_exists(&fixture.parameters));
    }

    #[test]
    fn missing_object_refuses_before_any_download() {
        let (adapter, probe) = camera("missing", false);
        let fixture = Fixture::new("missing", 3, false, &adapter);
        fixture.write_manifest();
        // The card no longer lists the object of one record.
        let mut objects = fixture.objects.clone();
        objects.remove(3);
        fixture.script_objects(&probe, &objects);
        let (result, _) = run(&fixture, &adapter, &AtomicBool::new(false));
        let failure = result.unwrap_err();
        assert_eq!(failure_code(&failure), IMPORT_OBJECT_MISSING);
        assert!(failure.message().contains(&format!("record={}", fixture.records[3].index)));
        assert_eq!(probe.operation_count(GET_OBJECT), 0, "no object is downloaded before the binding is proven");
        assert!(!fixture.target(&dropped_record(&fixture, 3)).exists());
        // The open task keeps its resume entry: no fabricated progress, no seal.
        let manifest = fixture.stored_manifest();
        assert_eq!(manifest["dataStatus"], DATA_IMPORT_FAILED);
        assert_eq!(manifest["completed"], false);
        assert!(manifest["frames"].as_array().unwrap().is_empty());
    }

    fn dropped_record(fixture: &Fixture, position: usize) -> CardCaptureRecord {
        fixture.records[position].clone()
    }

    #[test]
    fn ambiguous_candidates_refuse_with_their_code() {
        let (adapter, probe) = camera("ambiguous", false);
        let fixture = Fixture::new("ambiguous", 3, false, &adapter);
        fixture.write_manifest();
        let mut objects = fixture.objects.clone();
        let mut duplicate = ScriptedObject {
            storage: objects[0].storage,
            handle: 0x7FFF,
            name: objects[0].name.clone(),
            date: objects[0].date.clone(),
            parent: objects[0].parent,
            format: objects[0].format,
            bytes: objects[0].bytes.clone(),
        };
        duplicate.handle = 0x7FFF;
        objects.push(duplicate);
        fixture.script_objects(&probe, &objects);
        let (result, _) = run(&fixture, &adapter, &AtomicBool::new(false));
        let failure = result.unwrap_err();
        assert_eq!(failure_code(&failure), IMPORT_OBJECT_AMBIGUOUS);
        assert_eq!(probe.operation_count(GET_OBJECT), 0);
    }

    #[test]
    fn unexpected_object_in_the_acquisition_folder_refuses() {
        let (adapter, probe) = camera("unexpected", false);
        let fixture = Fixture::new("unexpected", 3, false, &adapter);
        fixture.write_manifest();
        let mut objects = fixture.objects.clone();
        objects.push(ScriptedObject {
            storage: STORAGE_ONE,
            handle: 0x7FFE,
            name: "DSC_7000.NEF".into(),
            date: "20261011T120000".into(),
            parent: PARENT_ONE,
            format: 0x3800,
            bytes: synthetic_nef(0xDD),
        });
        fixture.script_objects(&probe, &objects);
        let (result, _) = run(&fixture, &adapter, &AtomicBool::new(false));
        let failure = result.unwrap_err();
        assert_eq!(failure_code(&failure), IMPORT_OBJECT_UNEXPECTED);
        assert_eq!(probe.operation_count(GET_OBJECT), 0);
    }

    #[test]
    fn truncated_object_refuses_and_leaves_no_target() {
        let (adapter, probe) = camera("truncated", false);
        let fixture = Fixture::new("truncated", 3, false, &adapter);
        fixture.write_manifest();
        let mut objects = fixture.objects.clone();
        // Declared 256 bytes, transferred 200: the transfer is truncated.
        objects[0].bytes.truncate(200);
        fixture.script_objects(&probe, &objects);
        probe.set_card_object(
            objects[0].handle,
            card_object_info(
                objects[0].storage,
                0x3800,
                NEF_BYTES as u32,
                objects[0].parent,
                &objects[0].name,
                &objects[0].date,
            ),
        );
        let (result, _) = run(&fixture, &adapter, &AtomicBool::new(false));
        let failure = result.unwrap_err();
        assert_eq!(failure_code(&failure), IMPORT_TRANSFER_TRUNCATED);
        assert!(failure.message().contains("record=1"));
        let target = fixture.target(&fixture.records[0]);
        assert!(!target.exists());
        assert!(!fs::read_dir(target.parent().unwrap())
            .unwrap()
            .any(|entry| entry.unwrap().file_name().to_string_lossy().ends_with(".part")));
    }

    #[test]
    fn object_above_the_size_cap_is_refused() {
        let (adapter, probe) = camera("oversize", false);
        let fixture = Fixture::new("oversize", 3, false, &adapter);
        fixture.write_manifest();
        let mut objects = fixture.objects.clone();
        objects[0].bytes = vec![0u8; 16];
        fixture.script_objects(&probe, &objects);
        // Declare a size above the shared object bound for one handle.
        probe.set_card_object(
            objects[0].handle,
            card_object_info(
                objects[0].storage,
                0x3800,
                (MAX_OBJECT_BYTES + 1) as u32,
                objects[0].parent,
                &objects[0].name,
                &objects[0].date,
            ),
        );
        let (result, _) = run(&fixture, &adapter, &AtomicBool::new(false));
        assert_eq!(failure_code(&result.unwrap_err()), IMPORT_OBJECT_OVERSIZE);
        assert_eq!(probe.operation_count(GET_OBJECT), 0);

        // The download entry refuses an over-cap expectation before any read.
        let mut context = CallContext {
            deadline: Instant::now() + Duration::from_secs(5),
            epoch: 1,
            generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        };
        let (mut native, _probe) = super::super::tests::scripted_card_camera(
            ScriptedCameraConfig {
                card_capture: true,
                ..Default::default()
            },
            "session-oversize",
            &context,
        );
        let object = CardImportObject {
            record_index: 1,
            kind: "projection".into(),
            candidate: CardObjectCandidate {
                storage_id: STORAGE_ONE,
                handle: 0x1234,
                parent: PARENT_ONE,
                filename: "DSC_0001.NEF".into(),
                format: 0x3800,
                size: MAX_OBJECT_BYTES as u64 + 1,
                capture_date: "20261011T120000".into(),
            },
        };
        let staging = fixture.root.join("oversize.part");
        let error = download_card_object(&mut native, &object, &staging, &context, None).unwrap_err();
        assert!(error.to_string().contains(IMPORT_OBJECT_OVERSIZE), "{error}");
        assert!(!staging.exists());
        context.deadline = Instant::now() + Duration::from_secs(5);
    }

    #[test]
    fn target_conflict_with_a_different_hash_is_refused_and_never_overwritten() {
        let (adapter, probe) = camera("conflict", false);
        let fixture = Fixture::new("conflict", 3, false, &adapter);
        fixture.write_manifest();
        fixture.script(&probe);
        let target = fixture.target(&fixture.records[0]);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"an unrelated file that must never be overwritten").unwrap();
        let before = fs::read(&target).unwrap();
        let (result, _) = run(&fixture, &adapter, &AtomicBool::new(false));
        assert_eq!(failure_code(&result.unwrap_err()), IMPORT_TARGET_CONFLICT);
        assert_eq!(fs::read(&target).unwrap(), before);
        assert_eq!(fixture.stored_manifest()["dataStatus"], DATA_IMPORT_FAILED);
    }

    #[test]
    fn path_escapes_and_unsafe_targets_are_refused() {
        let (adapter, _probe) = camera("escape", false);
        let fixture = Fixture::new("escape", 3, false, &adapter);
        // A relative save root can never name the task directory.
        let mut relative = fixture.parameters.clone();
        relative.save_path = "relative-root".into();
        assert_eq!(
            verified_task_root(&relative).unwrap_err().code,
            IMPORT_PATH_ESCAPE
        );
        // A missing task directory is a source refusal, not an empty import.
        let mut absent = fixture.parameters.clone();
        absent.task_id = "absent-task".into();
        assert_eq!(
            verified_task_root(&absent).unwrap_err().code,
            IMPORT_SOURCE_INVALID
        );
        // A target outside the task directory is refused.
        let canonical = fixture.root.canonicalize().unwrap();
        let outside = fixture.root.parent().unwrap().join("somewhere-else.nef");
        assert_eq!(verify_inside(&canonical, &outside).unwrap_err().code, IMPORT_PATH_ESCAPE);
        let inside = record_target(&canonical, &fixture.records[0]).unwrap();
        assert!(verify_inside(&canonical, &inside).is_ok());
        // An index that cannot name a bounded target is refused.
        let mut zero = fixture.records[0].clone();
        zero.index = 0;
        assert_eq!(record_target(&canonical, &zero).unwrap_err().code, IMPORT_SOURCE_INVALID);
        let mut huge = fixture.records[0].clone();
        huge.index = MAX_RECORD_INDEX + 1;
        assert_eq!(record_target(&canonical, &huge).unwrap_err().code, IMPORT_SOURCE_INVALID);
        // An unknown record kind cannot be mapped to a directory.
        let mut unknown = fixture.records[0].clone();
        unknown.kind = "postFlat".into();
        assert_eq!(
            record_target(&canonical, &unknown).unwrap_err().code,
            IMPORT_RECORD_KIND_UNKNOWN
        );
        // A symbolic link (or Windows reparse point) on the task directory path is
        // refused. Creating one needs an OS privilege, so the check is reported as
        // unproven when the platform refuses to create it.
        #[cfg(windows)]
        {
            let real = fixture.root.join("real-task");
            fs::create_dir_all(&real).unwrap();
            let link = fixture.root.join("linked-task");
            match std::os::windows::fs::symlink_dir(&real, &link) {
                Ok(()) => {
                    let mut linked = fixture.parameters.clone();
                    linked.task_id = "linked-task".into();
                    assert_eq!(
                        verified_task_root(&linked).unwrap_err().code,
                        IMPORT_PATH_ESCAPE
                    );
                }
                Err(error) => println!(
                    "card import path evidence: the platform refused to create the reparse-point fixture ({error}); the reparse check itself is exercised only where symlink creation is permitted"
                ),
            }
            // A real system junction proves the reparse check without privileges.
            let system_junction = Path::new(r"C:\Documents and Settings");
            match fs::symlink_metadata(system_junction) {
                Ok(metadata) if {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                } =>
                {
                    assert_eq!(
                        reject_reparse(system_junction, IMPORT_PATH_ESCAPE)
                            .unwrap_err()
                            .code,
                        IMPORT_PATH_ESCAPE
                    );
                }
                _ => println!(
                    "card import path evidence: no system reparse point is available to prove the reparse refusal"
                ),
            }
        }
    }

    #[test]
    fn resume_after_a_cancelled_import_reuses_verified_files_and_refetches_the_rest() {
        let (adapter, probe) = camera("resume", false);
        let fixture = Fixture::new("resume", 3, false, &adapter);
        fixture.write_manifest();
        fixture.script(&probe);
        // The second object read raises the import's cancellation flag: the first
        // record is already committed and recorded, the second transfer is bounded
        // and abandoned.
        let cancel = Arc::new(AtomicBool::new(false));
        probe.cancel_after_object_reads(2, cancel.clone());
        let (result, reporter) = run(&fixture, &adapter, &cancel);
        assert_eq!(failure_code(&result.unwrap_err()), "CARD_IMPORT_CANCELLED");
        assert_eq!(reporter.progress.lock().unwrap().status, IMPORT_STATUS_CANCELLED);
        let after_cancel = fixture.stored_records();
        let verified_after_cancel = after_cancel
            .iter()
            .filter(|record| record.state == DATA_VERIFIED && record.imported_file.is_some())
            .count();
        assert!(
            (1..23).contains(&verified_after_cancel),
            "a cancelled import keeps the records it already verified: {verified_after_cancel}"
        );
        assert_eq!(fixture.stored_manifest()["dataStatus"], DATA_PENDING_IMPORT);
        assert_eq!(fixture.stored_manifest()["completed"], false);
        let first_target = fixture.target(&after_cancel[0]);
        let first_bytes = fs::read(&first_target).unwrap();
        assert_eq!(
            sha256_hex(&first_bytes),
            after_cancel[0].imported_file.as_ref().unwrap().sha256
        );

        // A reconnect re-enumerates from scratch and only the unfinished records
        // are fetched again.
        let (adapter, probe) = camera("resume-again", false);
        fixture.script(&probe);
        let (result, reporter) = run(&fixture, &adapter, &AtomicBool::new(false));
        let summary = result.unwrap();
        assert_eq!(summary.verified, 23);
        assert_eq!(
            probe.card_object_read_count(),
            23 - verified_after_cancel,
            "already verified records are never downloaded again"
        );
        assert_eq!(reporter.progress.lock().unwrap().status, IMPORT_STATUS_VERIFIED);
        assert_eq!(fs::read(&first_target).unwrap(), first_bytes);
        assert!(crate::scan::completed_manifest_exists(&fixture.parameters));
        let leftovers: Vec<String> = ["frames", "references/pre-dark", "references/pre-flat"]
            .iter()
            .flat_map(|directory| {
                fs::read_dir(fixture.task_root().join(directory))
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|name| name.ends_with(".part"))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn engine_end_to_end_import_seals_the_task_and_unlocks_reconstruction() {
        let (adapter, probe) = camera("engine", false);
        let fixture = Fixture::new("engine", 3, false, &adapter);
        fixture.script(&probe);
        let mut engine = crate::Engine::new_with_storage(false, fixture.root.join("engine-data"));
        engine.parameters = fixture.parameters.clone();
        engine.task_registry.register(&fixture.parameters).unwrap();
        fixture.write_manifest();
        engine
            .task_registry
            .mark_acquired_pending_import(&fixture.parameters)
            .unwrap();
        assert!(!engine.task_registry.locked());
        assert_eq!(engine.task_registry.pending_import_tasks().len(), 1);
        assert_eq!(engine.snapshot()["cardAcquisition"]["importStatus"], IMPORT_STATUS_IDLE);
        engine.camera = Some(adapter.clone());
        engine.cached_camera_health = Some(adapter.health());
        engine.card_import = Some(CardImportReporter::new());
        assert!(engine
            .device_command("camera_card_import", json!({}))
            .unwrap()
            .is_ok());
        assert_eq!(engine.snapshot()["cardAcquisition"]["importStatus"], IMPORT_STATUS_RUNNING);
        let deadline = Instant::now() + Duration::from_secs(30);
        while engine.operations.is_active() && Instant::now() < deadline {
            engine.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
        for _ in 0..5 {
            engine.tick();
        }
        assert!(!engine.operations.is_active());
        assert_eq!(engine.task_registry.pending_import_tasks().len(), 0);
        let snapshot = engine.snapshot();
        let card = &snapshot["cardAcquisition"];
        assert_eq!(card["importStatus"], IMPORT_STATUS_VERIFIED);
        assert_eq!(card["importError"], Value::Null);
        assert_eq!(card["importedRecords"], 23);
        assert_eq!(card["verifiedRecords"], 23);
        assert_eq!(card["acquiredProjectionCount"], 3);
        assert_eq!(card["importedProjectionCount"], 3);
        assert_eq!(card["verifiedProjectionCount"], 3);
        assert!(card["importOperationId"].is_string());
        assert_eq!(snapshot["workstation"]["reconstruction"]["unlocked"], true);
        assert!(engine.metadata_completed);
        assert_eq!(engine.phase, crate::Phase::Completed);
        assert!(fixture.stored_manifest()["completed"].as_bool().unwrap());
        assert_eq!(
            engine.task_registry.snapshot()["retained"][0]["status"],
            "completed"
        );
        // The import held the ordinary camera transport occupation for its whole
        // run, which is what keeps an independent capture, Live View, a parameter
        // change and a new scan off the single COM owner.
        let operation = engine
            .operations
            .history
            .iter()
            .find(|operation| operation.kind == "card_import")
            .expect("the import operation receipt");
        assert_eq!(operation.device, "camera");
        assert_eq!(operation.status, "completed");
        // A repeated import has nothing left to promote and is refused explicitly.
        assert!(engine
            .device_command("camera_card_import", json!({}))
            .unwrap()
            .is_err());
    }

    #[test]
    fn import_entry_refuses_without_a_pending_task_or_a_camera() {
        let (adapter, _probe) = camera("entry", false);
        let mut engine = crate::Engine::new_with_storage(
            false,
            root("entry-engine").join("engine-data"),
        );
        engine.camera = Some(adapter.clone());
        engine.cached_camera_health = Some(adapter.health());
        assert_eq!(
            engine.device_command("camera_card_import", json!({})).unwrap(),
            Err("NO_PENDING_IMPORT_TASK")
        );
        assert_eq!(
            engine.device_command("camera_card_import_cancel", json!({})).unwrap(),
            Err("CARD_IMPORT_NOT_RUNNING")
        );
        let mut preview = crate::Engine::new_with_storage(false, root("entry-preview").join("engine-data"));
        preview.preview = true;
        assert_eq!(
            preview.device_command("camera_card_import", json!({})).unwrap(),
            Err("PRODUCTION_LOCKED")
        );
    }

    #[test]
    fn an_import_never_runs_without_the_frozen_camera_identity() {
        let (adapter, probe) = camera("identity", false);
        let fixture = Fixture::new("identity", 3, false, &adapter);
        fixture.write_manifest();
        fixture.script(&probe);
        // A different body (serial) must never be bound to this task's card objects.
        let mut drifted = fixture.manifest(DATA_PENDING_IMPORT, &fixture.records);
        let mut profile = fixture.profile.clone();
        profile.identity.serial = Some("another-body".into());
        drifted["captureProfile"] = serde_json::to_value(&profile).unwrap();
        write_manifest(&fixture.task_root(), &drifted).unwrap();
        let (result, _) = run(&fixture, &adapter, &AtomicBool::new(false));
        assert_eq!(failure_code(&result.unwrap_err()), IMPORT_IDENTITY_CHANGED);
        assert_eq!(probe.operation_count(GET_OBJECT), 0);
        assert_eq!(probe.operation_count(GET_STORAGE_IDS), 0, "no card metadata is read for an unproven body");
    }
}

