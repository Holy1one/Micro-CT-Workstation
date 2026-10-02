//! Scripted transport exercises the production actor/protocol/capture pipeline.
//! No test creates a WPD object, enumerates devices or contacts a camera.

use super::capture::validate_nef;
use super::live_view::{d7100_frame, LatestFrameSlot};
use super::nikon::*;
use super::ptp::*;
use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

struct Script {
    nikon: bool,
    calls: Vec<u16>,
    opens: usize,
    discoveries: usize,
    values: BTreeMap<u16, CameraValue>,
    events: VecDeque<Vec<NikonEvent>>,
    capture_events: Vec<Vec<NikonEvent>>,
    raw: Vec<u8>,
    lv: bool,
    mismatch: Option<u16>,
    stall: Option<(u16, Duration, mpsc::Sender<()>)>,
    exposure_delay: Duration,
    transfer_delay: Duration,
    capture_started: Option<Instant>,
    fail_code: Option<u16>,
    panic_code: Option<u16>,
    close_failure: bool,
}
impl Script {
    fn new() -> Self {
        let default = |key: &str| match key {
            "shutter" => 0x10005,
            "iso" => 200,
            "aperture" => 800,
            "exposureMode" => 1,
            "rawFormat" => 4,
            "driveMode" => 1,
            "focusMode" => 4,
            "exposureDelay" => 3,
            _ => 0,
        };
        Self {
            nikon: true,
            calls: vec![],
            opens: 0,
            discoveries: 0,
            values: NIKON_PROPERTIES
                .iter()
                .map(|p| (p.2, CameraValue::Integer(default(p.0))))
                .collect(),
            events: VecDeque::new(),
            capture_events: vec![
                vec![NikonEvent {
                    code: CAPTURE_COMPLETE_RAM,
                    parameter: 0,
                }],
                vec![NikonEvent {
                    code: OBJECT_RAM,
                    parameter: 0xffff0001,
                }],
            ],
            raw: synthetic_nef(),
            lv: false,
            mismatch: None,
            stall: None,
            exposure_delay: Duration::ZERO,
            transfer_delay: Duration::ZERO,
            capture_started: None,
            fail_code: None,
            panic_code: None,
            close_failure: false,
        }
    }
}
struct Fake(Arc<Mutex<Script>>);
impl CameraTransport for Fake {
    fn discover(&mut self, _: &CallContext) -> Result<Vec<CameraIdentity>, CameraError> {
        self.0.lock().unwrap().discoveries += 1;
        Ok(vec![CameraIdentity {
            device_id: "fake-camera-1".into(),
            model: "unverified discovery".into(),
            ..Default::default()
        }])
    }
    fn open(&mut self, id: &str, _: &CallContext) -> Result<(), CameraError> {
        if id != "fake-camera-1" {
            return Err(CameraError::Unavailable("bad selected ID".into()));
        }
        self.0.lock().unwrap().opens += 1;
        Ok(())
    }
    fn close(&mut self) -> Result<(), CameraError> {
        if self.0.lock().unwrap().close_failure {
            Err(CameraError::Io("scripted camera close failure".into()))
        } else {
            Ok(())
        }
    }
    fn cancel(&mut self) -> Result<(), CameraError> {
        Ok(())
    }
    fn execute(
        &mut self,
        code: u16,
        params: &[u32],
        phase: DataPhase,
        context: &CallContext,
    ) -> Result<PtpResponse, CameraError> {
        context.check()?;
        if self.0.lock().unwrap().panic_code == Some(code) {
            panic!("scripted native transport panic");
        }
        if self.0.lock().unwrap().fail_code == Some(code) {
            return Err(CameraError::Unavailable(
                "scripted camera disconnected".into(),
            ));
        }
        let stall = {
            let mut script = self.0.lock().unwrap();
            script.calls.push(code);
            if script.stall.as_ref().is_some_and(|s| s.0 == code) {
                script.stall.take()
            } else {
                None
            }
        };
        if let Some((_, duration, started)) = stall {
            let _ = started.send(());
            std::thread::sleep(duration);
            context.check()?;
        }
        if code == GET_OBJECT {
            let delay = self.0.lock().unwrap().transfer_delay;
            let deadline = Instant::now() + delay;
            while Instant::now() < deadline {
                context.check()?;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let mut s = self.0.lock().unwrap();
        let data = match code {
            GET_DEVICE_INFO => device_info(s.nikon),
            GET_PROPERTY_DESC => {
                let code = params[0] as u16;
                let actual = s.values.get(&code).unwrap();
                let values: Vec<i64> = if code == 0xd100 {
                    vec![0x11f40, 0x1000a, 0x10008, 0x10005, 0x1e0001, 0xffffffff]
                } else {
                    vec![0, 1, 2, 3, 4, 5, 12, 14, 100, 200, 400, 800]
                };
                let mut b = code.to_le_bytes().to_vec();
                b.extend(6u16.to_le_bytes());
                b.push(1);
                b.extend(encode_value(actual, 6)?);
                b.extend(encode_value(actual, 6)?);
                b.push(2);
                b.extend((values.len() as u16).to_le_bytes());
                for value in values {
                    b.extend((value as u32).to_le_bytes());
                }
                b
            }
            GET_PROPERTY if params[0] as u16 == LV_STATUS => vec![s.lv as u8],
            GET_PROPERTY => encode_value(s.values.get(&(params[0] as u16)).unwrap(), 6)?,
            SET_PROPERTY => {
                let property = params[0] as u16;
                if let DataPhase::Write(bytes) = phase {
                    if s.mismatch != Some(property) {
                        s.values.insert(property, parse_value(&bytes, 6)?);
                    }
                } else {
                    panic!("property write has no data phase");
                }
                vec![]
            }
            CAPTURE_RAM => {
                assert_eq!(params, [0xffffffff]);
                let events = s.capture_events.clone();
                s.events.extend(events);
                s.capture_started = Some(Instant::now());
                vec![]
            }
            GET_EVENT => {
                let events = if s
                    .capture_started
                    .is_some_and(|t| t.elapsed() < s.exposure_delay)
                {
                    vec![]
                } else {
                    s.events.pop_front().unwrap_or_default()
                };
                event_bytes(&events)
            }
            DEVICE_READY | AF_DRIVE | FOCUS_DRIVE => vec![],
            GET_OBJECT_INFO => {
                assert_eq!(params, [0xffff0001]);
                object_info(s.raw.len())
            }
            GET_OBJECT => {
                assert_eq!(params, [0xffff0001]);
                s.raw.clone()
            }
            START_LV => {
                s.lv = true;
                vec![]
            }
            STOP_LV => {
                s.lv = false;
                vec![]
            }
            GET_LV => lv_bytes(),
            _ => {
                return Err(CameraError::Unsupported(format!(
                    "unhandled scripted code {code:04x}"
                )))
            }
        };
        Ok(PtpResponse { code: OK, data })
    }
}
fn adapter(script: Arc<Mutex<Script>>) -> CameraAdapter {
    CameraAdapter::with_factory(move || Ok(Box::new(Fake(script)))).unwrap()
}
fn connected(script: Arc<Mutex<Script>>) -> CameraAdapter {
    let a = adapter(script);
    a.connect_selected("fake-camera-1").unwrap();
    a
}
#[derive(Clone, Default)]
pub(crate) struct ScriptedCameraConfig {
    pub exposure_delay: Duration,
    pub transfer_delay: Duration,
    pub omit_exposure_end: bool,
    pub truncate_raw: bool,
    pub close_failure: bool,
}
/// Crate integration tests run the real camera owner and capture coordinator,
/// replacing only the physical WPD boundary with deterministic PTP datasets.
pub(crate) fn scripted_camera(config: ScriptedCameraConfig) -> CameraAdapter {
    let mut script = Script::new();
    script.exposure_delay = config.exposure_delay;
    script.transfer_delay = config.transfer_delay;
    script.close_failure = config.close_failure;
    if config.omit_exposure_end {
        script
            .capture_events
            .retain(|events| events.iter().all(|e| e.code != CAPTURE_COMPLETE_RAM));
    }
    if config.truncate_raw {
        script.raw.truncate(170);
    }
    connected(Arc::new(Mutex::new(script)))
}
fn ptp_string(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let mut b = vec![units.len() as u8];
    b.extend(units.into_iter().flat_map(u16::to_le_bytes));
    b
}
fn array(values: &[u16]) -> Vec<u8> {
    let mut b = (values.len() as u32).to_le_bytes().to_vec();
    for n in values {
        b.extend(n.to_le_bytes());
    }
    b
}
fn device_info(nikon: bool) -> Vec<u8> {
    let mut b = 100u16.to_le_bytes().to_vec();
    b.extend((if nikon { 10u32 } else { 11 }).to_le_bytes());
    b.extend(100u16.to_le_bytes());
    b.extend(ptp_string("extension"));
    b.extend(0u16.to_le_bytes());
    b.extend(array(&[
        GET_DEVICE_INFO,
        GET_PROPERTY,
        GET_PROPERTY_DESC,
        SET_PROPERTY,
        CAPTURE_RAM,
        DEVICE_READY,
        GET_EVENT,
        GET_OBJECT,
        GET_OBJECT_INFO,
        START_LV,
        STOP_LV,
        GET_LV,
        AF_DRIVE,
        FOCUS_DRIVE,
    ]));
    b.extend(array(&[OBJECT_RAM, CAPTURE_COMPLETE_RAM]));
    let mut properties: Vec<u16> = NIKON_PROPERTIES.iter().map(|p| p.2).collect();
    properties.push(LV_STATUS);
    b.extend(array(&properties));
    b.extend(array(&[0xb101]));
    b.extend(array(&[0xb101]));
    for text in [
        if nikon { "Nikon Corporation" } else { "Canon" },
        if nikon { "NIKON D7100" } else { "EOS unknown" },
        "firmware-test",
        "fake-serial",
    ] {
        b.extend(ptp_string(text));
    }
    b
}
fn event_bytes(events: &[NikonEvent]) -> Vec<u8> {
    let mut b = (events.len() as u16).to_le_bytes().to_vec();
    for e in events {
        b.extend(e.code.to_le_bytes());
        b.extend(e.parameter.to_le_bytes());
    }
    b
}
fn object_info(size: usize) -> Vec<u8> {
    let mut b = 0u32.to_le_bytes().to_vec();
    b.extend(0xb101u16.to_le_bytes());
    b.extend(0u16.to_le_bytes());
    b.extend((size as u32).to_le_bytes());
    b.extend([0; 40]);
    b.extend(ptp_string("DSC_0001.NEF"));
    b.extend([0; 3]);
    b
}
fn jpeg() -> Vec<u8> {
    let mut b = vec![];
    image::codecs::jpeg::JpegEncoder::new(&mut b)
        .encode(&vec![80; 8 * 6 * 3], 8, 6, image::ExtendedColorType::Rgb8)
        .unwrap();
    b
}
fn lv_bytes() -> Vec<u8> {
    let mut b = vec![0; 384];
    b[8..10].copy_from_slice(&8u16.to_le_bytes());
    b[10..12].copy_from_slice(&6u16.to_le_bytes());
    b.extend(jpeg());
    b
}
fn synthetic_nef() -> Vec<u8> {
    // Hand-built, fully bounded TIFF CFA payload used only for file-integrity
    // tests. It is explicitly synthetic and not a scientific Nikon sample.
    let mut b = vec![0; 256];
    b[..8].copy_from_slice(b"II\x2a\0\x08\0\0\0");
    b[8..10].copy_from_slice(&8u16.to_le_bytes());
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
    for (i, (tag, kind, count, value)) in tags.into_iter().enumerate() {
        let at = 10 + i * 12;
        b[at..at + 2].copy_from_slice(&tag.to_le_bytes());
        b[at + 2..at + 4].copy_from_slice(&kind.to_le_bytes());
        b[at + 4..at + 8].copy_from_slice(&count.to_le_bytes());
        b[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
    }
    b[112..118].copy_from_slice(b"NIKON\0");
    b[120..132].copy_from_slice(b"NIKON D7100\0");
    b[144..192].fill(80);
    b
}
fn output(topic: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/tests/camera")
        .join(format!(
            "{}-{topic}-{}-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("command.txt"), "cargo test -p ct-engine devices::camera --offline\nScripted transport only; no real WPD/device calls.\n").unwrap();
    std::fs::write(root.join("result.json"), r#"{"validation":"offline scripted camera pipeline","hardware":"BLOCKED","data":"synthetic"}"#).unwrap();
    root.join("capture.nef")
}

#[test]
fn creation_does_not_discover_open_or_touch_wpd() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = adapter(script.clone());
    assert!(!a.health().connected);
    assert!(a.connect().is_err());
    let s = script.lock().unwrap();
    assert!(s.calls.is_empty());
    assert_eq!((s.opens, s.discoveries), (0, 0));
}
#[test]
fn discovery_requires_explicit_device_selection() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = adapter(script.clone());
    let found = a.discover_list().unwrap();
    assert_eq!(found.len(), 1);
    assert!(!a.health().connected);
    let health = a.connect_selected(&found[0].device_id).unwrap();
    assert!(health.connected);
    assert!(!health.capabilities.hardware_certified);
    assert!(!health.busy);
    assert_eq!(health.identity.unwrap().model, "NIKON D7100");
}
#[test]
fn unknown_brand_never_receives_nikon_private_operations() {
    let mut s = Script::new();
    s.nikon = false;
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    assert!(!a.health().capabilities.host_capture);
    assert!(!a.health().capabilities.live_view);
    assert!(a.start_live_view().is_err());
    assert!(script
        .lock()
        .unwrap()
        .calls
        .iter()
        .all(|code| *code < 0x9000));
}
#[test]
fn exact_shutter_enum_never_rounds_and_bulb_is_excluded() {
    let a = connected(Arc::new(Mutex::new(Script::new())));
    assert_eq!(
        exact_shutter_value(&a.health(), 0.125).unwrap(),
        CameraValue::Integer(0x11f40)
    );
    assert_eq!(a.set_exposure_ms(125.0).unwrap(), "native Integer(65544)");
    assert!(a.set_exposure_ms(120.0).is_err());
    assert!(shutter_ms(&CameraValue::Integer(0xffffffff), true).is_none());
}
#[test]
fn shared_shutter_target_is_exact_cached_and_never_writes_implicitly() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    let actual = a.health().properties["shutter"].actual.clone();
    a.set_exposure_target(100.0).unwrap();
    assert_eq!(
        a.health().properties["shutter"].target,
        Some(CameraValue::Integer(0x1000a))
    );
    assert_eq!(a.health().properties["shutter"].actual, actual);
    assert!(a.health().properties["shutter"].confirmed);
    assert!(!script.lock().unwrap().calls.contains(&SET_PROPERTY));
    assert!(a.set_exposure_target(120.0).is_err());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        a.health().properties["shutter"].target,
        Some(CameraValue::Integer(0x1000a))
    );
    assert!(a.profile_readback().is_err());
    a.set_exposure_ms(100.0).unwrap();
    assert!(a.profile_readback().is_ok());
}
#[test]
fn camera_idle_disconnect_monitor_runs_without_snapshot_requests() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    script.lock().unwrap().fail_code = Some(DEVICE_READY);
    std::thread::sleep(Duration::from_millis(400));
    let health = a.health();
    assert!(!health.connected);
    assert!(health.isolated);
    assert!(health.last_seen_at.is_some());
}
#[test]
fn camera_worker_panic_revokes_cached_identity_without_ui_polling() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    script.lock().unwrap().panic_code = Some(DEVICE_READY);
    std::thread::sleep(Duration::from_millis(400));
    let health = a.health();
    assert!(!health.connected);
    assert!(health.isolated);
    assert!(!health.cleanup_confirmed);
    assert!(health.last_error.unwrap().contains("panicked"));
}
#[test]
fn capture_parameter_drift_or_old_pending_event_is_rejected_before_trigger() {
    for drift in [true, false] {
        let script = Arc::new(Mutex::new(Script::new()));
        let a = connected(script.clone());
        if drift {
            script
                .lock()
                .unwrap()
                .values
                .insert(0x500f, CameraValue::Integer(400));
        } else {
            script.lock().unwrap().events.push_back(vec![NikonEvent {
                code: CAPTURE_COMPLETE_RAM,
                parameter: 0,
            }]);
        }
        let path = output(if drift { "profile-drift" } else { "old-event" });
        assert!(a
            .capture_to_path(&path, "profile-evidence", &AtomicBool::new(false), None)
            .is_err());
        assert!(!path.exists());
        assert!(!script.lock().unwrap().calls.contains(&CAPTURE_RAM));
    }
}
#[test]
fn unsupported_apply_rejects_before_any_write_and_partial_readback_stays_actual() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    assert!(a
        .apply_properties(&BTreeMap::from([(
            "unknown".into(),
            CameraValue::Integer(1)
        )]))
        .is_err());
    assert!(!script.lock().unwrap().calls.contains(&SET_PROPERTY));
    script.lock().unwrap().mismatch = Some(0x500f);
    let applied = a
        .apply_properties(&BTreeMap::from([
            ("shutter".into(), CameraValue::Integer(0x1000a)),
            ("iso".into(), CameraValue::Integer(400)),
        ]))
        .unwrap();
    assert!(!applied.complete);
    assert_eq!(applied.applied, ["shutter"]);
    assert_eq!(
        applied.properties["iso"].actual,
        Some(CameraValue::Integer(200))
    );
    assert_eq!(
        applied.properties["iso"].target,
        Some(CameraValue::Integer(400))
    );
    assert!(applied.properties["iso"].confirmed);
    assert!(a.profile_readback().is_err());
}
#[test]
fn exposure_event_is_distinct_from_file_commit_and_hash_matches() {
    let a = connected(Arc::new(Mutex::new(Script::new())));
    let destination = output("capture-events");
    let (tx, rx) = mpsc::channel();
    let session = a.health().session_id.unwrap();
    let image = a
        .capture_to_path(
            &destination,
            "operation-123",
            &AtomicBool::new(false),
            Some(tx),
        )
        .unwrap();
    let events: Vec<_> = rx.try_iter().collect();
    assert!(
        matches!(&events[0], CameraCaptureEvent::ExposureEnded {session_id,operation_id} if session_id == &session && operation_id == "operation-123")
    );
    assert!(matches!(
        events.last(),
        Some(CameraCaptureEvent::FileComplete { .. })
    ));
    assert_eq!(image.session_id, session);
    assert_eq!(image.operation_id, "operation-123");
    assert_eq!(image.size, std::fs::metadata(&destination).unwrap().len());
    use sha2::{Digest, Sha256};
    assert_eq!(
        image.sha256,
        format!("{:x}", Sha256::digest(std::fs::read(&destination).unwrap()))
    );
    assert!(a
        .capture_to_path(
            &destination,
            "operation-overwrite",
            &AtomicBool::new(false),
            None
        )
        .is_err());
}
#[test]
fn duplicate_or_unassociated_events_and_truncated_raw_never_commit() {
    for (topic, events, truncate) in [
        (
            "duplicate",
            vec![vec![
                NikonEvent {
                    code: CAPTURE_COMPLETE_RAM,
                    parameter: 0,
                },
                NikonEvent {
                    code: CAPTURE_COMPLETE_RAM,
                    parameter: 0,
                },
                NikonEvent {
                    code: OBJECT_RAM,
                    parameter: 0xffff0001,
                },
            ]],
            false,
        ),
        (
            "foreign",
            vec![vec![NikonEvent {
                code: 0x4002,
                parameter: 12,
            }]],
            false,
        ),
        ("truncated", Script::new().capture_events, true),
    ] {
        let mut s = Script::new();
        s.capture_events = events;
        if truncate {
            s.raw.truncate(170);
        }
        let a = connected(Arc::new(Mutex::new(s)));
        let destination = output(topic);
        assert!(a
            .capture_to_path(&destination, topic, &AtomicBool::new(false), None)
            .is_err());
        assert!(!destination.exists());
        assert!(a.health().isolated);
    }
}
#[test]
fn missing_exposure_event_is_not_accepted_by_elapsed_time() {
    let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let context = CallContext {
        deadline: Instant::now() + Duration::from_millis(40),
        epoch: 1,
        generation,
    };
    let mut s = Script::new();
    s.capture_events = vec![vec![NikonEvent {
        code: OBJECT_RAM,
        parameter: 0xffff0001,
    }]];
    let mut camera = NativeCamera::new(Box::new(Fake(Arc::new(Mutex::new(s)))));
    camera
        .connect(
            "fake-camera-1",
            "00112233445566778899aabbccddeeff",
            &context,
        )
        .unwrap();
    let destination = output("missing-exposure");
    let result = super::capture::capture(
        &mut camera,
        &destination,
        "missing-exposure",
        &None,
        &context,
        &Mutex::new(()),
    );
    assert!(matches!(result, Err(CameraError::Timeout(_))));
    assert!(!destination.exists());
}
#[test]
fn stalled_native_call_does_not_block_status_or_stop_acceptance() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    let (tx, rx) = mpsc::channel();
    script.lock().unwrap().stall = Some((GET_PROPERTY_DESC, Duration::from_millis(300), tx));
    let other = a.clone();
    let pending = std::thread::spawn(move || other.read_properties());
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let start = Instant::now();
    let health = a.request_cancel();
    assert!(start.elapsed() < Duration::from_millis(250));
    assert!(health.isolated);
    assert!(!health.cleanup_confirmed);
    let start = Instant::now();
    let _ = a.health();
    assert!(start.elapsed() < Duration::from_millis(50));
    assert!(matches!(
        pending.join().unwrap(),
        Err(CameraError::Cancelled)
    ));
}
#[test]
fn cancelling_stalled_transfer_prevents_late_file_and_old_session_reuse() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    let (tx, rx) = mpsc::channel();
    script.lock().unwrap().stall = Some((GET_OBJECT, Duration::from_millis(250), tx));
    let path = output("late-transfer");
    let destination = path.clone();
    let other = a.clone();
    let task = std::thread::spawn(move || {
        other.capture_to_path(&path, "cancelled-operation", &AtomicBool::new(false), None)
    });
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    a.request_cancel();
    assert!(task.join().unwrap().is_err());
    std::thread::sleep(Duration::from_millis(300));
    assert!(!destination.exists());
    assert!(!a.health().connected);
}
#[test]
fn nef_structure_detects_raw_strip_truncation_and_identity_mismatch() {
    let valid = synthetic_nef();
    assert!(validate_nef(&valid).is_ok());
    assert!(validate_nef(&valid[..160]).is_err());
    let mut foreign = valid;
    foreign[112..118].copy_from_slice(b"CANON\0");
    assert!(validate_nef(&foreign).is_err());
}
#[test]
fn live_view_strict_header_decode_and_frame_envelope_reject_foreign_or_old_frames() {
    let session = "00112233445566778899aabbccddeeff";
    let frame = d7100_frame(&lv_bytes(), session, 7).unwrap();
    let bytes = encode_live_view_frame(&frame).unwrap();
    assert_eq!(
        decode_live_view_frame(&bytes, session, 6).unwrap().sequence,
        7
    );
    assert!(decode_live_view_frame(&bytes, session, 7).is_err());
    assert!(decode_live_view_frame(&bytes, "10112233445566778899aabbccddeeff", 0).is_err());
    assert!(decode_live_view_frame(&bytes[..bytes.len() - 1], session, 0).is_err());
    let mut wrong = lv_bytes();
    wrong[8] = 9;
    assert!(d7100_frame(&wrong, session, 1).is_err());
    assert!(d7100_frame(&vec![0; 384], session, 1).is_err());
}
#[test]
fn latest_slot_replaces_old_frame_and_revoke_removes_identity() {
    let slot = LatestFrameSlot::default();
    let session = "00112233445566778899aabbccddeeff";
    slot.install(LiveViewPipeDescriptor {
        endpoint: "offline-test".into(),
        session_id: session.into(),
        version: 1,
        max_frame_bytes: MAX_LIVE_VIEW_BYTES,
    });
    for n in 1..10 {
        slot.publish(d7100_frame(&lv_bytes(), session, n).unwrap());
    }
    assert_eq!(slot.latest().unwrap().sequence, 9);
    slot.revoke();
    assert!(slot.latest().is_none());
    assert!(slot.descriptor().is_none());
}
#[cfg(windows)]
#[test]
fn live_view_uses_user_acl_pipe_and_rejects_capture_without_restarting_preview() {
    let a = connected(Arc::new(Mutex::new(Script::new())));
    a.start_live_view().unwrap();
    assert!(a.health().live_view.active);
    assert!(a
        .pipe_descriptor()
        .unwrap()
        .endpoint
        .starts_with(r"\\.\pipe\microct-lv-"));
    let before = a.health().session_id;
    assert!(a
        .capture_to_path(
            &output("lv-capture-refused"),
            "lv-123",
            &AtomicBool::new(false),
            None
        )
        .is_err());
    assert!(a.health().live_view.active);
    assert_eq!(a.health().session_id, before);
    a.stop_live_view().unwrap();
    assert!(!a.health().live_view.active);
    assert!(a.pipe_descriptor().is_none());
}
#[cfg(windows)]
#[test]
fn owned_named_pipe_transfers_validated_jpeg_without_json_binary_or_device_io() {
    use std::io::Read;
    let a = connected(Arc::new(Mutex::new(Script::new())));
    a.start_live_view().unwrap();
    let descriptor = a.pipe_descriptor().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while a.health().live_view.sequence == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut received = None;
    while Instant::now() < deadline {
        if let Ok(mut file) = std::fs::File::open(&descriptor.endpoint) {
            let mut header = [0u8; LIVE_VIEW_HEADER_BYTES];
            if file.read_exact(&mut header).is_ok() {
                let len = u32::from_le_bytes(header[48..52].try_into().unwrap()) as usize;
                assert!(len <= MAX_LIVE_VIEW_BYTES);
                let mut bytes = header.to_vec();
                bytes.resize(LIVE_VIEW_HEADER_BYTES + len, 0);
                if file
                    .read_exact(&mut bytes[LIVE_VIEW_HEADER_BYTES..])
                    .is_ok()
                {
                    received =
                        Some(decode_live_view_frame(&bytes, &descriptor.session_id, 0).unwrap());
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let frame = received.expect("actual local named pipe frame, not a direct slot read");
    assert_eq!((frame.width, frame.height), (8, 6));
    assert!(!serde_json::to_string(&a.health())
        .unwrap()
        .contains("MCTLV001"));
    let sequence = a.health().live_view.sequence;
    a.stop_live_view().unwrap();
    a.start_live_view().unwrap();
    std::thread::sleep(Duration::from_millis(120));
    assert!(a.health().live_view.sequence > sequence);
    a.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn background_live_view_driver_deadline_isolates_without_ui_polling() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    let (tx, rx) = mpsc::channel();
    script.lock().unwrap().stall = Some((GET_LV, Duration::from_millis(2300), tx));
    a.start_live_view().unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    // No snapshot/UI polling during the driver stall. The independent watchdog
    // revokes the frame channel and publishes UNKNOWN/isolated before it returns.
    std::thread::sleep(Duration::from_millis(2100));
    let h = a.health();
    assert!(h.isolated);
    assert!(!h.connected);
    assert!(!h.cleanup_confirmed);
    assert!(a.pipe_descriptor().is_none());
    std::thread::sleep(Duration::from_millis(300));
    assert!(!a.health().live_view.active);
    assert!(a.pipe_descriptor().is_none());
}
#[test]
fn dataset_parsers_reject_truncated_arrays_unknown_types_and_extra_events() {
    assert!(parse_device_info(&[100, 0, 10, 0]).is_err());
    assert!(parse_nikon_events(&[1, 0, 1, 0]).is_err());
    assert!(parse_nikon_events(&[0, 0, 7]).is_err());
    assert!(parse_value(&[0; 4], 0x4006).is_err());
    assert!(encode_value(&CameraValue::Integer(256), 2).is_err());
}
