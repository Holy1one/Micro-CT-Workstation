//! Transactional projection-scan coordinator.
//!
//! A projection is committed only after turntable arrival, warning assertion,
//! verified X-ray exposure, host-side camera file confirmation, and
//! `CAPTURE_DONE`. A verified beam stays on across views until the configured
//! continuous limit, pause, stop, or final view. Pause is observed at transaction
//! boundaries. USB auto-shutdown is an operator-owned device setting: the
//! coordinator never changes it. Every exit path still attempts verified beam
//! OFF and warning OFF before returning.

use crate::capture_profile::{CaptureProfile, CardCapturePolicy};
use crate::devices::camera::{
    CameraAdapter, CameraCaptureEvent, CameraError, CardCapturePlan, CardPlan,
};
use crate::devices::turntable::{
    plan_projection, verify_fixed_configuration, verify_plan_capture_hold, MoveTicket, NanoAdapter,
    NanoIdentity,
};
use crate::devices::xray::{MoxtekAdapter, XrayError, XrayHealth};
use crate::{timestamp, Parameters};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CAMERA_TRIGGER_MARGIN: Duration = Duration::from_secs(2);
// Leave time for camera/transport jitter before the hard continuous-output cutoff.
const EXPOSURE_BUDGET_MARGIN: Duration = Duration::from_secs(30);
const MOVE_BUDGET_MARGIN: Duration = Duration::from_secs(30);
// Project OFF-interval policy, not a Moxtek-specified five-minute cooldown.
const XRAY_COOLDOWN: Duration = Duration::from_secs(5 * 60);
const CANCELLED: &str = "scan cancelled";
const REFERENCE_COUNT: usize = 10;
/// The controller firmware candidate this workspace builds and pins in
/// `firmware/rts9060_nano/build-config.json`. A card scan freezes the 150 ms
/// last-step-falling-edge settle contract of that candidate, so it starts only
/// when the controller advertises that contract and reports exactly this version
/// and build. Text cannot prove the flashed image; this is a fail-closed identity
/// gate, not a certification.
pub(crate) const CARD_FIRMWARE_VERSION: &str = "2.2.0";
pub(crate) const CARD_FIRMWARE_BUILD: &str = "20261010";
pub(crate) const CARD_FIRMWARE_SOURCE_SHA256: &str =
    "2843f57ecb3d304c47f061f5d342b1b809a807dc2a7980ccc0cf9d14f0ecaa1e";
/// The advertised capability token the card settle contract requires. It mirrors
/// `devices::turntable::MOVE_SETTLE_CAPABILITY`, which is not part of the public
/// turntable surface; the identity accessor is the authority for the check.
pub(crate) const CARD_SETTLE_CAPABILITY: &str = "MOVE_SETTLE_150MS";
/// Schema-4 acquisition states that precede safe finishing. `acquired` is only
/// written once every exposure has its receipt.
pub(crate) const ACQUISITION_PREPARED: &str = "prepared";
pub(crate) const ACQUISITION_RUNNING: &str = "running";
pub(crate) const ACQUISITION_FAULT: &str = "fault";
/// The card cleanup ledger: one line per persisted exposure intent and one per
/// persisted receipt, so a crash between the two is detectable afterwards.
pub(crate) const CARD_JOURNAL_FILE: &str = "acquisition-journal.jsonl";
const CARD_JOURNAL_KIND_INTENT: &str = "intent";
const CARD_JOURNAL_KIND_RECEIPT: &str = "receipt";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanGeometry {
    pub sod_mm: f64,
    pub object_to_detector_mm: f64,
    pub detector_width_mm: f64,
    pub center_offset_x_mm: f64,
    pub center_offset_y_mm: f64,
    pub rotation_direction: String,
    pub mirror_x: bool,
    pub measurement: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scintillator_layers: Vec<crate::reconstruction::spectral::StackLayer>,
}

impl ScanGeometry {
    pub fn validate(&self) -> Result<(), &'static str> {
        crate::reconstruction::spectral::validate_layers(&self.scintillator_layers)
            .map_err(|_| "INVALID_SCINTILLATOR_STACK")?;
        if !self.sod_mm.is_finite()
            || self.sod_mm <= 0.0
            || !self.object_to_detector_mm.is_finite()
            || self.object_to_detector_mm <= 0.0
            || !self.detector_width_mm.is_finite()
            || self.detector_width_mm <= 0.0
            || !self.center_offset_x_mm.is_finite()
            || !self.center_offset_y_mm.is_finite()
            || !matches!(
                self.rotation_direction.as_str(),
                "clockwise" | "counterclockwise"
            )
            || !matches!(self.measurement.as_str(), "measured" | "estimated")
        {
            return Err("INVALID_SCAN_GEOMETRY");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceFrames {
    pub pre_dark: Vec<ReferenceFrame>,
    pub pre_flat: Vec<ReferenceFrame>,
    pub post_flat: Vec<ReferenceFrame>,
    pub post_dark: Vec<ReferenceFrame>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceFrame {
    pub index: u32,
    pub exposure_ms: f64,
    pub file_name: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Unified continuous-X-ray limit for a scan: 1000 s, approved by the user after
/// consulting the vendor. Existing tasks keep their frozen value, the independent
/// manual beam default stays 600 s, and the hard real-time deadline enforcement in
/// the Moxtek owner is unchanged.
pub const SCAN_MIN_XRAY_SEC: u32 = 1;
pub const SCAN_MAX_XRAY_SEC: u32 = 1000;

pub fn scan_xray_seconds_valid(seconds: u32) -> bool {
    (SCAN_MIN_XRAY_SEC..=SCAN_MAX_XRAY_SEC).contains(&seconds)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScanResume {
    pub capture_profile: CaptureProfile,
    pub max_xray_sec: u32,
    pub geometry: ScanGeometry,
    pub frames: Vec<RealFrame>,
    pub references: ReferenceFrames,
    pub stage: String,
}

pub(crate) fn initial_scan_stage(resume: Option<&ScanResume>) -> &'static str {
    resume.map_or("preDark", |resume| match resume.stage.as_str() {
        "preDark" => "preDark",
        "preFlat" => "preFlat",
        "placeSample" => "placeSample",
        "projections" => "projections",
        "removeSample" | "postFlat" | "postDark" | "finalizing" => "finalizing",
        _ => "preDark",
    })
}

pub(crate) fn recovery_position_matches(resume: &ScanResume, parameters: &Parameters, position: i64, ppr: u32) -> bool {
    if resume.stage != "projections" || resume.frames.is_empty() || ppr != 96_000 || position > 0 {
        return false;
    }
    let phase = position.rem_euclid(i64::from(ppr));
    let last = resume.frames.last().unwrap().angle_deg;
    let next = projection_job(resume.frames.len() as u32, parameters.projection_count).angle_deg;
    [last, next].into_iter().any(|angle| {
        let pulses = -crate::devices::turntable::pulses_for_forward_mdeg((-angle * 1000.0).round() as i32);
        pulses.rem_euclid(i64::from(ppr)) == phase
    })
}

#[derive(Clone, Debug, Default)]
struct PromptGate {
    pending: Option<&'static str>,
    confirmed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanCompletion {
    Completed,
    /// A card acquisition finished safely: every exposure has its card receipt,
    /// the beam is confirmed OFF and the ledger is persisted. The NEFs are still on
    /// the camera card, so this is never reported as a completed project; the
    /// domain moves the task to the retained `acquired_pending_import` state.
    AcquiredPendingImport,
    Stopped,
}

#[derive(Clone)]
pub struct RealScanConfig {
    pub capture_profile: CaptureProfile,
    pub parameters: Parameters,
    pub max_xray_sec: u32,
    pub voltage_kv: f64,
    pub current_ua: f64,
    pub geometry: ScanGeometry,
    pub resume: Option<ScanResume>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ProjectionJob {
    index: u32,
    angle_mdeg: i32,
    angle_deg: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealFrame {
    pub index: u32,
    pub angle_deg: f64,
    pub exposure_ms: f64,
    pub file_name: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Schema-4 acquisition state of a card scan. `acquired` is the safe-finishing
/// terminal state; the data states stay separate so a pending import can never be
/// reported as a completed project.
pub const ACQUISITION_ACQUIRED: &str = "acquired";
pub const DATA_PENDING_IMPORT: &str = "pendingImport";
pub const DATA_IMPORTING: &str = "importing";
pub const DATA_VERIFIED: &str = "verified";
pub const DATA_IMPORT_FAILED: &str = "importFailed";

/// One card-resident exposure in a schema-4 manifest. It records the original
/// camera object identity, because at acquisition time there is no local path and
/// no content hash; the verified local file is attached only after the authorized
/// USB import. A record without an imported file can never feed reconstruction.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CardCaptureRecord {
    pub index: u32,
    /// `projection`, `preDark` or `preFlat`.
    pub kind: String,
    pub angle_deg: f64,
    pub exposure_ms: f64,
    /// One of the `DATA_*` states.
    pub state: String,
    pub camera_session_id: String,
    pub operation_id: String,
    pub storage_id: u32,
    pub storage_changed: bool,
    pub handle: u32,
    pub parent: u32,
    /// Original camera filename, kept as provenance for the later import.
    pub source_file_name: String,
    pub format: u16,
    pub size: u64,
    pub capture_date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_file: Option<RealFrame>,
}

impl CardCaptureRecord {
    pub(crate) fn from_receipt(
        index: u32,
        kind: &str,
        angle_deg: f64,
        exposure_ms: f64,
        receipt: &crate::devices::camera::CardObjectReceipt,
    ) -> Self {
        Self {
            index,
            kind: kind.into(),
            angle_deg,
            exposure_ms,
            state: DATA_PENDING_IMPORT.into(),
            camera_session_id: receipt.session_id.clone(),
            operation_id: receipt.operation_id.clone(),
            storage_id: receipt.storage_id,
            storage_changed: receipt.storage_changed,
            handle: receipt.handle,
            parent: receipt.parent,
            source_file_name: receipt.filename.clone(),
            format: receipt.format,
            size: receipt.size,
            capture_date: receipt.capture_date.clone(),
            imported_file: None,
        }
    }
}

/// A card record is reconstructable input only after its import was verified and
/// the local file carries the post-import identity.
pub(crate) fn card_record_is_reconstructable(record: &CardCaptureRecord) -> bool {
    record.state == DATA_VERIFIED && record.imported_file.is_some()
}

/// Every data state a schema-4 record may report. An unknown state is a schema
/// drift and blocks completion instead of being treated as verified.
pub(crate) fn card_data_status_valid(status: &str) -> bool {
    matches!(
        status,
        DATA_PENDING_IMPORT | DATA_IMPORTING | DATA_IMPORT_FAILED | DATA_VERIFIED
    )
}

/// Schema-4 completion rule: a card task counts as completed only when it wrote to
/// the card, froze the card profile, finished safely (`acquired`), verified the
/// whole import (`verified`), kept one reconstructable record per projection and
/// kept every reference record reconstructable too.
pub(crate) fn schema_four_completion_consistent(
    destination: Option<crate::capture_profile::CaptureDestination>,
    profile: Option<&CaptureProfile>,
    acquisition_status: Option<&str>,
    data_status: Option<&str>,
    records: &[CardCaptureRecord],
    projection_count: u32,
) -> bool {
    destination == Some(crate::capture_profile::CaptureDestination::CameraCard)
        && profile.is_some_and(CaptureProfile::is_card_destination)
        && acquisition_status == Some(ACQUISITION_ACQUIRED)
        && data_status == Some(DATA_VERIFIED)
        && records.iter().filter(|record| record.kind == "projection").count()
            == projection_count as usize
        && records
            .iter()
            .all(|record| card_data_status_valid(&record.state) && card_record_is_reconstructable(record))
}

/// The card acquisition ledger, one JSON line per persisted exposure intent and
/// one per persisted receipt. The intent is written BEFORE the trigger and the
/// receipt AFTER it, so a crash between the two leaves an intent with no receipt:
/// that window is detectable afterwards, and an intent without a receipt never
/// becomes a `CardCaptureRecord` and never lets the acquisition report `acquired`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CardJournalEntry {
    /// `intent` or `receipt`.
    pub kind: String,
    pub index: u32,
    /// `projection`, `preDark` or `preFlat`.
    pub group: String,
    pub angle_deg: f64,
    pub exposure_ms: f64,
    pub operation_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_pulse_position: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file_name: Option<String>,
    pub recorded_at: String,
}

fn card_journal_path(root: &Path) -> std::path::PathBuf {
    root.join(CARD_JOURNAL_FILE)
}

/// Appends one durable intent or receipt line. The intent is only ever written by
/// the coordinator before it triggers, and the receipt only after the camera
/// returned an associated object.
pub(crate) fn append_card_journal(root: &Path, entry: &CardJournalEntry) -> Result<(), String> {
    fs::create_dir_all(root).map_err(io_error)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(card_journal_path(root))
        .map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, entry).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)
}

/// Reads the ledger. A malformed line refuses the whole ledger instead of dropping
/// evidence: an unreadable window must never look like "no unresolved intent".
pub(crate) fn read_card_journal(root: &Path) -> Result<Vec<CardJournalEntry>, String> {
    let path = card_journal_path(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(io_error)?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<CardJournalEntry>(line)
                .map_err(|error| format!("card acquisition journal is invalid: {error}"))
        })
        .collect()
}

/// The crash window: intents whose operation never produced a receipt. The card
/// object of such an intent is never counted as acquired, is never deleted and is
/// never re-shot; it requires an explicit reconciliation.
pub(crate) fn unresolved_card_intents(root: &Path) -> Result<Vec<CardJournalEntry>, String> {
    let entries = read_card_journal(root)?;
    Ok(entries
        .iter()
        .filter(|entry| entry.kind == CARD_JOURNAL_KIND_INTENT)
        .filter(|intent| {
            !entries.iter().any(|entry| {
                entry.kind == CARD_JOURNAL_KIND_RECEIPT
                    && entry.operation_id == intent.operation_id
            })
        })
        .cloned()
        .collect())
}

/// Safe finishing of a card acquisition requires that no persisted exposure
/// intent is left without its receipt. The crash window between the two is the
/// only place a card exposure could otherwise be counted without proof, so this
/// gate is the single rule the terminal path uses before it may report the
/// acquisition as acquired.
pub(crate) fn card_acquisition_finish_gate(root: &Path) -> Result<(), String> {
    let unresolved = unresolved_card_intents(root)
        .map_err(|error| format!("card acquisition journal cannot be verified: {error}"))?;
    if unresolved.is_empty() {
        return Ok(());
    }
    Err(format!(
        "CARD_EXPOSURE_UNRESOLVED: {} persisted card exposure intent(s) have no receipt; the card object is not counted as acquired and requires reconciliation",
        unresolved.len()
    ))
}

/// The frozen card policy of a new card acquisition. The connected controller must
/// advertise the unified 150 ms last-step-falling-edge settle contract and report
/// exactly the version and build of the firmware candidate this workspace pins;
/// the policy then freezes that settle contract plus the pinned source identity.
/// A controller without the contract keeps working for host-RAM scans with its
/// longer wait, but no card scan may claim the contract for it. Advertised text is
/// required evidence here, and it is never treated as proof of the flashed image.
pub(crate) fn card_capture_policy(
    identity: &NanoIdentity,
    overflow_advertised: bool,
) -> Result<CardCapturePolicy, String> {
    if !identity.advertises_move_settle_150ms() {
        return Err(format!(
            "CARD_FIRMWARE_UNVERIFIED: the controller does not advertise {}; the card-scan settle contract is unproven",
            CARD_SETTLE_CAPABILITY
        ));
    }
    if identity.version.trim() != CARD_FIRMWARE_VERSION
        || identity.build.trim() != CARD_FIRMWARE_BUILD
    {
        return Err(format!(
            "CARD_FIRMWARE_UNVERIFIED: the controller reports {} / {} and does not prove the pinned card-capture settle firmware {} / {}",
            identity.version.trim(),
            identity.build.trim(),
            CARD_FIRMWARE_VERSION,
            CARD_FIRMWARE_BUILD
        ));
    }
    let policy = CardCapturePolicy {
        mode: if overflow_advertised { "overflow" } else { "single" }.into(),
        settle_ms: crate::capture_profile::MOVE_SETTLE_MS,
        settle_anchor: crate::capture_profile::MOVE_SETTLE_ANCHOR.into(),
        firmware_version: CARD_FIRMWARE_VERSION.into(),
        firmware_build: CARD_FIRMWARE_BUILD.into(),
        firmware_source_sha256: CARD_FIRMWARE_SOURCE_SHA256.into(),
    };
    policy.validate()?;
    Ok(policy)
}

/// Derives this scan's card plan from the freshly enumerated storage evidence.
///
/// Only the storages the camera itself reported as writable are planned, in the
/// camera's order. The overflow continuation is allowed only when the frozen
/// policy permits it, the body advertised the continuation events, and at least
/// two writable storages exist; a single writable card stays a complete plan.
/// Exactly one object per exposure is planned.
pub(crate) fn card_capture_plan(
    planned: &CardPlan,
    policy: &CardCapturePolicy,
    overflow_advertised: bool,
) -> Result<CardCapturePlan, String> {
    if planned.storages.is_empty() {
        return Err("CARD_READ_ONLY: no writable card storage was advertised".into());
    }
    Ok(CardCapturePlan {
        storages: planned.storages.iter().map(|storage| storage.id).collect(),
        allow_overflow: policy.mode == "overflow"
            && overflow_advertised
            && planned.storages.len() >= 2,
        expected_objects: 1,
    })
}

#[derive(Clone, Debug)]
pub struct ScanMessage {
    pub timestamp: String,
    pub level: &'static str,
    pub source: &'static str,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct RealScanProgress {
    pub revision: u64,
    pub phase: &'static str,
    pub captured: u32,
    pub angle_deg: f64,
    pub beam_on: bool,
    pub frames: Vec<RealFrame>,
    pub references: ReferenceFrames,
    /// Card destination only: one receipt per reference and projection. A host-RAM
    /// scan never fills it, and a card scan never fills `frames`/`references`.
    pub card_records: Vec<CardCaptureRecord>,
    /// Card destination only: the schema-4 acquisition state. It stays before
    /// `acquired` until every exposure has its receipt and safe finishing is done.
    pub card_acquisition_status: Option<&'static str>,
    /// Card destination only: the schema-4 data state (`pendingImport` until an
    /// authorized import is verified).
    pub card_data_status: Option<&'static str>,
    pub stage: &'static str,
    pub prompt: Option<&'static str>,
    pub messages: Vec<ScanMessage>,
    pub error: Option<String>,
    pub xray_health: Option<XrayHealth>,
    output_dir_owned: bool,
    pub estimated_remaining_seconds: Option<u64>,
    pub cooldown_remaining_seconds: Option<u64>,
    projection_count: u32,
    projection_started: Option<Instant>,
    // Mean projection work time; completed OFF cooldown waits are excluded.
    measured_projection_time: Option<Duration>,
    projection_cooling_elapsed: Duration,
    cooldown_until: Option<Instant>,
    cooldown_wait_started: Option<Instant>,
    beam_started: Option<Instant>,
    block_limit: Duration,
}

impl Default for RealScanProgress {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: "running",
            captured: 0,
            angle_deg: 0.0,
            beam_on: false,
            frames: Vec::new(),
            references: ReferenceFrames::default(),
            card_records: Vec::new(),
            card_acquisition_status: None,
            card_data_status: None,
            stage: "idle",
            prompt: None,
            messages: Vec::new(),
            error: None,
            xray_health: None,
            output_dir_owned: false,
            estimated_remaining_seconds: None,
            cooldown_remaining_seconds: None,
            projection_count: 0,
            projection_started: None,
            measured_projection_time: None,
            projection_cooling_elapsed: Duration::ZERO,
            cooldown_until: None,
            cooldown_wait_started: None,
            beam_started: None,
            block_limit: Duration::ZERO,
        }
    }
}

impl RealScanProgress {
    fn refresh_estimate(&mut self, now: Instant) {
        self.cooldown_remaining_seconds = self.cooldown_until.map(|deadline| {
            let remaining = deadline.saturating_duration_since(now);
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        });
        self.estimated_remaining_seconds = None;
        if !matches!(self.phase, "running" | "cooling") || self.error.is_some() {
            return;
        }
        let Some(sample) = self.measured_projection_time else {
            return;
        };
        let remaining = self.projection_count.saturating_sub(self.captured);
        if remaining == 0 {
            self.estimated_remaining_seconds = Some(0);
            return;
        }
        let cooling_in_progress = self
            .cooldown_wait_started
            .map(|started| now.saturating_duration_since(started))
            .unwrap_or_default();
        let current_elapsed = self
            .projection_started
            .map(|started| {
                now.saturating_duration_since(started)
                    .saturating_sub(self.projection_cooling_elapsed + cooling_in_progress)
            })
            .unwrap_or_default();
        let work = sample
            .saturating_mul(remaining)
            .saturating_sub(current_elapsed);
        let mut seconds = work.as_secs_f64();
        if let Some(deadline) = self.cooldown_until {
            let pending = deadline.saturating_duration_since(now);
            // Work still being done with the beam OFF overlaps this interval.
            let overlap = if self.phase == "running" && self.projection_started.is_some() {
                sample.saturating_sub(current_elapsed)
            } else {
                Duration::ZERO
            };
            seconds += pending.saturating_sub(overlap).as_secs_f64();
        }
        // Active projection time is an upper bound for future beam time. A
        // completed five-minute OFF interval must never enter that sample.
        if !self.block_limit.is_zero() {
            let active = self
                .beam_started
                .map(|started| now.saturating_duration_since(started).as_secs_f64())
                .unwrap_or(0.0);
            let limit = self.block_limit.as_secs_f64();
            let future_beam_work = sample.saturating_mul(remaining.saturating_sub(1));
            let future_blocks = ((active + future_beam_work.as_secs_f64()) / limit).floor() as u64;
            seconds += future_blocks.saturating_mul(XRAY_COOLDOWN.as_secs()) as f64;
        }
        self.estimated_remaining_seconds = Some(seconds.ceil().min(u64::MAX as f64) as u64);
    }
}

pub struct ScanOutcome {
    pub camera: CameraAdapter,
    pub xray: MoxtekAdapter,
    pub result: Result<ScanCompletion, String>,
}

pub struct RealScanHandle {
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    progress: Arc<Mutex<RealScanProgress>>,
    outcome: mpsc::Receiver<ScanOutcome>,
    worker: Option<JoinHandle<()>>,
    nano: Arc<NanoAdapter>,
    prompt_gate: Arc<Mutex<PromptGate>>,
}

impl RealScanHandle {
    pub fn start(
        nano: Arc<NanoAdapter>,
        camera: CameraAdapter,
        xray: MoxtekAdapter,
        config: RealScanConfig,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let prompt_gate = Arc::new(Mutex::new(PromptGate::default()));
        let initial_stage = initial_scan_stage(config.resume.as_ref());
        let progress = Arc::new(Mutex::new(RealScanProgress {
            captured: config
                .resume
                .as_ref()
                .map_or(0, |resume| resume.frames.len() as u32),
            frames: config
                .resume
                .as_ref()
                .map_or_else(Vec::new, |resume| resume.frames.clone()),
            references: config
                .resume
                .as_ref()
                .map_or_else(ReferenceFrames::default, |resume| resume.references.clone()),
            stage: initial_stage,
            ..RealScanProgress::default()
        }));
        let (sender, outcome) = mpsc::channel();
        let worker_cancel = cancel.clone();
        let worker_pause = pause.clone();
        let worker_progress = progress.clone();
        let worker_prompt_gate = prompt_gate.clone();
        let worker_nano = nano.clone();
        let worker = thread::Builder::new()
            .name("rts9060-real-scan".into())
            .spawn(move || {
                let mut camera = camera;
                let mut xray = xray;
                let mut result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_scan(
                    &worker_nano,
                    &mut camera,
                    &mut xray,
                    &config,
                    &worker_cancel,
                    &worker_pause,
                    &worker_progress,
                    &worker_prompt_gate,
                ))).unwrap_or_else(|_| {
                    let off = xray.force_off();
                    let warning = warning_after_off(&worker_nano,&xray);
                    Err(format!("scan worker panicked; shutdown attempted: X-ray={off:?}, warning={warning:?}"))
                });
                if matches!(result, Ok(ScanCompletion::Stopped)) {
                    update_phase(&worker_progress, "stopped");
                    push_message(&worker_progress, "INFO", "system", "Scan ended by operator; output OFF confirmed".into());
                }
                if let Err(error) = &result {
                    update(&worker_progress, |value| {
                        value.phase = "fault";
                        value.beam_on = xray.health().beam_on;
                        value.xray_health = Some(xray.health());
                        value.error = Some(error.clone());
                    });
                    push_message(&worker_progress, "ERR", "system", format!("Real scan failed closed · {error}"));
                }
                if worker_progress.lock().expect("scan progress mutex poisoned").output_dir_owned {
                    let root = Path::new(config.parameters.save_path.trim()).join(config.parameters.task_id.trim());
                    log_xray_diagnostics(&xray, &worker_progress);
                    if let Err(error) = persist_scan_log(&root, &worker_progress, &result) {
                        let detail = format!("scan log could not be saved: {error}");
                        let rollback = if matches!(result, Ok(ScanCompletion::Completed)) {
                            persist_progress(&root, &config.parameters, &config,
                                &worker_progress, "finalizing", false).err()
                        } else { None };
                        result = Err(match result {
                            Ok(_) => detail.clone(),
                            Err(previous) => format!("{previous}; {detail}"),
                        });
                        if let Some(rollback_error) = rollback {
                            let previous = result.as_ref().err().cloned().unwrap_or_default();
                            result = Err(format!("{previous}; completion checkpoint rollback failed: {rollback_error}"));
                        }
                        update(&worker_progress, |value| {
                            value.phase = "fault";
                            value.error = result.as_ref().err().cloned();
                        });
                        push_message(&worker_progress, "ERR", "system", detail);
                    }
                }
                let _ = sender.send(ScanOutcome {
                    camera,
                    xray,
                    result,
                });
            })
            .expect("failed to spawn real scan worker");
        Self {
            cancel,
            pause,
            progress,
            outcome,
            worker: Some(worker),
            nano,
            prompt_gate,
        }
    }

    pub fn progress(&self) -> RealScanProgress {
        let mut snapshot = self
            .progress
            .lock()
            .expect("scan progress mutex poisoned")
            .clone();
        snapshot.refresh_estimate(Instant::now());
        snapshot
    }

    pub fn request_pause(&self) {
        self.pause.store(true, Ordering::SeqCst);
    }

    pub fn confirm_stage(&self, stage: &str) -> Result<(), &'static str> {
        let mut gate = self.prompt_gate.lock().expect("scan prompt mutex poisoned");
        if gate.pending != Some(stage) || gate.confirmed {
            return Err("SCAN_PROMPT_MISMATCH");
        }
        gate.confirmed = true;
        Ok(())
    }

    pub fn resume(&self) {
        self.pause.store(false, Ordering::SeqCst);
    }

    /// Signal cancellation without waiting for the domain's parallel STOP/OFF.
    pub fn cancel_only(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        update_phase(&self.progress, "stopping");
    }

    pub fn request_stop(&self) -> Result<(), String> {
        if self.cancel.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        update_phase(&self.progress, "stopping");
        self.nano
            .stop()
            .map_err(|error| format!("Nano STOP failed: {error}"))
    }

    pub fn try_finish(&mut self) -> Option<Result<ScanOutcome, String>> {
        poll_scan_outcome(&self.outcome, &mut self.worker)
    }
}

fn poll_scan_outcome<T>(
    receiver: &mpsc::Receiver<T>,
    worker: &mut Option<JoinHandle<()>>,
) -> Option<Result<T, String>> {
    match receiver.try_recv() {
        Ok(outcome) => {
            // Receipt is the terminal ownership handoff. Do not leave a handle
            // whose Drop could send STOP after a successful completion.
            if let Some(worker) = worker.take() {
                if worker.is_finished() {
                    let _ = worker.join();
                }
            }
            Some(Ok(outcome))
        }
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => {
            if worker.as_ref().is_some_and(JoinHandle::is_finished) {
                let _ = worker.take().unwrap().join();
            }
            Some(Err(
                "scan worker exited without a cleanup outcome; output state is unconfirmed".into(),
            ))
        }
    }
}

impl Drop for RealScanHandle {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            if !worker.is_finished() {
                self.cancel.store(true, Ordering::SeqCst);
                let _ = self.nano.stop();
            }
            // Never block IPC or process shutdown on an unbounded join.
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanManifest<'a> {
    schema_version: u32,
    capture_profile: &'a CaptureProfile,
    max_xray_sec: u32,
    scan_id: &'a str,
    created_at: String,
    completed: bool,
    voltage_kv: f64,
    current_ua: f64,
    projection_count: u32,
    exposure_ms: f64,
    geometry: &'a ScanGeometry,
    references: &'a ReferenceFrames,
    stage: &'a str,
    frames: &'a [RealFrame],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredScanManifest {
    schema_version: u32,
    #[serde(default)]
    capture_profile: Option<CaptureProfile>,
    #[serde(default)]
    max_xray_sec: Option<u32>,
    pub(crate) scan_id: String,
    completed: bool,
    voltage_kv: f64,
    current_ua: f64,
    pub(crate) projection_count: u32,
    pub(crate) exposure_ms: f64,
    pub(crate) geometry: ScanGeometry,
    pub(crate) references: ReferenceFrames,
    stage: String,
    pub(crate) frames: Vec<RealFrame>,
    /// Schema 4 only: where this acquisition wrote its NEFs.
    #[serde(default)]
    capture_destination: Option<crate::capture_profile::CaptureDestination>,
    /// Schema 4 only: `acquired` after safe finishing.
    #[serde(default)]
    acquisition_status: Option<String>,
    /// Schema 4 only: `pendingImport` / `importing` / `verified` / `importFailed`.
    #[serde(default)]
    data_status: Option<String>,
    /// Schema 4 only: every card exposure receipt, references included.
    #[serde(default)]
    capture_records: Vec<CardCaptureRecord>,
}

/// Only the completion bit is needed for command gating. Unknown manifest
/// fields are ignored so older checkpoint files remain inspectable.
#[derive(Deserialize)]
struct StoredScanManifestHeader {
    completed: bool,
}

pub fn manifest_file_exists(parameters: &Parameters) -> bool {
    Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("manifest.json")
        .is_file()
}

pub fn completed_manifest_exists(parameters: &Parameters) -> bool {
    let path = Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("manifest.json");
    let Ok(file) = File::open(path) else {
        return false;
    };
    serde_json::from_reader::<_, StoredScanManifest>(file).is_ok_and(|manifest| {
        matches!(manifest.schema_version, 2 | 3 | 4)
            && manifest.completed
            && manifest.stage == "completed"
            && (manifest.schema_version == 2
                || manifest
                    .capture_profile
                    .as_ref()
                    .is_some_and(|profile| profile.validate().is_ok())
                    && manifest
                        .max_xray_sec
                        .is_some_and(|maximum| (SCAN_MIN_XRAY_SEC..=SCAN_MAX_XRAY_SEC).contains(&maximum)))
            // A schema-4 task is only "completed" once every card exposure was
            // acquired, imported and verified; a pending import keeps it open even
            // when safe finishing already happened.
            && (manifest.schema_version != 4
                || schema_four_completion_consistent(
                    manifest.capture_destination,
                    manifest.capture_profile.as_ref(),
                    manifest.acquisition_status.as_deref(),
                    manifest.data_status.as_deref(),
                    &manifest.capture_records,
                    manifest.projection_count,
                ))
            && manifest.scan_id == parameters.task_id
            && manifest.projection_count == parameters.projection_count
            && (manifest.exposure_ms - parameters.exposure_ms).abs() < 1e-6
            && manifest.frames.len() == parameters.projection_count as usize
            && [&manifest.references.pre_dark, &manifest.references.pre_flat]
                .iter()
                .all(|frames| frames.len() == REFERENCE_COUNT)
    })
}

pub fn preparation_checkpoint_exists(parameters: &Parameters) -> bool {
    Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("preparation-checkpoint.json")
        .is_file()
}

pub fn persist_preparation_checkpoint(
    parameters: &Parameters,
    voltage_kv: f64,
    current_ua: f64,
    max_xray_sec: u32,
    capture_profile: &CaptureProfile,
) -> Result<(), String> {
    capture_profile.validate()?;
    if !scan_xray_seconds_valid(max_xray_sec) {
        return Err("Invalid frozen continuous X-ray limit".into());
    }
    let root = Path::new(parameters.save_path.trim()).join(parameters.task_id.trim());
    fs::create_dir_all(&root).map_err(io_error)?;
    if root.join("manifest.json").exists() {
        return Err("scan manifest already exists".into());
    }
    let path = root.join("preparation-checkpoint.json");
    let checkpoint = json!({"schemaVersion":3,"captureProfile":capture_profile,"maxXraySec":max_xray_sec,"scanId":parameters.task_id,"createdAt":timestamp(),
        "stage":"geometry","projectionCount":parameters.projection_count,
        "exposureMs":parameters.exposure_ms,"voltageKv":voltage_kv,"currentUa":current_ua});
    crate::task_registry::atomic_store(
        &path,
        &serde_json::to_vec_pretty(&checkpoint).map_err(|error| error.to_string())?,
    )
}

fn preparation_checkpoint_matches(root: &Path, config: &RealScanConfig) -> Result<bool, String> {
    let path = root.join("preparation-checkpoint.json");
    if !path.exists() {
        return Ok(false);
    }
    let data: serde_json::Value = serde_json::from_reader(File::open(path).map_err(io_error)?)
        .map_err(|error| error.to_string())?;
    let profile: CaptureProfile =
        serde_json::from_value(data["captureProfile"].clone()).map_err(|error| {
            format!("Preparation checkpoint lacks a frozen capture profile: {error}")
        })?;
    config.capture_profile.matches(&profile)?;
    Ok(data["schemaVersion"].as_u64() == Some(3)
        && data["stage"].as_str() == Some("geometry")
        && data["maxXraySec"].as_u64() == Some(u64::from(config.max_xray_sec))
        && data["scanId"].as_str() == Some(config.parameters.task_id.as_str())
        && data["projectionCount"].as_u64() == Some(u64::from(config.parameters.projection_count))
        && data["exposureMs"]
            .as_f64()
            .is_some_and(|value| (value - config.parameters.exposure_ms).abs() < 1e-6)
        && data["voltageKv"]
            .as_f64()
            .is_some_and(|value| (value - config.voltage_kv).abs() < 0.01)
        && data["currentUa"]
            .as_f64()
            .is_some_and(|value| (value - config.current_ua).abs() < 0.01))
}

pub fn resume_manifest_exists(parameters: &Parameters) -> bool {
    let path = Path::new(parameters.save_path.trim())
        .join(parameters.task_id.trim())
        .join("manifest.json");
    let Ok(file) = File::open(path) else {
        return false;
    };
    serde_json::from_reader::<_, StoredScanManifestHeader>(file)
        .is_ok_and(|manifest| !manifest.completed)
}

pub fn load_resume(
    parameters: &Parameters,
    voltage_kv: f64,
    current_ua: f64,
    actual_profile: &CaptureProfile,
) -> Result<ScanResume, String> {
    let root = Path::new(parameters.save_path.trim()).join(parameters.task_id.trim());
    let file = File::open(root.join("manifest.json")).map_err(io_error)?;
    let manifest: StoredScanManifest =
        serde_json::from_reader(file).map_err(|error| format!("invalid scan manifest: {error}"))?;
    if manifest.schema_version == 2 {
        return Err("LEGACY_SCHEMA_2_READ_ONLY: native acquisition cannot append projections without complete camera evidence; use the retained legacy version".into());
    }

    if manifest.schema_version != 3
        || manifest.completed
        || manifest.scan_id != parameters.task_id
        || manifest.projection_count != parameters.projection_count
        || (manifest.exposure_ms - parameters.exposure_ms).abs() > 1e-6
        || (manifest.voltage_kv - voltage_kv).abs() > 0.01
        || (manifest.current_ua - current_ua).abs() > 0.01
        || manifest.frames.len() > parameters.projection_count as usize
    {
        return Err("unfinished scan manifest does not match the current task, exposure, projection count or X-ray setpoints".into());
    }
    let capture_profile = manifest
        .capture_profile
        .ok_or("Frozen capture profile is missing")?;
    capture_profile.matches(actual_profile)?;
    let max_xray_sec = manifest
        .max_xray_sec
        .filter(|maximum| scan_xray_seconds_valid(*maximum))
        .ok_or("Frozen continuous X-ray limit is missing or invalid")?;
    manifest.geometry.validate().map_err(str::to_owned)?;
    if !matches!(
        manifest.stage.as_str(),
        "preDark"
            | "preFlat"
            | "placeSample"
            | "projections"
            | "removeSample"
            | "postFlat"
            | "postDark"
            | "finalizing"
    ) {
        return Err("invalid scan checkpoint stage".into());
    }
    let pre_dark = manifest.references.pre_dark.len();
    let pre_flat = manifest.references.pre_flat.len();
    let post_flat = manifest.references.post_flat.len();
    let post_dark = manifest.references.post_dark.len();
    let projections = manifest.frames.len();
    let full_pre = pre_dark == REFERENCE_COUNT && pre_flat == REFERENCE_COUNT;
    let full_projection = projections == parameters.projection_count as usize;
    let stage_consistent = match manifest.stage.as_str() {
        "preDark" => pre_flat == 0 && post_flat == 0 && post_dark == 0 && projections == 0,
        "preFlat" => {
            pre_dark == REFERENCE_COUNT && post_flat == 0 && post_dark == 0 && projections == 0
        }
        "placeSample" => full_pre && post_flat == 0 && post_dark == 0,
        "projections" => full_pre && post_flat == 0 && post_dark == 0,
        "removeSample" => full_pre && full_projection && post_flat == 0 && post_dark == 0,
        "postFlat" => full_pre && full_projection && post_dark == 0,
        "postDark" => full_pre && full_projection && post_flat == REFERENCE_COUNT,
        "finalizing" => full_pre && full_projection,
        _ => false,
    };
    if !stage_consistent {
        return Err("scan checkpoint stage and committed frame counts disagree".into());
    }
    verify_references(&root, &manifest.references)?;
    for reference in manifest
        .references
        .pre_dark
        .iter()
        .chain(&manifest.references.pre_flat)
        .chain(&manifest.references.post_flat)
        .chain(&manifest.references.post_dark)
    {
        if (reference.exposure_ms - parameters.exposure_ms).abs() > 1e-6 {
            return Err("reference exposure does not match scan setup".into());
        }
    }

    let frames_dir = root.join("frames");
    for (zero_index, frame) in manifest.frames.iter().enumerate() {
        let index = zero_index as u32 + 1;
        let expected_name = format!("frame-{index:04}.nef");
        let expected_path = frames_dir.join(&expected_name);
        if frame.index != index
            || frame.file_name != expected_name
            || Path::new(&frame.path) != expected_path
            || (frame.angle_deg
                - projection_job(zero_index as u32, parameters.projection_count).angle_deg)
                .abs()
                > 0.01
            || (frame.exposure_ms - parameters.exposure_ms).abs() > 1e-6
        {
            return Err(format!(
                "resume frame {index} does not match the recorded scan geometry"
            ));
        }
        let (bytes, sha256) = hash_file(&expected_path)?;
        if bytes != frame.bytes || sha256 != frame.sha256 {
            return Err(format!(
                "resume frame {index} failed size or SHA-256 verification"
            ));
        }
    }
    let mut file_count = 0;
    for entry in fs::read_dir(&frames_dir).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if !entry.file_type().map_err(io_error)?.is_file() {
            return Err(
                "scan frames directory contains a non-file entry; inspect it before restoring"
                    .into(),
            );
        }
        file_count += 1;
    }
    if file_count != manifest.frames.len() {
        return Err("scan frames directory has an uncommitted or unexpected file; inspect it before restoring".into());
    }
    // Older recovery workers wrote placeSample even after committing projections.
    // Normalize only after checking every frozen setting and retained file.
    let stage = match manifest.stage.as_str() {
        "placeSample" if projections > 0 => "projections".to_owned(),
        "removeSample" | "postFlat" | "postDark" => "finalizing".to_owned(),
        _ => manifest.stage,
    };
    Ok(ScanResume {
        capture_profile,
        max_xray_sec,
        geometry: manifest.geometry,
        frames: manifest.frames,
        references: manifest.references,
        stage,
    })
}

fn verify_references(root: &Path, references: &ReferenceFrames) -> Result<(), String> {
    for (group, frames) in [
        ("pre-dark", &references.pre_dark),
        ("pre-flat", &references.pre_flat),
        ("post-flat", &references.post_flat),
        ("post-dark", &references.post_dark),
    ] {
        if frames.len() > REFERENCE_COUNT {
            return Err(format!("too many {group} references"));
        }
        let dir = root.join("references").join(group).join("frames");
        for (zero_index, frame) in frames.iter().enumerate() {
            let index = zero_index as u32 + 1;
            let expected_name = format!("frame-{index:04}.nef");
            let path = dir.join(&expected_name);
            if frame.index != index
                || frame.file_name != expected_name
                || Path::new(&frame.path) != path
                || !frame.exposure_ms.is_finite()
                || frame.exposure_ms <= 0.0
            {
                return Err(format!("{group} reference {index} path mismatch"));
            }
            let (bytes, sha256) = hash_file(&path)?;
            if bytes != frame.bytes || sha256 != frame.sha256 {
                return Err(format!(
                    "{group} reference {index} failed SHA-256 verification"
                ));
            }
        }
        if dir.exists() {
            let entries = fs::read_dir(&dir).map_err(io_error)?.count();
            if entries != frames.len() {
                return Err(format!("{group} has an uncommitted file"));
            }
        }
    }
    Ok(())
}

fn run_scan(
    nano: &NanoAdapter,
    camera: &mut CameraAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    pause: &AtomicBool,
    progress: &Mutex<RealScanProgress>,
    prompt_gate: &Mutex<PromptGate>,
) -> Result<ScanCompletion, String> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_scan_inner(
            nano,
            camera,
            xray,
            config,
            cancel,
            pause,
            progress,
            prompt_gate,
        )
    }))
    .unwrap_or_else(|_| Err("scan body panicked".into()));
    let off = xray.force_off();
    let warning_off = if off.is_ok() {
        warning_after_off(nano, xray)
    } else {
        // Unknown emission keeps the physical warning asserted.
        let _ = nano.set_xray_warning(true);
        Err(crate::devices::turntable::NanoError::Safety(
            "Warning retained because Moxtek OFF is unconfirmed".into(),
        ))
    };
    if off.is_ok() {
        push_message(
            progress,
            "PASS",
            "xray",
            "Moxtek OFF confirmed; operator-owned USB auto-shutdown setting unchanged".to_owned(),
        );
    }
    set_xray_health(progress, xray.health());
    set_beam(progress, xray.health().beam_on);
    if result.is_err() {
        let _ = nano.stop();
    }

    // An operator Stop and a device fault both cancel the inner loop, so the cancel flag
    // alone cannot label the outcome. Reporting a fault as "Scan ended by operator" would
    // hide the real cause from the operator and from the record; the worker owns the live
    // device evidence here, so it decides which of the two actually happened.
    let card_scan = config.capture_profile.is_card_destination();
    let success = if card_scan {
        ScanCompletion::AcquiredPendingImport
    } else {
        ScanCompletion::Completed
    };
    let off_result = off.map(|_| ()).map_err(|e| e.to_string());
    let warning_result = warning_off.map_err(|e| e.to_string());
    let completion = match if card_scan {
        finish_scan_result_for(
            result,
            off_result,
            warning_result,
            ScanCompletion::AcquiredPendingImport,
        )
    } else {
        finish_scan_result(result, off_result, warning_result)
    } {
        Ok(ScanCompletion::Stopped) => match device_fault_evidence(camera, xray) {
            Some(reason) => Err(reason),
            None => Ok(ScanCompletion::Stopped),
        },
        other => other,
    };
    let root = Path::new(config.parameters.save_path.trim()).join(config.parameters.task_id.trim());
    // Safe finishing of a card acquisition additionally requires that no persisted
    // exposure intent is left without its receipt. An unresolved intent is the
    // crash window between the intent and the object confirmation: the object is
    // never counted as acquired, never deleted and never re-shot, and the
    // acquisition is reported as a fault that needs reconciliation instead.
    let completion = if card_scan && completion == Ok(ScanCompletion::AcquiredPendingImport) {
        match card_acquisition_finish_gate(&root) {
            Ok(()) => Ok(ScanCompletion::AcquiredPendingImport),
            Err(detail) => {
                update(progress, |value| {
                    value.phase = "fault";
                    value.error = Some(detail.clone());
                });
                push_message(progress, "ERR", "camera", detail.clone());
                Err(detail)
            }
        }
    } else {
        completion
    };
    let owned = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .output_dir_owned;
    let checkpoint = if owned {
        let stage = progress.lock().expect("scan progress mutex poisoned").stage;
        let finished = completion == Ok(success);
        if card_scan {
            let (acquisition, data) = match &completion {
                Ok(ScanCompletion::AcquiredPendingImport) => {
                    (ACQUISITION_ACQUIRED, DATA_PENDING_IMPORT)
                }
                Ok(ScanCompletion::Stopped) => ("stopped", DATA_PENDING_IMPORT),
                _ => (ACQUISITION_FAULT, DATA_PENDING_IMPORT),
            };
            set_card_acquisition(progress, acquisition, data);
        }
        persist_progress(
            &root,
            &config.parameters,
            config,
            progress,
            if finished {
                if card_scan {
                    "finalizing"
                } else {
                    "completed"
                }
            } else {
                stage
            },
            finished,
        )
    } else {
        Ok(())
    };
    if let Err(error) = checkpoint {
        return Err(match completion {
            Err(previous) => format!("{previous}; checkpoint save failed: {error}"),
            _ => format!("checkpoint save failed: {error}"),
        });
    }
    if completion == Ok(ScanCompletion::Completed) {
        update_phase(progress, "completed");
        let frame_count = progress
            .lock()
            .expect("scan progress mutex poisoned")
            .frames
            .len();
        push_message(
            progress,
            "PASS",
            "system",
            format!(
                "Real scan complete · {} projections · X-ray OFF",
                frame_count
            ),
        );
    }
    if completion == Ok(ScanCompletion::AcquiredPendingImport) {
        update_phase(progress, "finishing");
        let (projections, references) = {
            let state = progress.lock().expect("scan progress mutex poisoned");
            (
                state
                    .card_records
                    .iter()
                    .filter(|record| record.kind == "projection")
                    .count(),
                state
                    .card_records
                    .iter()
                    .filter(|record| record.kind != "projection")
                    .count(),
            )
        };
        push_message(
            progress,
            "PASS",
            "system",
            format!(
                "Card acquisition safely finished · {projections} projections + {references} references on the camera card · X-ray OFF · USB import required"
            ),
        );
    }
    completion
}

/// The device evidence that a stop was caused by a fault instead of by the operator.
///
/// A confirmed OFF plus a healthy, non-isolated camera leaves an operator Stop a stop. A
/// latched X-ray error, output that is still on or no longer confirmed off, or a camera
/// session left isolated with unconfirmed cleanup is a fault: it must be reported as one so
/// the operator and the retained record see the real cause.
fn device_fault_evidence(camera: &CameraAdapter, xray: &MoxtekAdapter) -> Option<String> {
    let xray_health = xray.health();
    if let Some(error) = xray_health
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    {
        return Some(format!("device fault ended the scan · X-ray: {error}"));
    }
    if xray_health.beam_on {
        return Some("device fault ended the scan · Moxtek output is still ON".to_owned());
    }
    if !xray_health.beam_off_confirmed {
        return Some("device fault ended the scan · Moxtek OFF was not confirmed".to_owned());
    }
    let camera_health = camera.health();
    if camera_health.isolated && !camera_health.cleanup_confirmed {
        return Some(
            "device fault ended the scan · camera session isolated with unconfirmed cleanup"
                .to_owned(),
        );
    }
    None
}

fn finish_scan_result(
    result: Result<(), String>,
    off: Result<(), String>,
    warning: Result<(), String>,
) -> Result<ScanCompletion, String> {
    finish_scan_result_for(result, off, warning, ScanCompletion::Completed)
}

/// The same terminal rule for a chosen success outcome: a card acquisition
/// finishes safely as `AcquiredPendingImport`, never as a completed project.
fn finish_scan_result_for(
    result: Result<(), String>,
    off: Result<(), String>,
    warning: Result<(), String>,
    success: ScanCompletion,
) -> Result<ScanCompletion, String> {
    let stopped = matches!(&result, Err(error) if error == CANCELLED);
    let mut errors = Vec::new();
    if let Err(error) = result {
        if !stopped {
            errors.push(error);
        }
    }
    if let Err(error) = off {
        errors.push(format!("final Moxtek OFF unconfirmed: {error}"));
    }
    if let Err(error) = warning {
        errors.push(format!("final warning OFF unconfirmed: {error}"));
    }
    if !errors.is_empty() {
        Err(errors.join("; "))
    } else if stopped {
        Ok(ScanCompletion::Stopped)
    } else {
        Ok(success)
    }
}

fn run_scan_inner(
    nano: &NanoAdapter,
    camera: &mut CameraAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    pause: &AtomicBool,
    progress: &Mutex<RealScanProgress>,
    prompt_gate: &Mutex<PromptGate>,
) -> Result<(), String> {
    let parameters = &config.parameters;
    config.capture_profile.validate()?;
    if !scan_xray_seconds_valid(config.max_xray_sec)
        || config
            .resume
            .as_ref()
            .is_some_and(|resume| resume.max_xray_sec != config.max_xray_sec)
    {
        return Err("Continuous X-ray limit differs from the frozen acquisition".into());
    }
    let root = Path::new(parameters.save_path.trim()).join(parameters.task_id.trim());
    let card_scan = config.capture_profile.is_card_destination();
    // A card acquisition has no local file to restore: its exposures live in the
    // schema-4 manifest and on the card itself, so a schema-3 checkpoint can never
    // continue one. Refusing here also means no device action is taken.
    let resume_count = if card_scan {
        if config.resume.is_some() {
            return Err("CARD_ACQUISITION_NO_LOCAL_RESUME: a card acquisition cannot continue from a local file checkpoint".into());
        }
        0
    } else {
        config
            .resume
            .as_ref()
            .map_or(0, |resume| resume.frames.len())
    };
    if config.resume.is_none() {
        if root.exists() {
            let entries = fs::read_dir(&root)
                .map_err(io_error)?
                .map(|entry| entry.map(|value| value.file_name().to_string_lossy().into_owned()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(io_error)?;
            if !entries.is_empty()
                && !(entries.len() == 1
                    && entries[0] == "preparation-checkpoint.json"
                    && preparation_checkpoint_matches(&root, config)?)
            {
                return Err(format!(
                    "scan output already exists: {} · use Restore after Preflight and HOME",
                    root.display()
                ));
            }
        }
        if card_scan {
            // The card task keeps only its manifest, ledger and log on the host;
            // no frames/ directory is created because no image reaches the host.
            fs::create_dir_all(&root).map_err(io_error)?;
        } else {
            fs::create_dir_all(root.join("frames")).map_err(io_error)?;
        }
    } else if load_resume(
        parameters,
        config.voltage_kv,
        config.current_ua,
        &config.capture_profile,
    )? != *config.resume.as_ref().unwrap()
    {
        return Err("scan files changed after Restore; no device action was started".into());
    }
    update(progress, |value| value.output_dir_owned = true);
    push_message(
        progress,
        "ACTION",
        "operator",
        if resume_count == 0 {
            format!(
                "Real scan started · {} projections · {:.1} kV / {:.1} µA",
                parameters.projection_count, config.voltage_kv, config.current_ua
            )
        } else {
            format!(
                "Real scan restored · {resume_count} files verified · continuing at view {}",
                resume_count + 1
            )
        },
    );
    if camera.health().live_view.active { return Err("LIVE_VIEW_CAPTURE_FORBIDDEN: explicitly stop Live View".into()); }
    config.capture_profile.require_current_policy()?;
    camera
        .read_properties()
        .map_err(|error| format!("Camera profile readback failed: {error}"))?;
    // The frozen destination owns its own readback rule: a card acquisition proves
    // the card destination (`recordingMedia=0`) instead of the host SDRAM value, so
    // a RAM profile can never silently describe a card scan or the reverse.
    let actual_profile = if card_scan {
        let readback = camera
            .card_profile_readback()
            .map_err(|error| error.to_string())?;
        CaptureProfile::from_card_readback(
            &camera.health(),
            readback,
            config.capture_profile.preset.clone(),
            config
                .capture_profile
                .card_policy
                .clone()
                .ok_or("CARD_POLICY_MISSING: the frozen card acquisition has no card policy")?,
        )?
    } else {
        CaptureProfile::from_camera_readback(
            &camera.health(),
            camera
                .profile_readback()
                .map_err(|error| error.to_string())?,
            config.capture_profile.preset.clone(),
        )?
    };
    config.capture_profile.matches(&actual_profile)?;
    if actual_profile.exposure_ms() != Some(parameters.exposure_ms) {
        return Err(
            "Native shutter readback does not exactly match the scan Exposure target".into(),
        );
    }
    push_message(
        progress,
        "PASS",
        "camera",
        format!("D7100 native shutter verified at {:.6} ms",parameters.exposure_ms),
    );
    // A card scan enumerates the advertised storages exactly once, here, before any
    // exposure, movement or beam: the whole task (10 pre-dark + 10 pre-flat
    // references plus every projection at the conservative per-frame bound) must
    // fit the writable evidence, and no earlier exposure intent may be unresolved.
    // A refusal carries the stable storage code and precedes every device action.
    let card_plan = if card_scan {
        let unresolved = unresolved_card_intents(&root)?;
        if !unresolved.is_empty() {
            return Err(format!(
                "CARD_ACQUISITION_UNRESOLVED: {} persisted card exposure intent(s) have no receipt; the card object is never re-shot and this task requires reconciliation before another card scan",
                unresolved.len()
            ));
        }
        let planned = camera
            .plan_card_storages(parameters.projection_count)
            .map_err(|error| format!("CARD_STORAGE_PLAN_REFUSED: {error}"))?;
        let policy = config
            .capture_profile
            .card_policy
            .as_ref()
            .ok_or("CARD_POLICY_MISSING: the frozen card acquisition has no card policy")?;
        let plan = card_capture_plan(&planned, policy, camera.health().capabilities.card_overflow)
            .map_err(|error| format!("CARD_STORAGE_PLAN_REFUSED: {error}"))?;
        push_message(
            progress,
            "PASS",
            "camera",
            format!(
                "Card storage planned · {} writable storage(s) · {} bytes free · {} free images · overflow {} · one object per exposure",
                planned.storages.len(),
                planned.writable_bytes,
                planned.writable_images,
                if plan.allow_overflow { "permitted" } else { "not permitted" }
            ),
        );
        set_card_acquisition(progress, ACQUISITION_RUNNING, DATA_PENDING_IMPORT);
        Some(plan)
    } else {
        None
    };
    xray.set_parameters(config.voltage_kv, config.current_ua)
        .map_err(|error| format!("Moxtek setpoint configuration failed: {error}"))?;
    set_xray_health(progress, xray.health());

    let block_limit = Duration::from_secs(u64::from(config.max_xray_sec));
    let exposure_evidence_timeout = Duration::from_secs_f64(parameters.exposure_ms / 1000.0)
        .saturating_add(CAMERA_TRIGGER_MARGIN);
    let exposure_budget = exposure_evidence_timeout.saturating_add(EXPOSURE_BUDGET_MARGIN);
    if exposure_budget >= block_limit {
        return Err(format!(
            "max X-ray time of {} s cannot accommodate the {:.3} ms exposure and confirmation margin",
            config.max_xray_sec, parameters.exposure_ms
        ));
    }
    let mut beam_started: Option<Instant> = None;
    let mut cooldown_until: Option<Instant> = None;
    if config.resume.is_none() {
        persist_progress(&root, parameters, config, progress, "preDark", false)?;
        if preparation_checkpoint_exists(parameters) {
            fs::remove_file(root.join("preparation-checkpoint.json")).map_err(io_error)?;
        }
    }
    update(progress, |value| {
        value.projection_count = parameters.projection_count;
        value.block_limit = block_limit;
    });

    let resume_stage = initial_scan_stage(config.resume.as_ref());
    if !matches!(resume_stage, "projections" | "finalizing") {
        let pre_references_incomplete = {
            let state = progress.lock().expect("scan progress mutex poisoned");
            state.references.pre_dark.len() < REFERENCE_COUNT
                || state.references.pre_flat.len() < REFERENCE_COUNT
        };
        if pre_references_incomplete {
            wait_for_prompt(nano, xray, progress, prompt_gate, cancel, "preReferences")?;
        }
        if card_scan {
            capture_card_reference_group(
                nano,
                camera,
                xray,
                config,
                cancel,
                progress,
                &root,
                card_plan
                    .as_ref()
                    .expect("a card scan planned its storages before any exposure"),
                "pre-dark",
                "preDark",
                false,
            )?;
            capture_card_reference_group(
                nano,
                camera,
                xray,
                config,
                cancel,
                progress,
                &root,
                card_plan
                    .as_ref()
                    .expect("a card scan planned its storages before any exposure"),
                "pre-flat",
                "preFlat",
                true,
            )?;
        } else {
            capture_reference_group(
                nano, camera, xray, config, cancel, progress, &root, "pre-dark", "preDark", false,
            )?;
            capture_reference_group(
                nano, camera, xray, config, cancel, progress, &root, "pre-flat", "preFlat", true,
            )?;
        }
        if resume_count < parameters.projection_count as usize {
            update(progress, |value| value.stage = "placeSample");
            persist_progress(&root, parameters, config, progress, "placeSample", false)?;
            wait_for_prompt(nano, xray, progress, prompt_gate, cancel, "placeSample")?;
            persist_progress(&root, parameters, config, progress, "projections", false)?;
            update(progress, |value| value.stage = "projections");
        }
    } else if resume_stage == "projections" {
        persist_progress(&root, parameters, config, progress, "projections", false)?;
        update(progress, |value| value.stage = "projections");
        push_message(progress, "INFO", "system", format!(
            "Verified pre-scan references retained; continuing at projection {}",
            resume_count + 1
        ));
    }

    for zero_index in resume_count as u32..parameters.projection_count {
        let job = projection_job(zero_index, parameters.projection_count);
        check_cancel(cancel)?;
        if let Some(cooldown_deadline) = cooldown_until.take() {
            wait_for_cooldown(progress, cancel, cooldown_deadline)?;
        }
        if pause.load(Ordering::SeqCst) && beam_started.is_some() {
            close_scan_beam(nano, xray, progress, &mut beam_started, "Pause boundary")?;
        }
        while pause.load(Ordering::SeqCst) {
            update_phase(progress, "paused");
            if cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        if beam_started.is_some_and(|started| {
            needs_cooldown_before_exposure(
                started,
                Instant::now(),
                block_limit,
                exposure_budget.saturating_add(MOVE_BUDGET_MARGIN),
            )
        }) {
            close_scan_beam(
                nano,
                xray,
                progress,
                &mut beam_started,
                "Cooling before the next projection's move and exposure",
            )?;
            wait_for_cooldown(progress, cancel, Instant::now() + XRAY_COOLDOWN)?;
        }
        update_phase(progress, "running");
        update(progress, |value| {
            value.projection_started = Some(Instant::now());
            value.projection_cooling_elapsed = Duration::ZERO;
        });
        let ticket = move_to_projection_monitored(
            nano,
            xray,
            progress,
            job,
            cancel,
            &mut beam_started,
            block_limit,
            &mut cooldown_until,
        )?;

        // The limit can expire during a long MOVE_ABS. The next exposure must
        // wait for the entire OFF interval, even though the move has completed.
        if let Some(deadline) = cooldown_until.take() {
            wait_for_cooldown(progress, cancel, deadline)?;
            verify_capture_hold(nano, &ticket, job)?;
        }

        if beam_started.is_some_and(|started| {
            needs_cooldown_before_exposure(started, Instant::now(), block_limit, exposure_budget)
        }) {
            close_scan_beam(
                nano,
                xray,
                progress,
                &mut beam_started,
                "Cooling before the current projection's exposure",
            )?;
            wait_for_cooldown(progress, cancel, Instant::now() + XRAY_COOLDOWN)?;
            verify_capture_hold(nano, &ticket, job)?;
        }

        if beam_started.is_none() {
            nano.set_xray_warning(true)
                .map_err(|error| format!("XRAY_WARNING ON failed: {error}"))?;
            // Include the entire ON command latency in the continuous-output budget.
            beam_started = Some(Instant::now());
            update(progress, |value| value.beam_started = beam_started);
            let emission = xray.beam_on_shared(
                cancel.clone(),
                beam_started.expect("beam start set") + block_limit,
            );
            check_cancel(cancel)?;
            emission.map_err(|error| xray_operation_error(error, "Moxtek beam-on failed"))?;
            set_xray_health(progress, xray.health());
            set_beam(progress, true);
            if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
                close_scan_beam(
                    nano,
                    xray,
                    progress,
                    &mut beam_started,
                    "Continuous X-ray limit reached during beam-on confirmation",
                )?;
                return Err("continuous X-ray limit reached before exposure started".into());
            }
            if beam_started.is_some_and(|started| {
                needs_cooldown_before_exposure(
                    started,
                    Instant::now(),
                    block_limit,
                    exposure_budget,
                )
            }) {
                close_scan_beam(
                    nano,
                    xray,
                    progress,
                    &mut beam_started,
                    "Beam enable left insufficient time for a confirmed exposure",
                )?;
                return Err(
                    "configured X-ray time cannot accommodate beam enable and the current exposure"
                        .into(),
                );
            }
            push_message(
                progress,
                "ACTION",
                "xray",
                format!(
                "Continuous beam block confirmed ON · view={} · {:.1} kV / {:.1} µA · limit {} s",
                job.index, config.voltage_kv, config.current_ua, config.max_xray_sec
            ),
            );
        }

        let frame_index = job.index;
        let final_projection = frame_index == parameters.projection_count;
        let camera_for_capture = camera.clone();
        let capture_session = camera
            .health()
            .session_id
            .ok_or("Capture session is unknown")?;
        let capture_operation = format!(
            "scan-{}-{}-{}",
            parameters.task_id,
            frame_index,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        if card_scan {
            // MOVE and the matched READY(ticket) verification already happened above.
            // The order from here is fixed: persist the intent, one card exposure
            // (one object + one completion), persist the receipt, CAPTURE_DONE with
            // the same ticket, then the next MOVE. No GET_OBJECT, no local file, no
            // hash and no extra sleep: the coordinator advances only on matched
            // device evidence.
            let plan = card_plan
                .as_ref()
                .expect("a card scan planned its storages before any exposure");
            let intent = CardJournalEntry {
                kind: CARD_JOURNAL_KIND_INTENT.into(),
                index: frame_index,
                group: "projection".into(),
                angle_deg: job.angle_deg,
                exposure_ms: parameters.exposure_ms,
                operation_id: capture_operation.clone(),
                session_id: capture_session.clone(),
                planned_pulse_position: Some(ticket.position_pulses),
                handle: None,
                storage_id: None,
                source_file_name: None,
                recorded_at: timestamp(),
            };
            append_card_journal(&root, &intent).map_err(|error| {
                format!("card exposure intent could not be persisted; the exposure was not triggered: {error}")
            })?;
            let mut abort_reason = None;
            let mut last_xray_poll = Instant::now();
            let mut evidence =
                ExposureEvidence::new(capture_session.clone(), capture_operation.clone());
            let plan_for_worker = plan.clone();
            let worker_operation = capture_operation.clone();
            let capture_result = thread::scope(|scope| {
                let (capture_sender, capture_receiver) = mpsc::channel();
                let (event_sender, event_receiver) = mpsc::channel();
                scope.spawn(move || {
                    let result = camera_for_capture.capture_to_card(
                        plan_for_worker,
                        &worker_operation,
                        cancel,
                        Some(event_sender),
                    );
                    let _ = capture_sender.send(result);
                });
                loop {
                    for event in event_receiver.try_iter() {
                        if let Err(error) = evidence.observe(&event) {
                            abort_reason = Some(error);
                            cancel.store(true, Ordering::SeqCst);
                            let _ = xray.force_off();
                            let _ = nano.stop();
                        }
                    }
                    match capture_receiver.recv_timeout(Duration::from_millis(25)) {
                        Ok(result) => {
                            for event in event_receiver.try_iter() {
                                if let Err(error) = evidence.observe(&event) {
                                    abort_reason = Some(error);
                                }
                            }
                            break result;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if abort_reason.is_none()
                                && beam_started.is_some()
                                && last_xray_poll.elapsed() >= Duration::from_millis(250)
                            {
                                last_xray_poll = Instant::now();
                                match refresh_emission_monitored(xray, progress, cancel) {
                                    Ok(health) => {
                                        if let Some(warning) = health.telemetry_warning.clone() {
                                            push_message(progress, "WARN", "xray", warning);
                                        }
                                        set_xray_health(progress, health);
                                    }
                                    Err(error) => {
                                        abort_reason = Some(xray_operation_error(
                                            error,
                                            "Moxtek emission monitor failed",
                                        ));
                                        cancel.store(true, Ordering::SeqCst);
                                        set_xray_health(progress, xray.health());
                                        let _ = warning_after_off(nano, xray);
                                        set_beam(progress, xray.health().beam_on);
                                        let _ = nano.stop();
                                    }
                                }
                            }
                            if abort_reason.is_none() && cancel.load(Ordering::SeqCst) {
                                abort_reason = Some(CANCELLED.into());
                                let _ = xray.force_off();
                                set_xray_health(progress, xray.health());
                                let _ = warning_after_off(nano, xray);
                                set_beam(progress, xray.health().beam_on);
                                let _ = nano.stop();
                            }
                            if abort_reason.is_none() {
                                let exposure_complete = evidence.exposure_ended;
                                let limit_reached = beam_started
                                    .is_some_and(|started| started.elapsed() >= block_limit);
                                if limit_reached && !exposure_complete {
                                    abort_reason = Some(format!(
                                        "continuous X-ray limit of {} s reached before exposure completion",
                                        config.max_xray_sec
                                    ));
                                    cancel.store(true, Ordering::SeqCst);
                                    let _ = xray.force_off();
                                    set_xray_health(progress, xray.health());
                                    set_beam(progress, xray.health().beam_on);
                                    let _ = warning_after_off(nano, xray);
                                    let _ = nano.stop();
                                }
                                let pause_requested = pause.load(Ordering::SeqCst);
                                if abort_reason.is_none()
                                    && exposure_complete
                                    && beam_started.is_some()
                                    && (limit_reached || final_projection || pause_requested)
                                {
                                    let reason = if limit_reached {
                                        "Continuous X-ray limit reached after current exposure"
                                    } else if final_projection {
                                        "Final projection exposure complete"
                                    } else {
                                        "Pause requested after current exposure"
                                    };
                                    match close_scan_beam(
                                        nano,
                                        xray,
                                        progress,
                                        &mut beam_started,
                                        reason,
                                    ) {
                                        Ok(_) => {
                                            if limit_reached && !final_projection {
                                                cooldown_until =
                                                    Some(Instant::now() + XRAY_COOLDOWN);
                                                update(progress, |value| {
                                                    value.cooldown_until = cooldown_until
                                                });
                                            }
                                        }
                                        Err(error) => {
                                            abort_reason = Some(error);
                                            cancel.store(true, Ordering::SeqCst);
                                            let _ = nano.stop();
                                        }
                                    }
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break Err(CameraError::Io(
                                "camera card capture worker terminated without a result".into(),
                            ));
                        }
                    }
                }
            });
            if let Some(reason) = abort_reason {
                return Err(reason);
            }
            check_cancel(cancel)?;
            let outcome =
                capture_result.map_err(|error| format!("D7100 card capture failed: {error}"))?;
            if outcome.receipt.session_id != capture_session
                || outcome.receipt.operation_id != capture_operation
                || !outcome.exposure_ended
                || !evidence.exposure_ended
            {
                return Err(
                    "Card capture completed without this operation's reliable ExposureEnded event and card object association"
                        .into(),
                );
            }
            let limit_reached = beam_started.is_some_and(|started| started.elapsed() >= block_limit);
            if beam_started.is_some()
                && (limit_reached || final_projection || pause.load(Ordering::SeqCst))
            {
                let reason = if limit_reached {
                    "Continuous X-ray limit reached after completed exposure"
                } else if final_projection {
                    "Final projection capture complete"
                } else {
                    "Pause requested after completed exposure"
                };
                close_scan_beam(nano, xray, progress, &mut beam_started, reason)?;
                if limit_reached && !final_projection {
                    cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                    update(progress, |value| value.cooldown_until = cooldown_until);
                }
            }
            // The durable receipt is persisted BEFORE the same-ticket CAPTURE_DONE,
            // so a crash after the object confirmation can never lose the mapping
            // between this angle and the card object.
            let record = CardCaptureRecord::from_receipt(
                frame_index,
                "projection",
                job.angle_deg,
                parameters.exposure_ms,
                &outcome.receipt,
            );
            update(progress, |value| {
                value.card_records.push(record.clone());
                value.card_data_status = Some(DATA_PENDING_IMPORT);
            });
            persist_progress(&root, parameters, config, progress, "projections", false)?;
            append_card_journal(
                &root,
                &card_receipt_journal_entry(&record, "projection", &capture_operation, &capture_session, timestamp()),
            )
            .map_err(|error| format!("card receipt could not be persisted: {error}"))?;
            nano.capture_done_cancelled(ticket.command_id, cancel.clone())
                .map_err(|error| format!("CAPTURE_DONE view {} failed: {error}", job.index))?;
            check_cancel(cancel)?;
            update(progress, |value| {
                record_projection_sample(value, frame_index, Instant::now())
            });
            push_message(
                progress,
                "PASS",
                "nano",
                format!(
                    "CAPTURE_DONE view={} id={} · card object released",
                    job.index, ticket.command_id
                ),
            );
            push_message(
                progress,
                "PASS",
                "camera",
                format!(
                    "Card projection receipt · view={} · storage 0x{:08X} · handle 0x{:08X} · {} · {} bytes (no local file)",
                    record.index,
                    record.storage_id,
                    record.handle,
                    record.source_file_name,
                    record.size
                ),
            );
            continue;
        }

        let mut abort_reason = None;
        let mut last_xray_poll = Instant::now();
        let mut transfer_announced = false;
        let mut evidence =
            ExposureEvidence::new(capture_session.clone(), capture_operation.clone());
        let capture_result = thread::scope(|scope| {
            let (capture_sender, capture_receiver) = mpsc::channel();
            let (event_sender, event_receiver) = mpsc::channel();
            let capture_path = root
                .join("frames")
                .join(format!("frame-{frame_index:04}.nef"));
            let worker_operation = capture_operation.clone();
            scope.spawn(move || {
                let result = camera_for_capture.capture_to_path(
                    &capture_path,
                    &worker_operation,
                    cancel,
                    Some(event_sender),
                );
                let _ = capture_sender.send(result);
            });
            loop {
                for event in event_receiver.try_iter() {
                    if let Err(error) = evidence.observe(&event) {
                        abort_reason = Some(error);
                        cancel.store(true, Ordering::SeqCst);
                        let _ = xray.force_off();
                        let _ = nano.stop();
                    }
                }
                match capture_receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(result) => {
                        for event in event_receiver.try_iter() {
                            if let Err(error) = evidence.observe(&event) {
                                abort_reason = Some(error);
                            }
                        }
                        break result;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if abort_reason.is_none()
                            && beam_started.is_some()
                            && last_xray_poll.elapsed() >= Duration::from_millis(250)
                        {
                            last_xray_poll = Instant::now();
                            match refresh_emission_monitored(xray, progress, cancel) {
                                Ok(health) => {
                                    if let Some(warning) = health.telemetry_warning.clone() {
                                        push_message(progress, "WARN", "xray", warning);
                                    }
                                    set_xray_health(progress, health);
                                }
                                Err(error) => {
                                    abort_reason = Some(xray_operation_error(
                                        error,
                                        "Moxtek emission monitor failed",
                                    ));
                                    cancel.store(true, Ordering::SeqCst);
                                    set_xray_health(progress, xray.health());
                                    let _ = warning_after_off(nano, xray);
                                    set_beam(progress, xray.health().beam_on);
                                    let _ = nano.stop();
                                }
                            }
                        }
                        if abort_reason.is_none() && cancel.load(Ordering::SeqCst) {
                            abort_reason = Some(CANCELLED.into());
                            cancel.store(true, Ordering::SeqCst);
                            let _ = xray.force_off();
                            set_xray_health(progress, xray.health());
                            let _ = warning_after_off(nano, xray);
                            set_beam(progress, xray.health().beam_on);
                            let _ = nano.stop();
                        }
                        if abort_reason.is_none() {
                            let exposure_complete = evidence.exposure_ended;
                            let limit_reached = beam_started
                                .is_some_and(|started| started.elapsed() >= block_limit);
                            if limit_reached && !exposure_complete {
                                abort_reason = Some(format!(
                                    "continuous X-ray limit of {} s reached before exposure completion",
                                    config.max_xray_sec
                                ));
                                cancel.store(true, Ordering::SeqCst);
                                let _ = xray.force_off();
                                set_xray_health(progress, xray.health());
                                set_beam(progress, xray.health().beam_on);
                                let _ = warning_after_off(nano, xray);
                                let _ = nano.stop();
                            }
                            let pause_requested = pause.load(Ordering::SeqCst);
                            if abort_reason.is_none()
                                && exposure_complete
                                && beam_started.is_some()
                                && (limit_reached || final_projection || pause_requested)
                            {
                                let reason = if limit_reached {
                                    "Continuous X-ray limit reached after current exposure"
                                } else if final_projection {
                                    "Final projection exposure complete"
                                } else {
                                    "Pause requested after current exposure"
                                };
                                match close_scan_beam(
                                    nano,
                                    xray,
                                    progress,
                                    &mut beam_started,
                                    reason,
                                ) {
                                    Ok(_) => {
                                        if limit_reached && !final_projection {
                                            cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                                            update(progress, |value| {
                                                value.cooldown_until = cooldown_until
                                            });
                                        }
                                    }
                                    Err(error) => {
                                        abort_reason = Some(error);
                                        cancel.store(true, Ordering::SeqCst);
                                        let _ = nano.stop();
                                    }
                                }
                            }
                            if evidence.transfer_started {
                                if !transfer_announced {
                                    push_message(
                                        progress,
                                        "PASS",
                                        "camera",
                                        format!(
                                            "NEF transfer started · view={} · file confirmation pending",
                                            frame_index
                                        ),
                                    );
                                    transfer_announced = true;
                                }
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        break Err(CameraError::Io(
                            "camera capture worker terminated without a result".into(),
                        ));
                    }
                }
            }
        });
        if let Some(reason) = abort_reason {
            return Err(reason);
        }
        check_cancel(cancel)?;
        let image = capture_result.map_err(|error| format!("D7100 capture failed: {error}"))?;
        if image.session_id != capture_session
            || image.operation_id != capture_operation
            || !image.exposure_ended
            || !evidence.exposure_ended
        {
            return Err(
                "Capture completed without this operation's reliable ExposureEnded event".into(),
            );
        }
        let path = image.path;
        let limit_reached = beam_started.is_some_and(|started| started.elapsed() >= block_limit);
        if beam_started.is_some()
            && (limit_reached || final_projection || pause.load(Ordering::SeqCst))
        {
            let reason = if limit_reached {
                "Continuous X-ray limit reached after completed exposure"
            } else if final_projection {
                "Final projection capture complete"
            } else {
                "Pause requested after completed exposure"
            };
            close_scan_beam(nano, xray, progress, &mut beam_started, reason)?;
            if limit_reached && !final_projection {
                cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                update(progress, |value| value.cooldown_until = cooldown_until);
            }
        }
        let frame = commit_projection_monitored(
            nano,
            xray,
            progress,
            &root,
            parameters,
            config,
            cancel,
            &path,
            job,
            &ticket,
            &mut beam_started,
            block_limit,
            &mut cooldown_until,
            final_projection,
        )?;
        update(progress, |value| {
            record_projection_sample(value, frame.index, Instant::now())
        });
        push_message(
            progress,
            "PASS",
            "nano",
            format!(
                "CAPTURE_DONE view={} id={} · committed file confirmed",
                job.index, ticket.command_id
            ),
        );
        push_message(
            progress,
            "PASS",
            "camera",
            format!(
                "Projection committed · view={} · {} bytes · SHA-256 {}",
                frame.index, frame.bytes, frame.sha256
            ),
        );
    }

    if beam_started.is_some() {
        close_scan_beam(nano, xray, progress, &mut beam_started, "Scan completed")?;
    }

    update_phase(progress, "finishing");
    check_cancel(cancel)?;
    update(progress, |value| value.stage = "finalizing");
    Ok(())
}

fn xray_operation_error(error: crate::devices::xray::XrayError, context: &str) -> String {
    if matches!(error, crate::devices::xray::XrayError::Cancelled) {
        CANCELLED.into()
    } else {
        format!("{context}: {error}")
    }
}
fn warning_after_off(
    nano: &NanoAdapter,
    xray: &MoxtekAdapter,
) -> Result<(), crate::devices::turntable::NanoError> {
    let health = xray.health();
    if health.beam_off_confirmed && !health.beam_on {
        nano.set_xray_warning(false)
    } else {
        let _ = nano.set_xray_warning(true);
        Err(crate::devices::turntable::NanoError::Safety(
            "Radiation warning retained while output is unknown".into(),
        ))
    }
}

fn check_cancel(cancel: &Arc<AtomicBool>) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn wait_for_cooldown(
    progress: &Mutex<RealScanProgress>,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<(), String> {
    let wait_started = Instant::now();
    update(progress, |value| {
        value.phase = "cooling";
        value.cooldown_until = Some(deadline);
        value.cooldown_wait_started = Some(wait_started);
    });
    let remaining_duration = deadline.saturating_duration_since(wait_started);
    let remaining = remaining_duration.as_secs() + u64::from(remaining_duration.subsec_nanos() > 0);
    push_message(
        progress,
        "WARN",
        "xray",
        format!(
        "X-ray OFF interval · {remaining} seconds remain of the 300-second interval before resuming"
    ),
    );
    while Instant::now() < deadline {
        check_cancel(cancel)?;
        thread::sleep(Duration::from_millis(250));
    }

    let waited = wait_started.elapsed();
    update(progress, |value| {
        if value.projection_started.is_some() {
            value.projection_cooling_elapsed += waited;
        }
        value.cooldown_wait_started = None;
        value.cooldown_until = None;
        value.phase = "running";
    });
    push_message(
        progress,
        "PASS",
        "xray",
        "Five-minute X-ray cooldown complete · scan resuming".into(),
    );
    Ok(())
}

fn wait_for_prompt(
    nano: &NanoAdapter,
    xray: &mut MoxtekAdapter,
    progress: &Mutex<RealScanProgress>,
    gate: &Mutex<PromptGate>,
    cancel: &Arc<AtomicBool>,
    stage: &'static str,
) -> Result<(), String> {
    let off = xray
        .force_off()
        .map_err(|error| format!("{stage} prompt requires confirmed X-ray OFF: {error}"))?;
    warning_after_off(nano, xray)
        .map_err(|error| format!("{stage} prompt warning OFF failed: {error}"))?;
    set_xray_health(progress, off);
    set_beam(progress, false);
    {
        let mut state = gate.lock().expect("scan prompt mutex poisoned");
        state.pending = Some(stage);
        state.confirmed = false;
    }
    update(progress, |state| state.prompt = Some(stage));
    let mut last_poll = Instant::now();
    loop {
        check_cancel(cancel)?;
        if last_poll.elapsed() >= Duration::from_millis(250) {
            last_poll = Instant::now();
            let health = xray
                .refresh_status()
                .map_err(|error| format!("{stage} prompt X-ray status unavailable: {error}"))?;
            let beam_on = health.beam_on;
            let off_confirmed = health.beam_off_confirmed;
            set_xray_health(progress, health);
            set_beam(progress, beam_on);
            if beam_on || !off_confirmed {
                return Err(format!(
                    "{stage} prompt interrupted: X-ray OFF is unconfirmed"
                ));
            }
        }
        {
            let mut state = gate.lock().expect("scan prompt mutex poisoned");
            if state.confirmed {
                state.pending = None;
                state.confirmed = false;
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    update(progress, |state| state.prompt = None);
    check_cancel(cancel)?;
    let off = xray
        .force_off()
        .map_err(|error| format!("{stage} confirmation requires confirmed X-ray OFF: {error}"))?;
    warning_after_off(nano, xray)
        .map_err(|error| format!("{stage} confirmation warning OFF failed: {error}"))?;
    set_xray_health(progress, off);
    set_beam(progress, false);
    Ok(())
}

fn reference_list<'a>(references: &'a ReferenceFrames, group: &str) -> &'a Vec<ReferenceFrame> {
    match group {
        "pre-dark" => &references.pre_dark,
        "pre-flat" => &references.pre_flat,
        "post-flat" => &references.post_flat,
        "post-dark" => &references.post_dark,
        _ => unreachable!("reference group is fixed by the scan coordinator"),
    }
}

fn reference_list_mut<'a>(
    references: &'a mut ReferenceFrames,
    group: &str,
) -> &'a mut Vec<ReferenceFrame> {
    match group {
        "pre-dark" => &mut references.pre_dark,
        "pre-flat" => &mut references.pre_flat,
        "post-flat" => &mut references.post_flat,
        "post-dark" => &mut references.post_dark,
        _ => unreachable!("reference group is fixed by the scan coordinator"),
    }
}

fn capture_reference_group(
    nano: &NanoAdapter,
    camera: &mut CameraAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    progress: &Mutex<RealScanProgress>,
    root: &Path,
    group: &'static str,
    stage: &'static str,
    flat: bool,
) -> Result<(), String> {
    let starting_at = {
        let state = progress.lock().expect("scan progress mutex poisoned");
        reference_list(&state.references, group).len()
    };
    if starting_at == REFERENCE_COUNT {
        return Ok(());
    }
    update(progress, |state| state.stage = stage);
    persist_progress(root, &config.parameters, config, progress, stage, false)?;
    // Every dark image is taken only after verified OFF; no warning is asserted.
    if !flat {
        xray.force_off()
            .map_err(|error| format!("{group} beam OFF failed: {error}"))?;
        warning_after_off(nano, xray)
            .map_err(|error| format!("{group} warning OFF failed: {error}"))?;
        set_xray_health(progress, xray.health());
        set_beam(progress, false);
    }
    let save_path = root.join("references");
    let block_limit = Duration::from_secs(u64::from(config.max_xray_sec));
    let exposure_timeout = Duration::from_secs_f64(config.parameters.exposure_ms / 1000.0)
        .saturating_add(CAMERA_TRIGGER_MARGIN);
    let flat_started_at = if flat {
        check_cancel(cancel)?;
        nano.set_xray_warning(true)
            .map_err(|error| format!("{group} warning ON failed: {error}"))?;
        let started = Instant::now();
        xray.beam_on_shared(cancel.clone(), started + block_limit)
            .map_err(|error| format!("{group} beam ON failed: {error}"))?;
        set_xray_health(progress, xray.health());
        set_beam(progress, true);
        push_message(
            progress,
            "ACTION",
            "xray",
            format!(
                "{group} continuous beam confirmed ON · remaining references {}",
                REFERENCE_COUNT - starting_at
            ),
        );
        Some(started)
    } else {
        None
    };
    for index in starting_at as u32 + 1..=REFERENCE_COUNT as u32 {
        check_cancel(cancel)?;
        if !flat {
            let health = xray
                .refresh_status()
                .map_err(|error| format!("{group} dark capture X-ray status failed: {error}"))?;
            if !health.beam_off_confirmed || health.beam_on {
                return Err(format!("{group} dark capture requires confirmed X-ray OFF"));
            }
            set_xray_health(progress, health);
        } else {
            let started = flat_started_at.expect("flat group has one continuous beam block");
            if started
                .elapsed()
                .saturating_add(exposure_timeout)
                .saturating_add(EXPOSURE_BUDGET_MARGIN)
                >= block_limit
            {
                return Err(format!("{group} cannot start reference {index}: continuous X-ray limit would be exceeded"));
            }
            let health = refresh_emission_monitored(xray, progress, cancel)
                .map_err(|error| {
                    xray_operation_error(
                        error,
                        &format!("{group} emission check before reference {index} failed"),
                    )
                })?;
            set_xray_health(progress, health);
        }
        let mut abort = None;
        let mut beam_shutdown_attempted = false;
        let capture_result = thread::scope(|scope| {
            let (sender, receiver) = mpsc::channel();
            let save_path_for_capture = save_path.clone();
            let camera_for_capture = &mut *camera;
            scope.spawn(move || {
                let result = camera_for_capture.capture_frame_cancellable(
                    &save_path_for_capture,
                    group,
                    index,
                    cancel,
                );
                let _ = sender.send(result);
            });
            let mut last_poll = Instant::now();
            loop {
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(result) => break result,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        break Err(CameraError::Io(
                            "reference capture worker disconnected".into(),
                        ))
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if cancel.load(Ordering::SeqCst) && abort.is_none() {
                    abort = Some(CANCELLED.to_owned());
                }
                if !flat && last_poll.elapsed() >= Duration::from_millis(250) && abort.is_none() {
                    last_poll = Instant::now();
                    match xray.refresh_status() {
                        Ok(health) if health.beam_off_confirmed && !health.beam_on => {
                            set_xray_health(progress, health);
                        }
                        Ok(health) => {
                            set_xray_health(progress, health);
                            abort = Some(format!("{group} dark capture lost confirmed X-ray OFF"));
                        }
                        Err(error) => {
                            abort =
                                Some(format!("{group} dark capture X-ray status failed: {error}"));
                        }
                    }
                    if abort.is_some() {
                        cancel.store(true, Ordering::SeqCst);
                        let _ = xray.force_off();
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        let _ = nano.stop();
                    }
                }
                if flat {
                    if last_poll.elapsed() >= Duration::from_millis(250) && abort.is_none() {
                        last_poll = Instant::now();
                        if let Err(error) = refresh_emission_monitored(xray, progress, cancel) {
                            abort = Some(xray_operation_error(
                                error,
                                &format!("{group} emission monitor failed"),
                            ));
                            cancel.store(true, Ordering::SeqCst);
                        } else {
                            set_xray_health(progress, xray.health());
                        }
                    }
                    if abort.is_none()
                        && flat_started_at.is_some_and(|started| started.elapsed() >= block_limit)
                    {
                        abort = Some(format!(
                            "{group} continuous X-ray limit reached during reference {index}"
                        ));
                        cancel.store(true, Ordering::SeqCst);
                    }
                    if abort.is_some() && !beam_shutdown_attempted {
                        beam_shutdown_attempted = true;
                        let off = xray.force_off();
                        let warning_off = warning_after_off(nano, xray);
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        if let Err(error) = off {
                            abort = Some(format!("{group} beam OFF failed: {error}"));
                            cancel.store(true, Ordering::SeqCst);
                        }
                        if let Err(error) = warning_off {
                            abort = Some(format!("{group} warning OFF failed: {error}"));
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                }
                if abort.is_some() && !flat {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        });
        if let Some(error) = abort {
            return Err(error);
        }
        check_cancel(cancel)?;
        let path =
            capture_result.map_err(|error| format!("{group} capture {index} failed: {error}"))?;
        if flat {
            if flat_started_at.is_some_and(|started| started.elapsed() >= block_limit) {
                return Err(format!(
                    "{group} continuous X-ray limit reached during reference {index}"
                ));
            }
            if index == REFERENCE_COUNT as u32 {
                let off = xray
                    .force_off()
                    .map_err(|error| format!("{group} beam OFF failed: {error}"))?;
                set_xray_health(progress, off);
                warning_after_off(nano, xray)
                    .map_err(|error| format!("{group} warning OFF failed: {error}"))?;
                set_beam(progress, false);
                push_message(
                    progress,
                    "PASS",
                    "xray",
                    format!(
                        "{group} continuous beam OFF confirmed after {REFERENCE_COUNT} references"
                    ),
                );
            }
        }
        let (bytes, sha256) = hash_file(&path)?;
        let frame = ReferenceFrame {
            index,
            exposure_ms: config.parameters.exposure_ms,
            file_name: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned(),
            path: path.display().to_string(),
            bytes,
            sha256,
        };
        update(progress, |state| {
            reference_list_mut(&mut state.references, group).push(frame)
        });
        persist_progress(root, &config.parameters, config, progress, stage, false)?;
        push_message(
            progress,
            "PASS",
            "camera",
            format!("{group} reference {index}/{REFERENCE_COUNT} saved"),
        );
    }
    Ok(())
}

/// One durable receipt line for a card record. It closes exactly one intent of the
/// same operation, which is what makes the intent-without-receipt window
/// detectable instead of invisible.
fn card_receipt_journal_entry(
    record: &CardCaptureRecord,
    group: &str,
    operation_id: &str,
    session_id: &str,
    recorded_at: String,
) -> CardJournalEntry {
    CardJournalEntry {
        kind: CARD_JOURNAL_KIND_RECEIPT.into(),
        index: record.index,
        group: group.into(),
        angle_deg: record.angle_deg,
        exposure_ms: record.exposure_ms,
        operation_id: operation_id.into(),
        session_id: session_id.into(),
        planned_pulse_position: None,
        handle: Some(record.handle),
        storage_id: Some(record.storage_id),
        source_file_name: Some(record.source_file_name.clone()),
        recorded_at,
    }
}

fn card_group_kind(group: &str) -> &'static str {
    if group == "pre-dark" {
        "preDark"
    } else {
        "preFlat"
    }
}

/// One card reference group (10 pre-dark or 10 pre-flat exposures) with the same
/// beam, interlock, cancellation and continuous-limit discipline as the host-RAM
/// group. The difference is the transaction itself: each reference is one card
/// exposure whose intent is persisted before the trigger and whose receipt is
/// persisted after it. Nothing is downloaded, hashed or written locally, and the
/// card objects are never deleted or re-shot.
#[allow(clippy::too_many_arguments)]
fn capture_card_reference_group(
    nano: &NanoAdapter,
    camera: &CameraAdapter,
    xray: &mut MoxtekAdapter,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    progress: &Mutex<RealScanProgress>,
    root: &Path,
    plan: &CardCapturePlan,
    group: &'static str,
    stage: &'static str,
    flat: bool,
) -> Result<(), String> {
    let kind = card_group_kind(group);
    let starting_at = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .card_records
        .iter()
        .filter(|record| record.kind == kind)
        .count();
    if starting_at >= REFERENCE_COUNT {
        return Ok(());
    }
    update(progress, |state| state.stage = stage);
    persist_progress(root, &config.parameters, config, progress, stage, false)?;
    // Every dark image is taken only after verified OFF; no warning is asserted.
    if !flat {
        xray.force_off()
            .map_err(|error| format!("{group} beam OFF failed: {error}"))?;
        warning_after_off(nano, xray)
            .map_err(|error| format!("{group} warning OFF failed: {error}"))?;
        set_xray_health(progress, xray.health());
        set_beam(progress, false);
    }
    let block_limit = Duration::from_secs(u64::from(config.max_xray_sec));
    let exposure_timeout = Duration::from_secs_f64(config.parameters.exposure_ms / 1000.0)
        .saturating_add(CAMERA_TRIGGER_MARGIN);
    let flat_started_at = if flat {
        check_cancel(cancel)?;
        nano.set_xray_warning(true)
            .map_err(|error| format!("{group} warning ON failed: {error}"))?;
        let started = Instant::now();
        xray.beam_on_shared(cancel.clone(), started + block_limit)
            .map_err(|error| format!("{group} beam ON failed: {error}"))?;
        set_xray_health(progress, xray.health());
        set_beam(progress, true);
        push_message(
            progress,
            "ACTION",
            "xray",
            format!(
                "{group} continuous beam confirmed ON · remaining card references {}",
                REFERENCE_COUNT - starting_at
            ),
        );
        Some(started)
    } else {
        None
    };
    for index in starting_at as u32 + 1..=REFERENCE_COUNT as u32 {
        check_cancel(cancel)?;
        if !flat {
            let health = xray
                .refresh_status()
                .map_err(|error| format!("{group} dark capture X-ray status failed: {error}"))?;
            if !health.beam_off_confirmed || health.beam_on {
                return Err(format!("{group} dark capture requires confirmed X-ray OFF"));
            }
            set_xray_health(progress, health);
        } else {
            let started = flat_started_at.expect("flat group has one continuous beam block");
            if started
                .elapsed()
                .saturating_add(exposure_timeout)
                .saturating_add(EXPOSURE_BUDGET_MARGIN)
                >= block_limit
            {
                return Err(format!(
                    "{group} cannot start card reference {index}: continuous X-ray limit would be exceeded"
                ));
            }
            let health = refresh_emission_monitored(xray, progress, cancel).map_err(|error| {
                xray_operation_error(
                    error,
                    &format!("{group} emission check before card reference {index} failed"),
                )
            })?;
            set_xray_health(progress, health);
        }
        let session = camera
            .health()
            .session_id
            .ok_or("Capture session is unknown")?;
        let operation_id = format!(
            "scan-{}-{}-{}-{}",
            config.parameters.task_id,
            kind,
            index,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        append_card_journal(
            root,
            &CardJournalEntry {
                kind: CARD_JOURNAL_KIND_INTENT.into(),
                index,
                group: kind.into(),
                angle_deg: 0.0,
                exposure_ms: config.parameters.exposure_ms,
                operation_id: operation_id.clone(),
                session_id: session.clone(),
                planned_pulse_position: None,
                handle: None,
                storage_id: None,
                source_file_name: None,
                recorded_at: timestamp(),
            },
        )
        .map_err(|error| {
            format!(
                "{group} card reference {index} intent could not be persisted; the exposure was not triggered: {error}"
            )
        })?;
        let mut abort: Option<String> = None;
        let mut beam_shutdown_attempted = false;
        let plan_for_worker = plan.clone();
        let worker_operation = operation_id.clone();
        let camera_for_capture = camera.clone();
        let capture_result = thread::scope(|scope| {
            let (sender, receiver) = mpsc::channel();
            scope.spawn(move || {
                let result = camera_for_capture.capture_to_card(
                    plan_for_worker,
                    &worker_operation,
                    cancel,
                    None,
                );
                let _ = sender.send(result);
            });
            let mut last_poll = Instant::now();
            loop {
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(result) => break result,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        break Err(CameraError::Io(
                            "card reference capture worker disconnected".into(),
                        ))
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if cancel.load(Ordering::SeqCst) && abort.is_none() {
                    abort = Some(CANCELLED.to_owned());
                }
                if !flat && last_poll.elapsed() >= Duration::from_millis(250) && abort.is_none() {
                    last_poll = Instant::now();
                    match xray.refresh_status() {
                        Ok(health) if health.beam_off_confirmed && !health.beam_on => {
                            set_xray_health(progress, health);
                        }
                        Ok(health) => {
                            set_xray_health(progress, health);
                            abort = Some(format!("{group} dark capture lost confirmed X-ray OFF"));
                        }
                        Err(error) => {
                            abort =
                                Some(format!("{group} dark capture X-ray status failed: {error}"));
                        }
                    }
                    if abort.is_some() {
                        cancel.store(true, Ordering::SeqCst);
                        let _ = xray.force_off();
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        let _ = nano.stop();
                    }
                }
                if flat {
                    if last_poll.elapsed() >= Duration::from_millis(250) && abort.is_none() {
                        last_poll = Instant::now();
                        if let Err(error) = refresh_emission_monitored(xray, progress, cancel) {
                            abort = Some(xray_operation_error(
                                error,
                                &format!("{group} emission monitor failed"),
                            ));
                            cancel.store(true, Ordering::SeqCst);
                        } else {
                            set_xray_health(progress, xray.health());
                        }
                    }
                    if abort.is_none()
                        && flat_started_at.is_some_and(|started| started.elapsed() >= block_limit)
                    {
                        abort = Some(format!(
                            "{group} continuous X-ray limit reached during card reference {index}"
                        ));
                        cancel.store(true, Ordering::SeqCst);
                    }
                    if abort.is_some() && !beam_shutdown_attempted {
                        beam_shutdown_attempted = true;
                        let off = xray.force_off();
                        let warning_off = warning_after_off(nano, xray);
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        if let Err(error) = off {
                            abort = Some(format!("{group} beam OFF failed: {error}"));
                            cancel.store(true, Ordering::SeqCst);
                        }
                        if let Err(error) = warning_off {
                            abort = Some(format!("{group} warning OFF failed: {error}"));
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        });
        if let Some(error) = abort {
            return Err(error);
        }
        check_cancel(cancel)?;
        let outcome = capture_result
            .map_err(|error| format!("{group} card capture {index} failed: {error}"))?;
        if outcome.receipt.session_id != session
            || outcome.receipt.operation_id != operation_id
            || !outcome.exposure_ended
        {
            return Err(format!(
                "{group} card reference {index} lacks its own session, operation, exposure-end and object evidence"
            ));
        }
        if flat && index == REFERENCE_COUNT as u32 {
            let off = xray
                .force_off()
                .map_err(|error| format!("{group} beam OFF failed: {error}"))?;
            set_xray_health(progress, off);
            warning_after_off(nano, xray)
                .map_err(|error| format!("{group} warning OFF failed: {error}"))?;
            set_beam(progress, false);
            push_message(
                progress,
                "PASS",
                "xray",
                format!(
                    "{group} continuous beam OFF confirmed after {REFERENCE_COUNT} card references"
                ),
            );
        }
        let record = CardCaptureRecord::from_receipt(
            index,
            kind,
            0.0,
            config.parameters.exposure_ms,
            &outcome.receipt,
        );
        update(progress, |state| {
            state.card_records.push(record.clone());
            state.card_data_status = Some(DATA_PENDING_IMPORT);
        });
        persist_progress(root, &config.parameters, config, progress, stage, false)?;
        append_card_journal(
            root,
            &card_receipt_journal_entry(
                &record,
                kind,
                &operation_id,
                &session,
                timestamp(),
            ),
        )
        .map_err(|error| format!("{group} card receipt {index} could not be persisted: {error}"))?;
        push_message(
            progress,
            "PASS",
            "camera",
            format!(
                "{group} card reference {index}/{REFERENCE_COUNT} confirmed on storage 0x{:08X} · {} · no local file",
                record.storage_id, record.source_file_name
            ),
        );
    }
    Ok(())
}

fn record_projection_sample(value: &mut RealScanProgress, index: u32, now: Instant) {
    if let Some(started) = value.projection_started.take() {
        let active = now
            .saturating_duration_since(started)
            .saturating_sub(value.projection_cooling_elapsed);
        value.measured_projection_time = Some(match value.measured_projection_time {
            Some(previous) => (previous.saturating_mul(index - 1) + active) / index,
            None => active,
        });
    }
    value.projection_cooling_elapsed = Duration::ZERO;
}

fn close_scan_beam(
    nano: &NanoAdapter,
    xray: &mut MoxtekAdapter,
    progress: &Mutex<RealScanProgress>,
    beam_started: &mut Option<Instant>,
    reason: &str,
) -> Result<Duration, String> {
    let elapsed = beam_started
        .take()
        .map(|started| started.elapsed())
        .unwrap_or_default();
    let health = xray
        .force_off()
        .map_err(|error| format!("Moxtek OFF failed at {reason}: {error}"))?;
    set_xray_health(progress, health);
    warning_after_off(nano, xray)
        .map_err(|error| format!("XRAY_WARNING OFF failed at {reason}: {error}"))?;
    set_beam(progress, false);
    update(progress, |value| value.beam_started = None);
    push_message(
        progress,
        "PASS",
        "xray",
        format!(
            "{reason} · beam OFF after {:.3} s continuous output",
            elapsed.as_secs_f64()
        ),
    );
    Ok(elapsed)
}

fn projection_job(zero_index: u32, projection_count: u32) -> ProjectionJob {
    let angle_mdeg = -(((i64::from(zero_index) * 360_000) + i64::from(projection_count / 2))
        / i64::from(projection_count)) as i32;
    ProjectionJob {
        index: zero_index + 1,
        angle_mdeg,
        angle_deg: f64::from(angle_mdeg) / 1_000.0,
    }
}

fn expected_pulses(angle_mdeg: i32, pulses_per_rev: u32) -> i64 {
    let magnitude = i64::from(angle_mdeg).unsigned_abs() % 360_000;
    ((u64::from(pulses_per_rev) * magnitude + 180_000) / 360_000) as i64
        * if angle_mdeg < 0 { -1 } else { 1 }
}

fn verify_capture_hold(
    nano: &NanoAdapter,
    ticket: &MoveTicket,
    job: ProjectionJob,
) -> Result<f64, String> {
    let status = nano
        .status()
        .map_err(|error| format!("Nano STATUS failed before view {}: {error}", job.index))?;
    verify_fixed_configuration(&status).map_err(|error| error.to_string())?;
    let expected = expected_pulses(job.angle_mdeg, status.pulses_per_rev);
    if status.state != "CAPTURE_HOLD"
        || status.capture_id != ticket.command_id
        || ticket.position_pulses != status.position_pulses
        || status.target_pulses != status.position_pulses
        || status
            .position_pulses
            .rem_euclid(i64::from(status.pulses_per_rev))
            != expected.rem_euclid(i64::from(status.pulses_per_rev))
        || !status.reference_valid
        || !status.homed
        || !status.rearmed
    {
        return Err(format!(
            "view {} threshold check failed: state={} capture_id={} ticket={} status={} expected={} reference={} homed={} rearmed={}",
            job.index,
            status.state,
            status.capture_id,
            ticket.position_pulses,
            status.position_pulses,
            expected,
            status.reference_valid,
            status.homed,
            status.rearmed
        ));
    }
    Ok(status.position_pulses as f64 * 360.0 / f64::from(status.pulses_per_rev))
}

fn move_to_projection(
    nano: &NanoAdapter,
    progress: &Mutex<RealScanProgress>,
    job: ProjectionJob,
    cancel: &Arc<AtomicBool>,
) -> Result<MoveTicket, String> {
    check_cancel(cancel)?;
    push_message(
        progress,
        "ACTION",
        "nano",
        format!(
            "Prescribed-direction MOVE_REL view={} nominal angle={:.3}°",
            job.index, job.angle_deg
        ),
    );
    let starting_status = nano
        .status()
        .map_err(|error| format!("STATUS before view {} failed: {error}", job.index))?;
    let plan = plan_projection(&starting_status, job.angle_mdeg)
        .map_err(|error| format!("Plan view {} failed: {error}", job.index))?;
    let movement = nano.execute_plan_cancelled(&plan, cancel.clone());
    check_cancel(cancel)?;
    let ticket =
        movement.map_err(|error| format!("MOVE_REL view {} failed: {error}", job.index))?;
    let status = nano.status().map_err(|error| error.to_string())?;
    verify_plan_capture_hold(&plan, &ticket, &status).map_err(|error| error.to_string())?;
    let confirmed_angle = verify_capture_hold(nano, &ticket, job)?;
    // Only confirmed device position may advance the live scene. The planned
    // target is not a measurement, including while the move is still pending.
    set_angle(progress, confirmed_angle);
    push_message(
        progress,
        "PASS",
        "nano",
        format!(
            "READY_TO_CAPTURE view={} id={} pos={}",
            job.index, ticket.command_id, ticket.position_pulses
        ),
    );
    Ok(ticket)
}

fn move_to_projection_monitored(
    nano: &NanoAdapter,
    xray: &mut MoxtekAdapter,
    progress: &Mutex<RealScanProgress>,
    job: ProjectionJob,
    cancel: &Arc<AtomicBool>,
    beam_started: &mut Option<Instant>,
    block_limit: Duration,
    cooldown_until: &mut Option<Instant>,
) -> Result<MoveTicket, String> {
    if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
        close_scan_beam(
            nano,
            xray,
            progress,
            beam_started,
            "Continuous X-ray limit reached before turntable movement",
        )?;
        let deadline = Instant::now() + XRAY_COOLDOWN;
        wait_for_cooldown(progress, cancel, deadline)?;
    }
    if beam_started.is_none() {
        return move_to_projection(nano, progress, job, cancel);
    }
    // Nano MOVE_ABS may block for minutes. Keep the Moxtek watchdog alive and
    // enforce the continuous-output deadline while Nano owns its serial worker.
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        scope.spawn(move || {
            let _ = sender.send(move_to_projection(nano, progress, job, cancel));
        });
        let mut last_poll = Instant::now();
        let mut abort: Option<String> = None;
        let mut warning_off_after_move = false;
        loop {
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => {
                    if warning_off_after_move {
                        warning_after_off(nano, xray).map_err(|error| {
                            format!("XRAY_WARNING OFF failed after timed move: {error}")
                        })?;
                    }
                    if abort.is_none()
                        && beam_started.is_some_and(|started| started.elapsed() >= block_limit)
                    {
                        close_scan_beam(
                            nano,
                            xray,
                            progress,
                            beam_started,
                            "Continuous X-ray limit reached at turntable arrival",
                        )?;
                        *cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                        update(progress, |value| value.cooldown_until = *cooldown_until);
                    }
                    return abort.map_or(result, Err);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Nano move worker terminated without a result".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if abort.is_some() {
                continue;
            }
            if cancel.load(Ordering::SeqCst) {
                abort = Some(CANCELLED.into());
                let _ = xray.force_off();
                set_xray_health(progress, xray.health());
                set_beam(progress, xray.health().beam_on);
                let _ = nano.stop();
                continue;
            }
            if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
                // Moxtek is independent of Nano's serial worker. Shut the beam
                // immediately; queue warning OFF only once MOVE_ABS returns.
                match xray.force_off() {
                    Ok(health) => {
                        set_xray_health(progress, health);
                        set_beam(progress, false);
                        update(progress, |value| value.beam_started = None);
                        *beam_started = None;
                        warning_off_after_move = true;
                        *cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                        update(progress, |value| value.cooldown_until = *cooldown_until);
                        push_message(
                            progress,
                            "WARN",
                            "xray",
                            "Continuous X-ray limit reached during movement · beam OFF confirmed"
                                .into(),
                        );
                    }
                    Err(error) => {
                        abort = Some(format!("Moxtek OFF failed during move: {error}"));
                        let _ = nano.stop();
                    }
                }
            } else if beam_started.is_some() && last_poll.elapsed() >= Duration::from_millis(250) {
                last_poll = Instant::now();
                match refresh_emission_monitored(xray, progress, cancel) {
                    Ok(health) => set_xray_health(progress, health),
                    Err(error) => {
                        abort = Some(xray_operation_error(
                            error,
                            "Moxtek emission monitor failed during move",
                        ));
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        let _ = nano.stop();
                    }
                }
            }
        }
    })
}

fn commit_projection_monitored(
    nano: &NanoAdapter,
    xray: &mut MoxtekAdapter,
    progress: &Mutex<RealScanProgress>,
    root: &Path,
    parameters: &Parameters,
    config: &RealScanConfig,
    cancel: &Arc<AtomicBool>,
    path: &Path,
    job: ProjectionJob,
    ticket: &MoveTicket,
    beam_started: &mut Option<Instant>,
    block_limit: Duration,
    cooldown_until: &mut Option<Instant>,
    final_projection: bool,
) -> Result<RealFrame, String> {
    let mut frames = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .frames
        .clone();
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        scope.spawn(move || {
            let result = (|| {
                let (bytes, sha256) = hash_file(path)?;
                let frame = RealFrame {
                    index: job.index,
                    angle_deg: job.angle_deg,
                    exposure_ms: parameters.exposure_ms,
                    file_name: path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("frame.nef")
                        .to_owned(),
                    path: path.display().to_string(),
                    bytes,
                    sha256,
                };
                check_cancel(cancel)?;
                nano.capture_done_cancelled(ticket.command_id, cancel.clone())
                    .map_err(|error| format!("CAPTURE_DONE view {} failed: {error}", job.index))?;
                check_cancel(cancel)?;
                frames.push(frame.clone());
                let references = progress
                    .lock()
                    .expect("scan progress mutex poisoned")
                    .references
                    .clone();
                persist(
                    root,
                    parameters,
                    config,
                    &frames,
                    &references,
                    "projections",
                    false,
                )?;
                push_frame(progress, frame.clone());
                Ok(frame)
            })();
            let _ = sender.send(result);
        });
        let mut last_poll = Instant::now();
        let mut abort: Option<String> = None;
        let mut warning_off_after_commit = false;
        loop {
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => {
                    if warning_off_after_commit {
                        warning_after_off(nano, xray).map_err(|error| {
                            format!("XRAY_WARNING OFF failed after frame commit: {error}")
                        })?;
                    }
                    return abort.map_or(result, Err);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("frame commit worker terminated without a result".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if abort.is_some() || beam_started.is_none() {
                continue;
            }
            if cancel.load(Ordering::SeqCst) {
                abort = Some(CANCELLED.into());
                let _ = xray.force_off();
                set_xray_health(progress, xray.health());
                set_beam(progress, xray.health().beam_on);
                let _ = nano.stop();
            } else if beam_started.is_some_and(|started| started.elapsed() >= block_limit) {
                match xray.force_off() {
                    Ok(health) => {
                        set_xray_health(progress, health);
                        set_beam(progress, false);
                        update(progress, |value| value.beam_started = None);
                        *beam_started = None;
                        warning_off_after_commit = true;
                        if !final_projection {
                            *cooldown_until = Some(Instant::now() + XRAY_COOLDOWN);
                            update(progress, |value| value.cooldown_until = *cooldown_until);
                        }
                    }
                    Err(error) => {
                        abort = Some(format!("Moxtek OFF failed during frame commit: {error}"));
                        let _ = nano.stop();
                    }
                }
            } else if last_poll.elapsed() >= Duration::from_millis(250) {
                last_poll = Instant::now();
                match refresh_emission_monitored(xray, progress, cancel) {
                    Ok(health) => set_xray_health(progress, health),
                    Err(error) => {
                        abort = Some(xray_operation_error(
                            error,
                            "Moxtek emission monitor failed during frame commit",
                        ));
                        set_xray_health(progress, xray.health());
                        set_beam(progress, xray.health().beam_on);
                        let _ = nano.stop();
                    }
                }
            }
        }
    })
}

struct ExposureEvidence {
    session_id: String,
    operation_id: String,
    exposure_ended: bool,
    transfer_started: bool,
}
impl ExposureEvidence {
    fn new(session_id: String, operation_id: String) -> Self {
        Self {
            session_id,
            operation_id,
            exposure_ended: false,
            transfer_started: false,
        }
    }
    fn observe(&mut self, event: &CameraCaptureEvent) -> Result<(), String> {
        let (session, operation) = match event {
            CameraCaptureEvent::ExposureEnded {
                session_id,
                operation_id,
            }
            | CameraCaptureEvent::ObjectAdded {
                session_id,
                operation_id,
                ..
            }
            | CameraCaptureEvent::TransferStarted {
                session_id,
                operation_id,
                ..
            } => (session_id, operation_id),
            CameraCaptureEvent::FileComplete { image } => (&image.session_id, &image.operation_id),
        };
        if session != &self.session_id || operation != &self.operation_id {
            return Err("Capture event belongs to a different session or operation".into());
        }
        match event {
            CameraCaptureEvent::ExposureEnded { .. } => {
                if self.exposure_ended {
                    return Err("Duplicate ExposureEnded event".into());
                }
                self.exposure_ended = true;
            }
            CameraCaptureEvent::TransferStarted { .. } => self.transfer_started = true,
            _ => {}
        }
        Ok(())
    }
}

fn needs_cooldown_before_exposure(
    beam_started: Instant,
    now: Instant,
    block_limit: Duration,
    required_budget: Duration,
) -> bool {
    now.saturating_duration_since(beam_started)
        .saturating_add(required_budget)
        >= block_limit
}

fn update(progress: &Mutex<RealScanProgress>, apply: impl FnOnce(&mut RealScanProgress)) {
    if let Ok(mut progress) = progress.lock() {
        apply(&mut progress);
        progress.revision = progress.revision.wrapping_add(1);
    }
}

fn update_phase(progress: &Mutex<RealScanProgress>, phase: &'static str) {
    update(progress, |value| value.phase = phase);
}

fn set_angle(progress: &Mutex<RealScanProgress>, angle_deg: f64) {
    update(progress, |value| value.angle_deg = angle_deg);
}

fn set_beam(progress: &Mutex<RealScanProgress>, beam_on: bool) {
    update(progress, |value| value.beam_on = beam_on);
}

fn log_xray_diagnostics(xray: &MoxtekAdapter, progress: &Mutex<RealScanProgress>) {
    for message in xray.drain_diagnostics() { push_message(progress, "WARN", "xray", message); }
}

fn refresh_emission_monitored(xray: &mut MoxtekAdapter, progress: &Mutex<RealScanProgress>, cancel: &Arc<AtomicBool>) -> Result<XrayHealth, XrayError> {
    let result = xray.refresh_emission_status_shared(cancel.clone());
    // Scan owns the proxy while running. Persist serial diagnostics in scan.log
    // even when the domain's standalone observer has no X-ray adapter.
    log_xray_diagnostics(xray, progress);
    result
}

fn set_xray_health(progress: &Mutex<RealScanProgress>, health: XrayHealth) {
    update(progress, |value| value.xray_health = Some(health));
}

fn push_message(
    progress: &Mutex<RealScanProgress>,
    level: &'static str,
    source: &'static str,
    message: String,
) {
    update(progress, |value| {
        value.messages.push(ScanMessage {
            timestamp: timestamp(),
            level,
            source,
            message,
        });
    });
}

fn persist_scan_log(
    root: &Path,
    progress: &Mutex<RealScanProgress>,
    result: &Result<ScanCompletion, String>,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("scan.log"))
        .map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    let messages = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .messages
        .clone();
    write_scan_log(&mut writer, &messages, result)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)?;
    Ok(())
}

fn write_scan_log(
    writer: &mut impl Write,
    messages: &[ScanMessage],
    result: &Result<ScanCompletion, String>,
) -> Result<(), String> {
    writeln!(writer, "Micro-CT scan log").map_err(io_error)?;
    for event in messages {
        writeln!(
            writer,
            "{}\t{}\t{}\t{}",
            event.timestamp,
            event.level,
            event.source,
            event.message.replace(['\r', '\n'], " ")
        )
        .map_err(io_error)?;
    }
    let outcome = match result {
        Ok(ScanCompletion::Completed) => "COMPLETED".to_owned(),
        Ok(ScanCompletion::AcquiredPendingImport) => "ACQUIRED_PENDING_IMPORT".to_owned(),
        Ok(ScanCompletion::Stopped) => "STOPPED".to_owned(),
        Err(error) => format!("FAULT: {}", error.replace(['\r', '\n'], " ")),
    };
    writeln!(writer, "{}\tRESULT\tsystem\t{outcome}", timestamp()).map_err(io_error)?;
    Ok(())
}

fn push_frame(progress: &Mutex<RealScanProgress>, frame: RealFrame) {
    update(progress, |value| {
        value.captured = frame.index;
        // A prefetched move may already have reached the next view while this
        // older image transfers. Committing a frame must not rewind live position.
        value.frames.push(frame);
    });
}

fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = File::open(path).map_err(io_error)?;
    let bytes = file.metadata().map_err(io_error)?.len();
    if bytes == 0 {
        return Err(format!("captured file is empty: {}", path.display()));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}

fn persist(
    root: &Path,
    parameters: &Parameters,
    config: &RealScanConfig,
    frames: &[RealFrame],
    references: &ReferenceFrames,
    stage: &str,
    completed: bool,
) -> Result<(), String> {
    let manifest = ScanManifest {
        schema_version: 3,
        capture_profile: &config.capture_profile,
        max_xray_sec: config.max_xray_sec,
        scan_id: &parameters.task_id,
        created_at: timestamp(),
        completed,
        voltage_kv: config.voltage_kv,
        current_ua: config.current_ua,
        projection_count: parameters.projection_count,
        exposure_ms: parameters.exposure_ms,
        geometry: &config.geometry,
        references,
        stage,
        frames,
    };
    let staging_path = root.join("manifest.next.json");
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&staging_path)
        .map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &manifest).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)?;
    drop(writer);

    let csv = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(root.join("acquisition.csv"))
        .map_err(io_error)?;
    let mut csv = BufWriter::new(csv);
    csv.write_all(b"index,angle_deg,exposure_ms,file_name,bytes,sha256\n")
        .map_err(io_error)?;
    for frame in frames {
        writeln!(
            csv,
            "{},{:.6},{},{},{},{}",
            frame.index,
            frame.angle_deg,
            frame.exposure_ms,
            frame.file_name,
            frame.bytes,
            frame.sha256
        )
        .map_err(io_error)?;
    }
    csv.flush().map_err(io_error)?;
    csv.get_ref().sync_all().map_err(io_error)?;
    drop(csv);
    replace_manifest(&staging_path, &root.join("manifest.json"))?;
    Ok(())
}

#[cfg(windows)]
fn replace_manifest(staging: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "Kernel32")]
    extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *const std::ffi::c_void,
            reserved: *const std::ffi::c_void,
        ) -> i32;
    }
    if !destination.exists() {
        return fs::rename(staging, destination).map_err(io_error);
    }
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let destination_wide = wide(destination);
    let staging_wide = wide(staging);
    // ReplaceFileW keeps the prior manifest intact if replacement fails.
    let replaced = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            staging_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        Err(io_error(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_manifest(staging: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(staging, destination).map_err(io_error)
}

/// Schema-4 manifest of a card scan. It deliberately mirrors the schema-3 field
/// set so the frozen acquisition stays comparable, and adds the card destination,
/// the two independent acquisition/data states and one receipt per exposure. It
/// carries no local `frames` and no `references`: until the authorized USB import
/// is verified there is no local file identity to claim.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CardScanManifest<'a> {
    schema_version: u32,
    capture_profile: &'a CaptureProfile,
    max_xray_sec: u32,
    scan_id: &'a str,
    created_at: String,
    completed: bool,
    voltage_kv: f64,
    current_ua: f64,
    projection_count: u32,
    exposure_ms: f64,
    geometry: &'a ScanGeometry,
    references: &'a ReferenceFrames,
    stage: &'a str,
    frames: &'a [RealFrame],
    capture_destination: crate::capture_profile::CaptureDestination,
    acquisition_status: &'a str,
    data_status: &'a str,
    capture_records: &'a [CardCaptureRecord],
}

/// Writes the schema-4 card manifest used only by card scans. The schema-2/3
/// `persist` writer stays byte-identical for host-RAM scans.
///
/// `completed` is always written `false`: the acquisition path can prove safe
/// finishing, never a verified import, so only the import path may ever seal a
/// schema-4 manifest.
#[allow(clippy::too_many_arguments)]
fn persist_card(
    root: &Path,
    parameters: &Parameters,
    config: &RealScanConfig,
    records: &[CardCaptureRecord],
    stage: &str,
    acquisition_status: &str,
    data_status: &str,
) -> Result<(), String> {
    let reference_frames = ReferenceFrames::default();
    let local_frames: [RealFrame; 0] = [];
    let manifest = CardScanManifest {
        schema_version: 4,
        capture_profile: &config.capture_profile,
        max_xray_sec: config.max_xray_sec,
        scan_id: &parameters.task_id,
        created_at: timestamp(),
        completed: false,
        voltage_kv: config.voltage_kv,
        current_ua: config.current_ua,
        projection_count: parameters.projection_count,
        exposure_ms: parameters.exposure_ms,
        geometry: &config.geometry,
        references: &reference_frames,
        stage,
        frames: &local_frames,
        capture_destination: crate::capture_profile::CaptureDestination::CameraCard,
        acquisition_status,
        data_status,
        capture_records: records,
    };
    let staging_path = root.join("manifest.next.json");
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&staging_path)
        .map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &manifest).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)?;
    drop(writer);
    replace_manifest(&staging_path, &root.join("manifest.json"))
}

fn persist_progress(
    root: &Path,
    parameters: &Parameters,
    config: &RealScanConfig,
    progress: &Mutex<RealScanProgress>,
    stage: &str,
    completed: bool,
) -> Result<(), String> {
    let state = progress
        .lock()
        .expect("scan progress mutex poisoned")
        .clone();
    if config.capture_profile.is_card_destination() {
        // A card manifest is never sealed completed by the acquisition path, and
        // `acquired` is only claimed once safe finishing is explicitly recorded.
        let _ = completed;
        return persist_card(
            root,
            parameters,
            config,
            &state.card_records,
            stage,
            state.card_acquisition_status.unwrap_or(ACQUISITION_PREPARED),
            state.card_data_status.unwrap_or(DATA_PENDING_IMPORT),
        );
    }
    persist(
        root,
        parameters,
        config,
        &state.frames,
        &state.references,
        stage,
        completed,
    )
}

/// Records the card acquisition state that the next manifest write persists.
fn set_card_acquisition(
    progress: &Mutex<RealScanProgress>,
    acquisition: &'static str,
    data: &'static str,
) {
    update(progress, |value| {
        value.card_acquisition_status = Some(acquisition);
        value.card_data_status = Some(data);
    });
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn scintillator_layers_round_trip_with_geometry_and_legacy_geometry_stays_readable() {
        let legacy=serde_json::json!({"sodMm":40.0,"objectToDetectorMm":25.0,"detectorWidthMm":40.0,
            "centerOffsetXMm":0.0,"centerOffsetYMm":0.0,"rotationDirection":"counterclockwise","mirrorX":false,"measurement":"measured"});
        let mut geometry:ScanGeometry=serde_json::from_value(legacy.clone()).unwrap();
        assert!(geometry.scintillator_layers.is_empty());assert_eq!(serde_json::to_value(&geometry).unwrap(),legacy);
        geometry.scintillator_layers=vec![crate::reconstruction::spectral::StackLayer{material_name:"scintillator glass".into(),emission_color:"green".into(),thickness_mm:1.},
            crate::reconstruction::spectral::StackLayer{material_name:"scintillator glass".into(),emission_color:"blue".into(),thickness_mm:3.}];
        geometry.validate().unwrap();
        let frozen:ScanGeometry=serde_json::from_value(serde_json::to_value(&geometry).unwrap()).unwrap();assert_eq!(frozen,geometry);
        geometry.scintillator_layers[1].thickness_mm=0.;assert_eq!(geometry.validate(),Err("INVALID_SCINTILLATOR_STACK"));
    }
    #[test]
    fn scan_xray_limit_accepts_the_approved_1000_seconds_and_rejects_more() {
        assert_eq!(SCAN_MAX_XRAY_SEC, 1000);
        assert!(!scan_xray_seconds_valid(0));
        assert!(scan_xray_seconds_valid(1));
        assert!(scan_xray_seconds_valid(600));
        assert!(scan_xray_seconds_valid(1000));
        assert!(!scan_xray_seconds_valid(1001));
    }
    use super::*;
    use crate::devices::turntable::{LineTransport, EXPECTED_DEVICE};
    use std::collections::VecDeque;

    fn test_profile_for_exposure(exposure_ms: f64) -> CaptureProfile {
        let profile = crate::capture_profile::test_profile();
        let mut readback = profile.readback;
        assert_eq!(exposure_ms, 1000.0);
        readback.insert(
            "shutter".into(),
            crate::devices::camera::CameraValue::Integer(0x00010001),
        );
        CaptureProfile::from_readback(profile.identity, readback, None, 0xd100).unwrap()
    }

    fn test_reference_series(root: &Path, group: &str) -> Vec<ReferenceFrame> {
        let dir = root.join("references").join(group).join("frames");
        fs::create_dir_all(&dir).unwrap();
        (1..=REFERENCE_COUNT as u32)
            .map(|index| {
                let file_name = format!("frame-{index:04}.nef");
                let path = dir.join(&file_name);
                fs::write(&path, format!("{group} reference {index}")).unwrap();
                let (bytes, sha256) = hash_file(&path).unwrap();
                ReferenceFrame {
                    index,
                    exposure_ms: 1000.0,
                    file_name,
                    path: path.to_string_lossy().into_owned(),
                    bytes,
                    sha256,
                }
            })
            .collect()
    }

    #[test]
    fn restore_keeps_274_verified_frames_and_rejects_an_uncommitted_275th() {
        let run_id = format!(
            "{}-restore-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            std::process::id()
        );
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(run_id);
        let root = run.join("demo6");
        let frames_dir = root.join("frames");
        fs::create_dir_all(&frames_dir).unwrap();
        fs::write(run.join("command.txt"), "cargo test -p ct-engine scan::tests::restore_keeps_274_verified_frames_and_rejects_an_uncommitted_275th\n").unwrap();
        let parameters = Parameters {
            task_id: "demo6".into(),
            save_path: run.to_string_lossy().into_owned(),
            projection_count: 360,
            angle_step_deg: 1.0,
            exposure_ms: 1000.0,
        };
        let config = RealScanConfig {
            capture_profile: test_profile_for_exposure(parameters.exposure_ms),
            parameters: parameters.clone(),
            max_xray_sec: 600,
            voltage_kv: 59.9,
            current_ua: 200.0,
            resume: None,
            geometry: ScanGeometry {
                sod_mm: 100.0,
                object_to_detector_mm: 100.0,
                detector_width_mm: 100.0,
                center_offset_x_mm: 0.0,
                center_offset_y_mm: 0.0,
                rotation_direction: "clockwise".into(),
                mirror_x: false,
                measurement: "measured".into(),
                scintillator_layers:vec![],
            },
        };
        let mut committed = Vec::new();
        for index in 1..=274 {
            let path = frames_dir.join(format!("frame-{index:04}.nef"));
            fs::write(&path, format!("verified projection {index}")).unwrap();
            let (bytes, sha256) = hash_file(&path).unwrap();
            committed.push(RealFrame {
                index,
                angle_deg: projection_job(index - 1, 360).angle_deg,
                exposure_ms: 1000.0,
                file_name: format!("frame-{index:04}.nef"),
                path: path.to_string_lossy().into_owned(),
                bytes,
                sha256,
            });
        }
        persist(
            &root,
            &parameters,
            &config,
            &committed,
            &ReferenceFrames::default(),
            "projections",
            false,
        )
        .unwrap();
        assert!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap_err()
                .contains("stage and committed"),
            "projection restore must reject missing pre-scan references"
        );
        let mut references = ReferenceFrames {
            pre_dark: test_reference_series(&root, "pre-dark"),
            pre_flat: test_reference_series(&root, "pre-flat"),
            ..ReferenceFrames::default()
        };
        persist(
            &root,
            &parameters,
            &config,
            &committed,
            &references,
            "projections",
            false,
        )
        .unwrap();
        assert!(manifest_file_exists(&parameters));
        assert!(resume_manifest_exists(&parameters));
        assert_eq!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap()
                .frames,
            committed
        );
        assert_eq!(projection_job(committed.len() as u32, 360).index, 275);
        assert_eq!(
            projection_job(committed.len() as u32, 360).angle_deg,
            -274.0
        );
        let incomplete = frames_dir.join("frame-0275.nef");
        fs::write(&incomplete, b"incomplete exposure").unwrap();
        assert!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap_err()
                .contains("uncommitted")
        );
        fs::remove_file(incomplete).unwrap();
        assert_eq!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap()
                .frames
                .len(),
            274
        );
        references.post_flat = test_reference_series(&root, "post-flat");
        references.post_dark = test_reference_series(&root, "post-dark");
        persist(
            &root,
            &parameters,
            &config,
            &committed,
            &references,
            "postDark",
            false,
        )
        .unwrap();
        assert!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap_err()
                .contains("stage and committed"),
            "post-dark checkpoint cannot claim completion before the last projection"
        );
        persist(
            &root,
            &parameters,
            &config,
            &committed,
            &references,
            "completed",
            true,
        )
        .unwrap();
        assert!(
            !resume_manifest_exists(&parameters),
            "a completed manifest is not a resumable checkpoint"
        );
        assert!(
            !completed_manifest_exists(&parameters),
            "incomplete projections cannot unlock reconstruction"
        );
        fs::write(run.join("summary.md"), "PASS: 274 committed frames reload; an extra partial frame blocks restore; removing only the partial frame permits view 275.\n").unwrap();
    }

    #[test]
    fn completed_scan_needs_only_pre_scan_references() {
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(format!(
                "{}-pre-only-{}",
                chrono::Utc::now().format("%Y%m%d-%H%M%S"),
                std::process::id()
            ));
        let root = run.join("pre-only");
        let frames_dir = root.join("frames");
        fs::create_dir_all(&frames_dir).unwrap();
        let path = frames_dir.join("frame-0001.nef");
        fs::write(&path, b"projection").unwrap();
        let (bytes, sha256) = hash_file(&path).unwrap();
        let frame = RealFrame {
            index: 1,
            angle_deg: 0.0,
            exposure_ms: 1000.0,
            file_name: "frame-0001.nef".into(),
            path: path.to_string_lossy().into_owned(),
            bytes,
            sha256,
        };
        let parameters = Parameters {
            task_id: "pre-only".into(),
            save_path: run.to_string_lossy().into_owned(),
            projection_count: 1,
            angle_step_deg: 360.0,
            exposure_ms: 1000.0,
        };
        let config = RealScanConfig {
            capture_profile: test_profile_for_exposure(parameters.exposure_ms),
            parameters: parameters.clone(),
            voltage_kv: 59.9,
            current_ua: 200.0,
            max_xray_sec: 600,
            geometry: ScanGeometry {
                sod_mm: 100.0,
                object_to_detector_mm: 100.0,
                detector_width_mm: 40.0,
                center_offset_x_mm: 0.0,
                center_offset_y_mm: 0.0,
                rotation_direction: "counterclockwise".into(),
                mirror_x: false,
                measurement: "measured".into(),
                scintillator_layers:vec![],
            },
            resume: None,
        };
        let references = ReferenceFrames {
            pre_dark: test_reference_series(&root, "pre-dark"),
            pre_flat: test_reference_series(&root, "pre-flat"),
            ..ReferenceFrames::default()
        };
        persist(
            &root,
            &parameters,
            &config,
            &[frame],
            &references,
            "completed",
            true,
        )
        .unwrap();
        assert!(completed_manifest_exists(&parameters));
        let path = root.join("manifest.json");
        let mut legacy: serde_json::Value =
            serde_json::from_reader(File::open(&path).unwrap()).unwrap();
        legacy["schemaVersion"] = json!(2);
        legacy.as_object_mut().unwrap().remove("captureProfile");
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            completed_manifest_exists(&parameters),
            "legacy completed scans remain readable"
        );
        legacy["completed"] = json!(false);
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        let unfinished_before = fs::read(&path).unwrap();
        assert!(
            load_resume(&parameters, 59.9, 200.0, &config.capture_profile)
                .unwrap_err()
                .contains("LEGACY_SCHEMA_2")
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            unfinished_before,
            "rejected legacy resume never rewrites data"
        );
        fs::write(&path, before).unwrap();
    }

    #[test]
    fn stopped_outcome_requires_confirmed_beam_and_warning_off() {
        assert_eq!(
            finish_scan_result(Err(CANCELLED.into()), Ok(()), Ok(())).unwrap(),
            ScanCompletion::Stopped
        );
        assert!(
            finish_scan_result(Err(CANCELLED.into()), Err("readback lost".into()), Ok(())).is_err()
        );
        assert!(
            finish_scan_result(Err(CANCELLED.into()), Ok(()), Err("warning stuck".into())).is_err()
        );
        assert_eq!(
            finish_scan_result(Ok(()), Ok(()), Ok(())).unwrap(),
            ScanCompletion::Completed
        );
    }

    #[test]
    fn disconnected_worker_outcome_is_a_terminal_fault() {
        let (sender, receiver) = mpsc::channel::<u8>();
        drop(sender);
        assert!(poll_scan_outcome(&receiver, &mut None).unwrap().is_err());
    }

    #[test]
    fn scan_monitor_keeps_serial_diagnostics_in_the_durable_scan_log() {
        let (mut xray, probe) = crate::devices::xray::test_worker(60.0, 200.0);
        xray.beam_on(&AtomicBool::new(false)).unwrap();
        probe.fail_next_read();
        // A fresh verified pair may be reused for 250 ms. Wait for the real
        // autonomous failure before checking its diagnostic delivery.
        let deadline = Instant::now() + Duration::from_secs(2);
        while xray.health().last_error.is_none() && Instant::now() < deadline { thread::sleep(Duration::from_millis(5)); }
        assert!(xray.health().last_error.is_some());
        let progress = Mutex::new(RealScanProgress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(refresh_emission_monitored(&mut xray, &progress, &cancel).is_err());
        let messages = progress.lock().unwrap().messages.clone();
        assert!(messages.iter().any(|entry| entry.source == "xray" && entry.message.contains("Moxtek serial command")));
        let mut output = Vec::new();
        write_scan_log(&mut output, &messages, &Err("communication fault".into())).unwrap();
        let log = String::from_utf8(output).unwrap();
        assert!(log.contains("Moxtek serial command"));
        assert!(log.contains("budget="));
        assert!(xray.drain_diagnostics().is_empty(), "diagnostics must not be logged twice");
    }

    #[test]
    fn scan_log_keeps_the_fault_and_all_scan_events() {
        let messages = vec![
            ScanMessage {
                timestamp: "2026-09-28T10:00:00Z".into(),
                level: "ACTION",
                source: "nano",
                message: "Verifying final position".into(),
            },
            ScanMessage {
                timestamp: "2026-09-28T10:00:01Z".into(),
                level: "ERR",
                source: "system",
                message: "final Nano STATUS failed\nread timed out".into(),
            },
        ];
        let mut output = Vec::new();
        write_scan_log(
            &mut output,
            &messages,
            &Err("final Nano STATUS failed: read timed out".into()),
        )
        .unwrap();
        let log = String::from_utf8(output).unwrap();
        assert!(log.contains("2026-09-28T10:00:00Z\tACTION\tnano\tVerifying final position"));
        assert!(log.contains(
            "2026-09-28T10:00:01Z\tERR\tsystem\tfinal Nano STATUS failed read timed out"
        ));
        assert!(log.contains("\tRESULT\tsystem\tFAULT: final Nano STATUS failed: read timed out"));
    }

    #[derive(Default)]
    struct PipelineNanoState {
        lines: VecDeque<String>,
        writes: Vec<String>,
        position: i64,
        capture_id: u32,
        rearmed: bool,
        homed: bool,
        on_capture_done: Option<Box<dyn FnOnce() + Send>>,
    }
    struct PipelineNano(Arc<Mutex<PipelineNanoState>>);
    impl LineTransport for PipelineNano {
        fn write_line(&mut self, line: &str) -> std::io::Result<()> {
            let fields: Vec<_> = line.split_whitespace().collect();
            let id = fields.get(1).copied().unwrap_or("0");
            let mut state = self.0.lock().unwrap();
            state.writes.push(line.into());
            match fields[0] {
                "PING" => {
                    state.lines.push_back(format!("ACK {id} PING"));
                    state.lines.push_back(format!("PONG {id}"));
                }
                "INFO" => {
                    state.lines.push_back(format!("ACK {id} INFO"));
                    state.lines.push_back(format!("INFO {id} DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=offline-test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"));
                }
                "HEARTBEAT" => state.lines.push_back(format!("HBACK {id}")),
                "STATUS" => {
                    let position = state.position;
                    let capture_id = state.capture_id;
                    let phase = if !state.rearmed {
                        "LOCKED"
                    } else if capture_id == 0 {
                        "IDLE"
                    } else {
                        "CAPTURE_HOLD"
                    };
                    let homed = u8::from(state.homed);
                    let rearmed = u8::from(state.rearmed);
                    state.lines.push_back(format!("ACK {id} STATUS"));
                    state.lines.push_back(format!("STATUS {id} state={phase} pos={position} target={position} microsteps=8 ppr=96000 reference={homed} homed={homed} rearmed={rearmed} hall=0 capture_id={capture_id}"));
                }
                "MOVE_REL" => {
                    let delta = fields[2].parse::<i64>().unwrap();
                    assert!((-360000..=0).contains(&delta));
                    state.position -= ((-delta) * 96000 + 180000) / 360000;
                    state.capture_id = id.parse().unwrap();
                    let position = state.position;
                    state.lines.push_back(format!("ACK {id} MOVE_REL"));
                    state
                        .lines
                        .push_back(format!("READY_TO_CAPTURE {id} POS={position}"));
                }
                "CAPTURE_DONE" => {
                    if id.parse::<u32>().unwrap() != state.capture_id {
                        state
                            .lines
                            .push_back(format!("ERR {id} CAPTURE_ID_MISMATCH"));
                    } else {
                        if let Some(inject) = state.on_capture_done.take() {
                            inject();
                        }
                        state.capture_id = 0;
                        state.lines.push_back(format!("ACK {id} CAPTURE_DONE"));
                        state.lines.push_back(format!("IDLE {id} CAPTURE_RELEASED"));
                    }
                }
                "XRAY_WARNING" => {
                    state.lines.push_back(format!("ACK {id} XRAY_WARNING"));
                    state
                        .lines
                        .push_back(format!("OK {id} XRAY_WARNING={}", fields[2]));
                }
                "STOP" => {
                    state.rearmed = false;
                    state.homed = false;
                    state.capture_id = 0;
                    state.lines.push_back(format!("ACK {id} STOP"));
                    state
                        .lines
                        .push_back(format!("STOPPED {id} POSITION_UNKNOWN reason=HOST_STOP"));
                }
                _ => panic!("Unexpected physical command in offline scan fake: {line}"),
            }
            Ok(())
        }
        fn read_line(&mut self, timeout: Duration) -> std::io::Result<Option<String>> {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(line) = self.0.lock().unwrap().lines.pop_front() {
                    return Ok(Some(line));
                }
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    fn pipeline_nano() -> (Arc<NanoAdapter>, Arc<Mutex<PipelineNanoState>>) {
        let state = Arc::new(Mutex::new(PipelineNanoState {
            rearmed: true,
            homed: true,
            ..Default::default()
        }));
        state.lock().unwrap().lines.push_back(format!("READY DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=offline-test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"));
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("MEMORY-SCAN-ONLY", Box::new(PipelineNano(state.clone())))
            .unwrap();
        (Arc::new(adapter), state)
    }
    fn pipeline_config(run: &Path, task: &str, camera: &CameraAdapter) -> RealScanConfig {
        let profile = CaptureProfile::from_camera_readback(
            &camera.health(),
            camera.profile_readback().unwrap(),
            None,
        )
        .unwrap();
        RealScanConfig {
            parameters: Parameters {
                task_id: task.into(),
                save_path: run.to_string_lossy().into(),
                projection_count: 3,
                angle_step_deg: 120.0,
                exposure_ms: 200.0,
            },
            capture_profile: profile,
            max_xray_sec: 600,
            voltage_kv: 20.0,
            current_ua: 100.0,
            geometry: ScanGeometry {
                sod_mm: 100.0,
                object_to_detector_mm: 100.0,
                detector_width_mm: 40.0,
                center_offset_x_mm: 0.0,
                center_offset_y_mm: 0.0,
                rotation_direction: "counterclockwise".into(),
                mirror_x: false,
                measurement: "measured".into(),
                scintillator_layers:vec![],
            },
            resume: None,
        }
    }
    fn complete_pipeline(
        mut handle: RealScanHandle,
        camera: &CameraAdapter,
        probe: &crate::devices::xray::TestXrayProbe,
        task: &str,
    ) -> (ScanOutcome, bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut confirmed = std::collections::BTreeSet::new();
        let mut early_off = false;
        loop {
            let progress = handle.progress();
            if let Some(stage) = progress.prompt {
                if !confirmed.contains(stage) && handle.confirm_stage(stage).is_ok() {
                    confirmed.insert(stage);
                }
            }
            let health = camera.health();
            if health.busy
                && health
                    .operation_id
                    .as_ref()
                    .is_some_and(|id| id.starts_with(&format!("scan-{task}-3-")))
                && !probe.output_enabled()
                && progress.captured < 3
            {
                early_off = true;
            }
            if let Some(outcome) = handle.try_finish() {
                return (outcome.unwrap(), early_off);
            }
            if Instant::now() >= deadline {
                handle.cancel_only();
                panic!(
                    "Offline production coordinator did not complete: {:?}",
                    handle.progress()
                );
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn production_coordinator_two_rounds_preserve_continuous_groups_and_end_beam_before_final_file()
    {
        use crate::devices::camera::{scripted_camera, ScriptedCameraConfig};
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(format!(
                "{}-native-production-coordinator",
                chrono::Local::now().format("%Y%m%d-%H%M%S")
            ));
        fs::create_dir_all(&run).unwrap();
        fs::write(
            run.join("command.txt"),
            "cargo test -p ct-engine scan::tests::production_coordinator_two_rounds\n",
        )
        .unwrap();
        let (nano, state) = pipeline_nano();
        let camera = scripted_camera(ScriptedCameraConfig {
            exposure_delay: Duration::from_millis(5),
            transfer_delay: Duration::from_millis(150),
            omit_exposure_end: false,
            truncate_raw: false,
            close_failure: false,
            native_exposure_ms: None,
            capture_started_signal: None,
            discovered: 1,
            ..Default::default()
        });
        let (xray, probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let initial = probe.beam_transitions().len();
        for (task, expected_position) in [("round-a", -64000), ("round-b", -160000)] {
            let config = pipeline_config(&run, task, &camera);
            let parameters = config.parameters.clone();
            let handle = RealScanHandle::start(nano.clone(), camera.clone(), xray.clone(), config);
            let (outcome, early_off) = complete_pipeline(handle, &camera, &probe, task);
            assert_eq!(outcome.result.as_ref().unwrap(), &ScanCompletion::Completed);
            assert!(
                early_off,
                "final ExposureEnded closes output while transfer/file commit remains pending"
            );
            assert!(completed_manifest_exists(&parameters));
            let manifest: serde_json::Value =
                serde_json::from_reader(File::open(run.join(task).join("manifest.json")).unwrap())
                    .unwrap();
            assert_eq!(manifest["schemaVersion"], 3);
            assert_eq!(manifest["frames"].as_array().unwrap().len(), 3);
            assert_eq!(
                manifest["references"]["preDark"].as_array().unwrap().len(),
                10
            );
            assert_eq!(
                manifest["references"]["preFlat"].as_array().unwrap().len(),
                10
            );
            assert_eq!(
                manifest["references"]["postDark"].as_array().unwrap().len(),
                0
            );
            assert_eq!(
                manifest["references"]["postFlat"].as_array().unwrap().len(),
                0
            );
            assert_eq!(nano.status().unwrap().position_pulses, expected_position);
            assert!(!probe.output_enabled());
        }
        let transitions = probe.beam_transitions();
        assert_eq!(
            transitions[initial..]
                .iter()
                .filter(|(enabled, _)| *enabled)
                .count(),
            4,
            "one pre-flat block and one projection block per scan"
        );
        let state = state.lock().unwrap();
        assert!(!state
            .writes
            .iter()
            .any(|line| line.starts_with("HOME ") || line.starts_with("MOVE_ABS ")));
        assert_eq!(
            state
                .writes
                .iter()
                .filter(|line| line.starts_with("CAPTURE_DONE "))
                .count(),
            6
        );
        fs::write(run.join("summary.md"),"PASS: same production scan coordinator and all three production workers; two rounds, 10+10 front references, 3 projections, continuous blocks, ExposureEnded before file commit, matching hold release, no terminal HOME, original synthetic NEF retained. Only offline verification.\n").unwrap();
    }

    // R0 exercises the production coordinator and all three device owners.
    // Only their physical transports are scripted; UNKNOWN and file outcomes
    // are asserted after the complete cleanup path, never inferred from ACKs.
    fn r0_run(topic: &str) -> std::path::PathBuf {
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(format!("{}-system-coordinator-{topic}-{}",
                chrono::Local::now().format("%Y%m%d-%H%M%S"),
                chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("command.txt"), "cargo test -p ct-engine scan::tests::r0_ -- --nocapture\n").unwrap();
        println!("R0 EVIDENCE: {}", run.display());
        run
    }

    fn r0_reference_hashes(root: &Path) -> Vec<(String, u64, String)> {
        let references = root.join("references");
        let mut files: Vec<_> = ["pre-dark", "pre-flat"].into_iter()
            .flat_map(|group| {
                let files: Vec<_> = fs::read_dir(references.join(group).join("frames")).unwrap()
                    .map(|entry| entry.unwrap().path()).collect();
                assert_eq!(files.len(), REFERENCE_COUNT);
                files
            }).collect();
        files.sort();
        assert_eq!(files.len(), 20, "complete pre-dark and pre-flat groups exist before fault injection");
        files.into_iter().map(|path| {
            let (bytes, hash) = hash_file(&path).unwrap();
            (path.strip_prefix(&references).unwrap().to_string_lossy().into_owned(), bytes, hash)
        }).collect()
    }

    fn projection_resume_fixture(
        run: &Path, task: &str, camera: &CameraAdapter, stage: &str, committed_count: u32,
    ) -> RealScanConfig {
        let root = run.join(task);
        fs::create_dir_all(root.join("frames")).unwrap();
        let mut config = pipeline_config(run, task, camera);
        let mut references = ReferenceFrames {
            pre_dark: test_reference_series(&root, "pre-dark"),
            pre_flat: test_reference_series(&root, "pre-flat"),
            ..Default::default()
        };
        for frame in references.pre_dark.iter_mut().chain(&mut references.pre_flat) {
            frame.exposure_ms = config.parameters.exposure_ms;
        }
        let frames: Vec<_> = (1..=committed_count).map(|index| {
            let file_name = format!("frame-{index:04}.nef");
            let path = root.join("frames").join(&file_name);
            fs::write(&path, format!("retained projection {index}")).unwrap();
            let (bytes, sha256) = hash_file(&path).unwrap();
            RealFrame {
                index, angle_deg: projection_job(index - 1, config.parameters.projection_count).angle_deg,
                exposure_ms: config.parameters.exposure_ms, file_name,
                path: path.to_string_lossy().into_owned(), bytes, sha256,
            }
        }).collect();
        persist(&root, &config.parameters, &config, &frames, &references, stage, false).unwrap();
        let original_manifest = fs::read(root.join("manifest.json")).unwrap();
        config.resume = Some(load_resume(
            &config.parameters, config.voltage_kv, config.current_ua, &config.capture_profile,
        ).unwrap());
        assert_eq!(fs::read(root.join("manifest.json")).unwrap(), original_manifest,
            "Restore verifies and normalizes in memory without rewriting the checkpoint");
        config
    }

    #[test]
    fn projection_checkpoint_resume_skips_reference_and_sample_preparation() {
        use crate::devices::camera::{scripted_camera_with_probe, ScriptedCameraConfig};
        for (stage, retained) in [("projections", 1), ("placeSample", 1), ("projections", 0), ("finalizing", 3)] {
            let run = r0_run(&format!("projection-resume-{stage}-{retained}"));
            let root = run.join("resume-task");
            let (nano, nano_state) = pipeline_nano();
            let (camera, camera_probe) = scripted_camera_with_probe(ScriptedCameraConfig {
                exposure_delay: Duration::from_millis(5), ..Default::default()
            });
            let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
            let config = projection_resume_fixture(&run, "resume-task", &camera, stage, retained);
            let before = config.resume.as_ref().unwrap().clone();
            if retained == 1 {
                assert!(recovery_position_matches(&before, &config.parameters, 0, 96_000));
                assert!(recovery_position_matches(&before, &config.parameters, -32_000, 96_000));
                assert!(!recovery_position_matches(&before, &config.parameters, -31_999, 96_000));
                assert!(!recovery_position_matches(&before, &config.parameters, -32_000, 48_000));
            }
            let expected_stage = if stage == "finalizing" { "finalizing" } else { "projections" };
            assert_eq!(before.stage, expected_stage);
            assert_eq!(initial_scan_stage(config.resume.as_ref()), expected_stage);
            let references_before = r0_reference_hashes(&root);
            let mut handle = RealScanHandle::start(nano, camera, xray, config);
            let deadline = Instant::now() + Duration::from_secs(20);
            let outcome = loop {
                let progress = handle.progress();
                if progress.prompt.is_some() || matches!(progress.stage, "preDark" | "preFlat" | "placeSample") {
                    handle.cancel_only();
                    panic!("verified projection resume regressed to preparation: {progress:?}");
                }
                if let Some(outcome) = handle.try_finish() { break outcome.unwrap(); }
                if Instant::now() >= deadline {
                    handle.cancel_only();
                    panic!("projection resume did not finish: {:?}", handle.progress());
                }
                thread::sleep(Duration::from_millis(1));
            };
            assert_eq!(outcome.result.as_ref().unwrap(), &ScanCompletion::Completed);
            let progress = handle.progress();
            assert_eq!(&progress.frames[..retained as usize], before.frames.as_slice());
            assert_eq!(progress.references, before.references);
            assert_eq!(r0_reference_hashes(&root), references_before);
            for frame in &before.frames {
                assert_eq!(hash_file(Path::new(&frame.path)).unwrap(), (frame.bytes, frame.sha256.clone()));
            }
            assert_eq!(camera_probe.operation_count(0x90c0), (3 - retained) as usize,
                "only missing projections may cause physical captures");
            assert_eq!(nano_state.lock().unwrap().writes.iter().filter(|line| line.starts_with("CAPTURE_DONE ")).count(), (3 - retained) as usize);
            assert_eq!(xray_probe.beam_transitions().iter().filter(|(on, _)| *on).count(), usize::from(retained < 3),
                "no reference group may open the beam during projection recovery");
            assert!(!xray_probe.output_enabled());
        }
    }

    #[test]
    fn genuine_place_sample_checkpoint_still_requires_operator_confirmation() {
        use crate::devices::camera::{scripted_camera_with_probe, ScriptedCameraConfig};
        let run = r0_run("unfinished-sample-confirmation");
        let (nano, _) = pipeline_nano();
        let (camera, camera_probe) = scripted_camera_with_probe(ScriptedCameraConfig::default());
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config = projection_resume_fixture(&run, "sample-task", &camera, "placeSample", 0);
        assert_eq!(config.resume.as_ref().unwrap().stage, "placeSample");
        let mut handle = RealScanHandle::start(nano, camera, xray, config);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let progress = handle.progress();
            assert!(!xray_probe.output_enabled());
            assert_eq!(camera_probe.operation_count(0x90c0), 0);
            if progress.prompt == Some("placeSample") { break; }
            assert!(handle.try_finish().is_none(), "unfinished sample confirmation must remain pending");
            if Instant::now() >= deadline { handle.cancel_only(); panic!("sample prompt did not appear"); }
            thread::sleep(Duration::from_millis(1));
        }
        handle.cancel_only();
        while handle.try_finish().is_none() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(camera_probe.operation_count(0x90c0), 0);
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum R0ProjectionFault { CameraLoss, Interlock, StopTransfer, UnknownOff }

    fn r0_projection_fault(case: R0ProjectionFault) {
        use crate::devices::camera::{scripted_camera_with_probe, ScriptedCameraConfig};
        let topic = match case {
            R0ProjectionFault::CameraLoss => "camera-loss",
            R0ProjectionFault::Interlock => "interlock",
            R0ProjectionFault::StopTransfer => "stop-transfer",
            R0ProjectionFault::UnknownOff => "off-unconfirmed",
        };
        let run = r0_run(topic);
        let task = "fault-task";
        let root = run.join(task);
        let (nano, nano_state) = pipeline_nano();
        let (trigger_sender, trigger_receiver) = mpsc::channel();
        let (camera, camera_probe) = scripted_camera_with_probe(ScriptedCameraConfig {
            exposure_delay: Duration::from_millis(5),
            capture_started_signal: Some(trigger_sender),
            ..Default::default()
        });
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config = pipeline_config(&run, task, &camera);
        let parameters = config.parameters.clone();
        let mut handle = RealScanHandle::start(nano.clone(), camera.clone(), xray, config);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut references = None;
        let mut transfer_count = 0;
        let mut armed = false;
        let mut injected_at = None;
        let outcome = loop {
            let progress = handle.progress();
            if progress.prompt == Some("preReferences") {
                let _ = handle.confirm_stage("preReferences");
            }
            if progress.prompt == Some("placeSample") && !armed {
                references = Some(r0_reference_hashes(&root));
                trigger_receiver.try_iter().for_each(drop);
                transfer_count = camera_probe.operation_count(0x1009); // GetObject
                if case == R0ProjectionFault::StopTransfer {
                    camera_probe.set_transfer_delay(Duration::from_secs(2));
                } else {
                    camera_probe.set_exposure_delay(Duration::from_secs(2));
                }
                armed = true;
                handle.confirm_stage("placeSample").unwrap();
            }
            let accepted = armed && trigger_receiver.try_iter().count() > 0;
            let transfer_pending = armed && camera_probe.operation_count(0x1009) > transfer_count;
            if injected_at.is_none() && (if case == R0ProjectionFault::StopTransfer { transfer_pending } else { accepted }) {
                assert!(xray_probe.output_enabled(), "fault must occur in an actual scripted beam block");
                assert!(camera.health().operation_id.as_deref().is_some_and(|id| id.starts_with("scan-fault-task-1-")));
                injected_at = Some(Instant::now());
                match case {
                    R0ProjectionFault::CameraLoss => camera_probe.fail_transport_operation(0x90c7),
                    R0ProjectionFault::Interlock => xray_probe.set_interlock(true),
                    R0ProjectionFault::StopTransfer => handle.request_stop().unwrap(),
                    R0ProjectionFault::UnknownOff => {
                        xray_probe.ignore_off(true);
                        handle.request_pause(); // First projection's associated C102 must request OFF.
                    }
                }
            }
            if let Some(outcome) = handle.try_finish() { break outcome.unwrap(); }
            if Instant::now() >= deadline {
                handle.cancel_only();
                panic!("R0 {case:?} did not terminate: {:?}", handle.progress());
            }
            thread::sleep(Duration::from_millis(1));
        };
        let injected_at = injected_at.expect("the intended fault boundary was reached");
        let progress = handle.progress();
        assert_eq!(progress.captured, 0);
        assert!(progress.frames.is_empty(), "failed/cancelled projection cannot advance progress");
        assert!(!completed_manifest_exists(&parameters));
        let manifest: serde_json::Value = serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["completed"], false);
        assert_eq!(manifest["frames"].as_array().unwrap().len(), 0);
        assert_eq!(r0_reference_hashes(&root), references.unwrap(), "original reference files are retained byte-for-byte");
        let writes = nano_state.lock().unwrap().writes.clone();
        assert!(writes.iter().any(|line| line.starts_with("STOP ")));
        assert!(!writes.iter().any(|line| line.starts_with("CAPTURE_DONE ")), "failed first projection cannot release a completed capture ticket");
        let camera_health = outcome.camera.health();
        let xray_health = outcome.xray.health();
        if case == R0ProjectionFault::StopTransfer {
            assert_eq!(outcome.result.as_ref().unwrap(), &ScanCompletion::Stopped);
            assert_eq!(progress.phase, "stopped");
            assert!(camera_health.cleanup_confirmed);
        } else {
            assert!(outcome.result.is_err());
            assert_eq!(progress.phase, "fault");
        }
        if case == R0ProjectionFault::CameraLoss {
            assert!(camera_health.isolated && !camera_health.cleanup_confirmed);
            assert!(camera_health.last_error.as_deref().is_some_and(|error| error.contains("scripted camera disconnected")));
        }
        if case == R0ProjectionFault::UnknownOff {
            assert!(!xray_health.beam_off_confirmed);
            assert!(xray_probe.output_enabled(), "fake OFF ACK alone cannot prove output is off");
            assert!(outcome.result.as_ref().unwrap_err().contains("OFF"));
            let last_warning = writes.iter().rev().find(|line| line.starts_with("XRAY_WARNING ")).unwrap();
            assert_eq!(last_warning.split_whitespace().last(), Some("ON"),
                "unknown output must retain warning ON; writes={writes:?}");
            xray_probe.ignore_off(false);
        } else {
            assert!(xray_health.beam_off_confirmed && !xray_probe.output_enabled());
            if case == R0ProjectionFault::StopTransfer {
                assert!(xray_probe.beam_transitions().iter().any(|(on, at)| !*on && *at >= injected_at && at.duration_since(injected_at) < Duration::from_secs(1)),
                    "OFF must precede the two-second stalled transfer deadline; this is an offline timing assertion");
            }
            assert_eq!(fs::read_dir(root.join("frames")).unwrap().count(), 0, "no late NEF commit after cancellation/fault");
        }
        fs::write(run.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "status":"PASS", "boundary":"offline production coordinator and scripted device owners", "case":topic,
            "outcome":format!("{:?}",outcome.result), "phase":progress.phase, "captured":progress.captured,
            "cameraCleanupConfirmed":camera_health.cleanup_confirmed, "cameraIsolated":camera_health.isolated,
            "xrayOffConfirmed":xray_health.beam_off_confirmed, "referenceFilesRetained":20, "nanoWrites":writes,
            "fullLinkedHardwareCertified":false
        })).unwrap()).unwrap();
    }

    #[test]
    fn r0_camera_loss_during_projection_isolates_camera_and_never_commits() {
        r0_projection_fault(R0ProjectionFault::CameraLoss);
    }

    #[test]
    fn r0_interlock_during_projection_confirms_off_and_never_commits() {
        r0_projection_fault(R0ProjectionFault::Interlock);
    }

    #[test]
    fn r0_stop_during_projection_transfer_confirms_off_before_transfer_and_never_commits() {
        r0_projection_fault(R0ProjectionFault::StopTransfer);
    }

    #[test]
    fn r0_unconfirmed_off_retains_warning_and_cannot_report_safe_completion() {
        r0_projection_fault(R0ProjectionFault::UnknownOff);
    }

    #[test]
    fn r0_manifest_write_failure_retains_nef_and_refuses_projection_progress() {
        use crate::devices::camera::{scripted_camera, ScriptedCameraConfig};
        let run = r0_run("manifest-failure");
        let root = run.join("commit-task");
        let (nano, nano_state) = pipeline_nano();
        let camera = scripted_camera(ScriptedCameraConfig { exposure_delay:Duration::from_millis(5), ..Default::default() });
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config = pipeline_config(&run, "commit-task", &camera);
        let parameters = config.parameters.clone();
        let mut handle = RealScanHandle::start(nano, camera, xray, config);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut references = None;
        let mut armed = false;
        let outcome = loop {
            let progress = handle.progress();
            if progress.prompt == Some("preReferences") { let _ = handle.confirm_stage("preReferences"); }
            if progress.prompt == Some("placeSample") && !armed {
                references = Some(r0_reference_hashes(&root));
                let fault_root = root.clone();
                nano_state.lock().unwrap().on_capture_done = Some(Box::new(move || {
                    assert!(fault_root.join("frames/frame-0001.nef").is_file());
                    // Fail persistence after the matching device release, before
                    // the sole domain owner can publish committed frame progress.
                    fs::rename(fault_root.join("acquisition.csv"), fault_root.join("acquisition-before-fault.csv")).unwrap();
                    fs::create_dir(fault_root.join("acquisition.csv")).unwrap();
                }));
                armed = true;
                handle.confirm_stage("placeSample").unwrap();
            }
            if let Some(outcome) = handle.try_finish() { break outcome.unwrap(); }
            if Instant::now() >= deadline { handle.cancel_only(); panic!("R0 manifest fault did not finish: {:?}", handle.progress()); }
            thread::sleep(Duration::from_millis(1));
        };
        assert!(armed && root.join("acquisition.csv").is_dir(), "the commit fault was actually injected");
        assert!(outcome.result.is_err());
        let progress = handle.progress();
        assert_eq!(progress.phase, "fault");
        assert_eq!(progress.captured, 0);
        assert!(progress.frames.is_empty());
        assert!(!completed_manifest_exists(&parameters));
        let manifest:serde_json::Value = serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["completed"], false);
        assert_eq!(manifest["frames"].as_array().unwrap().len(), 0);
        let (bytes, sha256) = hash_file(&root.join("frames/frame-0001.nef")).unwrap();
        assert!(bytes > 0 && sha256.len() == 64, "captured NEF survives failed manifest commit");
        assert_eq!(r0_reference_hashes(&root), references.unwrap());
        assert!(!xray_probe.output_enabled() && outcome.xray.health().beam_off_confirmed);
        let writes=nano_state.lock().unwrap().writes.clone();
        assert_eq!(writes.iter().filter(|line|line.starts_with("CAPTURE_DONE ")).count(),1);
        assert!(writes.iter().any(|line|line.starts_with("STOP ")));
        fs::write(run.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({
            "status":"PASS", "boundary":"offline production coordinator and scripted device owners", "case":"manifest-failure",
            "phase":progress.phase, "captured":progress.captured, "nefBytes":bytes, "nefSha256":sha256,
            "referenceFilesRetained":20, "manifestCompleted":false, "fullLinkedHardwareCertified":false
        })).unwrap()).unwrap();
    }

    #[test]
    fn r0_pause_commits_current_projection_before_resume_without_repeating_files() {
        use crate::devices::camera::{scripted_camera_with_probe, ScriptedCameraConfig};
        let run = r0_run("pause-resume");
        let root = run.join("pause-task");
        let (nano, nano_state) = pipeline_nano();
        let (trigger_sender, trigger_receiver) = mpsc::channel();
        let (camera, camera_probe) = scripted_camera_with_probe(ScriptedCameraConfig {
            exposure_delay:Duration::from_millis(5), capture_started_signal:Some(trigger_sender), ..Default::default()
        });
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config=pipeline_config(&run,"pause-task",&camera);
        let parameters=config.parameters.clone();
        let mut handle=RealScanHandle::start(nano,camera,xray,config);
        let deadline=Instant::now()+Duration::from_secs(30);
        let mut armed=false;
        let mut pause_requested=false;
        let mut pause_verified=false;
        let mut reference_hashes=None;
        let outcome=loop {
            let progress=handle.progress();
            if progress.prompt==Some("preReferences"){let _=handle.confirm_stage("preReferences");}
            if progress.prompt==Some("placeSample")&&!armed {
                reference_hashes=Some(r0_reference_hashes(&root));
                trigger_receiver.try_iter().for_each(drop);
                camera_probe.set_exposure_delay(Duration::from_millis(200));
                camera_probe.set_transfer_delay(Duration::from_millis(100));
                armed=true;handle.confirm_stage("placeSample").unwrap();
            }
            if armed&&!pause_requested&&trigger_receiver.try_iter().count()>0 {
                assert!(xray_probe.output_enabled());
                handle.request_pause();pause_requested=true;
            }
            if progress.phase=="paused"&&!pause_verified {
                assert_eq!(progress.captured,1);
                assert_eq!(progress.frames.len(),1);
                assert!(!xray_probe.output_enabled());
                let manifest:serde_json::Value=serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
                assert_eq!(manifest["frames"].as_array().unwrap().len(),1);
                assert_eq!(manifest["completed"],false);
                assert_eq!(nano_state.lock().unwrap().writes.iter().filter(|line|line.starts_with("CAPTURE_DONE ")).count(),1);
                pause_verified=true;handle.resume();
            }
            if let Some(outcome)=handle.try_finish(){break outcome.unwrap();}
            if Instant::now()>=deadline{handle.cancel_only();panic!("R0 pause/resume did not finish: {:?}",handle.progress());}
            thread::sleep(Duration::from_millis(1));
        };
        assert!(pause_requested&&pause_verified);
        assert_eq!(outcome.result.as_ref().unwrap(),&ScanCompletion::Completed);
        assert!(completed_manifest_exists(&parameters));
        assert_eq!(handle.progress().captured,3);
        assert_eq!(camera_probe.operation_count(0x90c0),23,"20 references plus exactly three projections");
        assert_eq!(fs::read_dir(root.join("frames")).unwrap().count(),3);
        assert_eq!(r0_reference_hashes(&root),reference_hashes.unwrap());
        let transitions=xray_probe.beam_transitions();
        assert_eq!(transitions.iter().filter(|(on,_)|*on).count(),3,"one flat group and two projection blocks separated by pause");
        let mut last_off=None;
        for (on,at) in &transitions {
            if *on {assert!(last_off.is_some_and(|off|at.duration_since(off)>=Duration::from_secs(2)),"pause/resume cannot bypass the adapter OFF-to-ON gate");}
            else {last_off=Some(*at);}
        }
        assert!(!xray_probe.output_enabled()&&outcome.xray.health().beam_off_confirmed);
        fs::write(run.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({
            "status":"PASS", "boundary":"offline production coordinator and scripted device owners", "case":"pause-resume",
            "pauseCaptured":1, "completedCaptured":3, "totalCaptureCommands":23, "beamOnBlocks":3,
            "referenceFilesRetained":20, "fullLinkedHardwareCertified":false
        })).unwrap()).unwrap();
    }

    struct DeferredMoveTransport {
        lines: Arc<Mutex<VecDeque<String>>>,
        announced: std::sync::mpsc::Sender<u32>,
        capture_id: u32,
    }

    impl LineTransport for DeferredMoveTransport {
        fn write_line(&mut self, line: &str) -> std::io::Result<()> {
            let fields: Vec<_> = line.split_whitespace().collect();
            let mut lines = self.lines.lock().unwrap();
            let id = fields.get(1).copied().unwrap_or("0");
            match fields[0] {
                "PING" => {
                    lines.push_back(format!("ACK {id} PING"));
                    lines.push_back(format!("PONG {id}"));
                }
                "INFO" => {
                    lines.push_back(format!("ACK {id} INFO"));
                    lines.push_back(format!("INFO {id} DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"));
                }
                "HEARTBEAT" => lines.push_back(format!("HBACK {id}")),
                "MOVE_REL" => {
                    self.capture_id = id.parse().unwrap();
                    lines.push_back(format!("ACK {id} MOVE_REL"));
                    self.announced.send(self.capture_id).unwrap();
                    // The test withholds READY_TO_CAPTURE until it has sampled
                    // the live progress while this motion is still pending.
                }
                "STATUS" => {
                    lines.push_back(format!("ACK {id} STATUS"));
                    let (state, position) = if self.capture_id == 0 {
                        ("IDLE", 0)
                    } else {
                        ("CAPTURE_HOLD", -24000)
                    };
                    lines.push_back(format!("STATUS {id} state={state} pos={position} target={position} microsteps=8 ppr=96000 reference=1 homed=1 rearmed=1 hall=0 capture_id={}", self.capture_id));
                }
                "STOP" => {
                    lines.push_back(format!("ACK {id} STOP"));
                    lines.push_back(format!("STOPPED {id} POS=-24000"));
                }
                "XRAY_WARNING" => {
                    lines.push_back(format!("ACK {id} XRAY_WARNING"));
                    lines.push_back(format!(
                        "OK {id} XRAY_WARNING={}",
                        fields.get(2).unwrap_or(&"OFF")
                    ));
                }
                _ => {}
            }
            Ok(())
        }
        fn read_line(&mut self, timeout: Duration) -> std::io::Result<Option<String>> {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(line) = self.lines.lock().unwrap().pop_front() {
                    return Ok(Some(line));
                }
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn pending_move_keeps_old_angle_until_device_confirms_arrival() {
        let lines = Arc::new(Mutex::new(VecDeque::from([format!(
            "READY DEVICE={EXPECTED_DEVICE} VERSION=2.1.0 PROTOCOL=2 BUILD=test BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"
        )])));
        let (announced, receiver) = std::sync::mpsc::channel();
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test(
                "MEMORY-ONLY",
                Box::new(DeferredMoveTransport {
                    lines: lines.clone(),
                    announced,
                    capture_id: 0,
                }),
            )
            .unwrap();
        let adapter = Arc::new(adapter);
        let progress = Arc::new(Mutex::new(RealScanProgress::default()));
        let worker_adapter = adapter.clone();
        let worker_progress = progress.clone();
        let worker = thread::spawn(move || {
            move_to_projection(
                &worker_adapter,
                &worker_progress,
                projection_job(1, 4),
                &Arc::new(AtomicBool::new(false)),
            )
        });
        let arrival = receiver.recv_timeout(Duration::from_secs(2));
        let before = progress.lock().unwrap().angle_deg;
        if let Ok(id) = arrival {
            lines
                .lock()
                .unwrap()
                .push_back(format!("READY_TO_CAPTURE {id} POS=-24000"));
        } else {
            let _ = adapter.stop();
        }
        let result = worker.join().unwrap();
        assert!(arrival.is_ok(), "in-memory motion was not issued");
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            before, 0.0,
            "a target must not appear as current feedback before arrival"
        );
        assert_eq!(progress.lock().unwrap().angle_deg, -90.0);
    }

    #[test]
    fn committing_an_older_frame_does_not_rewind_prefetched_live_position() {
        let progress = Mutex::new(RealScanProgress::default());
        set_angle(&progress, -90.0);
        push_frame(
            &progress,
            RealFrame {
                index: 1,
                angle_deg: 0.0,
                exposure_ms: 200.0,
                file_name: "frame-0001.nef".into(),
                path: "memory-only".into(),
                bytes: 1,
                sha256: "test".into(),
            },
        );
        let state = progress.lock().unwrap();
        assert_eq!(state.captured, 1);
        assert_eq!(state.frames[0].angle_deg, 0.0);
        assert_eq!(state.angle_deg, -90.0);
    }

    #[test]
    fn exposure_boundary_requires_this_operations_event_and_rejects_old_or_duplicate_events() {
        let mut evidence = ExposureEvidence::new("session-1".into(), "capture-1".into());
        evidence
            .observe(&CameraCaptureEvent::TransferStarted {
                session_id: "session-1".into(),
                operation_id: "capture-1".into(),
                size: 50,
            })
            .unwrap();
        assert!(
            !evidence.exposure_ended,
            "file transfer does not prove exposure end"
        );
        assert!(evidence
            .observe(&CameraCaptureEvent::ExposureEnded {
                session_id: "old-session".into(),
                operation_id: "capture-1".into()
            })
            .is_err());
        assert!(evidence
            .observe(&CameraCaptureEvent::ExposureEnded {
                session_id: "session-1".into(),
                operation_id: "old-capture".into()
            })
            .is_err());
        assert!(!evidence.exposure_ended);
        let event = CameraCaptureEvent::ExposureEnded {
            session_id: "session-1".into(),
            operation_id: "capture-1".into(),
        };
        evidence.observe(&event).unwrap();
        assert!(evidence.exposure_ended);
        assert!(evidence.observe(&event).is_err());
    }

    #[test]
    fn short_beam_budget_cools_before_starting_the_next_exposure() {
        let now = Instant::now();
        let started = now - Duration::from_secs(591);
        let limit = Duration::from_secs(600);
        let exposure_budget = Duration::from_secs(5);
        assert!(needs_cooldown_before_exposure(
            started,
            now,
            limit,
            exposure_budget + MOVE_BUDGET_MARGIN,
        ));
        assert!(!needs_cooldown_before_exposure(
            started,
            now,
            limit,
            exposure_budget,
        ));
        assert!(needs_cooldown_before_exposure(
            started,
            now + Duration::from_secs(4),
            limit,
            exposure_budget,
        ));
    }

    #[test]
    fn projection_jobs_are_fifo_and_opposite_for_two_views() {
        assert_eq!(projection_job(0, 2).angle_mdeg, 0);
        assert_eq!(projection_job(1, 2).angle_mdeg, -180_000);
        assert_eq!(expected_pulses(-180_000, 96_000), -48_000);
        assert_eq!(expected_pulses(-360_000, 96_000), 0);
        let jobs: Vec<_> = (0..180).map(|index| projection_job(index, 180)).collect();
        assert_eq!(jobs.len(), 180);
        assert!(jobs
            .iter()
            .enumerate()
            .all(|(index, job)| job.index == index as u32 + 1
                && job.angle_mdeg == -(index as i32 * 2_000)));
        assert_eq!(jobs.last().unwrap().angle_mdeg, -358_000);
    }

    #[test]
    fn estimate_requires_completed_sample_and_hides_operator_pause_or_fault() {
        let mut state = RealScanProgress::default();
        state.projection_count = 3;
        state.refresh_estimate(Instant::now());
        assert_eq!(state.estimated_remaining_seconds, None);
        state.captured = 1;
        state.measured_projection_time = Some(Duration::from_secs(12));
        state.block_limit = Duration::from_secs(10);
        state.refresh_estimate(Instant::now());
        assert_eq!(state.estimated_remaining_seconds, Some(324));
        state.phase = "paused";
        state.refresh_estimate(Instant::now());
        assert_eq!(state.estimated_remaining_seconds, None);
        state.phase = "fault";
        state.refresh_estimate(Instant::now());
        assert_eq!(state.estimated_remaining_seconds, None);
    }

    #[test]
    fn estimate_does_not_count_a_completed_cooldown_twice() {
        let now = Instant::now();
        let mut state = RealScanProgress::default();
        state.projection_count = 360;
        state.captured = 133;
        state.measured_projection_time = Some(Duration::from_secs(9));
        state.projection_started = Some(now - Duration::from_secs(309));
        state.projection_cooling_elapsed = Duration::from_secs(300);
        record_projection_sample(&mut state, 134, now);
        assert_eq!(state.measured_projection_time, Some(Duration::from_secs(9)));
        state.captured = 134;
        state.block_limit = Duration::from_secs(600);
        state.refresh_estimate(now);
        assert_eq!(state.estimated_remaining_seconds, Some(2_934));
    }

    #[test]
    fn cooldown_countdown_uses_monotonic_elapsed_time() {
        let now = Instant::now();
        let mut state = RealScanProgress::default();
        state.cooldown_until = Some(now + XRAY_COOLDOWN);
        state.refresh_estimate(now);
        assert_eq!(state.cooldown_remaining_seconds, Some(300));
        state.refresh_estimate(now + Duration::from_secs(301));
        assert_eq!(state.cooldown_remaining_seconds, Some(0));
    }

    #[test]
    fn estimate_overlaps_remaining_off_interval_with_current_file_work() {
        let now = Instant::now();
        let mut state = RealScanProgress::default();
        state.projection_count = 2;
        state.captured = 1;
        state.measured_projection_time = Some(Duration::from_secs(10));
        state.projection_started = Some(now - Duration::from_secs(4));
        state.cooldown_until = Some(now + Duration::from_secs(300));
        state.refresh_estimate(now);
        assert_eq!(state.estimated_remaining_seconds, Some(300));
    }

    fn card_record(index: u32, kind: &str, state: &str, imported: bool) -> CardCaptureRecord {
        CardCaptureRecord {
            index,
            kind: kind.into(),
            angle_deg: -(index as f64),
            exposure_ms: 200.0,
            state: state.into(),
            camera_session_id: "session-card".into(),
            operation_id: format!("op-{index}"),
            storage_id: 0x0001_0001,
            storage_changed: false,
            handle: 0x0000_0100 + index,
            parent: 0x0000_0001,
            source_file_name: format!("DSC_{index:04}.NEF"),
            format: 0x3000,
            size: 40_000_000,
            capture_date: "20261010T120000".into(),
            imported_file: imported.then(|| RealFrame {
                index,
                angle_deg: -(index as f64),
                exposure_ms: 200.0,
                file_name: format!("frame-{index:04}.nef"),
                path: format!("/data/frame-{index:04}.nef"),
                bytes: 40_000_000,
                sha256: format!("{index:064x}"),
            }),
        }
    }

    #[test]
    fn schema_four_completion_requires_a_verified_import_and_reconstructable_records() {
        let profile = crate::capture_profile::test_card_profile("single");
        let card = crate::capture_profile::CaptureDestination::CameraCard;
        let mut records: Vec<CardCaptureRecord> = (1..=3)
            .map(|index| card_record(index, "projection", DATA_VERIFIED, true))
            .collect();
        records.extend((1..=2).map(|index| card_record(index, "preDark", DATA_VERIFIED, true)));
        let consistent = |records: &[CardCaptureRecord], acquisition: &str, data: &str, projections: u32| {
            schema_four_completion_consistent(
                Some(card),
                Some(&profile),
                Some(acquisition),
                Some(data),
                records,
                projections,
            )
        };
        assert!(consistent(&records, ACQUISITION_ACQUIRED, DATA_VERIFIED, 3));
        // Safe finishing alone is not a completed project while the import is open.
        assert!(!consistent(&records, ACQUISITION_ACQUIRED, DATA_PENDING_IMPORT, 3));
        assert!(!consistent(&records, ACQUISITION_ACQUIRED, DATA_IMPORTING, 3));
        assert!(!consistent(&records, ACQUISITION_ACQUIRED, DATA_IMPORT_FAILED, 3));
        assert!(!consistent(&records, "running", DATA_VERIFIED, 3));
        // One projection without a reconstructable record blocks completion.
        assert!(!consistent(&records, ACQUISITION_ACQUIRED, DATA_VERIFIED, 4));
        let mut missing_file = records.clone();
        missing_file[0].imported_file = None;
        assert!(!consistent(&missing_file, ACQUISITION_ACQUIRED, DATA_VERIFIED, 3));
        let mut regressed = records.clone();
        regressed[1].state = DATA_PENDING_IMPORT.into();
        assert!(!consistent(&regressed, ACQUISITION_ACQUIRED, DATA_VERIFIED, 3));
        // An unverified reference frame blocks completion too.
        let mut reference_open = records.clone();
        reference_open[3].imported_file = None;
        assert!(!consistent(&reference_open, ACQUISITION_ACQUIRED, DATA_VERIFIED, 3));
        // Only the card destination with a card profile can complete schema 4.
        assert!(!schema_four_completion_consistent(
            Some(crate::capture_profile::CaptureDestination::HostRam),
            Some(&profile),
            Some(ACQUISITION_ACQUIRED),
            Some(DATA_VERIFIED),
            &records,
            3,
        ));
        assert!(!schema_four_completion_consistent(
            Some(card),
            Some(&crate::capture_profile::test_profile()),
            Some(ACQUISITION_ACQUIRED),
            Some(DATA_VERIFIED),
            &records,
            3,
        ));
        assert!(!schema_four_completion_consistent(
            None,
            Some(&profile),
            Some(ACQUISITION_ACQUIRED),
            Some(DATA_VERIFIED),
            &records,
            3,
        ));
    }

    #[test]
    fn card_records_keep_object_provenance_and_attach_the_verified_file_later() {
        let receipt = crate::devices::camera::CardObjectReceipt {
            session_id: "session-card".into(),
            operation_id: "op-1".into(),
            storage_id: 0x0001_0001,
            storage_changed: false,
            handle: 0x0000_0100,
            parent: 0x0000_0001,
            filename: "DSC_0100.NEF".into(),
            format: 0x3000,
            size: 40_000_000,
            capture_date: "20261010T120000".into(),
        };
        let mut record = CardCaptureRecord::from_receipt(7, "projection", -7.0, 200.0, &receipt);
        assert_eq!(record.state, DATA_PENDING_IMPORT);
        assert!(record.imported_file.is_none());
        assert!(!card_record_is_reconstructable(&record));
        assert_eq!(record.source_file_name, "DSC_0100.NEF");
        assert_eq!(record.storage_id, 0x0001_0001);
        assert_eq!(record.camera_session_id, "session-card");
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["sourceFileName"], "DSC_0100.NEF");
        assert_eq!(json["storageChanged"], false);
        assert!(json.get("importedFile").is_none());
        assert_eq!(
            serde_json::from_value::<CardCaptureRecord>(json).unwrap(),
            record
        );
        // After a verified import the record carries the local identity.
        record.state = DATA_VERIFIED.into();
        record.imported_file = Some(RealFrame {
            index: 7,
            angle_deg: -7.0,
            exposure_ms: 200.0,
            file_name: "frame-0007.nef".into(),
            path: "/data/frame-0007.nef".into(),
            bytes: 40_000_000,
            sha256: "aa".repeat(32),
        });
        assert!(card_record_is_reconstructable(&record));
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["state"], DATA_VERIFIED);
        assert_eq!(json["importedFile"]["fileName"], "frame-0007.nef");
        assert_eq!(
            serde_json::from_value::<CardCaptureRecord>(json).unwrap(),
            record
        );
        // Unknown fields are refused so a schema drift cannot pass silently.
        let mut drifted = serde_json::to_value(&record).unwrap();
        drifted["unknownField"] = serde_json::json!(1);
        assert!(serde_json::from_value::<CardCaptureRecord>(drifted).is_err());
        // Only the declared data states are accepted by the completion rule.
        assert!(card_data_status_valid(DATA_PENDING_IMPORT));
        assert!(card_data_status_valid(DATA_IMPORTING));
        assert!(card_data_status_valid(DATA_IMPORT_FAILED));
        assert!(card_data_status_valid(DATA_VERIFIED));
        assert!(!card_data_status_valid("import-unknown"));
        assert!(!card_data_status_valid(""));
    }

    // ---- Card acquisition (schema 4 / profile 7) -------------------------------
    // Every card test runs the production coordinator and the real camera COM
    // owner over the scripted card session: only the physical WPD boundary and the
    // controllers are scripted, and no device is contacted. PTP opcodes are spelled
    // out because the vendor protocol constants are private to the camera module.

    const CARD_TEST_GIB: u64 = 1024 * 1024 * 1024;
    const CARD_OP_GET_OBJECT: u16 = 0x1009;
    const CARD_OP_GET_PARTIAL_OBJECT: u16 = 0x101b;
    const CARD_OP_GET_THUMB: u16 = 0x100a;
    const CARD_OP_INITIATE_CAPTURE: u16 = 0x100e;
    const CARD_OP_GET_OBJECT_INFO: u16 = 0x1008;
    const CARD_OP_GET_STORAGE_IDS: u16 = 0x1004;
    const CARD_OP_GET_STORAGE_INFO: u16 = 0x1005;
    const CARD_OP_GET_EVENT: u16 = 0x90c7;
    const CARD_OP_GET_PROPERTY: u16 = 0x1015;
    const CARD_OP_GET_PROPERTY_DESC: u16 = 0x1014;
    /// The largest object dataset a card operation may request. A larger read
    /// would be an image-sized transfer, which a card scan must never issue.
    const CARD_OBJECT_INFO_MAX: usize = 64 * 1024;

    macro_rules! card_scripted_camera {
        () => {
            crate::devices::camera::scripted_camera_with_probe(
                crate::devices::camera::ScriptedCameraConfig {
                    card_capture: true,
                    exposure_delay: Duration::from_millis(2),
                    ..Default::default()
                },
            )
        };
    }

    /// Every read a card scan may request after it started: control, event,
    /// property and bounded object/storage metadata. An image read refuses the
    /// assertion; the connect-time identity read is excluded by the caller's mark.
    macro_rules! assert_card_reads_are_metadata_only {
        ($reads:expr) => {{
            for &(code, max) in $reads {
                assert!(
                    matches!(
                        code,
                        CARD_OP_GET_EVENT | CARD_OP_GET_PROPERTY | CARD_OP_GET_PROPERTY_DESC
                    ) || code == CARD_OP_GET_OBJECT_INFO && max == CARD_OBJECT_INFO_MAX
                        || matches!(code, CARD_OP_GET_STORAGE_IDS | CARD_OP_GET_STORAGE_INFO),
                    "card scan requested an unplanned read: opcode=0x{code:04X} max={max}"
                );
            }
        }};
    }

    /// No image payload operation may ever be issued by a card scan.
    macro_rules! assert_no_card_image_operation {
        ($probe:expr) => {{
            for opcode in [
                CARD_OP_GET_OBJECT,
                CARD_OP_GET_PARTIAL_OBJECT,
                CARD_OP_GET_THUMB,
            ] {
                assert_eq!(
                    $probe.operation_count(opcode),
                    0,
                    "no image payload operation is permitted during a card scan (0x{opcode:04X})"
                );
            }
        }};
    }

    fn card_run(topic: &str, task: &str) -> (std::path::PathBuf, std::path::PathBuf, Parameters) {
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/ct-engine")
            .join(format!(
                "{}-card-scan-{topic}-{}",
                chrono::Local::now().format("%Y%m%d-%H%M%S"),
                chrono::Utc::now().timestamp_nanos_opt().unwrap()
            ));
        fs::create_dir_all(&run).unwrap();
        let _ = fs::write(
            run.join("command.txt"),
            "cargo test -p ct-engine scan::tests::card -- --nocapture\nScripted card session only; no real device is contacted.\n",
        );
        let parameters = Parameters {
            task_id: task.into(),
            save_path: run.to_string_lossy().into(),
            projection_count: 3,
            angle_step_deg: 120.0,
            exposure_ms: 200.0,
        };
        let root = run.join(task);
        (run, root, parameters)
    }

    fn card_profile(camera: &CameraAdapter, overflow: bool) -> CaptureProfile {
        let policy = card_capture_policy(&card_controller_identity(), overflow).unwrap();
        CaptureProfile::from_card_readback(
            &camera.health(),
            camera.card_profile_readback().unwrap(),
            None,
            policy,
        )
        .unwrap()
    }

    /// The pinned controller identity a new card acquisition freezes. The
    /// scan-level tests pass the profile directly, so only the policy provenance
    /// depends on this fixture.
    fn card_controller_identity() -> NanoIdentity {
        NanoIdentity {
            device: "RTS9060_NANO".into(),
            version: CARD_FIRMWARE_VERSION.into(),
            protocol: 2,
            build: CARD_FIRMWARE_BUILD.into(),
            buzzer_installed: true,
            capabilities: vec![
                "HEARTBEAT_ACK".into(),
                "XRAY_WARNING".into(),
                CARD_SETTLE_CAPABILITY.into(),
            ],
        }
    }

    fn card_scan_config(
        run: &Path,
        parameters: &Parameters,
        camera: &CameraAdapter,
        overflow: bool,
    ) -> RealScanConfig {
        let _ = run;
        RealScanConfig {
            parameters: parameters.clone(),
            capture_profile: card_profile(camera, overflow),
            max_xray_sec: SCAN_MAX_XRAY_SEC,
            voltage_kv: 20.0,
            current_ua: 100.0,
            geometry: ScanGeometry {
                sod_mm: 100.0,
                object_to_detector_mm: 100.0,
                detector_width_mm: 40.0,
                center_offset_x_mm: 0.0,
                center_offset_y_mm: 0.0,
                rotation_direction: "counterclockwise".into(),
                mirror_x: false,
                measurement: "measured".into(),
                scintillator_layers: vec![],
            },
            resume: None,
        }
    }

    /// A PTP string: UINT8 count of UTF-16 units including the NUL terminator.
    fn card_ptp_string(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let mut bytes = vec![units.len() as u8];
        bytes.extend(units.into_iter().flat_map(u16::to_le_bytes));
        bytes
    }

    /// Card ObjectInfo metadata of one NEF object in the declared PTP field order:
    /// storage, format, protection, size, the 26-byte thumbnail block, parent, the
    /// 10-byte association block, filename and the D7100 date strings.
    fn card_nef_object(storage_id: u32, filename: &str) -> Vec<u8> {
        let mut bytes = storage_id.to_le_bytes().to_vec();
        bytes.extend(0x3800u16.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(26_673_241u32.to_le_bytes());
        bytes.extend([0u8; 26]);
        bytes.extend(0x0002_0005u32.to_le_bytes());
        bytes.extend([0u8; 10]);
        bytes.extend(card_ptp_string(filename));
        bytes.extend(card_ptp_string("20261011T120000"));
        bytes.extend(card_ptp_string("20261011T120000"));
        bytes.extend(card_ptp_string(""));
        bytes
    }

    /// Scripts one complete card exposure association for every trigger: the card
    /// object metadata the handle answers plus the `ObjectAdded`/card-completion
    /// batches the trigger releases.
    macro_rules! script_card_exposure {
        ($probe:expr, $storage:expr, $handle:expr, $filename:expr) => {{
            $probe.set_card_object($handle, card_nef_object($storage, $filename));
            $probe.set_card_events(vec![
                vec![(0x4002 /*ObjectAdded*/, $handle)],
                vec![(0x400d /*CaptureCompleteCard*/, 0)],
            ]);
        }};
    }

    /// The task directory of a card scan holds only its manifest and ledger: no
    /// `frames/` directory, no local image and no copied NEF may appear.
    fn assert_no_local_card_file(root: &Path) {
        assert!(
            !root.join("frames").exists(),
            "a card scan must not create a local frames/ directory"
        );
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            assert!(
                !name.ends_with(".nef") && !name.ends_with(".tif") && !name.ends_with(".raw"),
                "a card scan wrote a local image: {}",
                path.display()
            );
        }
    }

    #[test]
    fn card_scan_end_to_end_keeps_every_image_on_the_card_and_registers_pending_import() {
        let (run, root, parameters) = card_run("end-to-end", "card-task");
        let (camera, probe) = card_scripted_camera!();
        const HANDLE: u32 = 0x000b_0001;
        script_card_exposure!(probe, 1, HANDLE, "DSC_0001.NEF");
        // The durable registration exists before the acquisition, exactly like the
        // domain path, so the registry transition is evaluated against a real record.
        let mut registry = crate::task_registry::TaskRegistry::load(run.join("active-task.json"));
        registry.register(&parameters).unwrap();
        let (nano, _nano_state) = pipeline_nano();
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config = card_scan_config(&run, &parameters, &camera, false);
        let read_mark = probe.reads().len();
        let mut handle = RealScanHandle::start(nano, camera, xray, config);
        let deadline = Instant::now() + Duration::from_secs(90);
        let outcome = loop {
            if let Some(stage) = handle.progress().prompt {
                let _ = handle.confirm_stage(stage);
            }
            if let Some(outcome) = handle.try_finish() {
                break outcome.unwrap();
            }
            if Instant::now() >= deadline {
                handle.cancel_only();
                panic!("card scan did not finish: {:?}", handle.progress());
            }
            thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(
            outcome.result.as_ref().unwrap(),
            &ScanCompletion::AcquiredPendingImport,
            "a card acquisition finishes as pending import, never as a completed project"
        );
        assert!(!outcome.xray.health().beam_on, "the beam must end OFF");
        assert!(outcome.xray.health().beam_off_confirmed);
        assert!(!xray_probe.output_enabled());
        let progress = handle.progress();
        let records = progress.card_records.clone();
        assert_eq!(
            records.iter().filter(|record| record.kind == "projection").count(),
            3
        );
        assert_eq!(
            records.iter().filter(|record| record.kind == "preDark").count(),
            10
        );
        assert_eq!(
            records.iter().filter(|record| record.kind == "preFlat").count(),
            10
        );
        assert!(records.iter().all(|record| {
            record.state == DATA_PENDING_IMPORT
                && record.imported_file.is_none()
                && record.storage_id != 0
                && record.handle != 0
                && !record.source_file_name.is_empty()
                && !record.operation_id.is_empty()
        }));
        // A card scan keeps no local frame and no local reference record.
        assert!(progress.frames.is_empty());
        assert!(progress.references.pre_dark.is_empty());
        assert!(progress.references.pre_flat.is_empty());
        let manifest: serde_json::Value =
            serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["schemaVersion"], 4);
        assert_eq!(manifest["captureDestination"], "cameraCard");
        assert_eq!(manifest["acquisitionStatus"], ACQUISITION_ACQUIRED);
        assert_eq!(manifest["dataStatus"], DATA_PENDING_IMPORT);
        assert_eq!(manifest["completed"], false);
        assert!(manifest["frames"].as_array().unwrap().is_empty());
        assert!(manifest["references"]["preDark"].as_array().unwrap().is_empty());
        assert_eq!(manifest["captureRecords"].as_array().unwrap().len(), 23);
        assert!(manifest["captureRecords"]
            .as_array()
            .unwrap()
            .iter()
            .all(|record| record["state"] == DATA_PENDING_IMPORT
                && record.get("importedFile").is_none()));
        // One intent and one receipt per exposure: no crash window remains.
        let ledger = read_card_journal(&root).unwrap();
        assert_eq!(
            ledger
                .iter()
                .filter(|entry| entry.kind == CARD_JOURNAL_KIND_INTENT)
                .count(),
            23
        );
        assert_eq!(
            ledger
                .iter()
                .filter(|entry| entry.kind == CARD_JOURNAL_KIND_RECEIPT)
                .count(),
            23
        );
        assert!(unresolved_card_intents(&root).unwrap().is_empty());
        assert!(card_acquisition_finish_gate(&root).is_ok());
        // Zero image transfer for the whole scan: no GetObject/GetPartialObject/
        // GetThumb, one bounded metadata read per exposure, no local file at all.
        assert_no_card_image_operation!(probe);
        let reads = probe.reads();
        assert_card_reads_are_metadata_only!(&reads[read_mark..]);
        assert_eq!(
            reads[read_mark..]
                .iter()
                .filter(|(code, _)| *code == CARD_OP_GET_OBJECT_INFO)
                .count(),
            23,
            "one bounded card metadata read per exposure"
        );
        assert_no_local_card_file(&root);
        assert!(!completed_manifest_exists(&parameters));
        // The domain transition: the task leaves the active slot, the directory
        // stays reserved, no hardware lock remains and nothing is completed.
        registry.mark_acquired_pending_import(&parameters).unwrap();
        assert!(!registry.locked(), "safe finishing releases the scan lock");
        assert_eq!(registry.pending_import_tasks().len(), 1);
        assert!(registry.check_retained_location(&parameters).unwrap());
        assert_eq!(
            registry.snapshot()["retained"][0]["status"],
            crate::task_registry::TASK_STATUS_ACQUIRED_PENDING_IMPORT
        );
        assert_ne!(registry.snapshot()["retained"][0]["status"], "completed");
        assert!(registry.mark_import_verified(&parameters).is_err());
    }

    #[test]
    fn a_stop_during_a_card_projection_leaves_that_object_unresolved_and_the_task_retained() {
        let (run, root, parameters) = card_run("stop", "card-task");
        let (camera, probe) = card_scripted_camera!();
        const HANDLE: u32 = 0x000b_0001;
        script_card_exposure!(probe, 1, HANDLE, "DSC_0001.NEF");
        let mut registry = crate::task_registry::TaskRegistry::load(run.join("active-task.json"));
        registry.register(&parameters).unwrap();
        let (nano, _nano_state) = pipeline_nano();
        let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
        let config = card_scan_config(&run, &parameters, &camera, false);
        let mut handle = RealScanHandle::start(nano, camera.clone(), xray, config);
        let deadline = Instant::now() + Duration::from_secs(90);
        let projection_prefix = format!("scan-{}-1-", parameters.task_id);
        let mut delayed = false;
        let mut stopped = false;
        let outcome = loop {
            let progress = handle.progress();
            if let Some(stage) = progress.prompt {
                let _ = handle.confirm_stage(stage);
            }
            // Slow the projection exposure down once the projection stage starts,
            // so the operator Stop deterministically lands inside it.
            if !delayed && progress.stage == "projections" {
                probe.set_exposure_delay(Duration::from_millis(900));
                delayed = true;
            }
            if delayed && !stopped {
                let health = camera.health();
                if health
                    .busy
                    && health
                        .operation_id
                        .as_deref()
                        .is_some_and(|id| id.starts_with(&projection_prefix))
                {
                    handle.request_stop().unwrap();
                    stopped = true;
                }
            }
            if let Some(outcome) = handle.try_finish() {
                break outcome.unwrap();
            }
            if Instant::now() >= deadline {
                handle.cancel_only();
                panic!("card stop test did not terminate: {:?}", handle.progress());
            }
            thread::sleep(Duration::from_millis(2));
        };
        assert!(stopped, "the Stop must land inside a card projection exposure");
        // Never a completed or safely finished acquisition: the interrupted
        // exposure has an intent and no receipt.
        match &outcome.result {
            Ok(ScanCompletion::Stopped) => {}
            Err(error) => assert!(
                error.contains("isolated") || error.contains("unconfirmed"),
                "unexpected card stop outcome: {error}"
            ),
            Ok(other) => panic!("a stopped card scan must not report {other:?}"),
        }
        assert!(!outcome.xray.health().beam_on, "beam OFF is mandatory");
        assert!(outcome.xray.health().beam_off_confirmed);
        assert!(!xray_probe.output_enabled());
        let unresolved = unresolved_card_intents(&root).unwrap();
        assert_eq!(
            unresolved.len(),
            1,
            "exactly the interrupted exposure stays unresolved: {unresolved:?}"
        );
        assert_eq!(unresolved[0].group, "projection");
        let interrupted = unresolved[0].index;
        let manifest: serde_json::Value =
            serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
        assert_ne!(manifest["acquisitionStatus"], ACQUISITION_ACQUIRED);
        assert_eq!(manifest["completed"], false);
        assert!(
            !manifest["captureRecords"]
                .as_array()
                .unwrap()
                .iter()
                .any(|record| record["kind"] == "projection" && record["index"] == interrupted),
            "the interrupted projection must never receive a receipt"
        );
        assert!(manifest["captureRecords"]
            .as_array()
            .unwrap()
            .iter()
            .all(|record| record["kind"] != "projection"));
        // The task keeps its lock: a stopped card acquisition is never promoted to
        // pending import, and nothing can be reconstructed from it.
        assert!(registry.locked(), "a stopped card task stays retained and locked");
        assert!(registry.pending_import_tasks().is_empty());
        assert!(registry.mark_acquired_pending_import(&parameters).is_err());
        assert!(registry.error().is_none());
        assert!(!completed_manifest_exists(&parameters));
        // No image payload, no local file and no local frame appeared.
        assert_no_card_image_operation!(probe);
        assert_no_local_card_file(&root);
    }

    #[test]
    fn card_storage_plan_is_refused_before_any_device_action() {
        // Read-only, insufficient capacity, unknown capacity and insufficient free
        // images are all refused with their stable code, and the scan never
        // triggers, moves or opens the beam.
        for (topic, storage, code) in [
            (
                "read-only",
                (1u32, 1u16, 64 * CARD_TEST_GIB, 64 * CARD_TEST_GIB, 5000u32),
                "CARD_READ_ONLY",
            ),
            (
                "insufficient-bytes",
                (1, 0, 512 * 1024 * 1024, 512 * 1024 * 1024, 5000),
                "CARD_CAPACITY_INSUFFICIENT",
            ),
            (
                "insufficient-images",
                (1, 0, 64 * CARD_TEST_GIB, 64 * CARD_TEST_GIB, 5),
                "CARD_CAPACITY_INSUFFICIENT",
            ),
            ("unknown-capacity", (1, 0, 0, 0, 0), "CARD_CAPACITY_UNKNOWN"),
        ] {
            let (run, root, parameters) = card_run(topic, "card-task");
            let (camera, probe) = card_scripted_camera!();
            probe.set_storages(&[storage]);
            let (nano, nano_state) = pipeline_nano();
            let (xray, xray_probe) = crate::devices::xray::test_worker(20.0, 100.0);
            let config = card_scan_config(&run, &parameters, &camera, false);
            let read_mark = probe.reads().len();
            let beam_mark = xray_probe.beam_transitions().len();
            let mut handle = RealScanHandle::start(nano, camera, xray, config);
            let deadline = Instant::now() + Duration::from_secs(20);
            let outcome = loop {
                if let Some(stage) = handle.progress().prompt {
                    let _ = handle.confirm_stage(stage);
                }
                if let Some(outcome) = handle.try_finish() {
                    break outcome.unwrap();
                }
                if Instant::now() >= deadline {
                    handle.cancel_only();
                    panic!(
                        "refused card plan did not terminate: {:?}",
                        handle.progress()
                    );
                }
                thread::sleep(Duration::from_millis(2));
            };
            let error = outcome
                .result
                .err()
                .unwrap_or_else(|| panic!("{topic}: a storage-refused card scan must not report success"));
            assert!(error.contains(code), "{topic}: {error}");
            assert!(error.contains("CARD_STORAGE_PLAN_REFUSED"), "{topic}: {error}");
            // The enumeration itself is the only card-specific request; no
            // exposure, movement or beam was ever issued.
            assert_eq!(
                probe.operation_count(CARD_OP_INITIATE_CAPTURE),
                0,
                "{topic}: no trigger may be sent"
            );
            assert_eq!(
                probe.operation_count(CARD_OP_GET_STORAGE_IDS),
                1,
                "{topic}: the advertised storages are enumerated exactly once at scan start"
            );
            assert!(probe.operation_count(CARD_OP_GET_STORAGE_INFO) >= 1);
            let reads = probe.reads();
            assert_card_reads_are_metadata_only!(&reads[read_mark..]);
            assert_no_card_image_operation!(probe);
            let writes = nano_state.lock().unwrap().writes.clone();
            assert!(
                !writes.iter().any(|line| line.starts_with("MOVE_REL ")
                    || line.starts_with("MOVE_ABS ")
                    || line.starts_with("HOME ")
                    || line.starts_with("CAPTURE_DONE ")),
                "{topic}: no motion may be issued for a refused card plan: {writes:?}"
            );
            let transitions = xray_probe.beam_transitions();
            // A refused card plan must never switch the beam ON. A safety OFF
            // before the refusal is allowed (and desirable), so only an ON
            // transition violates this invariant.
            assert!(
                transitions[beam_mark..].iter().all(|(on, _)| !on),
                "{topic}: no beam ON may happen for a refused card plan: {:?}",
                &transitions[beam_mark..]
            );
            assert!(!xray_probe.output_enabled(), "{topic}: output stays OFF");
            assert!(
                !root.join(CARD_JOURNAL_FILE).exists(),
                "{topic}: a refused plan must not persist an exposure intent"
            );
            // A refused plan is never an acquisition: whatever checkpoint the
            // failure path leaves behind stays unacquired with no card record, and
            // it can never be read as a completed task.
            assert!(!completed_manifest_exists(&parameters), "{topic}");
            if root.join("manifest.json").is_file() {
                let manifest: serde_json::Value =
                    serde_json::from_reader(File::open(root.join("manifest.json")).unwrap())
                        .unwrap();
                assert_ne!(manifest["acquisitionStatus"], ACQUISITION_ACQUIRED, "{topic}");
                assert_eq!(manifest["completed"], false, "{topic}");
                assert!(
                    manifest["captureRecords"].as_array().unwrap().is_empty(),
                    "{topic}: no card receipt may exist for a refused plan"
                );
            }
        }
    }

    #[test]
    fn card_intent_without_receipt_is_detected_and_never_counted_as_acquired() {
        let (run, root, parameters) = card_run("crash-window", "card-task");
        let (camera, _probe) = card_scripted_camera!();
        let config = card_scan_config(&run, &parameters, &camera, false);
        // The durable registration exists before any acquisition file, exactly like
        // the domain path, so the crash window is evaluated against a real record.
        let mut registry = crate::task_registry::TaskRegistry::load(run.join("active-task.json"));
        registry.register(&parameters).unwrap();
        fs::create_dir_all(&root).unwrap();
        let session = "card-crash-session";
        let operation = "scan-card-task-1-1";
        let intent = CardJournalEntry {
            kind: CARD_JOURNAL_KIND_INTENT.into(),
            index: 1,
            group: "projection".into(),
            angle_deg: 0.0,
            exposure_ms: 200.0,
            operation_id: operation.into(),
            session_id: session.into(),
            planned_pulse_position: Some(0),
            handle: None,
            storage_id: None,
            source_file_name: None,
            recorded_at: timestamp(),
        };
        append_card_journal(&root, &intent).unwrap();
        // The intent is persisted before the trigger, so it is the only trace of an
        // exposure whose receipt never arrived.
        let unresolved = unresolved_card_intents(&root).unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].operation_id, operation);
        let gate = card_acquisition_finish_gate(&root).unwrap_err();
        assert!(gate.contains("CARD_EXPOSURE_UNRESOLVED"), "{gate}");
        // Even a manifest that claims `acquired` cannot be registered while that
        // intent has no receipt: the exposure is never counted as acquired.
        persist_card(
            &root,
            &parameters,
            &config,
            &[],
            "projections",
            ACQUISITION_ACQUIRED,
            DATA_PENDING_IMPORT,
        )
        .unwrap();
        assert!(registry.mark_acquired_pending_import(&parameters).is_err());
        assert!(registry.locked(), "the crashed card task stays locked");
        assert!(registry.pending_import_tasks().is_empty());
        // A receipt for the same operation resolves the window; the ledger keeps
        // both lines as the acquisition provenance.
        append_card_journal(
            &root,
            &CardJournalEntry {
                kind: CARD_JOURNAL_KIND_RECEIPT.into(),
                operation_id: operation.into(),
                handle: Some(0x0000_0101),
                storage_id: Some(1),
                source_file_name: Some("DSC_0101.NEF".into()),
                recorded_at: timestamp(),
                ..intent.clone()
            },
        )
        .unwrap();
        assert!(unresolved_card_intents(&root).unwrap().is_empty());
        assert!(card_acquisition_finish_gate(&root).is_ok());
        let ledger = read_card_journal(&root).unwrap();
        assert_eq!(ledger.len(), 2, "the ledger keeps the intent and its receipt");
    }

    #[test]
    fn card_plan_allows_overflow_only_with_the_policy_evidence_and_two_writable_storages() {
        use crate::devices::camera::{
            CardPlan, StorageAccess, StorageCapacity, StorageId, StorageInfo,
        };
        let info = |id: u32, access: StorageAccess| StorageInfo {
            id: StorageId(id),
            access,
            capacity: StorageCapacity {
                max_bytes: 64 * CARD_TEST_GIB,
                free_bytes: 64 * CARD_TEST_GIB,
                free_images: 5000,
            },
            storage_type: 4,
            filesystem_type: 2,
            description: "SD card".into(),
            volume_label: String::new(),
        };
        let plan = |storages: Vec<StorageInfo>| CardPlan {
            writable_bytes: 64 * CARD_TEST_GIB * storages.len() as u64,
            writable_images: 5000 * storages.len() as u32,
            storages,
        };
        let single = plan(vec![info(1, StorageAccess::ReadWrite)]);
        let dual = plan(vec![
            info(1, StorageAccess::ReadWrite),
            info(2, StorageAccess::ReadWrite),
        ]);
        // A single writable card is a complete plan and never claims overflow.
        let single_plan = card_capture_plan(&single, &card_policy("single"), false).unwrap();
        assert_eq!(single_plan.expected_objects, 1);
        assert!(!single_plan.allow_overflow);
        assert_eq!(single_plan.storages, vec![StorageId(1)]);
        // Overflow needs the frozen policy, the advertised continuation events and
        // a second writable storage.
        assert!(card_capture_plan(&dual, &card_policy("overflow"), true)
            .unwrap()
            .allow_overflow);
        assert!(
            !card_capture_plan(&single, &card_policy("overflow"), true)
                .unwrap()
                .allow_overflow
        );
        assert!(
            !card_capture_plan(&dual, &card_policy("single"), false)
                .unwrap()
                .allow_overflow
        );
        assert!(
            !card_capture_plan(&dual, &card_policy("overflow"), false)
                .unwrap()
                .allow_overflow
        );
        // No writable storage is refused with the stable code.
        assert_eq!(
            card_capture_plan(&plan(vec![]), &card_policy("single"), false).unwrap_err(),
            "CARD_READ_ONLY: no writable card storage was advertised"
        );
    }

    fn card_policy(mode: &str) -> CardCapturePolicy {
        CardCapturePolicy {
            mode: mode.into(),
            settle_ms: crate::capture_profile::MOVE_SETTLE_MS,
            settle_anchor: crate::capture_profile::MOVE_SETTLE_ANCHOR.into(),
            firmware_version: CARD_FIRMWARE_VERSION.into(),
            firmware_build: CARD_FIRMWARE_BUILD.into(),
            firmware_source_sha256: CARD_FIRMWARE_SOURCE_SHA256.into(),
        }
    }

    #[test]
    fn card_policy_requires_the_pinned_settle_firmware_identity() {
        let identity = card_controller_identity();
        let policy = card_capture_policy(&identity, true).unwrap();
        assert_eq!(policy.mode, "overflow");
        assert_eq!(policy.settle_ms, 150);
        assert_eq!(policy.settle_anchor, "lastStepFallingEdge");
        assert_eq!(policy.firmware_source_sha256, CARD_FIRMWARE_SOURCE_SHA256);
        assert_eq!(card_capture_policy(&identity, false).unwrap().mode, "single");
        // A controller that does not advertise the unified settle contract cannot
        // freeze a card policy: the 150 ms contract would be unproven.
        let mut no_settle = identity.clone();
        no_settle
            .capabilities
            .retain(|capability| capability != CARD_SETTLE_CAPABILITY);
        let error = card_capture_policy(&no_settle, false).unwrap_err();
        assert!(error.contains("CARD_FIRMWARE_UNVERIFIED"), "{error}");
        assert!(error.contains(CARD_SETTLE_CAPABILITY), "{error}");
        // A different version or build cannot be pinned to this source identity.
        for (version, build) in [("2.1.0", CARD_FIRMWARE_BUILD), (CARD_FIRMWARE_VERSION, "offline-test")] {
            let mut drifted = identity.clone();
            drifted.version = version.into();
            drifted.build = build.into();
            let error = card_capture_policy(&drifted, false).unwrap_err();
            assert!(error.contains("CARD_FIRMWARE_UNVERIFIED"), "{error}");
        }
    }

    #[test]
    fn schema_four_manifest_writer_records_the_card_destination_without_local_frames() {
        let (run, root, parameters) = card_run("manifest", "card-task");
        let (camera, _probe) = card_scripted_camera!();
        let config = card_scan_config(&run, &parameters, &camera, false);
        fs::create_dir_all(&root).unwrap();
        let records: Vec<CardCaptureRecord> = (1..=3)
            .map(|index| card_record(index, "projection", DATA_PENDING_IMPORT, false))
            .chain((1..=2).map(|index| card_record(index, "preDark", DATA_PENDING_IMPORT, false)))
            .collect();
        persist_card(
            &root,
            &parameters,
            &config,
            &records,
            "finalizing",
            ACQUISITION_ACQUIRED,
            DATA_PENDING_IMPORT,
        )
        .unwrap();
        let manifest: serde_json::Value =
            serde_json::from_reader(File::open(root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["schemaVersion"], 4);
        assert_eq!(manifest["captureDestination"], "cameraCard");
        assert_eq!(manifest["acquisitionStatus"], ACQUISITION_ACQUIRED);
        assert_eq!(manifest["dataStatus"], DATA_PENDING_IMPORT);
        assert_eq!(manifest["completed"], false);
        assert_eq!(manifest["stage"], "finalizing");
        assert!(manifest["frames"].as_array().unwrap().is_empty());
        assert!(manifest["references"]["preDark"].as_array().unwrap().is_empty());
        assert!(manifest["references"]["preFlat"].as_array().unwrap().is_empty());
        assert_eq!(manifest["captureRecords"].as_array().unwrap().len(), 5);
        assert!(manifest["captureRecords"]
            .as_array()
            .unwrap()
            .iter()
            .all(|record| record["state"] == DATA_PENDING_IMPORT
                && record.get("importedFile").is_none()));
        assert_eq!(manifest["maxXraySec"], SCAN_MAX_XRAY_SEC);
        // A pending-import card task is never a completed or reconstructable
        // project, and the schema-4 reader agrees.
        assert!(!completed_manifest_exists(&parameters));
        assert!(!card_record_is_reconstructable(&records[0]));
        // The host-RAM writer is untouched by the card writer: the same call for a
        // RAM profile still writes schema 3 without card fields.
        let (ram_camera, _) =
            crate::devices::camera::scripted_camera_with_probe(Default::default());
        let ram_config = pipeline_config(&run, "ram-task", &ram_camera);
        let ram_root = run.join("ram-task");
        fs::create_dir_all(ram_root.join("frames")).unwrap();
        persist(
            &ram_root,
            &ram_config.parameters,
            &ram_config,
            &[],
            &ReferenceFrames::default(),
            "preDark",
            false,
        )
        .unwrap();
        let ram_manifest: serde_json::Value =
            serde_json::from_reader(File::open(ram_root.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(ram_manifest["schemaVersion"], 3);
        assert!(ram_manifest.get("captureDestination").is_none());
        assert!(ram_manifest.get("captureRecords").is_none());
    }
}
