//! Scripted transport exercises the production actor/protocol/capture pipeline.
//! Default tests never create WPD objects, enumerate devices or contact cameras.
//! The ignored Windows metadata diagnostic requires explicit environment gates.
//! Its optional RAM read/discard mode has a separate gate and is manual only.

use super::capture::validate_nef;
use super::live_view::{d7100_frame, LatestFrameSlot};
use super::nikon::*;
use super::ptp::*;
use super::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
#[test]
#[ignore = "manual authorized D7100 ObjectInfo-only read; explicit environment gates required"]
fn hardware_readonly_d7100_object_info_metadata() {
    const DEVICE_ID: &str = r"\\?\usb#vid_04b0&pid_0430#8322769#{6ac27878-a6fa-4155-ba85-f98f491d4f33}";
    assert_eq!(std::env::var("MICRO_CT_READONLY_OBJECT_INFO").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MICRO_CT_OBJECT_INFO_DEVICE_ID").as_deref(), Ok(DEVICE_ID));
    let handle = match std::env::var("MICRO_CT_OBJECT_INFO_HANDLE").as_deref() {
        Ok("0B000001") => 0x0B000001u32,
        Ok("0B000002") => 0x0B000002u32,
        _ => panic!("only explicitly known uncommitted RAM handles 0B000001/0B000002 permitted"),
    };
    let drain_ram = match std::env::var("MICRO_CT_OBJECT_INFO_DRAIN_RAM") {
        Err(std::env::VarError::NotPresent) => false,
        Ok(value) if value == "1" => true,
        _ => panic!("RAM read/discard requires MICRO_CT_OBJECT_INFO_DRAIN_RAM=1 or unset"),
    };
    let output = std::path::PathBuf::from(
        std::env::var("MICRO_CT_OBJECT_INFO_OUTPUT_DIR").expect("absolute camera run directory required"),
    );
    assert!(output.is_absolute());
    let allowed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.cache/tests/camera")
        .canonicalize().unwrap();
    assert_eq!(output.parent().unwrap().canonicalize().unwrap(), allowed);
    if !output.exists() {
        std::fs::create_dir(&output).unwrap();
    }
    assert_eq!(output.canonicalize().unwrap().parent().unwrap(), allowed);
    for name in ["metadata.bin", "metadata.json", "command.txt"] {
        assert!(!output.join(name).exists(), "diagnostic output must not overwrite evidence");
    }
    let write_new = |name: &str, bytes: &[u8]| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
            .open(output.join(name)).unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
    };
    write_new("command.txt", format!("cargo test -p ct-engine hardware_readonly_d7100_object_info_metadata -- --ignored --exact devices::camera::tests::hardware_readonly_d7100_object_info_metadata --nocapture\nRAM read/discard explicitly enabled: {drain_ram}. Default: only WPD open, 0x1001, 0x1008(handle 0x{handle:08X}), close. Optional gated mode: fresh NEF-only property read, 0x1009 in-memory read/discard, bounded 0x90C7/0x90C8. No capture, writes, delete, reset, Live View, pixel files or old-business completion credit.\n").as_bytes());
    let context = CallContext {
        deadline: Instant::now() + Duration::from_secs(if drain_ram { 75 } else { 30 }),
        epoch: 1,
        generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
    };
    let mut transport = super::wpd::create_transport().unwrap();
    let mut report = serde_json::json!({
        "kind": if drain_ram { "manual_ram_read_discard" } else { "manual_readonly_object_info" }, "deviceId": DEVICE_ID,
        "opcode": "0x1008", "handle": format!("0x{handle:08X}"),
        "exposureCredit": false, "oldBusinessCompletionCredited": false,
        "pixelFilesWritten": 0, "closeScope": "ownInterface",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let result = (|| -> Result<(), CameraError> {
        transport.open(DEVICE_ID, &context)?;
        let info_bytes = transport.execute(GET_DEVICE_INFO, &[], DataPhase::Read { max: 64 * 1024 }, &context)?.data()?;
        let info = parse_device_info(&info_bytes)?;
        if info.manufacturer != "Nikon Corporation" || info.model != "D7100"
            || info.serial != "83227690000000000000000000000000"
            || info.diagnostic.functional_mode != 0
            || !info.operations.contains(&GET_OBJECT_INFO)
        {
            return Err(invalid("fresh DeviceInfo does not match authorized D7100 identity/mode/ObjectInfo support"));
        }
        report["identityVerified"] = serde_json::json!(true);
        let response = transport.execute(GET_OBJECT_INFO, &[handle], DataPhase::Read { max: 64 * 1024 }, &context)?;
        let shown = response.data.len().min(4096);
        let hex: String = response.data[..shown].iter().map(|byte| format!("{byte:02X}")).collect();
        report["responseCode"] = serde_json::json!(format!("0x{:04X}", response.code));
        report["metadataLength"] = serde_json::json!(response.data.len());
        report["metadataShown"] = serde_json::json!(shown);
        report["metadataTruncated"] = serde_json::json!(response.data.len() > shown);
        report["metadataHex"] = serde_json::json!(hex);
        write_new("metadata.bin", &response.data);
        let data = response.data()?;
        report["strictParserResult"] = serde_json::json!(match parse_object_info(&data) {
            Ok(info) => format!("OK: format=0x{:04X}, size={}, filename={}", info.format, info.size, info.filename),
            Err(error) => error.to_string(),
        });
        if drain_ram {
            if ![GET_OBJECT, GET_EVENT, DEVICE_READY, GET_PROPERTY, GET_PROPERTY_DESC].iter()
                .all(|opcode| info.operations.contains(opcode))
            {
                return Err(invalid("RAM discard requires advertised GetObject/GetEvent/DeviceReady/GetProperty"));
            }
            let mut metadata = Cursor::new(&data);
            if metadata.u32()? != 0 {
                return Err(invalid("RAM discard requires fresh ObjectInfo storageId=0"));
            }
            let object = parse_d7100_ram_object_info(&data)?;
            let raw_descriptor = transport.execute(GET_PROPERTY_DESC, &[0x5004], DataPhase::Read { max: 32 * 1024 }, &context)?.data()?;
            let raw_property = parse_property(&raw_descriptor, "rawFormat", "Raw format", 0x5004)?;
            if !matches!(raw_property.data_type, 2 | 4) || raw_property.actual.as_ref().and_then(CameraValue::integer) != Some(4) {
                return Err(invalid("RAM discard requires descriptor-confirmed NEF-only rawFormat=4"));
            }
            let raw_format = transport.execute(GET_PROPERTY, &[0x5004], DataPhase::Read { max: 16 }, &context)?.data()?;
            let mut raw_cursor = Cursor::new(&raw_format);
            let raw_actual = raw_cursor.value(raw_property.data_type)?;
            raw_cursor.finish()?;
            if raw_actual.integer() != Some(4) {
                return Err(invalid("RAM discard requires fresh native-typed NEF-only rawFormat=4"));
            }
            report["rawFormatNativeType"] = serde_json::json!(raw_property.data_type);
            let bytes = transport.execute(GET_OBJECT, &[handle], DataPhase::Read { max: object.size }, &context)?.data()?;
            if bytes.len() != object.size {
                return Err(invalid("discarded RAM NEF length differs from ObjectInfo"));
            }
            validate_nef(&bytes)?;
            use sha2::{Digest, Sha256};
            report["discardedObjectSize"] = serde_json::json!(bytes.len());
            report["discardedObjectSha256"] = serde_json::json!(format!("{:x}", Sha256::digest(&bytes)));
            drop(bytes);
            report["pixelsDiscardedFromPrivateMemory"] = serde_json::json!(true);
            let drain_context = CallContext {
                deadline: (Instant::now() + Duration::from_secs(5)).min(context.deadline),
                ..context.clone()
            };
            let mut batches = Vec::new();
            let mut ready = false;
            let mut empty = false;
            let mut completion_observed = false;
            let mut additional_handles = BTreeSet::new();
            report["additionalRamHandlesPending"] = serde_json::json!([]);
            for sequence in 1..=50 {
                drain_context.check()?;
                let response = transport.execute(GET_EVENT, &[], DataPhase::Read { max: 8192 }, &drain_context)?;
                let hex: String = response.data.iter().map(|byte| format!("{byte:02X}")).collect();
                batches.push(serde_json::json!({ "sequence": sequence,
                    "responseCode": format!("0x{:04X}", response.code), "rawHex": hex }));
                report["drainedEventBatches"] = serde_json::json!(&batches);
                if response.code == DEVICE_BUSY {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                let events = parse_nikon_events(&response.data()?)?;
                additional_handles.extend(events.iter()
                    .filter(|event| event.code == OBJECT_RAM && event.parameter != handle)
                    .map(|event| format!("0x{:08X}", event.parameter)));
                report["additionalRamHandlesPending"] = serde_json::json!(&additional_handles);
                completion_observed |= events.iter().any(|event| event.code == CAPTURE_COMPLETE_RAM);
                report["oldRamC102ObservedWithoutCredit"] = serde_json::json!(completion_observed);
                if events.is_empty() {
                    if ready {
                        empty = true;
                        break;
                    }
                    let status = transport.execute(DEVICE_READY, &[], DataPhase::None, &drain_context)?;
                    report["deviceReadyResponseCode"] = serde_json::json!(format!("0x{:04X}", status.code));
                    if status.code != DEVICE_BUSY {
                        status.success()?;
                        ready = true;
                    }
                } else {
                    ready = false; // require a fresh ready/empty boundary after new events
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            report["freshDeviceReadyConfirmed"] = serde_json::json!(ready);
            report["freshEventQueueEmptyConfirmed"] = serde_json::json!(empty);
            report["noAdditionalRamObjectsObserved"] = serde_json::json!(additional_handles.is_empty());
            if !ready || !empty {
                return Err(invalid("RAM discard diagnostic did not confirm fresh DeviceReady and empty queue within bound"));
            }
        }
        Ok(())
    })();
    let close = transport.close();
    report["operationResult"] = serde_json::json!(match &result { Ok(()) => "PASS".into(), Err(error) => error.to_string() });
    report["closeResult"] = serde_json::json!(match &close { Ok(()) => "PASS".into(), Err(error) => error.to_string() });
    write_new("metadata.json", &serde_json::to_vec_pretty(&report).unwrap());
    assert!(result.is_ok() && close.is_ok(), "{}", report);
}

struct Script {
    nikon: bool,
    calls: Vec<u16>,
    requests: Vec<(u16, Vec<u32>)>,
    writes:Vec<(u16,Vec<u8>)>,
    opens: usize,
    open: bool,
    closes: usize,
    closed_reads: usize,
    discoveries: usize,
    /// Devices reported by discovery; 1 unless a test needs another count.
    discovered: usize,
    /// How many transport-level cancels had happened when the end-Live-View
    /// command was issued. `Some(0)` proves Live View was stopped while the
    /// session was still valid, which is the property the Stop path must keep.
    lv_stop_at_cancels: Option<usize>,
    cancels: usize,
    values: BTreeMap<u16, CameraValue>,
    property_types: BTreeMap<u16, u16>,
    descriptor_values: BTreeMap<u16, Vec<CameraValue>>,
    descriptor_ranges: BTreeMap<u16, [i64; 3]>,
    descriptor_change_on_set: BTreeMap<u16, (u16, Vec<CameraValue>)>,
    descriptor_type_change_on_set: BTreeMap<u16, u16>,
    readonly_properties: BTreeSet<u16>,
    unreadable_property: Option<u16>,
    capture_value_changes: BTreeMap<u16, CameraValue>,
    events: VecDeque<Vec<NikonEvent>>,
    capture_events: Vec<Vec<NikonEvent>>,
    raw: Vec<u8>,
    lv: bool,
    mismatch: Option<u16>,
    stall: Option<(u16, Duration, mpsc::Sender<()>)>,
    exposure_delay: Duration,
    transfer_delay: Duration,
    capture_started: Option<Instant>,
    capture_started_signal: Option<mpsc::Sender<()>>,
    fail_code: Option<u16>,
    panic_code: Option<u16>,
    close_failure: bool,
    device_info: Option<Vec<u8>>,
    object_info: Option<Vec<u8>>,
    vendor_property_data: Option<Vec<u8>>,
    notify_property_after_set: bool,
    notify_runtime_after_set: bool,
    device_ready_responses: VecDeque<u16>,
    get_event_responses: VecDeque<u16>,
    property_responses: BTreeMap<(u16, u16), VecDeque<u16>>,
    busy_on_capture: bool,
    complete_after_download: bool,
    poll_after_download_signal: Option<mpsc::Sender<()>>,
    /// Additive card mode: GetObjectInfo answers keyed by the requested handle.
    /// Empty in every RAM session, where `object_info` stays the single RAM
    /// answer for handle `0xffff0001`.
    card_object_info: BTreeMap<u32, Vec<u8>>,
    /// Additive card import mode: GetObject answers keyed by the requested
    /// handle. Empty in every RAM session, where `raw` stays the single RAM
    /// object answer for handle `0xffff0001`. A scripted answer shorter than the
    /// ObjectInfo size is exactly the truncated-transfer case.
    card_object_bytes: BTreeMap<u32, Vec<u8>>,
    /// Additive card import mode: object reads served so far, and the optional
    /// `(count, flag)` that makes the Nth completed `GetObject` raise the
    /// import's cancellation flag. Test-only, fully deterministic: no timing.
    card_object_reads: usize,
    cancel_after_object_reads: Option<(usize, Arc<AtomicBool>)>,
    /// Every read phase this transport served as `(opcode, max)`, so a card test
    /// can prove no image-sized transfer was ever requested.
    reads: Vec<(u16, usize)>,
    /// Additive storage scripting: the `GetStorageIDs` payload and the per-id
    /// `GetStorageInfo` payloads. Empty in every RAM session, where storage
    /// operations stay unsupported instead of inventing a card.
    storage_ids: Vec<u8>,
    storage_infos: BTreeMap<u32, Vec<u8>>,
    /// Additive: per-storage `GetObjectHandles` payloads.
    object_handles: BTreeMap<u32, Vec<u8>>,
}
impl Script {
    /// Scripts the advertised storages as `(id, access capability, max bytes, free
    /// bytes, free images)`. Access 0 is read-write and 1/2 read-only, matching the
    /// production parser.
    fn script_storages(&mut self, cards: &[(u32, u16, u64, u64, u32)]) {
        let mut ids = (cards.len() as u32).to_le_bytes().to_vec();
        self.storage_infos.clear();
        for (id, access, max_bytes, free_bytes, free_images) in cards {
            ids.extend(id.to_le_bytes());
            let mut info = Vec::new();
            info.extend((0u16).to_le_bytes()); // StorageType fixed disk
            info.extend((2u16).to_le_bytes()); // FilesystemType DCF
            info.extend(access.to_le_bytes());
            info.extend(max_bytes.to_le_bytes());
            info.extend(free_bytes.to_le_bytes());
            info.extend(free_images.to_le_bytes());
            info.extend(ptp_string("SD card"));
            info.extend(ptp_string(""));
            self.storage_infos.insert(*id, info);
        }
        self.storage_ids = ids;
    }
    /// Scripts one storage's object listing.
    fn script_object_handles(&mut self, id: u32, handles: &[u32]) {
        let mut payload = (handles.len() as u32).to_le_bytes().to_vec();
        for handle in handles {
            payload.extend(handle.to_le_bytes());
        }
        self.object_handles.insert(id, payload);
    }
    fn new() -> Self {
        let default = |key: &str| match key {
            "shutter" => 0x10005,
            "iso" => 200,
            "aperture" => 800,
            "exposureMode" => 1,
            "rawFormat" => 4,
            "driveMode" | "burstNumber" | "recordingMedia" => 1,
            "focusMode" => 4,
            "exposureDelay" => 3,
            "whiteBalance" | "activeDLighting" => 4,
            "meteringMode" => 3,
            _ => 0,
        };
        Self {
            nikon: true,
            calls: vec![],
            requests: vec![],
            writes:vec![],
            opens: 0,
            open: false,
            closes: 0,
            closed_reads: 0,
            discoveries: 0,
            discovered: 1,
            lv_stop_at_cancels: None,
            cancels: 0,
            values: NIKON_PROPERTIES
                .iter()
                .map(|p| {
                    (
                        p.2,
                        if p.0 == "imageSize" {
                            CameraValue::Text("6000x4000".into())
                        } else {
                            CameraValue::Integer(default(p.0))
                        },
                    )
                })
                .chain([(INFO_DISPLAY_ERROR_STATUS, CameraValue::Integer(0))])
                .chain([(LV_PROHIBIT, CameraValue::Integer(0))])
                .chain([(LV_ZOOM,CameraValue::Integer(0)),(LV_FOCUS_MODE,CameraValue::Integer(0))])
                .chain(
                    RUNTIME_DIAGNOSTICS
                        .iter()
                        .filter(|row| !matches!(row.2, LV_PROHIBIT | INFO_DISPLAY_ERROR_STATUS))
                        .map(|row| (row.2, CameraValue::Integer(if row.2==LENS_SORT{1}else{0}))),
                )
                .collect(),
            property_types: BTreeMap::from([
                (0x5001, 2),
                (RECORDING_MEDIA, 2),
                (0xd0c0,2),
                (BURST_NUMBER, 4),
                (INFO_DISPLAY_ERROR_STATUS, 2),
                (0xd1b0, 2),
                (0xd1b1, 1),
                (0xd1b3, 2),
                (0xd1b4, 2),
                (0x5003, 0xffff),
                (0x5005, 4),
                (LV_ZOOM,2),(LV_FOCUS_MODE,2),(LENS_SORT,2),(LV_SELECTOR,2),(ACTIVE_SLOT,2),
            ]),
            descriptor_values: BTreeMap::from([
                (LV_ZOOM,(0..=6).map(CameraValue::Integer).collect()),
                (LV_FOCUS_MODE,[0,2,3,4].into_iter().map(CameraValue::Integer).collect()),
                (LENS_SORT,[0,1].into_iter().map(CameraValue::Integer).collect()),
                (LV_SELECTOR,[0,1].into_iter().map(CameraValue::Integer).collect()),
                (ACTIVE_SLOT,[0,1,2,3].into_iter().map(CameraValue::Integer).collect()),
                (
                    0x5003,
                    ["6000x4000", "4496x3000", "2992x2000"]
                        .iter()
                        .map(|s| CameraValue::Text((*s).into()))
                        .collect(),
                ),
                (
                    0x5005,
                    [2, 4, 5, 6, 7, 0x8010, 0x8011, 0x8012, 0x8013]
                        .iter()
                        .map(|v| CameraValue::Integer(*v))
                        .collect(),
                ),
            ]),
            descriptor_ranges: BTreeMap::from([
                (0x5001, [0, 100, 1]),
                (0xd0c0,[0,1,1]),
                (LV_PROHIBIT, [0, u32::MAX as i64, 1]),
                (0xd1b0, [0, 255, 1]),
                (0xd1b1, [-128, 127, 1]),
                (0xd1b3, [0, 255, 1]),
                (0xd1b4, [0, 255, 1]),
            ]),
            descriptor_change_on_set: BTreeMap::new(),
            descriptor_type_change_on_set: BTreeMap::new(),
            readonly_properties: RUNTIME_DIAGNOSTICS.iter().map(|row| row.2).collect(),
            unreadable_property: None,
            capture_value_changes: BTreeMap::new(),
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
            capture_started_signal: None,
            fail_code: None,
            panic_code: None,
            close_failure: false,
            device_info: None,
            object_info: None,
            vendor_property_data: None,
            notify_property_after_set: false,
            notify_runtime_after_set: false,
            device_ready_responses: VecDeque::new(),
            get_event_responses: VecDeque::new(),
            property_responses: BTreeMap::new(),
            busy_on_capture: false,
            complete_after_download: false,
            poll_after_download_signal: None,
            card_object_info: BTreeMap::new(),
            card_object_bytes: BTreeMap::new(),
            card_object_reads: 0,
            cancel_after_object_reads: None,
            storage_ids: Vec::new(),
            storage_infos: BTreeMap::new(),
            object_handles: BTreeMap::new(),
            reads: vec![],
        }
    }
}
struct Fake(Arc<Mutex<Script>>);
impl CameraTransport for Fake {
    fn discover(&mut self, _: &CallContext) -> Result<Vec<CameraIdentity>, CameraError> {
        let mut script = self.0.lock().unwrap();
        script.discoveries += 1;
        let count = script.discovered;
        drop(script);
        Ok((0..count)
            .map(|index| CameraIdentity {
                device_id: if index == 0 {
                    "fake-camera-1".into()
                } else {
                    format!("fake-camera-{}", index + 1)
                },
                model: "unverified discovery".into(),
                ..Default::default()
            })
            .collect())
    }
    fn open(&mut self, id: &str, _: &CallContext) -> Result<(), CameraError> {
        if id != "fake-camera-1" {
            return Err(CameraError::Unavailable("bad selected ID".into()));
        }
        self.0.lock().unwrap().opens += 1;
        self.0.lock().unwrap().open = true;
        Ok(())
    }
    fn close(&mut self) -> Result<(), CameraError> {
        let mut script = self.0.lock().unwrap();
        script.open = false;
        script.closes += 1;
        if script.close_failure {
            Err(CameraError::Io("scripted camera close failure".into()))
        } else {
            Ok(())
        }
    }
    fn cancel(&mut self) -> Result<(), CameraError> {
        self.0.lock().unwrap().cancels += 1;
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
        {
            let mut script = self.0.lock().unwrap();
            if !script.open {
                script.closed_reads += 1;
                return Err(CameraError::Unavailable(
                    "scripted transport is closed".into(),
                ));
            }
        }
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
            script.requests.push((code, params.to_vec()));
            if let DataPhase::Read { max } = &phase {
                script.reads.push((code, *max));
            }
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
        if matches!(code, GET_PROPERTY_DESC | GET_PROPERTY) {
            if let Some(response) = s
                .property_responses
                .get_mut(&(code, params[0] as u16))
                .and_then(VecDeque::pop_front)
            {
                if response != OK {
                    return Ok(PtpResponse {
                        code: response,
                        data: vec![],
                    });
                }
            }
        }
        if code == GET_EVENT {
            if let Some(response) = s.get_event_responses.pop_front() {
                if response != OK {
                    return Ok(PtpResponse {
                        code: response,
                        data: vec![],
                    });
                }
            }
        }
        if code == DEVICE_READY {
            if let Some(response) = s.device_ready_responses.pop_front() {
                return Ok(PtpResponse {
                    code: response,
                    data: vec![],
                });
            }
        }
        let data = match code {
            GET_VENDOR_PROPERTY_CODES => {
                assert!(params.is_empty());
                s.vendor_property_data.clone().unwrap_or_else(|| array(&[]))
            }
            GET_DEVICE_INFO => s
                .device_info
                .clone()
                .unwrap_or_else(|| device_info(s.nikon)),
            GET_PROPERTY_DESC => {
                let code = params[0] as u16;
                let actual = s.values.get(&code).unwrap();
                let kind = s.property_types.get(&code).copied().unwrap_or(6);
                let values: Vec<i64> = if code == 0xd100 {
                    vec![0x11f40, 0x1000a, 0x10008, 0x10005, 0x1e0001, 0xffffffff]
                } else if code == 0x500d {
                    vec![1000, 1250, 2000, 10000, 20000, 300000]
                } else if matches!(code, RECORDING_MEDIA | INFO_DISPLAY_ERROR_STATUS) {
                    vec![0, 1]
                } else {
                    vec![0, 1, 2, 3, 4, 5, 12, 14, 100, 200, 400, 800]
                };
                let mut b = code.to_le_bytes().to_vec();
                b.extend(kind.to_le_bytes());
                b.push(u8::from(!s.readonly_properties.contains(&code)));
                b.extend(encode_value(actual, kind)?);
                b.extend(encode_value(actual, kind)?);
                if let Some(range) = s.descriptor_ranges.get(&code) {
                    b.push(1);
                    for value in range {
                        b.extend(encode_value(&CameraValue::Integer(*value), kind)?);
                    }
                } else {
                    let values: Vec<CameraValue> =
                        s.descriptor_values.get(&code).cloned().unwrap_or_else(|| {
                            values.into_iter().map(CameraValue::Integer).collect()
                        });
                    b.push(2);
                    b.extend((values.len() as u16).to_le_bytes());
                    for value in values {
                        b.extend(encode_value(&value, kind)?);
                    }
                }
                b
            }
            GET_PROPERTY if params[0] as u16 == LV_STATUS => vec![s.lv as u8],
            GET_PROPERTY => {
                let property = params[0] as u16;
                if s.unreadable_property == Some(property) {
                    return Err(CameraError::Command(
                        "scripted property value unknown".into(),
                    ));
                }
                encode_value(
                    s.values.get(&property).unwrap(),
                    s.property_types.get(&property).copied().unwrap_or(6),
                )?
            }
            SET_PROPERTY => {
                let property = params[0] as u16;
                if let DataPhase::Write(bytes) = phase {
                    s.writes.push((property,bytes.clone()));
                    if s.mismatch != Some(property) {
                        let kind = s.property_types.get(&property).copied().unwrap_or(6);
                        s.values.insert(property, parse_value(&bytes, kind)?);
                    }
                } else {
                    panic!("property write has no data phase");
                }
                if let Some((child, values)) = s.descriptor_change_on_set.get(&property).cloned() {
                    s.descriptor_values.insert(child, values);
                }
                if let Some(kind) = s.descriptor_type_change_on_set.get(&property).copied() {
                    s.property_types.insert(property, kind);
                }
                if s.notify_runtime_after_set {
                    s.events.push_back(vec![
                        NikonEvent {
                            code: 0x4006,
                            parameter: property as u32,
                        },
                        NikonEvent {
                            code: 0x4006,
                            parameter: INFO_DISPLAY_ERROR_STATUS as u32,
                        },
                    ]);
                } else if s.notify_property_after_set {
                    s.events.push_back(vec![
                        NikonEvent {
                            code: 0x4006,
                            parameter: property as u32,
                        },
                        NikonEvent {
                            code: 0x4006,
                            parameter: 0xd16a,
                        },
                        NikonEvent {
                            code: 0x4006,
                            parameter: 0xd1b1,
                        },
                    ]);
                }
                vec![]
            }
            CAPTURE_RAM => {
                assert_eq!(params, [0xffffffff]);
                let events = s.capture_events.clone();
                s.events.extend(events);
                let changes = s.capture_value_changes.clone();
                s.values.extend(changes);
                s.capture_started = Some(Instant::now());
                if s.busy_on_capture {
                    s.get_event_responses = VecDeque::from([DEVICE_BUSY, DEVICE_BUSY, OK]);
                    s.property_responses.insert(
                        (GET_PROPERTY_DESC, LV_PROHIBIT),
                        VecDeque::from([DEVICE_BUSY, DEVICE_BUSY, OK]),
                    );
                }
                if let Some(signal) = s.capture_started_signal.as_ref() {
                    let _ = signal.send(());
                }
                vec![]
            }
            INITIATE_CAPTURE => {
                // Standard PTP still capture, the only trigger the card path may
                // send. The private SDRAM code stays asserted above, so a card
                // test that reached `CAPTURE_RAM` would already fail.
                assert_eq!(params, [0xffffffff]);
                let events = s.capture_events.clone();
                s.events.extend(events);
                let changes = s.capture_value_changes.clone();
                s.values.extend(changes);
                s.capture_started = Some(Instant::now());
                if let Some(signal) = s.capture_started_signal.as_ref() {
                    let _ = signal.send(());
                }
                vec![]
            }
            GET_EVENT => {
                // Reaching the next event poll proves the capture pipeline has
                // returned from GET_OBJECT and validated its complete NEF.
                if s.calls.contains(&GET_OBJECT) {
                    if let Some(signal) = s.poll_after_download_signal.take() {
                        let _ = signal.send(());
                    }
                }
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
                // Additive card mode resolves metadata by the requested handle;
                // without it the single RAM object answer is unchanged.
                if let Some(card) = s.card_object_info.get(&params[0]) {
                    card.clone()
                } else {
                    assert_eq!(params, [0xffff0001]);
                    s.object_info.clone().unwrap_or_else(|| object_info(s.raw.len()))
                }
            }
            GET_STORAGE_IDS => {
                assert!(params.is_empty());
                if s.storage_ids.is_empty() {
                    return Err(CameraError::Unsupported(
                        "scripted storage enumeration is not scripted".into(),
                    ));
                }
                s.storage_ids.clone()
            }
            GET_STORAGE_INFO => s
                .storage_infos
                .get(&params[0])
                .cloned()
                .ok_or_else(|| {
                    CameraError::Unsupported("scripted storage info is not scripted".into())
                })?,
            GET_OBJECT_HANDLES => s
                .object_handles
                .get(&params[0])
                .cloned()
                .ok_or_else(|| {
                    CameraError::Unsupported("scripted object handles are not scripted".into())
                })?,
            GET_OBJECT => {
                // Additive card import mode resolves the object bytes by the
                // requested handle, so a bounded card download can be scripted
                // per object. Without a scripted card answer the single RAM
                // object answer is unchanged.
                if let Some(bytes) = s.card_object_bytes.get(&params[0]) {
                    let bytes = bytes.clone();
                    s.card_object_reads += 1;
                    if let Some((count, flag)) = s.cancel_after_object_reads.clone() {
                        if s.card_object_reads >= count {
                            flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    return Ok(PtpResponse { code: OK, data: bytes });
                }
                assert_eq!(params, [0xffff0001]);
                if s.complete_after_download {
                    s.complete_after_download = false;
                    s.events.push_back(vec![NikonEvent {
                        code: CAPTURE_COMPLETE_RAM,
                        parameter: 0,
                    }]);
                }
                s.raw.clone()
            }
            START_LV => {
                s.lv = true;
                vec![]
            }
            STOP_LV => {
                if s.lv_stop_at_cancels.is_none() {
                    s.lv_stop_at_cancels = Some(s.cancels);
                }
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
#[derive(Clone)]
pub(crate) struct ScriptedCameraConfig {
    pub exposure_delay: Duration,
    pub transfer_delay: Duration,
    pub omit_exposure_end: bool,
    pub truncate_raw: bool,
    pub close_failure: bool,
    pub native_exposure_ms: Option<u32>,
    pub capture_started_signal: Option<mpsc::Sender<()>>,
    /// Number of devices discovery reports. Defaults to 1 so every existing
    /// scripted session keeps seeing exactly one selectable camera.
    pub discovered: usize,
    /// Additive card mode (default false): negotiate the standard
    /// `InitiateCapture`/storage operations plus the `ObjectAdded`/
    /// `CaptureCompleteCard` events, and script the SD destination
    /// (`recordingMedia = 0`) before Connect. `scripted_camera` never sets this,
    /// so every existing RAM session is unchanged.
    pub card_capture: bool,
    /// Additive: also advertise the `StoreFull`/`StoreRemoved` overflow
    /// continuation. Ignored unless `card_capture` is set.
    pub card_overflow: bool,
}
impl Default for ScriptedCameraConfig {
    fn default() -> Self {
        Self {
            exposure_delay: Duration::from_secs(0),
            transfer_delay: Duration::from_secs(0),
            omit_exposure_end: false,
            truncate_raw: false,
            close_failure: false,
            native_exposure_ms: None,
            capture_started_signal: None,
            discovered: 1,
            card_capture: false,
            card_overflow: false,
        }
    }
}
/// Crate integration tests run the real camera owner and capture coordinator,
/// replacing only the physical WPD boundary with deterministic PTP datasets.
pub(crate) fn scripted_camera(config: ScriptedCameraConfig) -> CameraAdapter { scripted_camera_with_probe(config).0 }
#[derive(Clone)]
pub(crate) struct ScriptedCameraProbe { script:Arc<Mutex<Script>> }
impl ScriptedCameraProbe {
    pub(crate) fn set_actual(&self,key:&str,value:CameraValue){let code=NIKON_PROPERTIES.iter().find(|entry|entry.0==key).expect("known scripted property").2;self.script.lock().unwrap().values.insert(code,value);}
    pub(crate) fn write_count(&self)->usize{self.script.lock().unwrap().calls.iter().filter(|code|**code==SET_PROPERTY).count()}
    pub(crate) fn live_view_stop_count(&self)->usize{self.script.lock().unwrap().calls.iter().filter(|code|**code==STOP_LV).count()}
    /// `Some(0)` means the end-Live-View command was issued before any transport
    /// cancel — the invariant the Stop path must hold.
    pub(crate) fn live_view_stop_before_cancel(&self)->Option<usize>{self.script.lock().unwrap().lv_stop_at_cancels}
    /// Start counting cancels/end-Live-View ordering from now, so a test's own
    /// setup (connect/disconnect round trips) cannot mask the Stop path ordering.
    pub(crate) fn reset_live_view_stop_tracking(&self){let mut s=self.script.lock().unwrap();s.lv_stop_at_cancels=None;s.cancels=0;}
    /// Make the named property write report success but not take effect, so the
    /// readback mismatches: the device-refused-value case (`0x201c`-class).
    pub(crate) fn set_write_mismatch(&self,property:u16){self.script.lock().unwrap().mismatch=Some(property);}
    /// Faults are injected only at the scripted transport boundary; the real
    /// camera owner, event association and file commit remain under test.
    pub(crate) fn fail_transport_operation(&self,opcode:u16){self.script.lock().unwrap().fail_code=Some(opcode);}
    pub(crate) fn set_exposure_delay(&self,delay:Duration){self.script.lock().unwrap().exposure_delay=delay;}
    pub(crate) fn set_transfer_delay(&self,delay:Duration){self.script.lock().unwrap().transfer_delay=delay;}
    pub(crate) fn operation_count(&self,opcode:u16)->usize{self.script.lock().unwrap().calls.iter().filter(|code|**code==opcode).count()}
    /// Scripts the advertised card set: `(id, access, max bytes, free bytes, free
    /// images)`. Access 0 is read-write, 1/2 read-only.
    pub(crate) fn set_storages(&self, cards: &[(u32, u16, u64, u64, u32)]) {
        self.script.lock().unwrap().script_storages(cards);
    }
    /// Scripts one storage's object listing.
    pub(crate) fn set_object_handles(&self, id: u32, handles: &[u32]) {
        self.script.lock().unwrap().script_object_handles(id, handles);
    }
    /// Replace the event batches released by the next trigger, as
    /// `(code, parameter)` pairs. `scan.rs` and the domain tests cannot name the
    /// private `NikonEvent`, so they script the association this way.
    pub(crate) fn set_capture_event_codes(&self, batches: Vec<Vec<(u16, u32)>>) {
        self.script.lock().unwrap().capture_events = batches
            .into_iter()
            .map(|batch| {
                batch
                    .into_iter()
                    .map(|(code, parameter)| NikonEvent { code, parameter })
                    .collect()
            })
            .collect();
    }
    /// Scripts the event batches the next card trigger releases, as
    /// `(code, parameter)` pairs per batch.
    pub(crate) fn set_card_events(&self, batches: Vec<Vec<(u16, u32)>>) {
        self.set_capture_event_codes(batches)
    }
    /// Additive card mode: answer `GetObjectInfo [handle]` with this exact card
    /// dataset instead of the single RAM object answer.
    pub(crate) fn set_card_object(&self,handle:u32,bytes:Vec<u8>){self.script.lock().unwrap().card_object_info.insert(handle,bytes);}
    /// Additive card import mode: answer `GetObject [handle]` with these exact
    /// object bytes. A length shorter than the scripted ObjectInfo size is the
    /// truncated-transfer case; nothing here fabricates a length.
    pub(crate) fn set_card_object_bytes(&self,handle:u32,bytes:Vec<u8>){self.script.lock().unwrap().card_object_bytes.insert(handle,bytes);}
    /// Additive card import mode: raise this cancellation flag once `count`
    /// `GetObject` reads have completed. Deterministic, no timing.
    pub(crate) fn cancel_after_object_reads(&self,count:usize,flag:Arc<AtomicBool>){self.script.lock().unwrap().cancel_after_object_reads=Some((count,flag));}
    /// Completed card object reads, so a resume test can prove an
    /// already-verified record was never downloaded again.
    pub(crate) fn card_object_read_count(&self)->usize{self.script.lock().unwrap().card_object_reads}
    /// Replace the event batches released by the next card trigger.
    pub(crate) fn set_capture_events(&self,batches:Vec<Vec<NikonEvent>>){self.script.lock().unwrap().capture_events=batches;}
    /// Script the body Live View state a fresh `GET_PROPERTY [LV_STATUS]` reads.
    pub(crate) fn set_live_view_state(&self,active:bool){self.script.lock().unwrap().lv=active;}
    pub(crate) fn calls(&self)->Vec<u16>{self.script.lock().unwrap().calls.clone()}
    pub(crate) fn requests(&self)->Vec<(u16,Vec<u32>)>{self.script.lock().unwrap().requests.clone()}
    /// Every read phase as `(opcode, max)`. A card operation must show metadata,
    /// property and event reads only: no `GetObject` and no image-sized read.
    pub(crate) fn reads(&self)->Vec<(u16,usize)>{self.script.lock().unwrap().reads.clone()}
}
pub(crate) fn scripted_camera_with_probe(config: ScriptedCameraConfig) -> (CameraAdapter,ScriptedCameraProbe) {
    // A card configuration must produce a card-negotiated session: the RAM script
    // below advertises no InitiateCapture/storage operations and recordingMedia=1,
    // which the card destination gate always refuses.
    if config.card_capture || config.card_overflow {
        return scripted_card_adapter(config);
    }
    let mut script = Script::new();
    script.exposure_delay = config.exposure_delay;
    script.transfer_delay = config.transfer_delay;
    script.close_failure = config.close_failure;
    script.capture_started_signal = config.capture_started_signal;
    script.discovered = config.discovered;
    if let Some(ms) = config.native_exposure_ms {
        assert!((1..=30000).contains(&ms));
        let value = CameraValue::Integer(if ms % 1000 == 0 {
            ((ms / 1000) << 16 | 1) as i64
        } else {
            (ms << 16 | 1000) as i64
        });
        script.values.insert(0xd100, value.clone());
        script.descriptor_values.insert(0xd100, vec![value]);
    }
    if config.omit_exposure_end {
        script
            .capture_events
            .retain(|events| events.iter().all(|e| e.code != CAPTURE_COMPLETE_RAM));
    }
    if config.truncate_raw {
        script.raw.truncate(170);
    }
    let shared=Arc::new(Mutex::new(script));let camera=connected(shared.clone());(camera,ScriptedCameraProbe{script:shared})
}
/// Builds the additive card-capture script shared by both card kits: the standard
/// card path negotiated, the SD destination scripted, no RAM association batches,
/// and a scripted single writable 64 GB card. Every RAM kit keeps its own
/// unchanged script.
fn card_script(config: ScriptedCameraConfig) -> Script {
    let mut script = Script::new();
    script.exposure_delay = config.exposure_delay;
    script.transfer_delay = config.transfer_delay;
    script.close_failure = config.close_failure;
    script.capture_started_signal = config.capture_started_signal;
    script.discovered = config.discovered;
    if config.card_capture || config.card_overflow {
        script.device_info = Some(device_info_catalog_card(
            true,
            10,
            "extension",
            true,
            0xd100,
            false,
            false,
            if config.card_overflow { 2 } else { 1 },
        ));
        // A card plan needs a confirmed card destination, so the scripted body
        // starts on the SD slot. A test that wants the destination refusal
        // scripts SDRAM back through the probe and re-reads the properties.
        script.values.insert(RECORDING_MEDIA, CameraValue::Integer(0));
        // Each card test scripts its own association batches; the RAM defaults
        // never apply to a card operation.
        script.capture_events = vec![];
        // A scripted 64 GB writable card (two for overflow) covers a whole scan; a
        // test that needs another capacity or a read-only card re-scripts it
        // through the probe.
        const GIB: u64 = 1024 * 1024 * 1024;
        if config.card_overflow {
            script.script_storages(&[
                (1, 0, 64 * GIB, 64 * GIB, 5000),
                (2, 0, 64 * GIB, 64 * GIB, 5000),
            ]);
        } else {
            script.script_storages(&[(1, 0, 64 * GIB, 64 * GIB, 5000)]);
        }
    }
    if let Some(ms) = config.native_exposure_ms {
        assert!((1..=30000).contains(&ms));
        let value = CameraValue::Integer(if ms % 1000 == 0 {
            ((ms / 1000) << 16 | 1) as i64
        } else {
            (ms << 16 | 1000) as i64
        });
        script.values.insert(0xd100, value.clone());
        script.descriptor_values.insert(0xd100, vec![value]);
    }
    if config.omit_exposure_end {
        script
            .capture_events
            .retain(|events| events.iter().all(|e| e.code != CAPTURE_COMPLETE_CARD));
    }
    if config.truncate_raw {
        script.raw.truncate(170);
    }
    script
}
/// The actor-proxy card kit, so a scan coordinator can run a card scan offline
/// through the same `CameraAdapter` every scan test uses.
pub(crate) fn scripted_card_adapter(config: ScriptedCameraConfig) -> (CameraAdapter, ScriptedCameraProbe) {
    let shared = Arc::new(Mutex::new(card_script(config)));
    (connected(shared.clone()), ScriptedCameraProbe { script: shared })
}
/// Additive card-capture kit: the real `NativeCamera` over the scripted
/// transport with the standard card path negotiated, plus the same
/// probe accessors the RAM kit uses. `scripted_camera_with_probe` keeps
/// returning the actor proxy for every existing sibling test.
pub(crate) fn scripted_card_camera(
    config: ScriptedCameraConfig,
    session: &str,
    context: &CallContext,
) -> (NativeCamera, ScriptedCameraProbe) {
    let shared = Arc::new(Mutex::new(card_script(config)));
    let mut camera = NativeCamera::new(Box::new(Fake(shared.clone())));
    camera.connect("fake-camera-1", session, context).unwrap();
    (camera, ScriptedCameraProbe { script: shared })
}
fn device_info_without_property(code:u16)->Vec<u8>{
    // DeviceInfo's property array is the final u16 array before the four strings.
    let mut data=device_info(true);let info=parse_device_info(&data).unwrap();let original=array(&info.properties);let filtered:Vec<u16>=info.properties.into_iter().filter(|value|*value!=code).collect();let offset=data.windows(original.len()).position(|window|window==original).expect("scripted property array");data.splice(offset..offset+original.len(),array(&filtered));data
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
    device_info_extension(nikon, if nikon { 10 } else { 11 }, "extension", true)
}
fn device_info_extension(
    nikon: bool,
    vendor: u32,
    extension: &str,
    vendor_operations: bool,
) -> Vec<u8> {
    device_info_shutter(nikon, vendor, extension, vendor_operations, 0xd100)
}
fn device_info_shutter(
    nikon: bool,
    vendor: u32,
    extension: &str,
    vendor_operations: bool,
    shutter_code: u16,
) -> Vec<u8> {
    device_info_catalog(
        nikon,
        vendor,
        extension,
        vendor_operations,
        shutter_code,
        false,
        false,
    )
}
fn device_info_catalog(
    nikon: bool,
    vendor: u32,
    extension: &str,
    vendor_operations: bool,
    shutter_code: u16,
    vendor_discovery: bool,
    standard_properties_only: bool,
) -> Vec<u8> {
    device_info_catalog_card(
        nikon,
        vendor,
        extension,
        vendor_operations,
        shutter_code,
        vendor_discovery,
        standard_properties_only,
        0,
    )
}
/// Additive card selection: `0` keeps the original RAM-only operations and
/// events byte for byte, `1` negociates the standard card capture with the
/// `ObjectAdded`/`CaptureCompleteCard` association events, and `2` additionally
/// advertises the `StoreFull`/`StoreRemoved` overflow continuation. Card mode
/// drops the RAM events, so the scripted body reports `card_capture` without
/// `host_capture` and the card path cannot borrow RAM evidence.
fn device_info_catalog_card(
    nikon: bool,
    vendor: u32,
    extension: &str,
    vendor_operations: bool,
    shutter_code: u16,
    vendor_discovery: bool,
    standard_properties_only: bool,
    card: u8,
) -> Vec<u8> {
    let mut b = 100u16.to_le_bytes().to_vec();
    b.extend(vendor.to_le_bytes());
    b.extend(100u16.to_le_bytes());
    b.extend(ptp_string(extension));
    b.extend(0u16.to_le_bytes());
    let mut operations = vec![
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
    ];
    if vendor_discovery {
        operations.push(GET_VENDOR_PROPERTY_CODES);
    }
    if !vendor_operations {
        operations.retain(|code| *code < 0x9000);
    }
    if card > 0 {
        operations.extend([INITIATE_CAPTURE, GET_STORAGE_IDS, GET_STORAGE_INFO, GET_OBJECT_HANDLES]);
    }
    b.extend(array(&operations));
    if card > 0 {
        let mut events = vec![OBJECT_ADDED, CAPTURE_COMPLETE_CARD];
        if card > 1 {
            events.extend([STORE_FULL, STORE_REMOVED]);
        }
        b.extend(array(&events));
    } else {
        b.extend(array(&[OBJECT_RAM, CAPTURE_COMPLETE_RAM]));
    }
    let mut properties: Vec<u16> = NIKON_PROPERTIES.iter().map(|p| p.2).collect();
    for code in &mut properties {
        if *code == 0xd100 {
            *code = shutter_code;
        }
    }
    properties.push(LV_STATUS);
    properties.push(INFO_DISPLAY_ERROR_STATUS);
    properties.push(LV_PROHIBIT);
    properties.extend([0x5001, 0xd1b0, 0xd1b1, 0xd1b3, 0xd1b4]);
    properties.extend([LENS_SORT,LV_SELECTOR,ACTIVE_SLOT,LV_ZOOM,LV_FOCUS_MODE]);
    if standard_properties_only {
        properties.retain(|code| *code < 0x9000);
    }
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
    b[8..10].copy_from_slice(&8u16.to_be_bytes());
    b[10..12].copy_from_slice(&6u16.to_be_bytes());
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
        .join("../../.cache/tests/camera")
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

fn replace_info_string(mut bytes: Vec<u8>, old: &str, replacement: &str) -> Vec<u8> {
    let encoded = ptp_string(old);
    let start = bytes
        .windows(encoded.len())
        .position(|part| part == encoded)
        .unwrap();
    bytes.splice(start..start + encoded.len(), ptp_string(replacement));
    bytes
}

#[test]
fn measured_d7100_object_info_accepts_only_zero_date_padding_and_nef_metadata() {
    let metadata = measured_d7100_object_info();
    assert_eq!(metadata.len(), 162);
    assert!(parse_object_info(&metadata).is_err());
    let info = parse_d7100_ram_object_info(&metadata).unwrap();
    assert_eq!(info.format, 0x3800);
    assert_eq!(info.size, 26_673_241);
    assert_eq!(info.filename, "DSC_0000.NEF");
    let mut standard_dates = object_info(info.size);
    standard_dates[4..6].copy_from_slice(&0x3800u16.to_le_bytes());
    assert!(parse_object_info(&standard_dates).is_err());
    assert!(parse_d7100_ram_object_info(&standard_dates).is_ok());
    for end in 0..metadata.len() {
        assert!(parse_d7100_ram_object_info(&metadata[..end]).is_err(), "truncated at {end}");
    }
    for offset in [118, 159] {
        let mut invalid = metadata.clone();
        invalid[offset] = b'X';
        assert!(parse_d7100_ram_object_info(&invalid).is_err());
    }
    for start in [110, 151] {
        let mut invalid = metadata.clone();
        for unit in invalid[start..start + 10].chunks_exact_mut(2) {
            unit.copy_from_slice(&[b'X', 0]);
        }
        assert!(parse_d7100_ram_object_info(&invalid).is_err());
    }
    for start in [80, 121] {
        let mut invalid = metadata.clone();
        invalid[start..start + 2].copy_from_slice(&0xd800u16.to_le_bytes());
        assert!(parse_d7100_ram_object_info(&invalid).is_err());
    }
    for size in [0, 7, (MAX_OBJECT_BYTES + 1) as u32] {
        let mut invalid = metadata.clone();
        invalid[8..12].copy_from_slice(&size.to_le_bytes());
        assert!(parse_d7100_ram_object_info(&invalid).is_err());
    }
    for format in [0x3001u16, 0x3801, 0xffff] {
        let mut invalid = metadata.clone();
        invalid[4..6].copy_from_slice(&format.to_le_bytes());
        assert!(parse_d7100_ram_object_info(&invalid).is_err());
    }
    for offset in [75, 77] {
        let mut invalid = metadata.clone();
        invalid[offset] = b'X'; // filename extension or terminator
        assert!(parse_d7100_ram_object_info(&invalid).is_err());
    }
    let mut invalid = metadata.clone();
    invalid[161] = 2;
    invalid.extend([0, 0, 0, 0]); // embedded NUL keyword remains invalid
    assert!(parse_d7100_ram_object_info(&invalid).is_err());
    let mut invalid = metadata;
    invalid.push(0);
    assert!(parse_d7100_ram_object_info(&invalid).is_err());
}

// Actual 162-byte 0x1008 response, read-only D7100 handle 0x0B000001,
// 2026-10-04; contains metadata only, no image or exposure-end evidence.
fn measured_d7100_object_info() -> Vec<u8> {
    const HEX: &str = "00000000003800005900970108386F1E0000A00000007800000070170000A00F00000000000000000000000000000000000000000D4400530043005F0030003000300030002E004E004500460000001432003000320036003100300030003400540032003200340039003300360000000000000000000000143200300032003600310030003000340054003200320034003900330036000000000000000000000000";
    HEX.as_bytes().chunks_exact(2).map(|pair| {
        u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
    }).collect()
}

#[test]
fn d7100_padded_object_info_still_requires_valid_nef_and_associated_object_evidence() {
    // Renamed 2026-10-11 only for accuracy: the SDRAM completion is optional and
    // the associated RAM object is the commit evidence (digiCamControl
    // Nikon/NikonBase.cs:2330-2367), so this test still proves that a padded
    // ObjectInfo never relaxes NEF completeness.
    for valid_nef in [false, true] {
        let mut s = Script::new();
        if !valid_nef { s.raw[0] = 0; }
        let mut metadata = measured_d7100_object_info();
        metadata[8..12].copy_from_slice(&(s.raw.len() as u32).to_le_bytes());
        s.object_info = Some(metadata);
        s.complete_after_download = true;
        s.capture_events = vec![vec![NikonEvent { code: OBJECT_RAM, parameter: 0xffff0001 }]];
        let script = Arc::new(Mutex::new(s));
        let a = connected(script);
        let path = output("d7100-padded-object-info");
        let (tx, rx) = mpsc::channel();
        let result = a.capture_to_path(&path, "padded-object-info", &AtomicBool::new(false), Some(tx));
        assert_eq!(result.is_ok(), valid_nef);
        assert_eq!(path.exists(), valid_nef);
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(events.iter().any(|event| matches!(event, CameraCaptureEvent::ExposureEnded { .. })), valid_nef);
        assert_eq!(events.iter().any(|event| matches!(event, CameraCaptureEvent::FileComplete { .. })), valid_nef);
    }
}

#[test]
fn invalid_object_info_retains_bounded_metadata_without_transfer_or_completion() {
    for oversized in [false, true] {
        let mut s = Script::new();
        let mut metadata = object_info(s.raw.len());
        // Corrupt the filename terminator at its declared position.
        let terminator = 53 + (metadata[52] as usize - 1) * 2;
        metadata[terminator] = b'X';
        if oversized {
            metadata.resize(8192, 0xAB);
        }
        let shown = metadata.len().min(4096);
        let expected_hex: String = metadata[..shown]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        let length = metadata.len();
        s.object_info = Some(metadata);
        s.capture_events = vec![vec![NikonEvent {
            code: OBJECT_RAM,
            parameter: 0xffff0001,
        }]];
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let path = output("invalid-object-info-metadata");
        let (tx, rx) = mpsc::channel();
        let error = a
            .capture_to_path(&path, "invalid-object-info", &AtomicBool::new(false), Some(tx))
            .unwrap_err();
        assert!(error.to_string().contains("invalid PTP string terminator"));
        assert!(!path.exists());
        assert!(!script.lock().unwrap().calls.contains(&GET_OBJECT));
        assert!(!rx.try_iter().any(|event| matches!(event,
            CameraCaptureEvent::TransferStarted { .. }
                | CameraCaptureEvent::ExposureEnded { .. }
                | CameraCaptureEvent::FileComplete { .. })));
        let health = a.health();
        assert!(health.isolated && !health.cleanup_confirmed);
        let diagnostic = health.last_capture_event_diagnostic.unwrap();
        assert_eq!(diagnostic.operation_id, "invalid-object-info");
        assert!(!diagnostic.exposure_ended_observed);
        let error = diagnostic.response.error.unwrap();
        assert!(error.contains("opcode=0x1008, handle=0xFFFF0001"));
        assert!(error.contains(&format!("metadataLength={length}, metadataShown={shown}, metadataTruncated={oversized}")));
        assert!(error.contains(&format!("metadataHex={expected_hex};")));
        assert!(error.len() < 8500);
        assert!(std::fs::read_dir(path.parent().unwrap()).unwrap().all(|entry| {
            !matches!(entry.unwrap().path().extension().and_then(|ext| ext.to_str()), Some("nef" | "part"))
        }));
    }
}

#[test]
fn runtime_status_cluster_and_object_before_completion_preserve_commit_and_history_gates() {
    let mut s = Script::new();
    s.complete_after_download = true;
    s.capture_events = vec![
        RUNTIME_DIAGNOSTICS
            .iter()
            .map(|row| NikonEvent {
                code: 0x4006,
                parameter: row.2 as u32,
            })
            .collect(),
        vec![NikonEvent {
            code: OBJECT_RAM,
            parameter: 0xffff0001,
        }],
    ];
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let path = output("status-cluster-object-first");
    let (tx, rx) = mpsc::channel();
    let image = a
        .capture_to_path(&path, "status-cluster", &AtomicBool::new(false), Some(tx))
        .unwrap();
    assert!(path.exists() && image.exposure_ended);
    let events: Vec<_> = rx.try_iter().collect();
    let transfer = events
        .iter()
        .position(|event| matches!(event, CameraCaptureEvent::TransferStarted { .. }))
        .unwrap();
    let ended = events
        .iter()
        .position(|event| matches!(event, CameraCaptureEvent::ExposureEnded { .. }))
        .unwrap();
    assert!(transfer < ended);
    let h = a.health();
    assert_eq!(h.runtime_properties["continuousShootingCount"].data_type, 2);
    assert_eq!(h.runtime_properties["exposureIndicateStatus"].data_type, 1);
    assert!(RUNTIME_DIAGNOSTICS
        .iter()
        .all(|row| h.runtime_properties[row.0].semantic_status
            == CameraRuntimeSemanticStatus::Unknown));
    let batches = h.last_capture_event_diagnostic.unwrap().event_batches;
    assert_eq!(batches.len(), 3);
    assert_eq!(batches[0].event_count, RUNTIME_DIAGNOSTICS.len());
    assert_eq!(batches[1].response.events[0].code, OBJECT_RAM);
    assert_eq!(batches[2].response.events[0].code, CAPTURE_COMPLETE_RAM);
}

#[test]
fn host_ram_object_event_alone_completes_and_registers_one_complete_file() {
    // Expectation changed 2026-10-11: the associated RAM object is sufficient
    // "image available" evidence and the SDRAM completion became optional, because
    // the reference library starts ObjectInfo/transfer on the object event and only
    // feeds OnCaptureCompleted from the completion (digiCamControl
    // Nikon/NikonBase.cs:2330-2367 and :2369-2375).
    let mut s = Script::new();
    let mut metadata = measured_d7100_object_info();
    metadata[8..12].copy_from_slice(&(s.raw.len() as u32).to_le_bytes());
    s.object_info = Some(metadata);
    // Only `0xC101`: no `0xC102` RAM completion and no `0x400D` card completion.
    s.capture_events = vec![vec![NikonEvent {
        code: OBJECT_RAM,
        parameter: 0xffff0001,
    }]];
    let expected_size = s.raw.len() as u64;
    assert!(validate_nef(&s.raw).is_ok());
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let session = a.health().session_id.unwrap();
    let path = output("host-ram-object-only");
    let image = a
        .capture_independent_to_path(&path, "object-only", &AtomicBool::new(false))
        .unwrap();
    assert!(path.exists());
    assert_eq!(image.path, path);
    assert_eq!(image.session_id, session);
    assert_eq!(image.operation_id, "object-only");
    assert_eq!(image.object_handle, 0xffff0001);
    assert_eq!(image.format, "NEF");
    assert_eq!(image.size, std::fs::metadata(&path).unwrap().len());
    assert_eq!(image.size, expected_size);
    assert_eq!(
        image.sha256,
        format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(std::fs::read(&path).unwrap()))
    );
    {
        // Exactly one object was requested and transferred; no partial file
        // remains and no image-payload alternative opcode was used.
        let s = script.lock().unwrap();
        assert_eq!(s.calls.iter().filter(|code| **code == GET_OBJECT_INFO).count(), 1);
        assert_eq!(s.calls.iter().filter(|code| **code == GET_OBJECT).count(), 1);
        assert!(!s.calls.contains(&0x101b));
    }
    assert!(std::fs::read_dir(path.parent().unwrap()).unwrap().all(|entry| {
        !matches!(
            entry.unwrap().path().extension().and_then(|ext| ext.to_str()),
            Some("part")
        )
    }));
    let health = a.health();
    assert!(health.connected && !health.isolated && health.cleanup_confirmed);
    // The committed host file is registered for this session and operation.
    assert_eq!(health.last_capture.as_deref(), Some(path.to_string_lossy().as_ref()));
    let diagnostic = health.last_capture_event_diagnostic.unwrap();
    assert_eq!(diagnostic.session_id, session);
    assert_eq!(diagnostic.operation_id, "object-only");
    assert_eq!(diagnostic.capture_complete_count, 0);
    assert!(!diagnostic.exposure_ended_observed);
    assert!(!diagnostic.exposure_observed);
    assert_eq!(diagnostic.event_batches.len(), 1);
    assert_eq!(diagnostic.event_batches[0].response.events[0].code, OBJECT_RAM);
}

#[test]
fn late_ram_completion_after_the_object_transfers_and_commits_exactly_once() {
    // The optional SDRAM completion may arrive after the object and its transfer
    // (digiCamControl Nikon/NikonBase.cs:2330-2367 transfers on the object event;
    // :2369-2375 consumes the completion only for OnCaptureCompleted), so a single
    // late completion for this same operation is absorbed and must never double
    // the transfer or the commit.
    let mut s = Script::new();
    s.capture_events = vec![
        vec![NikonEvent {
            code: OBJECT_RAM,
            parameter: 0xffff0001,
        }],
        vec![NikonEvent {
            code: CAPTURE_COMPLETE_RAM,
            parameter: 0,
        }],
    ];
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let session = a.health().session_id.unwrap();
    let path = output("late-ram-completion");
    let (tx, rx) = mpsc::channel();
    let image = a
        .capture_to_path(&path, "late-completion", &AtomicBool::new(false), Some(tx))
        .unwrap();
    assert!(path.exists() && image.exposure_ended);
    assert_eq!(image.size, std::fs::metadata(&path).unwrap().len());
    let events: Vec<_> = rx.try_iter().collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, CameraCaptureEvent::ObjectAdded { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, CameraCaptureEvent::TransferStarted { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, CameraCaptureEvent::ExposureEnded { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, CameraCaptureEvent::FileComplete { .. }))
            .count(),
        1
    );
    // One object, one transfer, one file: the late completion added no second
    // retrieval and no second commit.
    {
        let s = script.lock().unwrap();
        assert_eq!(s.calls.iter().filter(|code| **code == GET_OBJECT_INFO).count(), 1);
        assert_eq!(s.calls.iter().filter(|code| **code == GET_OBJECT).count(), 1);
    }
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().extension().is_some_and(|ext| ext == "nef"))
            .count(),
        1
    );
    let diagnostic = a.health().last_capture_event_diagnostic.unwrap();
    assert_eq!(diagnostic.session_id, session);
    assert_eq!(diagnostic.operation_id, "late-completion");
    assert_eq!(diagnostic.capture_complete_count, 1);
    assert!(diagnostic.exposure_ended_observed);
    assert!(!diagnostic.exposure_observed);
    assert_eq!(
        diagnostic
            .event_batches
            .last()
            .unwrap()
            .response
            .events[0]
            .code,
        CAPTURE_COMPLETE_RAM
    );
}

#[test]
fn property_only_exposure_notification_never_completes_or_commits_a_capture() {
    // The real D7100 reported one exposure only as `0x4006 PropertyChanged
    // (0xD1B4)` for a host RAM capture that produced no object in the engine's
    // window (docs/validation/card-sd-migration-hardware-2026-10-11.md section 7).
    // The reference library treats `0x4006` purely as "refresh this property"
    // (digiCamControl Nikon/NikonBase.cs:2326-2328) and defines no `0xD1B4`, so
    // this notification is a diagnostic and never commit evidence.
    let context = CallContext {
        deadline: Instant::now() + Duration::from_millis(300),
        epoch: 1,
        generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
    };
    let mut s = Script::new();
    s.capture_events = vec![vec![NikonEvent {
        code: PROPERTY_CHANGED,
        parameter: CONTINUOUS_SHOOTING_COUNT as u32,
    }]];
    let script = Arc::new(Mutex::new(s));
    let mut camera = NativeCamera::new(Box::new(Fake(script.clone())));
    camera
        .connect(
            "fake-camera-1",
            "00112233445566778899aabbccddeeff",
            &context,
        )
        .unwrap();
    let destination = output("property-only-exposure");
    let result = super::capture::capture(
        &mut camera,
        &destination,
        "property-only",
        &None,
        &context,
        &Mutex::new(()),
    );
    // Timeout-class refusal: no object, no file, and the pending exposure stays
    // unresolved so the session is never reported as a successful capture.
    assert!(matches!(result, Err(CameraError::Timeout(_))), "{result:?}");
    assert!(!destination.exists());
    assert!(camera.capture_pending && camera.pending_capture.is_some());
    assert!(!script.lock().unwrap().calls.contains(&GET_OBJECT));
    let diagnostic = camera.health.last_capture_event_diagnostic.unwrap();
    assert!(diagnostic.exposure_observed);
    assert!(!diagnostic.exposure_ended_observed);
    assert_eq!(diagnostic.capture_complete_count, 0);
    assert_eq!(
        diagnostic.event_batches[0].response.events[0].parameter,
        CONTINUOUS_SHOOTING_COUNT as u32
    );
}

#[test]
fn host_ram_path_sends_one_private_sdram_trigger_and_never_the_card_trigger() {
    // The reference library's host/SDRAM path sends the Nikon-private
    // `0x90C0 InitiateCaptureRecInSdram` with parameter `0xFFFFFFFF`
    // (digiCamControl Nikon/NikonBase.cs:61, :1871, :1908); its card path sends
    // standard `0x100E InitiateCapture` instead (BaseMTPCamera.cs:82). This host
    // RAM transaction must send exactly the former (with zero image payload in the
    // trigger itself) and never the latter.
    let (camera, probe) = scripted_camera_with_probe(ScriptedCameraConfig {
        omit_exposure_end: true,
        ..Default::default()
    });
    let path = output("host-ram-trigger-opcode");
    let image = camera
        .capture_independent_to_path(&path, "ram-trigger", &AtomicBool::new(false))
        .unwrap();
    assert!(path.exists() && image.exposure_ended);
    let requests = probe.requests();
    let triggers: Vec<_> = requests
        .iter()
        .filter(|(code, _)| *code == CAPTURE_RAM)
        .collect();
    assert_eq!(triggers.len(), 1, "{requests:X?}");
    assert_eq!(triggers[0].1, vec![0xffffffff]);
    assert_eq!(probe.operation_count(INITIATE_CAPTURE), 0);
    assert_eq!(probe.operation_count(GET_OBJECT_INFO), 1);
    assert_eq!(probe.operation_count(GET_OBJECT), 1);
    // No partial-object opcode is ever used: the reference transfers the whole
    // object with `0x1009 GetObject` and never sends `0x101B`.
    assert!(!probe.calls().contains(&0x101b));
    assert!(probe
        .requests()
        .iter()
        .all(|(code, _)| *code != 0x101b));
}

#[test]
fn host_ram_object_window_is_second_scale_inside_the_owner_deadline() {
    // The object-arrival window must stay a real second-scale wait; `actor.rs`
    // asserts at compile time that it also stays strictly inside the unchanged
    // 75 s camera-owner deadline.
    let window = super::actor::OBJECT_ARRIVAL_WINDOW;
    assert!(window >= Duration::from_secs(1), "{window:?}");
    assert!(window.as_secs() < 75, "{window:?}");
}

#[test]
fn standard_battery_notification_reads_uint8_without_frozen_profile_or_completion_credit() {
    let mut s = Script::new();
    s.capture_value_changes.insert(0x5001, CameraValue::Integer(90));
    s.capture_events.insert(0, vec![NikonEvent { code: 0x4006, parameter: 0x5001 }]);
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let profile = profile_readback(&a.health()).unwrap();
    let path = output("standard-battery-notification");
    let (tx, rx) = mpsc::channel();
    a.capture_to_path(&path, "battery-notification", &AtomicBool::new(false), Some(tx)).unwrap();
    let health = a.health();
    assert_eq!(profile_readback(&health).unwrap(), profile);
    assert!(!health.properties.contains_key("batteryLevel"));
    let battery = &health.runtime_properties["batteryLevel"];
    assert_eq!(battery.data_type, 2);
    assert_eq!(battery.actual, Some(CameraValue::Integer(90)));
    assert!(battery.confirmed && !battery.writable);
    assert_eq!(battery.semantic_status, CameraRuntimeSemanticStatus::Unknown);
    let diagnostic = health.last_capture_event_diagnostic.unwrap();
    assert_eq!(diagnostic.event_batches[0].response.events[0].parameter, 0x5001);
    assert_eq!(diagnostic.event_batches[0].event_count, 1);
    let events: Vec<_> = rx.try_iter().collect();
    assert_eq!(events.iter().filter(|event| matches!(event, CameraCaptureEvent::ExposureEnded { .. })).count(), 1);
    assert!(!script.lock().unwrap().requests.iter().any(|(opcode, params)| *opcode == SET_PROPERTY && params == &[0x5001]));
}

#[test]
fn standard_battery_runtime_refuses_unadvertised_wrong_type_writable_and_unknown_notifications() {
    for failure in ["unadvertised", "wrong-type", "writable", "unknown"] {
        let script = Arc::new(Mutex::new(Script::new()));
        let mut native = NativeCamera::new(Box::new(Fake(script.clone())));
        let context = CallContext {
            deadline: Instant::now() + Duration::from_secs(2), epoch: 1,
            generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        };
        native.connect("fake-camera-1", "battery-test-session", &context).unwrap();
        {
            let mut s = script.lock().unwrap();
            let code = if failure == "unknown" { 0x5002 } else { 0x5001 };
            s.capture_events.insert(0, vec![NikonEvent { code: 0x4006, parameter: code }]);
            if failure == "wrong-type" { s.property_types.insert(0x5001, 4); }
            if failure == "writable" { s.readonly_properties.remove(&0x5001); }
        }
        if failure == "unadvertised" {
            native.info.as_mut().unwrap().properties.retain(|code| *code != 0x5001);
        }
        let path = output("battery-runtime-refused");
        let (tx, rx) = mpsc::channel();
        assert!(super::capture::capture(&mut native, &path, "battery-refused", &Some(tx), &context, &Mutex::new(())).is_err());
        assert!(!path.exists());
        assert!(!rx.try_iter().any(|event| matches!(event,
            CameraCaptureEvent::ExposureEnded { .. } | CameraCaptureEvent::FileComplete { .. })));
        assert!(!script.lock().unwrap().calls.contains(&GET_OBJECT));
    }
}

#[test]
fn runtime_status_cluster_never_accepts_writable_unknown_id_or_frozen_drift() {
    for failure in ["writable", "unknown", "drift"] {
        let script = Arc::new(Mutex::new(Script::new()));
        let a = connected(script.clone());
        {
            let mut s = script.lock().unwrap();
            let code = if failure == "unknown" { 0xd1b5 } else { 0xd1b4 };
            if failure == "writable" {
                s.readonly_properties.remove(&0xd1b4);
            }
            if failure == "drift" {
                s.capture_value_changes
                    .insert(0x500f, CameraValue::Integer(400));
            }
            s.capture_events.insert(
                0,
                vec![NikonEvent {
                    code: 0x4006,
                    parameter: code,
                }],
            );
        }
        let path = output("status-cluster-refused");
        assert!(a
            .capture_to_path(&path, "cluster-rejected", &AtomicBool::new(false), None)
            .is_err());
        assert!(!path.exists());
    }
}

#[test]
fn nonempty_event_history_is_bounded_and_cleanup_keeps_object_before_empty_polls() {
    let mut s = Script::new();
    s.capture_events = (0..19)
        .map(|_| {
            vec![NikonEvent {
                code: 0x4006,
                parameter: 0xd1b4,
            }]
        })
        .collect();
    s.capture_events.extend(Script::new().capture_events);
    let a = connected(Arc::new(Mutex::new(s)));
    let path = output("bounded-event-history");
    a.capture_to_path(&path, "bounded-history", &AtomicBool::new(false), None)
        .unwrap();
    let batches = a
        .health()
        .last_capture_event_diagnostic
        .unwrap()
        .event_batches;
    assert_eq!(batches.len(), 16);
    assert_eq!(batches[0].sequence, 6);
    assert_eq!(batches.last().unwrap().response.events[0].code, OBJECT_RAM);
    let mut s = Script::new();
    s.exposure_delay = Duration::from_millis(100);
    s.capture_events = vec![vec![NikonEvent {
        code: OBJECT_RAM,
        parameter: 0xffff0001,
    }]];
    let (started, receive) = mpsc::channel();
    s.capture_started_signal = Some(started);
    let a = connected(Arc::new(Mutex::new(s)));
    let capture = a.clone();
    let path = output("cleanup-nonempty-history");
    let capture_path = path.clone();
    let job = std::thread::spawn(move || {
        capture.capture_to_path(
            &capture_path,
            "no-complete-history",
            &AtomicBool::new(false),
            None,
        )
    });
    receive.recv_timeout(Duration::from_secs(2)).unwrap();
    a.request_cancel();
    let deadline = Instant::now() + Duration::from_secs(4);
    while a.health().phase != "cleanup_unconfirmed" && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(job.join().unwrap().is_err());
    let d = a.health().last_cleanup_diagnostic.unwrap();
    assert!(!d.exposure_ended_observed);
    assert_eq!(d.event_batches.len(), 1);
    assert_eq!(d.event_batches[0].response.events[0].code, OBJECT_RAM);
    assert!(d.last_event_diagnostic.unwrap().events.is_empty());
    assert!(!path.exists());
}

#[test]
fn d1a4_runtime_notifications_with_busy_reads_keep_frozen_profile_and_commit_current_nef() {
    let mut s = Script::new();
    s.busy_on_capture = true;
    s.capture_value_changes
        .insert(LV_PROHIBIT, CameraValue::Integer(1 << 16));
    s.capture_events.insert(
        0,
        vec![NikonEvent {
            code: 0x4006,
            parameter: LV_PROHIBIT as u32,
        }],
    );
    s.capture_events.push(vec![NikonEvent {
        code: 0x4006,
        parameter: LV_PROHIBIT as u32,
    }]);
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let original = a.profile_readback().unwrap();
    let path = output("d1a4-current-capture");
    let image = a
        .capture_to_path(&path, "d1a4-current", &AtomicBool::new(false), None)
        .unwrap();
    assert!(path.exists() && image.exposure_ended);
    assert_eq!(a.profile_readback().unwrap(), original);
    let h = a.health();
    let diagnostic = &h.runtime_properties["liveViewProhibitCondition"];
    assert_eq!(diagnostic.data_type, 6);
    assert!(!diagnostic.writable);
    assert!(diagnostic.confirmed);
    assert_eq!(diagnostic.actual, Some(CameraValue::Integer(1 << 16)));
    assert_eq!(
        diagnostic.semantic_status,
        CameraRuntimeSemanticStatus::Unknown
    );
    assert!(!a
        .profile_readback()
        .unwrap()
        .contains_key("liveViewProhibitCondition"));
    let s = script.lock().unwrap();
    assert!(
        s.requests
            .iter()
            .filter(|(code, param)| *code == GET_PROPERTY && param == &[0x500f])
            .count()
            >= 4
    );
    assert!(!s.calls.contains(&SET_PROPERTY) && !s.calls.contains(&AF_DRIVE));
}

#[test]
fn d1a4_runtime_notification_cannot_hide_frozen_iso_media_or_burst_drift() {
    for (code, value) in [(0x500f, 400), (RECORDING_MEDIA, 0), (BURST_NUMBER, 2)] {
        let mut s = Script::new();
        s.capture_value_changes
            .insert(code, CameraValue::Integer(value));
        s.capture_events.insert(
            0,
            vec![NikonEvent {
                code: 0x4006,
                parameter: LV_PROHIBIT as u32,
            }],
        );
        let a = connected(Arc::new(Mutex::new(s)));
        let path = output("d1a4-frozen-drift");
        assert!(a
            .capture_to_path(&path, "d1a4-drift", &AtomicBool::new(false), None)
            .is_err());
        assert!(!path.exists());
        assert!(a
            .health()
            .last_capture_event_diagnostic
            .unwrap()
            .response
            .error
            .is_some());
    }
}

#[test]
fn detached_cleanup_survives_repeated_stop_and_confirms_only_same_session_c102() {
    let mut s = Script::new();
    s.exposure_delay = Duration::from_millis(2500);
    s.values.insert(0xd100, CameraValue::Integer((5 << 16) | 2));
    s.descriptor_values
        .insert(0xd100, vec![CameraValue::Integer((5 << 16) | 2)]);
    let (started, receive) = mpsc::channel();
    s.capture_started_signal = Some(started);
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let original_session = a.health().session_id;
    let path = output("detached-c102-cleanup");
    let capture_path = path.clone();
    let capture = a.clone();
    let job = std::thread::spawn(move || {
        capture.capture_to_path(
            &capture_path,
            "cancelled-current",
            &AtomicBool::new(false),
            None,
        )
    });
    receive.recv_timeout(Duration::from_secs(2)).unwrap();
    for _ in 0..4 {
        a.request_cancel();
        std::thread::sleep(Duration::from_millis(40));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !a.health().cleanup_confirmed && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(job.join().unwrap().is_err());
    let h = a.health();
    assert!(h.cleanup_confirmed && !h.connected);
    let d = h.last_cleanup_diagnostic.unwrap();
    assert!(d.pending_at_start && d.exposure_ended_observed && d.close_confirmed);
    assert_eq!(d.session_id, original_session);
    assert_eq!(d.wait_limit_ms, 4500);
    assert!(d.poll_count > 0);
    assert!(!path.exists());
    let s = script.lock().unwrap();
    assert_eq!(s.closes, 1);
    assert_eq!(s.closed_reads, 0);
    assert!(!s.calls.contains(&GET_OBJECT));
}

#[test]
fn detached_cleanup_without_associated_object_stays_unknown_and_closed_transport_cannot_fake_recovery() {
    // Expectation narrowed 2026-10-11: the associated RAM object, not the SDRAM
    // completion, is what resolves a pending exposure (digiCamControl
    // Nikon/NikonBase.cs:2330-2367 retrieves/commits on the object; :2369-2375
    // makes the completion optional), so the unresolved case this test must keep
    // proving is a cancelled exposure with no associated object at all.
    let mut s = Script::new();
    s.capture_events = vec![];
    let (started, receive) = mpsc::channel();
    s.capture_started_signal = Some(started);
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let path = output("detached-missing-object");
    let capture_path = path.clone();
    let capture = a.clone();
    let job = std::thread::spawn(move || {
        capture.capture_to_path(
            &capture_path,
            "missing-current",
            &AtomicBool::new(false),
            None,
        )
    });
    receive.recv_timeout(Duration::from_secs(2)).unwrap();
    a.request_cancel();
    let deadline = Instant::now() + Duration::from_secs(4);
    while a.health().phase != "cleanup_unconfirmed" && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(job.join().unwrap().is_err());
    let initial = a.health();
    assert!(!initial.cleanup_confirmed);
    assert!(
        !initial
            .last_cleanup_diagnostic
            .as_ref()
            .unwrap()
            .exposure_ended_observed
    );
    for _ in 0..3 {
        a.request_cancel();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(a.disconnect_confirmed().is_err());
    assert!(!a.health().cleanup_confirmed);
    assert!(!path.exists());
    let s = script.lock().unwrap();
    assert_eq!(s.closes, 1);
    assert_eq!(s.closed_reads, 0);
    assert_eq!(
        a.health().last_cleanup_diagnostic.unwrap().poll_count,
        initial.last_cleanup_diagnostic.unwrap().poll_count
    );
}

#[test]
fn idle_device_info_changed_with_real_nonzero_parameter_refreshes_without_reopening_or_capture() {
    let mut s = Script::new();
    s.device_info = Some(device_info_extension(
        true,
        6,
        "microsoft.com:1.0; nikon.com:1.0;",
        true,
    ));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let session = a.health().session_id;
    script.lock().unwrap().events.push_back(vec![NikonEvent {
        code: DEVICE_INFO_CHANGED,
        parameter: 0x1000fea8,
    }]);
    std::thread::sleep(Duration::from_millis(650));
    let h = a.health();
    assert!(h.connected && !h.isolated);
    assert_eq!(h.session_id, session);
    assert!(h.last_error.is_none());
    assert!(h
        .properties
        .values()
        .filter(|p| p.supported)
        .all(|p| p.confirmed));
    let diagnostic = h.last_device_info_changed_diagnostic.unwrap();
    assert_eq!(diagnostic.data_prefix_hex, "01000840a8fe0010");
    assert_eq!(diagnostic.events[0].parameter, 0x1000fea8);
    assert!(diagnostic.error.is_none());
    assert_eq!(h.device_info.unwrap().vendor_extension_id, 6);
    let s = script.lock().unwrap();
    assert_eq!(s.opens, 1);
    assert!(
        s.calls
            .iter()
            .filter(|code| **code == GET_DEVICE_INFO)
            .count()
            >= 2
    );
    assert!(!s.calls.iter().any(|code| matches!(
        *code,
        SET_PROPERTY | CAPTURE_RAM | GET_OBJECT | GET_OBJECT_INFO | AF_DRIVE
    )));
}

#[test]
fn device_info_changed_identity_serial_or_binding_drift_retires_before_vendor_discovery() {
    for change in ["manufacturer", "serial", "missingSerial", "namespace"] {
        let mut s = Script::new();
        s.device_info = Some(device_info_catalog(
            true,
            10,
            "extension",
            true,
            0xd100,
            true,
            false,
        ));
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let changed = match change {
            "manufacturer" => {
                device_info_catalog(false, 10, "extension", true, 0xd100, true, false)
            }
            "namespace" => device_info_catalog(true, 11, "extension", true, 0xd100, true, false),
            "serial" => replace_info_string(
                device_info_catalog(true, 10, "extension", true, 0xd100, true, false),
                "fake-serial",
                "another-camera",
            ),
            _ => replace_info_string(
                device_info_catalog(true, 10, "extension", true, 0xd100, true, false),
                "fake-serial",
                "",
            ),
        };
        {
            let mut s = script.lock().unwrap();
            s.calls.clear();
            s.device_info = Some(changed);
            s.events.push_back(vec![NikonEvent {
                code: DEVICE_INFO_CHANGED,
                parameter: 0x1000fea8,
            }]);
        }
        std::thread::sleep(Duration::from_millis(400));
        let h = a.health();
        assert!(!h.connected && h.isolated, "{change}");
        assert!(h
            .last_error
            .unwrap()
            .contains("identity/model/serial or Nikon binding"));
        assert!(h
            .last_device_info_changed_diagnostic
            .unwrap()
            .error
            .is_some());
        assert!(!script
            .lock()
            .unwrap()
            .calls
            .contains(&GET_VENDOR_PROPERTY_CODES));
        assert!(!script.lock().unwrap().calls.contains(&CAPTURE_RAM));
    }
}

#[test]
fn device_info_changed_sleep_or_unknown_functional_mode_never_reuses_old_capture_qualification() {
    for mode in [1u16, 2, 0x8001] {
        let mut s = Script::new();
        s.device_info = Some(device_info_catalog(
            true,
            10,
            "extension",
            true,
            0xd100,
            true,
            false,
        ));
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let mut changed = device_info_catalog(true, 10, "extension", true, 0xd100, true, false);
        let offset = 8 + ptp_string("extension").len();
        changed[offset..offset + 2].copy_from_slice(&mode.to_le_bytes());
        {
            let mut s = script.lock().unwrap();
            s.calls.clear();
            s.device_info = Some(changed);
            s.events.push_back(vec![NikonEvent {
                code: DEVICE_INFO_CHANGED,
                parameter: 0x1000fea8,
            }]);
        }
        std::thread::sleep(Duration::from_millis(400));
        let h = a.health();
        assert!(!h.connected && h.isolated);
        assert_eq!(h.device_info.unwrap().functional_mode, mode);
        assert!(h.last_error.unwrap().contains("FunctionalMode"));
        assert!(!script
            .lock()
            .unwrap()
            .calls
            .contains(&GET_VENDOR_PROPERTY_CODES));
        assert!(!script.lock().unwrap().calls.contains(&CAPTURE_RAM));
    }
}

#[test]
fn device_info_changed_removes_capture_capability_without_old_private_operation_use() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    {
        let mut s = script.lock().unwrap();
        s.requests.clear();
        s.device_info = Some(device_info_extension(true, 10, "extension", false));
        s.events.push_back(vec![NikonEvent {
            code: DEVICE_INFO_CHANGED,
            parameter: 0,
        }]);
    }
    std::thread::sleep(Duration::from_millis(400));
    let h = a.health();
    assert!(h.connected);
    assert!(!h.capabilities.host_capture && !h.capabilities.scan_nef && !h.capabilities.live_view);
    assert!(h.capabilities.disabled_reasons.contains_key("capture"));
    let s = script.lock().unwrap();
    let refresh = s
        .requests
        .iter()
        .position(|(code, _)| *code == GET_DEVICE_INFO)
        .unwrap();
    assert!(!s.requests[refresh + 1..]
        .iter()
        .any(|(code, _)| *code >= 0x9000));
}

#[test]
fn device_info_changed_rediscovers_vendor_properties_and_revokes_removed_parameter() {
    let mut s = Script::new();
    s.device_info = Some(device_info_catalog(
        true,
        10,
        "extension",
        true,
        0xd100,
        true,
        true,
    ));
    let extended: Vec<u16> = NIKON_PROPERTIES
        .iter()
        .map(|p| p.2)
        .filter(|code| *code >= 0x9000)
        .chain([LV_STATUS, INFO_DISPLAY_ERROR_STATUS])
        .collect();
    s.vendor_property_data = Some(array(&extended));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    {
        let mut s = script.lock().unwrap();
        s.requests.clear();
        s.vendor_property_data = Some(array(
            &extended
                .iter()
                .copied()
                .filter(|code| *code != 0xd149)
                .collect::<Vec<_>>(),
        ));
        s.events.push_back(vec![NikonEvent {
            code: DEVICE_INFO_CHANGED,
            parameter: 0x1000fea8,
        }]);
    }
    std::thread::sleep(Duration::from_millis(400));
    let h = a.health();
    assert!(h.connected);
    assert!(!h.properties["rawBitDepth"].supported && !h.properties["rawBitDepth"].confirmed);
    assert!(!h
        .device_info
        .unwrap()
        .effective_properties
        .contains(&0xd149));
    assert!(a.profile_readback().is_err());
    let s = script.lock().unwrap();
    assert!(
        s.calls
            .iter()
            .filter(|code| **code == GET_VENDOR_PROPERTY_CODES)
            .count()
            >= 2
    );
    assert!(!s.requests.iter().any(|(code, params)| matches!(
        *code,
        GET_PROPERTY_DESC | GET_PROPERTY
    ) && params == &[0xd149]));
}

#[test]
fn device_info_changed_does_not_mask_old_capture_in_same_idle_batch() {
    for old_capture in [OBJECT_RAM, CAPTURE_COMPLETE_RAM, 0x4002, 0x400d, 0x400b] {
        let script = Arc::new(Mutex::new(Script::new()));
        let a = connected(script.clone());
        script.lock().unwrap().events.push_back(vec![
            NikonEvent {
                code: DEVICE_INFO_CHANGED,
                parameter: 0x1000fea8,
            },
            NikonEvent {
                code: old_capture,
                parameter: 0xffff0001,
            },
        ]);
        std::thread::sleep(Duration::from_millis(400));
        let h = a.health();
        assert!(!h.connected && h.isolated);
        assert_eq!(
            h.last_device_info_changed_diagnostic.unwrap().events.len(),
            2
        );
        assert!(h.last_capture.is_none());
        let s = script.lock().unwrap();
        assert_eq!(
            s.calls
                .iter()
                .filter(|code| **code == GET_DEVICE_INFO)
                .count(),
            1
        );
        assert!(!s.calls.contains(&GET_OBJECT) && !s.calls.contains(&CAPTURE_RAM));
    }
}

#[test]
fn device_info_changed_refresh_failure_or_malformed_dataset_remains_fail_closed() {
    for malformed in [false, true] {
        let script = Arc::new(Mutex::new(Script::new()));
        let a = connected(script.clone());
        {
            let mut s = script.lock().unwrap();
            if malformed {
                s.device_info = Some(vec![1, 2]);
            } else {
                s.fail_code = Some(GET_DEVICE_INFO);
            }
            s.events.push_back(vec![NikonEvent {
                code: DEVICE_INFO_CHANGED,
                parameter: 0x1000fea8,
            }]);
        }
        std::thread::sleep(Duration::from_millis(400));
        let h = a.health();
        assert!(!h.connected && h.isolated);
        let diagnostic = h.last_device_info_changed_diagnostic.unwrap();
        assert_eq!(diagnostic.events[0].code, DEVICE_INFO_CHANGED);
        assert!(diagnostic.error.is_some());
        assert!(h.last_capture.is_none());
    }
}

#[test]
fn device_info_refresh_uses_existing_shared_deadline_and_refuses_active_acquisition() {
    use std::sync::atomic::AtomicU64;
    let script = Arc::new(Mutex::new(Script::new()));
    let mut camera = NativeCamera::new(Box::new(Fake(script.clone())));
    let mut context = CallContext {
        deadline: Instant::now() + Duration::from_secs(2),
        epoch: 1,
        generation: Arc::new(AtomicU64::new(1)),
    };
    camera
        .connect("fake-camera-1", "fake-session", &context)
        .unwrap();
    camera.capture_pending = true;
    let before = script.lock().unwrap().calls.len();
    assert!(camera.refresh_device_info(&context).is_err());
    assert_eq!(script.lock().unwrap().calls.len(), before);
    camera.capture_pending = false;
    let (started, _) = mpsc::channel();
    script.lock().unwrap().stall = Some((GET_DEVICE_INFO, Duration::from_millis(60), started));
    context.deadline = Instant::now() + Duration::from_millis(20);
    assert!(matches!(
        camera.refresh_device_info(&context),
        Err(CameraError::Timeout(_))
    ));
}

#[test]
fn device_info_changed_at_pretrigger_during_capture_or_after_transfer_never_commits() {
    for stage in ["pretrigger", "capture", "afterTransfer"] {
        let mut s = Script::new();
        let notice = NikonEvent {
            code: DEVICE_INFO_CHANGED,
            parameter: 0x1000fea8,
        };
        if stage == "capture" {
            s.capture_events = vec![vec![
                NikonEvent {
                    code: CAPTURE_COMPLETE_RAM,
                    parameter: 0,
                },
                notice,
                NikonEvent {
                    code: OBJECT_RAM,
                    parameter: 0xffff0001,
                },
            ]];
        } else if stage == "afterTransfer" {
            s.capture_events.push(vec![notice]);
        }
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        if stage == "pretrigger" {
            script.lock().unwrap().events.push_back(vec![notice]);
        }
        let path = output("device-info-capture-boundary");
        let (tx, rx) = mpsc::channel();
        assert!(a
            .capture_to_path(
                &path,
                "device-info-capture",
                &AtomicBool::new(false),
                Some(tx)
            )
            .is_err());
        assert!(!path.exists());
        let h = a.health();
        assert_eq!(
            h.last_device_info_changed_diagnostic
                .unwrap()
                .events
                .iter()
                .find(|event| event.code == DEVICE_INFO_CHANGED)
                .unwrap()
                .parameter,
            0x1000fea8
        );
        assert_eq!(
            script
                .lock()
                .unwrap()
                .calls
                .iter()
                .filter(|code| **code == GET_DEVICE_INFO)
                .count(),
            1
        );
        let exposure_events = rx
            .try_iter()
            .filter(|event| matches!(event, CameraCaptureEvent::ExposureEnded { .. }))
            .count();
        if stage == "pretrigger" {
            assert!(!script.lock().unwrap().calls.contains(&CAPTURE_RAM));
            assert_eq!(exposure_events, 0);
        } else {
            assert_eq!(exposure_events, 1);
            assert!(
                h.last_capture_event_diagnostic
                    .unwrap()
                    .exposure_ended_observed
            );
        }
    }
}

#[test]
fn photographic_controls_follow_confirmed_exposure_modes_and_parent_targets_are_ordered() {
    for (mode, shutter_writable, aperture_writable) in [
        (1, true, true),
        (2, false, false),
        (3, false, true),
        (4, true, false),
    ] {
        let mut s = Script::new();
        s.values.insert(0x500e, CameraValue::Integer(mode));
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let health = a.health();
        assert_eq!(health.properties["shutter"].writable, shutter_writable);
        assert_eq!(health.properties["aperture"].writable, aperture_writable);
        if !shutter_writable {
            assert!(health.properties["shutter"]
                .disabled_reason
                .as_ref()
                .unwrap()
                .contains("M/S"));
            assert!(a
                .apply_properties(&BTreeMap::from([(
                    "shutter".into(),
                    CameraValue::Integer(0x1000a)
                )]))
                .is_err());
            assert!(!script.lock().unwrap().calls.contains(&SET_PROPERTY));
        }
    }
    let mut s = Script::new();
    s.values.insert(0x500e, CameraValue::Integer(3));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    assert!(
        a.apply_properties(&BTreeMap::from([
            ("shutter".into(), CameraValue::Integer(0x1000a)),
            ("exposureMode".into(), CameraValue::Integer(1))
        ]))
        .unwrap()
        .complete
    );
    let s = script.lock().unwrap();
    let writes: Vec<_> = s
        .requests
        .iter()
        .enumerate()
        .filter(|(_, (op, _))| *op == SET_PROPERTY)
        .collect();
    assert_eq!(writes[0].1 .1, [0x500e]);
    assert_eq!(writes[1].1 .1, [0xd100]);
    assert!(s.requests[writes[0].0 + 1..writes[1].0]
        .iter()
        .any(|(op, args)| *op == GET_PROPERTY_DESC && args == &[0xd100]));
}

#[test]
fn white_balance_parent_is_applied_before_its_native_temperature_child() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    assert!(!a.health().properties["whiteBalanceTemperature"].effective);
    assert!(a
        .apply_properties(&BTreeMap::from([(
            "whiteBalanceTemperature".into(),
            CameraValue::Integer(2)
        )]))
        .is_err());
    let result = a
        .apply_properties(&BTreeMap::from([
            ("whiteBalanceTemperature".into(), CameraValue::Integer(2)),
            ("whiteBalance".into(), CameraValue::Integer(0x8012)),
        ]))
        .unwrap();
    assert!(result.complete);
    assert!(result.properties["whiteBalanceTemperature"].effective);
    let writes: Vec<_> = script
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|(op, _)| *op == SET_PROPERTY)
        .map(|(_, args)| args[0])
        .collect();
    assert_eq!(writes, [0x5005, 0xd01e]);
}

#[test]
fn raw_jpeg_controls_keep_iso_baseline_and_descriptor_limited_options() {
    for (format, raw_effective, jpeg_effective) in [
        (4, true, false),
        (2, false, true),
        (5, true, true),
        (6, true, true),
        (7, true, true),
    ] {
        let mut s = Script::new();
        s.values.insert(0x5004, CameraValue::Integer(format));
        s.values.insert(0xd054, CameraValue::Integer(1));
        s.descriptor_values.insert(
            0xd016,
            vec![CameraValue::Integer(0), CameraValue::Integer(1)],
        );
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let h = a.health();
        assert_eq!(h.properties["rawBitDepth"].effective, raw_effective);
        assert_eq!(h.properties["imageSize"].effective, jpeg_effective);
        assert!(h.properties["iso"].writable);
        assert!(h.properties["iso"].label.contains("基准"));
        assert!(h.properties["autoIsoHighLimit"].effective);
        let uncompressed = h.properties["rawCompression"]
            .options
            .iter()
            .find(|o| o.value == CameraValue::Integer(2))
            .unwrap();
        assert!(!uncompressed.supported && !uncompressed.enabled && uncompressed.reason.is_some());
        assert!(a
            .apply_properties(&BTreeMap::from([(
                "rawCompression".into(),
                CameraValue::Integer(2)
            )]))
            .is_err());
        assert!(!script.lock().unwrap().calls.contains(&SET_PROPERTY));
    }
}

#[test]
fn confirmed_noop_full_draft_skips_writes_for_readonly_and_inactive_properties() {
    let mut s = Script::new();
    s.values.insert(0xd161, CameraValue::Integer(3));
    s.readonly_properties.insert(0x500e);
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let values: CameraProperties = a
        .health()
        .properties
        .iter()
        .filter_map(|(key, p)| p.actual.clone().map(|v| (key.clone(), v)))
        .collect();
    assert!(values.len() > 32);
    let result = a.apply_properties(&values).unwrap();
    assert!(result.complete);
    assert_eq!(result.applied.len(), values.len());
    assert!(!script.lock().unwrap().calls.contains(&SET_PROPERTY));
    assert!(a.profile_readback().is_ok());
}

#[test]
fn parent_readback_failure_or_changed_child_descriptor_never_writes_the_child() {
    for mismatch in [true, false] {
        let mut s = Script::new();
        if mismatch {
            s.mismatch = Some(0x5005);
        } else {
            s.descriptor_change_on_set.insert(
                0x5005,
                (
                    0xd01e,
                    vec![CameraValue::Integer(0), CameraValue::Integer(1)],
                ),
            );
        }
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let result = a
            .apply_properties(&BTreeMap::from([
                ("whiteBalance".into(), CameraValue::Integer(0x8012)),
                ("whiteBalanceTemperature".into(), CameraValue::Integer(2)),
            ]))
            .unwrap();
        assert!(!result.complete);
        let writes: Vec<_> = script
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(op, _)| *op == SET_PROPERTY)
            .map(|(_, args)| args[0])
            .collect();
        assert_eq!(writes, [0x5005]);
        assert!(a.profile_readback().is_err());
        assert_eq!(
            result.properties["whiteBalanceTemperature"].actual,
            Some(CameraValue::Integer(0))
        );
    }
}

#[test]
fn native_range_stays_complete_and_image_size_strings_use_described_type() {
    let mut s = Script::new();
    s.values.insert(0x5005, CameraValue::Integer(0x8012));
    s.values.insert(0x5004, CameraValue::Integer(2));
    s.descriptor_ranges.insert(0xd01e, [0, 30, 1]);
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let h = a.health();
    let p = &h.properties["whiteBalanceTemperature"];
    assert_eq!(p.range, Some([0, 30, 1]));
    assert!(p.supported_values.is_empty());
    assert!(p.options.iter().any(|o| o.value == CameraValue::Integer(29)
        && o.supported
        && o.enabled
        && o.label == "9090 K"));
    assert!(
        a.apply_properties(&BTreeMap::from([
            ("whiteBalanceTemperature".into(), CameraValue::Integer(29)),
            ("imageSize".into(), CameraValue::Text("4496x3000".into()))
        ]))
        .unwrap()
        .complete
    );
    assert_eq!(
        script.lock().unwrap().values[&0x5003],
        CameraValue::Text("4496x3000".into())
    );
}

#[test]
fn unadvertised_known_controls_are_visible_without_private_property_probes() {
    let mut s = Script::new();
    s.device_info = Some(device_info_catalog(
        true,
        10,
        "extension",
        true,
        0xd100,
        false,
        true,
    ));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let h = a.health();
    let unsupported = &h.properties["whiteBalanceTemperature"];
    assert!(!unsupported.supported && !unsupported.writable && !unsupported.effective);
    assert!(unsupported.disabled_reason.is_some());
    assert!(unsupported
        .options
        .iter()
        .all(|o| !o.supported && !o.enabled && o.reason.is_some()));
    assert!(!script
        .lock()
        .unwrap()
        .requests
        .iter()
        .any(
            |(op, args)| matches!(*op, GET_PROPERTY | GET_PROPERTY_DESC | SET_PROPERTY)
                && args == &[0xd01e]
        ));
}

#[test]
fn hdr_off_stays_selectable_in_nef_and_capture_readiness_tracks_actual_without_losing_capability() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    let h = a.health();
    assert!(h.properties["hdrMode"].effective && h.properties["hdrMode"].writable);
    assert!(
        h.properties["hdrMode"]
            .options
            .iter()
            .find(|o| o.value == CameraValue::Integer(0))
            .unwrap()
            .enabled
    );
    assert!(
        !h.properties["hdrMode"]
            .options
            .iter()
            .find(|o| o.value == CameraValue::Integer(1))
            .unwrap()
            .enabled
    );
    assert!(!h.capabilities.disabled_reasons.contains_key("captureReady"));
    script
        .lock()
        .unwrap()
        .values
        .insert(0xd130, CameraValue::Integer(1));
    a.read_properties().unwrap();
    let h = a.health();
    assert!(h.capabilities.scan_nef && h.capabilities.host_capture);
    assert!(h.capabilities.disabled_reasons["captureReady"].contains("HDR"));
    assert!(
        a.apply_properties(&BTreeMap::from([(
            "hdrMode".into(),
            CameraValue::Integer(0)
        )]))
        .unwrap()
        .complete
    );
    assert!(!a
        .health()
        .capabilities
        .disabled_reasons
        .contains_key("captureReady"));
}

#[test]
fn live_view_and_unknown_dependency_readbacks_disable_controls_and_capture_readiness() {
    let a = connected(Arc::new(Mutex::new(Script::new())));
    a.start_live_view().unwrap();
    let h = a.health();
    assert!(h.properties.values().all(|p| !p.writable && !p.effective));
    assert!(h.properties["exposureMode"]
        .disabled_reason
        .as_ref()
        .unwrap()
        .contains("Live View"));
    assert!(h.properties["rawFormat"].options.iter().all(|o| !o.enabled));
    assert!(h.capabilities.disabled_reasons["captureReady"].contains("Live View"));
    a.stop_live_view().unwrap();
    assert!(a.health().properties["shutter"].writable);
    let mut h = a.health();
    h.properties.get_mut("exposureMode").unwrap().confirmed = false;
    super::properties::refresh_property_controls(&mut h);
    assert!(!h.properties["shutter"].effective && !h.properties["shutter"].writable);
    h.properties.get_mut("exposureMode").unwrap().confirmed = true;
    h.properties.get_mut("exposureMode").unwrap().error = Some("stale readback".into());
    super::properties::refresh_property_controls(&mut h);
    assert!(!h.properties["aperture"].writable);
}

#[test]
fn manual_pasm_dependencies_preserve_actuals_and_do_not_unlock_from_unknown_mode() {
    let a = connected(Arc::new(Mutex::new(Script::new())));
    let baseline = a.health();
    let keys = ["autoIso", "whiteBalance", "pictureControl", "meteringMode",
        "activeDLighting", "bracketing", "hdrMode", "flashCompensation"];
    for unknown in [false, true] {
        let mut health = baseline.clone();
        let mode = health.properties.get_mut("exposureMode").unwrap();
        mode.actual = Some(CameraValue::Integer(0x8010));
        mode.confirmed = !unknown;
        super::properties::refresh_property_controls(&mut health);
        for key in keys {
            let property = &health.properties[key];
            assert!(!property.effective && !property.writable, "{key} must require confirmed P/S/A/M");
            assert!(property.options.iter().all(|o| !o.enabled));
            assert_eq!(property.actual, baseline.properties[key].actual);
        }
        assert!(!health.properties["exposureCompensation"].writable);
    }
}

#[test]
fn hdr_mutual_exclusion_disables_only_incompatible_options_and_keeps_off_available() {
    let mut script = Script::new();
    script.values.insert(0x5004, CameraValue::Integer(2));
    script.values.insert(0xd130, CameraValue::Integer(1));
    let shared = Arc::new(Mutex::new(script));
    let a = connected(shared.clone());
    let health = a.health();
    for (key, value) in [("rawFormat", 4), ("bracketing", 1), ("flashMode", 0x8010)] {
        assert!(super::properties::option_reason(key, &CameraValue::Integer(value), health.properties[key].code, &health.properties).is_some());
        assert!(a.apply_properties(&BTreeMap::from([(key.into(), CameraValue::Integer(value))])).is_err());
    }
    assert!(!shared.lock().unwrap().calls.contains(&SET_PROPERTY));
    assert!(!health.properties["flashCompensation"].writable);
    assert!(!health.properties["flashSyncSpeed"].writable);
    assert!(health.properties["hdrMode"].options.iter().any(|o| o.value == CameraValue::Integer(0) && o.enabled));
    assert!(health.properties["bracketing"].options.iter().any(|o| o.value == CameraValue::Integer(0) && o.enabled));
}

#[test]
fn explicit_hdr_and_bracketing_off_are_written_before_entering_the_other_mode() {
    for (start_hdr, start_bracket, target) in [
        (1, 0, BTreeMap::from([("hdrMode".into(), CameraValue::Integer(0)), ("rawFormat".into(), CameraValue::Integer(4))])),
        (0, 1, BTreeMap::from([("bracketing".into(), CameraValue::Integer(0)), ("hdrMode".into(), CameraValue::Integer(1))])),
    ] {
        let mut script = Script::new();
        script.values.insert(0x5004, CameraValue::Integer(2));
        script.values.insert(0xd130, CameraValue::Integer(start_hdr));
        script.values.insert(0xd0c0, CameraValue::Integer(start_bracket));
        let shared = Arc::new(Mutex::new(script));
        let a = connected(shared.clone());
        assert!(a.apply_properties(&target).unwrap().complete);
        let script = shared.lock().unwrap();
        let writes = script.requests.iter().enumerate().filter(|(_, (op, _))| *op == SET_PROPERTY).collect::<Vec<_>>();
        let expected = if start_hdr == 1 { [0xd130, 0x5004] } else { [0xd0c0, 0xd130] };
        assert_eq!(writes.iter().map(|(_, (_, args))| args[0]).collect::<Vec<_>>(), expected);
        assert!(script.requests[writes[0].0 + 1..writes[1].0].iter().any(|(op, args)| *op == GET_PROPERTY_DESC && args == &[expected[1]]));
    }
}

#[test]
fn contradictory_hdr_targets_are_rejected_before_any_write() {
    for conflict in [BTreeMap::from([("hdrMode".into(), CameraValue::Integer(1)), ("bracketing".into(), CameraValue::Integer(1))]),
        BTreeMap::from([("hdrMode".into(), CameraValue::Integer(1)), ("rawFormat".into(), CameraValue::Integer(4))])] {
        let mut script = Script::new();
        script.values.insert(0x5004, CameraValue::Integer(2));
        let shared = Arc::new(Mutex::new(script));
        let a = connected(shared.clone());
        assert!(a.apply_properties(&conflict).is_err());
        assert!(!shared.lock().unwrap().calls.contains(&SET_PROPERTY));
    }
}

#[test]
fn slow_sync_and_unknown_hdr_follow_manual_without_disabling_all_bracketing_in_nef() {
    let a = connected(Arc::new(Mutex::new(Script::new())));
    let mut health = a.health();
    assert!(health.properties["bracketing"].options.iter().any(|o| o.value == CameraValue::Integer(1) && o.enabled));
    for (mode, allowed) in [(1, false), (2, true), (3, true), (4, false)] {
        health.properties.get_mut("exposureMode").unwrap().actual = Some(CameraValue::Integer(mode));
        for value in [0x8011, 0x8013] {
            assert_eq!(super::properties::option_reason("flashMode", &CameraValue::Integer(value), 0x500c, &health.properties).is_none(), allowed);
        }
    }
    health.properties.get_mut("hdrMode").unwrap().confirmed = false;
    super::properties::refresh_property_controls(&mut health);
    assert!(health.properties["bracketing"].options.iter().any(|o| o.value == CameraValue::Integer(0) && o.enabled));
    assert!(health.properties["bracketing"].options.iter().any(|o| o.value == CameraValue::Integer(1) && !o.enabled));
    assert!(super::properties::option_reason("rawFormat", &CameraValue::Integer(5), 0x5004, &health.properties).is_some());
}

#[test]
fn standard_unknown_brand_image_size_uses_its_descriptor_without_nikon_dependencies() {
    let mut s = Script::new();
    s.nikon = false;
    s.values.insert(0x500d, CameraValue::Integer(2000));
    s.device_info = Some(device_info_shutter(false, 11, "extension", false, 0x500d));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    assert!(a.health().properties["imageSize"].writable);
    assert!(
        a.apply_properties(&BTreeMap::from([(
            "imageSize".into(),
            CameraValue::Text("4496x3000".into())
        )]))
        .unwrap()
        .complete
    );
    assert!(!script
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|code| *code >= 0x9000));
}

#[test]
fn inherited_d7100_labels_match_auto_iso_and_center_weighted_native_codes() {
    let a = connected(Arc::new(Mutex::new(Script::new())));
    let h = a.health();
    assert_eq!(
        h.properties["autoIsoHighLimit"]
            .options
            .iter()
            .find(|o| o.value == CameraValue::Integer(1))
            .unwrap()
            .label,
        "ISO 250"
    );
    assert_eq!(
        h.properties["autoIsoHighLimit"]
            .options
            .iter()
            .find(|o| o.value == CameraValue::Integer(20))
            .unwrap()
            .label,
        "ISO 6400"
    );
    assert_eq!(
        h.properties["centerWeightedArea"]
            .options
            .iter()
            .find(|o| o.value == CameraValue::Integer(3))
            .unwrap()
            .label,
        "13 mm"
    );
    let bulb = h.properties["shutter"]
        .options
        .iter()
        .find(|o| o.value == CameraValue::Integer(0xffffffff))
        .unwrap();
    assert!(bulb.supported && !bulb.enabled && bulb.reason.is_some());
}

#[test]
fn host_capture_explicitly_prepares_typed_sdram_and_single_image_before_trigger() {
    let mut s = Script::new();
    s.values.insert(RECORDING_MEDIA, CameraValue::Integer(0));
    s.values.insert(BURST_NUMBER, CameraValue::Integer(2));
    s.values.insert(0xd161, CameraValue::Integer(3));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    assert!(a.profile_readback().is_err());
    a.configure_host_capture(200.0).unwrap();
    let profile = a.profile_readback().unwrap();
    assert_eq!(profile["recordingMedia"], CameraValue::Integer(1));
    assert_eq!(profile["burstNumber"], CameraValue::Integer(1));
    assert!(!profile.contains_key("infoDisplayErrorStatus"));
    let path = output("explicit-sdram");
    a.capture_to_path(&path, "explicit-sdram", &AtomicBool::new(false), None)
        .unwrap();
    let s = script.lock().unwrap();
    let trigger = s
        .requests
        .iter()
        .position(|(code, _)| *code == CAPTURE_RAM)
        .unwrap();
    for property in [RECORDING_MEDIA, BURST_NUMBER] {
        let before = &s.requests[..trigger];
        let write = before
            .iter()
            .position(|(code, params)| *code == SET_PROPERTY && params == &[property as u32])
            .unwrap();
        assert!(before[..write]
            .iter()
            .any(|(code, params)| *code == GET_PROPERTY_DESC && params == &[property as u32]));
        assert!(before[..write]
            .iter()
            .any(|(code, params)| *code == GET_PROPERTY && params == &[property as u32]));
        assert!(before[write + 1..]
            .iter()
            .any(|(code, params)| *code == GET_PROPERTY && params == &[property as u32]));
    }
    assert!(!s
        .requests
        .iter()
        .any(|(code, params)| *code == SET_PROPERTY && params == &[0xd161]));
}

#[test]
fn host_capture_preparation_refuses_readonly_wrong_type_and_readback_mismatch() {
    for property in [RECORDING_MEDIA, BURST_NUMBER] {
        for failure in ["readonly", "type", "mismatch"] {
            let mut s = Script::new();
            s.values.insert(property, CameraValue::Integer(0));
            match failure {
                "readonly" => {
                    s.readonly_properties.insert(property);
                }
                "type" => {
                    s.property_types.insert(property, 6);
                }
                _ => s.mismatch = Some(property),
            }
            let script = Arc::new(Mutex::new(s));
            let a = connected(script.clone());
            assert!(
                a.configure_host_capture(200.0).is_err(),
                "{property:04x} {failure}"
            );
            assert!(!script.lock().unwrap().calls.contains(&CAPTURE_RAM));
            assert!(a.profile_readback().is_err());
        }
    }
}

#[test]
fn capture_validation_does_not_repair_changed_sdram_or_burst_from_frozen_profile() {
    for property in [RECORDING_MEDIA, BURST_NUMBER] {
        let script = Arc::new(Mutex::new(Script::new()));
        let a = connected(script.clone());
        script
            .lock()
            .unwrap()
            .values
            .insert(property, CameraValue::Integer(0));
        let path = output("frozen-host-drift");
        assert!(a
            .capture_to_path(&path, "frozen-host-drift", &AtomicBool::new(false), None)
            .is_err());
        let s = script.lock().unwrap();
        assert!(!s.calls.contains(&CAPTURE_RAM));
        assert!(!s.calls.contains(&SET_PROPERTY));
        assert!(!path.exists());
    }
}

#[test]
fn runtime_raw_zero_and_one_are_opaque_diagnostics_with_complete_capture_evidence() {
    for raw_value in [0, 1] {
        let mut s = Script::new();
        s.values
            .insert(INFO_DISPLAY_ERROR_STATUS, CameraValue::Integer(raw_value));
        s.capture_events.insert(
            0,
            vec![NikonEvent {
                code: 0x4006,
                parameter: INFO_DISPLAY_ERROR_STATUS as u32,
            }],
        );
        s.capture_events.push(vec![NikonEvent {
            code: 0x4006,
            parameter: INFO_DISPLAY_ERROR_STATUS as u32,
        }]);
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let status = &a.health().runtime_properties["infoDisplayErrorStatus"];
        assert_eq!(status.data_type, 2);
        assert!(!status.writable);
        assert_eq!(status.actual, Some(CameraValue::Integer(raw_value)));
        assert_eq!(status.semantic_status, CameraRuntimeSemanticStatus::Unknown);
        let json = serde_json::to_value(a.health()).unwrap();
        assert_eq!(
            json["runtimeProperties"]["infoDisplayErrorStatus"]["semanticStatus"],
            "unknown"
        );
        assert!(json["properties"]["recordingMedia"]
            .get("semanticStatus")
            .is_none());
        assert!(!a
            .profile_readback()
            .unwrap()
            .contains_key("infoDisplayErrorStatus"));
        let path = output("opaque-runtime-notifications");
        let image = a
            .capture_to_path(&path, "opaque-runtime", &AtomicBool::new(false), None)
            .unwrap();
        assert!(path.exists());
        assert!(image.exposure_ended);
        assert_eq!(
            image.sha256,
            format!(
                "{:x}",
                <sha2::Sha256 as sha2::Digest>::digest(std::fs::read(&path).unwrap())
            )
        );
        let health = a.health();
        assert!(health.connected);
        assert!(!health.capabilities.hardware_certified);
        assert_eq!(
            health.runtime_properties["infoDisplayErrorStatus"].semantic_status,
            CameraRuntimeSemanticStatus::Unknown
        );
        assert_eq!(
            health
                .last_capture_event_diagnostic
                .unwrap()
                .response
                .events[0]
                .parameter,
            INFO_DISPLAY_ERROR_STATUS as u32
        );
    }
}

#[test]
fn runtime_notification_cannot_hide_unreadable_status_or_changed_recording_media() {
    for (property, value) in [
        (
            INFO_DISPLAY_ERROR_STATUS,
            CameraValue::Text("unknown".into()),
        ),
        (RECORDING_MEDIA, CameraValue::Integer(0)),
    ] {
        let mut s = Script::new();
        s.capture_value_changes.insert(property, value);
        s.capture_events.insert(
            0,
            vec![NikonEvent {
                code: 0x4006,
                parameter: INFO_DISPLAY_ERROR_STATUS as u32,
            }],
        );
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let path = output("unsafe-runtime-notification");
        assert!(a
            .capture_to_path(&path, "unsafe-runtime", &AtomicBool::new(false), None)
            .is_err());
        assert!(script.lock().unwrap().calls.contains(&CAPTURE_RAM));
        assert!(!path.exists());
        assert!(a
            .health()
            .last_capture_event_diagnostic
            .unwrap()
            .response
            .error
            .is_some());
    }
}

#[test]
fn runtime_status_wrong_type_writable_or_unreadable_prevents_trigger() {
    for failure in ["type", "writable", "unreadable"] {
        let script = Arc::new(Mutex::new(Script::new()));
        let a = connected(script.clone());
        {
            let mut s = script.lock().unwrap();
            match failure {
                "type" => {
                    s.property_types.insert(INFO_DISPLAY_ERROR_STATUS, 6);
                }
                "writable" => {
                    s.readonly_properties.remove(&INFO_DISPLAY_ERROR_STATUS);
                }
                _ => s.unreadable_property = Some(INFO_DISPLAY_ERROR_STATUS),
            }
        }
        let path = output("runtime-pretrigger-refusal");
        assert!(
            a.capture_to_path(&path, "runtime-refusal", &AtomicBool::new(false), None)
                .is_err(),
            "{failure}"
        );
        assert!(!script.lock().unwrap().calls.contains(&CAPTURE_RAM));
        assert!(!path.exists());
    }
}

#[test]
fn acquisition_and_unknown_notifications_are_rejected_during_and_after_transfer() {
    for property in [RECORDING_MEDIA, BURST_NUMBER, 0x500f, 0xd1b5] {
        for after_transfer in [false, true] {
            let mut s = Script::new();
            let batch = vec![NikonEvent {
                code: 0x4006,
                parameter: property as u32,
            }];
            if after_transfer {
                s.capture_events.push(batch);
            } else {
                s.capture_events.insert(0, batch);
            }
            let a = connected(Arc::new(Mutex::new(s)));
            let path = output("acquisition-notification-refusal");
            assert!(a
                .capture_to_path(&path, "acquisition-change", &AtomicBool::new(false), None)
                .is_err());
            assert!(!path.exists());
            assert_eq!(
                a.health()
                    .last_capture_event_diagnostic
                    .unwrap()
                    .response
                    .events[0]
                    .parameter,
                property as u32
            );
        }
    }
}

#[test]
fn posttransfer_runtime_notifications_require_true_empty_boundary_and_reject_duplicates() {
    for endless in [false, true] {
        let mut s = Script::new();
        s.capture_events.extend((0..8).map(|_| {
            vec![NikonEvent {
                code: 0x4006,
                parameter: INFO_DISPLAY_ERROR_STATUS as u32,
            }]
        }));
        if !endless {
            s.capture_events[3] = vec![NikonEvent {
                code: CAPTURE_COMPLETE_RAM,
                parameter: 0,
            }];
        }
        let a = connected(Arc::new(Mutex::new(s)));
        let path = output("posttransfer-boundary-refusal");
        assert!(a
            .capture_to_path(
                &path,
                "posttransfer-boundary",
                &AtomicBool::new(false),
                None
            )
            .is_err());
        assert!(!path.exists());
    }
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
fn d7100_mtp_namespace_is_bound_to_exact_identity_and_preserves_reported_diagnostics() {
    for vendor in [6, u32::MAX] {
        let packet =
            device_info_extension(true, vendor, "microsoft.com: 1.0; nikon.com: 1.0;", true);
        let info = parse_device_info(&packet).unwrap();
        assert!(d7100_selection(&info, "fake-camera-1").is_some());
        let mut script = Script::new();
        script.device_info = Some(packet.clone());
        let a = connected(Arc::new(Mutex::new(script)));
        let health = a.health();
        assert_eq!(health.backend, "windows_wpd_nikon_d7100");
        assert!(health.capabilities.host_capture && health.capabilities.live_view);
        let diagnostic = health.device_info.unwrap();
        assert_eq!(diagnostic.vendor_extension_id, vendor);
        assert_eq!(
            diagnostic.vendor_extension_description,
            "microsoft.com: 1.0; nikon.com: 1.0;"
        );
        assert_eq!(diagnostic.standard_version, 100);
        assert_eq!(diagnostic.dataset_size, packet.len());
        assert_eq!(diagnostic.device_version, "firmware-test");
        assert!(diagnostic.operations.contains(&START_LV));
        assert_eq!(diagnostic.dataset_sha256.len(), 64);
        assert!(!health.capabilities.hardware_certified);
    }
}
#[test]
fn mtp_d7100_usb_binding_does_not_accept_unknown_brand_model_or_namespace() {
    let usb = r"\\?\usb#vid_04b0&pid_0430#test-camera";
    let info =
        parse_device_info(&device_info_extension(true, 6, "microsoft.com: 1.0;", true)).unwrap();
    assert!(d7100_selection(&info, usb).is_some());
    assert!(d7100_selection(&info, "fake-camera-1").is_none());
    let mut info = info;
    info.manufacturer = "Not Nikon Corporation".into();
    assert!(d7100_selection(&info, usb).is_none());
    info.manufacturer = "Nikon Corporation".into();
    info.model = "D7200".into();
    assert!(d7100_selection(&info, usb).is_none());
    info.model = "D7100".into();
    info.vendor_id = 0x0b;
    assert!(d7100_selection(&info, usb).is_none());
    let mut script = Script::new();
    script.nikon = false;
    script.device_info = Some(device_info_extension(
        false,
        6,
        "microsoft.com: 1.0; nikon.com: 1.0;",
        true,
    ));
    let script = Arc::new(Mutex::new(script));
    let a = connected(script.clone());
    assert!(!a.health().capabilities.host_capture);
    assert!(a.start_live_view().is_err());
    assert!(script
        .lock()
        .unwrap()
        .calls
        .iter()
        .all(|code| *code < 0x9000));
}
#[test]
fn identified_d7100_without_advertised_private_operations_is_diagnostic_only() {
    let mut script = Script::new();
    script.device_info = Some(device_info_extension(
        true,
        6,
        "microsoft.com: 1.0; nikon.com: 1.0;",
        false,
    ));
    let script = Arc::new(Mutex::new(script));
    let a = connected(script.clone());
    assert_eq!(a.health().backend, "windows_wpd_nikon_d7100");
    assert!(!a.health().capabilities.host_capture);
    assert!(!a.health().capabilities.live_view);
    assert!(!a.health().capabilities.autofocus);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        script
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|code| *code < 0x9000),
        "discovery/idle must not probe unadvertised private commands"
    );
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
    assert!(shutter_ms(&CameraValue::Integer(0xffffffff), 0xd100).is_none());
}

#[test]
fn d7100_standard_shutter_uses_advertised_code_for_range_apply_and_profile() {
    let mut s = Script::new();
    s.values.remove(&0xd100);
    s.values.insert(0x500d, CameraValue::Integer(20000));
    s.device_info = Some(device_info_shutter(
        true,
        6,
        "microsoft.com: 1.0; nikon.com: 1.0;",
        true,
        0x500d,
    ));
    let a = connected(Arc::new(Mutex::new(s)));
    let health = a.health();
    assert_eq!(health.backend, "windows_wpd_nikon_d7100");
    assert_eq!(health.properties["shutter"].code, 0x500d);
    assert_eq!(health.exposure_min_ms, Some(100.0));
    assert_eq!(health.exposure_max_ms, Some(30000.0));
    assert_eq!(
        shutter_property_ms(&health.properties["shutter"]).unwrap(),
        2000.0
    );
    assert_eq!(
        exact_shutter_value(&health, 2000.0).unwrap(),
        CameraValue::Integer(20000)
    );
    assert!(exact_shutter_value(&health, 1999.9).is_err());
    assert_eq!(
        a.profile_readback().unwrap()["shutter"],
        CameraValue::Integer(20000)
    );
    a.set_exposure_ms(2000.0).unwrap();
    assert_eq!(
        shutter_property_ms(&a.health().properties["shutter"]).unwrap(),
        2000.0
    );
}
#[test]
fn mixed_d7100_standard_shutter_does_not_probe_unadvertised_private_commands() {
    let mut s = Script::new();
    s.values.remove(&0xd100);
    s.values.insert(0x500d, CameraValue::Integer(20000));
    s.device_info = Some(device_info_shutter(
        true,
        6,
        "microsoft.com: 1.0; nikon.com: 1.0;",
        false,
        0x500d,
    ));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    assert_eq!(
        shutter_property_ms(&a.health().properties["shutter"]).unwrap(),
        2000.0
    );
    assert_eq!(
        exact_shutter_value(&a.health(), 2000.0).unwrap(),
        CameraValue::Integer(20000)
    );
    assert!(!a.health().capabilities.host_capture);
    assert!(a.profile_readback().is_err());
    std::thread::sleep(Duration::from_millis(300));
    assert!(script
        .lock()
        .unwrap()
        .calls
        .iter()
        .all(|code| *code < 0x9000));
}
#[test]
fn shutter_decoder_rejects_unknown_code_type_or_unconfirmed_actual_without_model_guessing() {
    let a = connected(Arc::new(Mutex::new(Script::new())));
    let mut property = a.health().properties["shutter"].clone();
    property.actual = Some(CameraValue::Integer(20000));
    property.code = 0x500d;
    assert_eq!(shutter_property_ms(&property).unwrap(), 2000.0);
    property.code = 0xd100;
    assert!(shutter_property_ms(&property).is_err());
    property.code = 0xd101;
    assert!(shutter_property_ms(&property).is_err());
    assert!(shutter_ms(&CameraValue::Integer(20000), 0xd101).is_none());
    property.code = 0x500d;
    property.data_type = 4;
    assert!(shutter_property_ms(&property).is_err());
    property.data_type = 6;
    property.confirmed = false;
    assert!(shutter_property_ms(&property).is_err());
    property.confirmed = true;
    property.actual = None;
    assert!(shutter_property_ms(&property).is_err());
}
#[test]
fn advertised_vendor_property_discovery_preserves_base_and_enables_only_returned_properties() {
    let mut s = Script::new();
    s.values.insert(0x500d, CameraValue::Integer(20000));
    let base = device_info_catalog(
        true,
        6,
        "microsoft.com: 1.0; nikon.com: 1.0;",
        true,
        0x500d,
        true,
        true,
    );
    let base_codes = parse_device_info(&base).unwrap().properties;
    let extended: Vec<u16> = NIKON_PROPERTIES
        .iter()
        .map(|p| p.2)
        .filter(|code| *code >= 0x9000)
        .chain([LV_STATUS, 0xd100])
        .collect();
    s.device_info = Some(base);
    s.vendor_property_data = Some(array(&extended));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let health = a.health();
    let diagnostic = health.device_info.unwrap();
    assert_eq!(diagnostic.properties, base_codes);
    assert!(!diagnostic.properties.contains(&LV_STATUS));
    assert!(diagnostic.effective_properties.contains(&LV_STATUS));
    assert!(health.capabilities.live_view);
    assert_eq!(health.properties["shutter"].code, 0xd100);
    assert!(a.profile_readback().is_ok());
    let extension = diagnostic.vendor_property_codes.unwrap();
    assert_eq!(extension.properties, extended);
    assert_eq!(extension.response_code, OK);
    assert_eq!(extension.operation_code, GET_VENDOR_PROPERTY_CODES);
    assert_eq!(extension.dataset_sha256.len(), 64);
    assert_eq!(
        script
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|code| **code == GET_VENDOR_PROPERTY_CODES)
            .count(),
        1
    );
}
#[test]
fn malformed_or_unknown_brand_vendor_discovery_never_manufactures_capabilities() {
    for payload in [
        vec![2, 0, 0, 0, 0x00, 0xd1],
        vec![1, 0, 0, 0, 0xff, 0xff],
        vec![0, 0, 0, 0, 1],
    ] {
        let mut s = Script::new();
        s.device_info = Some(device_info_catalog(
            true,
            6,
            "nikon.com: 1.0;",
            true,
            0x500d,
            true,
            true,
        ));
        s.vendor_property_data = Some(payload);
        let a = adapter(Arc::new(Mutex::new(s)));
        assert!(a.connect_selected("fake-camera-1").is_err());
        let health = a.health();
        assert!(!health.connected);
        assert!(health
            .device_info
            .unwrap()
            .vendor_property_codes
            .unwrap()
            .error
            .is_some());
    }
    let mut s = Script::new();
    s.nikon = false;
    s.device_info = Some(device_info_catalog(
        false,
        6,
        "nikon.com: 1.0;",
        true,
        0x500d,
        true,
        true,
    ));
    s.values.insert(0x500d, CameraValue::Integer(20000));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    assert!(!a.health().capabilities.live_view);
    assert!(!script
        .lock()
        .unwrap()
        .calls
        .contains(&GET_VENDOR_PROPERTY_CODES));
    assert!(parse_vendor_property_codes(&[0, 0, 0, 0])
        .unwrap()
        .is_empty());
    assert!(parse_vendor_property_codes(&[1, 0x10, 0, 0]).is_err());
}
#[test]
fn idle_unknown_event_retirement_keeps_code_parameter_and_raw_response_until_reconnect() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    script.lock().unwrap().events.push_back(vec![NikonEvent {
        code: 0xc105,
        parameter: 0xdeadbeef,
    }]);
    std::thread::sleep(Duration::from_millis(400));
    let health = a.health();
    assert!(!health.connected);
    assert!(health.isolated);
    assert!(health.cleanup_confirmed);
    let error = health.last_error.unwrap();
    assert!(error.contains("0xC105"));
    assert!(error.contains("0xDEADBEEF"));
    let event = health.last_event_diagnostic.unwrap();
    assert_eq!(event.response_code, Some(OK));
    assert_eq!(event.events[0].code, 0xc105);
    assert_eq!(event.events[0].parameter, 0xdeadbeef);
    assert_eq!(event.data_prefix_hex.len(), 16);
    let fresh = a.connect_selected("fake-camera-1").unwrap();
    assert!(fresh.last_error.is_none());
    assert!(fresh.last_event_diagnostic.is_none());
}
#[test]
fn idle_unassociated_ram_object_event_stays_fail_closed_and_grants_no_capture_credit() {
    // The real hardware left an orphan `0xC101` (`parameter=0x0B000001`) that the
    // next session had to judge while idle (docs/validation/
    // card-sd-migration-hardware-2026-10-11.md section 7). It stays fail-closed and
    // isolated, and only the wording names it as a RAM object event with an
    // unverified parameter rather than the standard `0x400D` completion.
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    let session = a.health().session_id;
    script.lock().unwrap().events.push_back(vec![NikonEvent {
        code: OBJECT_RAM,
        parameter: 0x0b00_0001,
    }]);
    std::thread::sleep(Duration::from_millis(400));
    let health = a.health();
    assert!(!health.connected);
    assert!(health.isolated);
    assert!(health.cleanup_confirmed);
    assert!(health.last_capture.is_none());
    let error = health.last_error.unwrap();
    assert!(error.contains("0xC101"), "{error}");
    assert!(error.contains("0x0B000001"), "{error}");
    assert!(error.contains("0x400D"), "{error}");
    assert!(error.contains("unverified"), "{error}");
    let event = health.last_event_diagnostic.unwrap();
    assert_eq!(event.events[0].code, OBJECT_RAM);
    assert_eq!(event.events[0].parameter, 0x0b00_0001);
    assert_eq!(event.response_code, Some(OK));
    // A fresh explicit selection still starts a clean session.
    let fresh = a.connect_selected("fake-camera-1").unwrap();
    assert!(fresh.connected && !fresh.isolated);
    assert!(fresh.last_error.is_none());
    assert_ne!(fresh.session_id, session);
}

#[test]
fn idle_apply_from_card_and_runtime_raw_one_preserves_diagnostics_and_allows_evidenced_capture() {
    for raw_value in [0, 1] {
        let mut s = Script::new();
        s.values.insert(RECORDING_MEDIA, CameraValue::Integer(0));
        s.values
            .insert(INFO_DISPLAY_ERROR_STATUS, CameraValue::Integer(raw_value));
        s.notify_runtime_after_set = true;
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        script.lock().unwrap().requests.clear();
        assert!(
            a.apply_properties(&BTreeMap::from([(
                "recordingMedia".into(),
                CameraValue::Integer(1)
            )]))
            .unwrap()
            .complete
        );
        std::thread::sleep(Duration::from_millis(400));
        let health = a.health();
        assert!(
            health.connected,
            "raw diagnostic {raw_value}: {:?}",
            health.last_error
        );
        assert!(!health.isolated);
        let media = &health.properties["recordingMedia"];
        assert!(media.confirmed);
        assert_eq!(media.actual, Some(CameraValue::Integer(1)));
        assert_eq!(media.target, Some(CameraValue::Integer(1)));
        assert!(media.error.is_none());
        let status = &health.runtime_properties["infoDisplayErrorStatus"];
        assert!(status.confirmed);
        assert_eq!(status.actual, Some(CameraValue::Integer(raw_value)));
        assert_eq!(status.semantic_status, CameraRuntimeSemanticStatus::Unknown);
        let script_guard = script.lock().unwrap();
        let event_poll = script_guard
            .requests
            .iter()
            .position(|(code, _)| *code == GET_EVENT)
            .unwrap();
        let after_poll = &script_guard.requests[event_poll + 1..];
        let media_read = after_poll
            .iter()
            .position(|(code, params)| *code == GET_PROPERTY && params == &[RECORDING_MEDIA as u32])
            .unwrap();
        let runtime_read = after_poll
            .iter()
            .position(|(code, params)| {
                *code == GET_PROPERTY && params == &[INFO_DISPLAY_ERROR_STATUS as u32]
            })
            .unwrap();
        assert!(media_read < runtime_read);
        drop(script_guard);
        let path = output("idle-opaque-runtime-capture");
        let image = a
            .capture_to_path(&path, "idle-opaque-runtime", &AtomicBool::new(false), None)
            .unwrap();
        assert!(script.lock().unwrap().calls.contains(&CAPTURE_RAM));
        assert!(path.exists());
        assert!(image.exposure_ended);
        assert_eq!(
            a.health().runtime_properties["infoDisplayErrorStatus"].actual,
            Some(CameraValue::Integer(raw_value))
        );
        assert_eq!(
            a.health().runtime_properties["infoDisplayErrorStatus"].semantic_status,
            CameraRuntimeSemanticStatus::Unknown
        );
        assert!(!a.health().capabilities.hardware_certified);
    }
}

#[test]
fn idle_runtime_status_keeps_card_media_and_nonzero_diagnostics_without_capture_preconditions() {
    let mut s = Script::new();
    s.values.insert(RECORDING_MEDIA, CameraValue::Integer(0));
    s.values
        .insert(INFO_DISPLAY_ERROR_STATUS, CameraValue::Integer(1));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    {
        let mut s = script.lock().unwrap();
        s.requests.clear();
        s.events.push_back(vec![NikonEvent {
            code: 0x4006,
            parameter: INFO_DISPLAY_ERROR_STATUS as u32,
        }]);
    }
    std::thread::sleep(Duration::from_millis(400));
    let health = a.health();
    assert!(health.connected);
    assert!(!health.isolated);
    assert_eq!(
        health.properties["recordingMedia"].actual,
        Some(CameraValue::Integer(0))
    );
    assert_eq!(
        health.runtime_properties["infoDisplayErrorStatus"].actual,
        Some(CameraValue::Integer(1))
    );
    let s = script.lock().unwrap();
    assert!(s.calls.contains(&GET_EVENT));
    assert!(!s
        .requests
        .iter()
        .any(|(code, params)| *code == GET_PROPERTY && params == &[RECORDING_MEDIA as u32]));
    assert!(!s.calls.contains(&SET_PROPERTY));
    drop(s);
    let path = output("idle-card-capture-refusal");
    assert!(a
        .capture_to_path(&path, "idle-card", &AtomicBool::new(false), None)
        .is_err());
    assert!(!script.lock().unwrap().calls.contains(&CAPTURE_RAM));
    assert!(!path.exists());
}

#[test]
fn owner_reads_back_post_apply_notifications_without_leaving_confirmed_iso_unknown() {
    let mut s = Script::new();
    s.notify_property_after_set = true;
    let script = Arc::new(Mutex::new(s));
    let a = connected(script);
    assert!(
        a.apply_properties(&BTreeMap::from([("iso".into(), CameraValue::Integer(400))]))
            .unwrap()
            .complete
    );
    std::thread::sleep(Duration::from_millis(400));
    let h = a.health();
    let iso = &h.properties["iso"];
    assert!(h.connected);
    assert!(iso.confirmed);
    assert_eq!(iso.actual, Some(CameraValue::Integer(400)));
    assert_eq!(iso.target, Some(CameraValue::Integer(400)));
    assert!(iso.error.is_none());
    assert!(a.profile_readback().is_ok());
}
#[test]
fn owner_reads_external_change_as_real_actual_but_keeps_target_mismatch_and_profile_locked() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    assert!(
        a.apply_properties(&BTreeMap::from([("iso".into(), CameraValue::Integer(200))]))
            .unwrap()
            .complete
    );
    {
        let mut s = script.lock().unwrap();
        s.values.insert(0x500f, CameraValue::Integer(400));
        s.events.push_back(vec![NikonEvent {
            code: 0x4006,
            parameter: 0x500f,
        }]);
    }
    std::thread::sleep(Duration::from_millis(400));
    let h = a.health();
    let iso = &h.properties["iso"];
    assert!(h.connected);
    assert!(iso.confirmed);
    assert_eq!(iso.actual, Some(CameraValue::Integer(400)));
    assert_eq!(iso.target, Some(CameraValue::Integer(200)));
    assert!(iso.error.as_ref().unwrap().contains("differs"));
    assert!(a.profile_readback().is_err());
}
#[test]
fn idle_transient_device_busy_uses_bounded_ready_retry_without_retiring_session() {
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    script.lock().unwrap().device_ready_responses = VecDeque::from([DEVICE_BUSY, DEVICE_BUSY, OK]);
    std::thread::sleep(Duration::from_millis(400));
    let health = a.health();
    assert!(health.connected);
    assert!(!health.isolated);
    assert!(health.last_error.is_none());
    assert!(
        script
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|code| **code == DEVICE_READY)
            .count()
            >= 3
    );
}
#[test]
fn device_busy_retry_cannot_exceed_the_shared_monotonic_deadline() {
    let script = Arc::new(Mutex::new(Script::new()));
    let mut native = NativeCamera::new(Box::new(Fake(script.clone())));
    let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let context = CallContext {
        deadline: Instant::now() + Duration::from_secs(1),
        epoch: 1,
        generation: generation.clone(),
    };
    native
        .connect(
            "fake-camera-1",
            "00112233445566778899aabbccddeeff",
            &context,
        )
        .unwrap();
    script.lock().unwrap().device_ready_responses = VecDeque::from(vec![DEVICE_BUSY; 100]);
    let context = CallContext {
        deadline: Instant::now() + Duration::from_millis(25),
        epoch: 1,
        generation,
    };
    assert!(matches!(
        native.device_ready(&context),
        Err(CameraError::Timeout(_))
    ));
}
#[test]
fn physical_manual_focus_is_readonly_and_captures_without_a_software_mode_change() {
    let mut s = Script::new();
    s.values.insert(0xd161, CameraValue::Integer(3));
    let script = Arc::new(Mutex::new(s));
    let a = connected(script.clone());
    let health = a.health();
    assert!(!health.properties["focusMode"].writable);
    assert!(!health.capabilities.autofocus);
    assert!(!health.capabilities.focus_step);
    assert_eq!(
        a.profile_readback().unwrap()["focusMode"],
        CameraValue::Integer(3)
    );
    assert!(a
        .validate_properties(&BTreeMap::from([(
            "focusMode".into(),
            CameraValue::Integer(4)
        )]))
        .is_err());
    let destination = output("physical-mf-capture");
    assert!(a
        .capture_to_path(&destination, "mf-hard", &AtomicBool::new(false), None)
        .is_ok());
    assert!(!script.lock().unwrap().calls.contains(&SET_PROPERTY));
    assert!(!script.lock().unwrap().calls.contains(&AF_DRIVE));
    assert!(destination.exists());
}
#[test]
fn automatic_focus_modes_still_refuse_capture_and_hard_mf_drive_is_not_guessed() {
    for mode in [0, 1, 2] {
        let mut s = Script::new();
        s.values.insert(0xd161, CameraValue::Integer(mode));
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        assert!(a.profile_readback().is_err());
        assert!(a
            .capture_to_path(
                &output("af-mode-refused"),
                "af",
                &AtomicBool::new(false),
                None
            )
            .is_err());
        assert!(!script.lock().unwrap().calls.contains(&CAPTURE_RAM));
    }
    let script = Arc::new(Mutex::new(Script::new()));
    let a = connected(script.clone());
    // A physical switch after the cached advertisement is re-read before drive.
    script
        .lock()
        .unwrap()
        .values
        .insert(0xd161, CameraValue::Integer(3));
    assert!(a.autofocus().is_err());
    assert!(!script.lock().unwrap().calls.contains(&AF_DRIVE));
    assert!(a.focus_step(10).is_err());
    assert!(!script.lock().unwrap().calls.contains(&FOCUS_DRIVE));
}
#[test]
fn capture_completion_in_rejected_batch_is_preserved_before_property_or_unknown_error() {
    for (code, parameter) in [(0x4006, 0x500f), (0x4006, 0xd1b5), (0xbeef, 0x1234)] {
        let mut s = Script::new();
        s.capture_events = vec![vec![
            NikonEvent { code, parameter },
            NikonEvent {
                code: CAPTURE_COMPLETE_RAM,
                parameter: 0,
            },
            NikonEvent {
                code: OBJECT_RAM,
                parameter: 0xffff0001,
            },
        ]];
        let script = Arc::new(Mutex::new(s));
        let a = connected(script.clone());
        let session = a.health().session_id.unwrap();
        let destination = output("rejected-batch-end");
        let (tx, rx) = mpsc::channel();
        let error = a
            .capture_to_path(
                &destination,
                "rejected-op",
                &AtomicBool::new(false),
                Some(tx),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("0x{parameter:08X}")));
        if code == 0x4006 {
            assert!(error.contains(if parameter == 0x500f {
                "frozenProperty=iso"
            } else {
                "frozenProperty=none"
            }));
        }
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, CameraCaptureEvent::ExposureEnded { .. }))
                .count(),
            1
        );
        assert!(!events
            .iter()
            .any(|e| matches!(e, CameraCaptureEvent::FileComplete { .. })));
        assert!(!destination.exists());
        assert!(!script.lock().unwrap().calls.contains(&GET_OBJECT));
        let health = a.health();
        assert!(health.cleanup_confirmed);
        let batch = health.last_capture_event_diagnostic.unwrap();
        assert_eq!(batch.session_id, session);
        assert_eq!(batch.operation_id, "rejected-op");
        assert_eq!(batch.event_count, 3);
        assert_eq!(batch.capture_complete_count, 1);
        assert!(batch.exposure_ended_observed);
        assert_eq!(batch.response.events[0].parameter, parameter);
        assert!(batch.response.error.is_some());
        assert_eq!(batch.response.dataset_sha256.len(), 64);
    }
}
#[test]
fn duplicate_capture_completion_never_emits_end_or_commits_a_file() {
    let mut s = Script::new();
    s.capture_events = vec![vec![
        NikonEvent {
            code: CAPTURE_COMPLETE_RAM,
            parameter: 0,
        },
        NikonEvent {
            code: CAPTURE_COMPLETE_RAM,
            parameter: 0,
        },
    ]];
    let script = Arc::new(Mutex::new(s));
    let mut native = NativeCamera::new(Box::new(Fake(script)));
    let context = CallContext {
        deadline: Instant::now() + Duration::from_secs(1),
        epoch: 1,
        generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
    };
    native
        .connect(
            "fake-camera-1",
            "00112233445566778899aabbccddeeff",
            &context,
        )
        .unwrap();
    let destination = output("duplicate-end-evidence");
    let (tx, rx) = mpsc::channel();
    assert!(super::capture::capture(
        &mut native,
        &destination,
        "duplicate-op",
        &Some(tx),
        &context,
        &Mutex::new(())
    )
    .is_err());
    assert!(native.capture_pending);
    assert!(!destination.exists());
    assert!(!rx
        .try_iter()
        .any(|event| matches!(event, CameraCaptureEvent::ExposureEnded { .. })));
    let batch = native.health.last_capture_event_diagnostic.unwrap();
    assert_eq!(batch.capture_complete_count, 2);
    assert!(!batch.exposure_ended_observed);
}
#[test]
fn capture_batch_diagnostic_survives_later_cleanup_event_transport_error() {
    let mut s = Script::new();
    s.capture_events = vec![vec![
        NikonEvent {
            code: 0x4006,
            parameter: 0xd1b1,
        },
        NikonEvent {
            code: CAPTURE_COMPLETE_RAM,
            parameter: 0,
        },
    ]];
    let script = Arc::new(Mutex::new(s));
    let mut native = NativeCamera::new(Box::new(Fake(script.clone())));
    let context = CallContext {
        deadline: Instant::now() + Duration::from_secs(1),
        epoch: 1,
        generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
    };
    native
        .connect(
            "fake-camera-1",
            "00112233445566778899aabbccddeeff",
            &context,
        )
        .unwrap();
    assert!(super::capture::capture(
        &mut native,
        &output("capture-diagnostic-retained"),
        "retained-op",
        &None,
        &context,
        &Mutex::new(())
    )
    .is_err());
    assert!(!native.capture_pending);
    let captured = native.health.last_capture_event_diagnostic.clone().unwrap();
    script.lock().unwrap().fail_code = Some(GET_EVENT);
    assert!(native.events(&context).is_err());
    assert!(native
        .health
        .last_event_diagnostic
        .as_ref()
        .unwrap()
        .error
        .is_some());
    let retained = native.health.last_capture_event_diagnostic.unwrap();
    assert_eq!(
        retained.response.data_prefix_hex,
        captured.response.data_prefix_hex
    );
    assert_eq!(retained.operation_id, "retained-op");
    assert!(retained.exposure_ended_observed);
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
    // Expectation narrowed 2026-10-11: the object event, not the SDRAM completion,
    // is the transfer/commit evidence (digiCamControl Nikon/NikonBase.cs:2330-2367
    // transfers on the object and :2369-2375 only reports the completion), so the
    // stream that must never be accepted by elapsed time is one with no capture
    // evidence at all.
    let generation = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let context = CallContext {
        deadline: Instant::now() + Duration::from_millis(40),
        epoch: 1,
        generation,
    };
    let mut s = Script::new();
    s.capture_events = vec![];
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
    assert!(camera.capture_pending);
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
fn live_controls_use_fresh_uint8_descriptors_without_changing_photographic_properties() {
    let script = Arc::new(Mutex::new(Script::new()));
    let camera = connected(script.clone());
    let connection_session = camera.health().session_id;
    let started = camera.start_live_view().unwrap();
    let session = started.live_view.session_id.clone().unwrap();
    assert_eq!(started.live_view_controls.session_id.as_deref(), Some(session.as_str()));
    assert_eq!(started.live_view_controls.properties.len(), 2);
    assert!(!started.properties.contains_key("cameraZoom"));
    assert_eq!(started.properties["focusMode"].code, 0xd161);
    assert_eq!(started.live_view_controls.properties["focusMode"].code, LV_FOCUS_MODE);
    script.lock().unwrap().requests.clear();
    let result = camera.configure_live_view(&session, &BTreeMap::from([
        ("focusMode".into(), CameraValue::Integer(4)),
        ("cameraZoom".into(), CameraValue::Integer(5)),
    ])).unwrap();
    assert!(result.complete);
    assert_eq!(result.applied, ["focusMode", "cameraZoom"]);
    let s = script.lock().unwrap();
    assert_eq!(s.writes, [(LV_FOCUS_MODE, vec![4]), (LV_ZOOM, vec![5])]);
    for code in [LV_FOCUS_MODE, LV_ZOOM] {
        assert!(s.requests.iter().filter(|(op, args)| *op == GET_PROPERTY_DESC && args == &[code as u32]).count() >= 3);
        assert!(s.requests.iter().filter(|(op, args)| *op == GET_PROPERTY && args == &[code as u32]).count() >= 3);
    }
    assert!(s.calls.contains(&GET_DEVICE_INFO));
    drop(s);
    let health = camera.health();
    assert_eq!(health.properties["focusMode"].actual, Some(CameraValue::Integer(4)));
    assert_eq!(health.live_view_controls.properties["cameraZoom"].actual, Some(CameraValue::Integer(5)));
    assert_eq!(health.session_id, connection_session);
    assert!(camera.apply_properties(&BTreeMap::from([("iso".into(), CameraValue::Integer(800))])).is_err());
    assert!(camera.profile_readback().is_err());
    camera.stop_live_view().unwrap();
    assert!(!camera.profile_readback().unwrap().contains_key("cameraZoom"));
}
#[test]
fn normal_focus_hard_mf_target_is_disabled_while_confirmed_hard_mf_noop_is_valid() {
    for mode in [0, 4] {
        let mut s = Script::new();
        s.values.insert(0xd161, CameraValue::Integer(mode));
        let script = Arc::new(Mutex::new(s));
        let camera = connected(script.clone());
        let property = &camera.health().properties["focusMode"];
        let option = property.options.iter().find(|option| option.value == CameraValue::Integer(3)).unwrap();
        assert!(option.supported, "native enum evidence must remain visible");
        assert!(!option.enabled, "physical MF is a read-only state, not a writable target");
        let values = BTreeMap::from([("focusMode".into(), CameraValue::Integer(3))]);
        assert!(camera.validate_properties(&values).is_err());
        assert!(camera.apply_properties(&values).is_err());
        assert!(script.lock().unwrap().writes.is_empty());
    }
    let mut s = Script::new();
    s.values.insert(0xd161, CameraValue::Integer(3));
    let script = Arc::new(Mutex::new(s));
    let camera = connected(script.clone());
    let values = BTreeMap::from([("focusMode".into(), CameraValue::Integer(3))]);
    camera.validate_properties(&values).unwrap();
    assert!(camera.apply_properties(&values).unwrap().complete);
    assert!(!camera.health().properties["focusMode"].writable);
    assert!(script.lock().unwrap().writes.is_empty());
}
fn device_info_without_operation(code:u16)->Vec<u8>{
    let mut data=device_info(true);let info=parse_device_info(&data).unwrap();let original=array(&info.operations);let filtered:Vec<u16>=info.operations.into_iter().filter(|value|*value!=code).collect();let offset=data.windows(original.len()).position(|window|window==original).expect("scripted operation array");data.splice(offset..offset+original.len(),array(&filtered));data
}
#[cfg(windows)]
#[test]
fn live_controls_reject_invalid_keys_values_and_stale_sessions_without_set() {
    let script = Arc::new(Mutex::new(Script::new()));
    let camera = connected(script.clone());
    let first = camera.start_live_view().unwrap().live_view.session_id.unwrap();
    for values in [BTreeMap::new(), BTreeMap::from([("iso".into(), CameraValue::Integer(200))]),
        BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(256))]),
        BTreeMap::from([("cameraZoom".into(), CameraValue::Text("5".into()))])] {
        assert!(camera.configure_live_view(&first, &values).is_err());
    }
    assert!(camera.configure_live_view("bad", &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(1))])).is_err());
    camera.stop_live_view().unwrap();
    let stopped = camera.health();
    assert!(stopped.live_view.session_id.is_none());
    assert!(stopped.live_view_controls.session_id.is_none());
    assert!(stopped.live_view_controls.properties.values().all(|p| !p.writable && !p.effective && p.options.iter().all(|o| !o.enabled)));
    let second = camera.start_live_view().unwrap().live_view.session_id.unwrap();
    assert_ne!(first, second);
    assert!(second.len() == 32 && second.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(camera.pipe_descriptor().unwrap().session_id, second);
    assert!(camera.read_live_view_controls(&first).is_err());
    assert!(camera.configure_live_view(&first, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(1))])).is_err());
    assert!(script.lock().unwrap().writes.is_empty());
    camera.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn live_controls_require_advertisement_scalar_type_and_current_native_write_permission() {
    for condition in ["unadvertised", "wrongType", "readonly", "newDescriptor"] {
        let script = Arc::new(Mutex::new(Script::new()));
        let camera = connected(script.clone());
        let session = camera.start_live_view().unwrap().live_view.session_id.unwrap();
        {
            let mut s = script.lock().unwrap();
            s.requests.clear();
            match condition {
                "unadvertised" => s.device_info = Some(device_info_without_property(LV_ZOOM)),
                "wrongType" => {
                    s.property_types.insert(LV_ZOOM, 0xffff);
                    s.values.insert(LV_ZOOM, CameraValue::Text("0".into()));
                    s.descriptor_values.insert(LV_ZOOM, vec![CameraValue::Text("0".into())]);
                }
                "readonly" => { s.readonly_properties.insert(LV_ZOOM); }
                _ => { s.descriptor_values.insert(LV_ZOOM, vec![CameraValue::Integer(0), CameraValue::Integer(1)]); }
            }
        }
        let result = camera.configure_live_view(&session, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(5))])).unwrap();
        assert!(!result.complete, "{condition}");
        assert!(script.lock().unwrap().writes.is_empty(), "{condition}");
        let property = &camera.health().live_view_controls.properties["cameraZoom"];
        if matches!(condition, "unadvertised" | "wrongType") {
            assert!(!property.supported && !property.confirmed && property.actual.is_none(), "{condition}");
            assert!(!script.lock().unwrap().requests.iter().any(|(op, args)| *op == GET_PROPERTY && args == &[LV_ZOOM as u32]), "{condition}");
        }
        if condition == "unadvertised" {
            assert!(!script.lock().unwrap().requests.iter().any(|(op, args)| *op == GET_PROPERTY_DESC && args == &[LV_ZOOM as u32]));
        }
        camera.stop_live_view().unwrap();
    }
}
#[cfg(windows)]
#[test]
fn live_zoom_uses_the_current_descriptor_scalar_width_for_read_and_write() {
    for kind in 1..=8 {
        let mut s = Script::new();
        s.property_types.insert(LV_ZOOM, kind);
        let script = Arc::new(Mutex::new(s));
        let camera = connected(script.clone());
        let session = camera.start_live_view().unwrap().live_view.session_id.unwrap();
        let result = camera.configure_live_view(&session, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(5))])).unwrap();
        assert!(result.complete, "native type {kind}: {:?}", result.failed);
        assert_eq!(result.properties["cameraZoom"].data_type, kind);
        assert_eq!(result.properties["cameraZoom"].actual, Some(CameraValue::Integer(5)));
        assert_eq!(script.lock().unwrap().writes, [(LV_ZOOM, encode_value(&CameraValue::Integer(5), kind).unwrap())]);
        camera.stop_live_view().unwrap();
    }
}
#[cfg(windows)]
#[test]
fn live_zoom_redescribes_width_after_live_view_start_without_relaxing_focus_type() {
    let script = Arc::new(Mutex::new(Script::new()));
    let camera = connected(script.clone());
    let session = camera.start_live_view().unwrap().live_view.session_id.unwrap();
    script.lock().unwrap().property_types.insert(LV_ZOOM, 6);
    let result = camera.configure_live_view(&session, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(5))])).unwrap();
    assert!(result.complete);
    assert_eq!(script.lock().unwrap().writes, [(LV_ZOOM, vec![5,0,0,0])]);
    script.lock().unwrap().property_types.insert(LV_FOCUS_MODE, 4);
    script.lock().unwrap().writes.clear();
    let result = camera.configure_live_view(&session, &BTreeMap::from([("focusMode".into(), CameraValue::Integer(4))])).unwrap();
    assert!(!result.complete);
    assert!(script.lock().unwrap().writes.is_empty());
    camera.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn live_zoom_does_not_confirm_equal_value_after_descriptor_type_changes_during_set() {
    let script = Arc::new(Mutex::new(Script::new()));
    let camera = connected(script.clone());
    let session = camera.start_live_view().unwrap().live_view.session_id.unwrap();
    script.lock().unwrap().descriptor_type_change_on_set.insert(LV_ZOOM, 6);
    let result = camera.configure_live_view(&session, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(5))])).unwrap();
    assert!(!result.complete);
    let property = &result.properties["cameraZoom"];
    assert_eq!(property.actual, property.target);
    assert_eq!(property.data_type, 6);
    assert!(property.error.as_ref().unwrap().contains("descriptor type changed"));
    assert!(!property.writable);
    camera.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn unadvertised_active_slot_remains_unknown_without_probe() {
    let mut s = Script::new();
    s.device_info = Some(device_info_without_property(ACTIVE_SLOT));
    let script = Arc::new(Mutex::new(s));
    let camera = connected(script.clone());
    let session = camera.start_live_view().unwrap().live_view.session_id.unwrap();
    camera.read_live_view_controls(&session).unwrap();
    let health = camera.health();
    let slot = &health.runtime_properties["activeSlot"];
    assert!(!slot.readable && !slot.confirmed && slot.actual.is_none());
    assert!(!script.lock().unwrap().requests.iter().any(|(op, args)| matches!(*op, GET_PROPERTY_DESC | GET_PROPERTY | SET_PROPERTY) && args == &[ACTIVE_SLOT as u32]));
    camera.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn live_motor_qualification_uses_fresh_d061_lens_and_photo_selector() {
    for (mode, lens, selector, autofocus, step) in [
        (0, 1, 0, true, true), (4, 1, 0, false, true),
        (2, 1, 0, false, false), (3, 1, 0, false, false),
        (1, 1, 0, false, false), (0, 0, 0, false, false),
        (0, 1, 1, false, false),
    ] {
        let script = Arc::new(Mutex::new(Script::new()));
        let camera = connected(script.clone());
        camera.start_live_view().unwrap();
        {
            let mut s = script.lock().unwrap();
            s.values.insert(LV_FOCUS_MODE, CameraValue::Integer(mode));
            s.values.insert(LENS_SORT, CameraValue::Integer(lens));
            s.values.insert(LV_SELECTOR, CameraValue::Integer(selector));
        }
        assert_eq!(camera.autofocus().is_ok(), autofocus, "mode={mode}, lens={lens}, selector={selector}");
        assert_eq!(camera.focus_step(10).is_ok(), step, "mode={mode}, lens={lens}, selector={selector}");
        let health = camera.health();
        assert_eq!(health.properties["focusMode"].actual, Some(CameraValue::Integer(4)), "normal D161 must remain separate");
        let s = script.lock().unwrap();
        assert_eq!(s.calls.contains(&AF_DRIVE), autofocus);
        assert_eq!(s.calls.contains(&FOCUS_DRIVE), step);
        drop(s);
        if mode == 3 || mode == 1 {
            let session = health.live_view.session_id.unwrap();
            assert!(!camera.configure_live_view(&session, &BTreeMap::from([("focusMode".into(), CameraValue::Integer(0))])).unwrap().complete);
            assert!(script.lock().unwrap().writes.is_empty());
        }
        camera.stop_live_view().unwrap();
    }
}
#[cfg(windows)]
#[test]
fn live_focus_readback_unknown_never_drives_lens_or_guesses_from_normal_mode() {
    let script = Arc::new(Mutex::new(Script::new()));
    let camera = connected(script.clone());
    camera.start_live_view().unwrap();
    script.lock().unwrap().unreadable_property = Some(LV_FOCUS_MODE);
    assert!(camera.autofocus().is_err());
    assert!(camera.focus_step(10).is_err());
    assert!(!camera.health().capabilities.autofocus && !camera.health().capabilities.focus_step);
    let s = script.lock().unwrap();
    assert!(!s.calls.contains(&AF_DRIVE) && !s.calls.contains(&FOCUS_DRIVE));
    drop(s);
    camera.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn live_motor_missing_or_failed_current_evidence_never_uses_cached_qualification() {
    for condition in ["lensMissing", "selectorMissing", "lensUnreadable", "selectorUnreadable", "afMissing", "stepMissing", "readyMissing"] {
        let script = Arc::new(Mutex::new(Script::new()));
        let camera = connected(script.clone());
        camera.start_live_view().unwrap();
        {
            let mut s = script.lock().unwrap();
            match condition {
                "lensMissing" => s.device_info = Some(device_info_without_property(LENS_SORT)),
                "selectorMissing" => s.device_info = Some(device_info_without_property(LV_SELECTOR)),
                "lensUnreadable" => s.unreadable_property = Some(LENS_SORT),
                "selectorUnreadable" => s.unreadable_property = Some(LV_SELECTOR),
                "afMissing" => s.device_info = Some(device_info_without_operation(AF_DRIVE)),
                "stepMissing" => s.device_info = Some(device_info_without_operation(FOCUS_DRIVE)),
                _ => s.device_info = Some(device_info_without_operation(DEVICE_READY)),
            }
            s.requests.clear();
        }
        let af_expected = condition == "stepMissing";
        let step_expected = condition == "afMissing";
        assert_eq!(camera.autofocus().is_ok(), af_expected, "{condition}");
        assert_eq!(camera.focus_step(10).is_ok(), step_expected, "{condition}");
        let s = script.lock().unwrap();
        assert_eq!(s.requests.iter().any(|(op, _)| *op == AF_DRIVE), af_expected, "{condition}");
        assert_eq!(s.requests.iter().any(|(op, _)| *op == FOCUS_DRIVE), step_expected, "{condition}");
        drop(s);
        camera.stop_live_view().unwrap();
    }
}
#[cfg(windows)]
#[test]
fn live_partial_apply_retains_actual_target_and_recovers_with_explicit_new_target_or_session() {
    let script = Arc::new(Mutex::new(Script::new()));
    let camera = connected(script.clone());
    let first = camera.start_live_view().unwrap().live_view.session_id.unwrap();
    script.lock().unwrap().mismatch = Some(LV_ZOOM);
    let result = camera.configure_live_view(&first, &BTreeMap::from([
        ("focusMode".into(), CameraValue::Integer(4)),
        ("cameraZoom".into(), CameraValue::Integer(3)),
    ])).unwrap();
    assert!(!result.complete);
    assert_eq!(result.applied, ["focusMode"]);
    assert!(result.failed.contains_key("cameraZoom"));
    let read = camera.read_live_view_controls(&first).unwrap();
    let zoom = &read.live_view_controls.properties["cameraZoom"];
    assert_eq!(zoom.actual, Some(CameraValue::Integer(0)));
    assert_eq!(zoom.target, Some(CameraValue::Integer(3)));
    assert!(zoom.confirmed && zoom.error.is_some() && !zoom.writable);
    assert!(zoom.disabled_reason.as_ref().unwrap().contains("停止后重新启动"));
    script.lock().unwrap().mismatch = None;
    let renewed = camera.configure_live_view(&first, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(2))])).unwrap();
    assert!(renewed.complete);
    assert_eq!(renewed.properties["cameraZoom"].actual, Some(CameraValue::Integer(2)));
    assert_eq!(renewed.properties["cameraZoom"].target, Some(CameraValue::Integer(2)));
    camera.stop_live_view().unwrap();
    let next = camera.start_live_view().unwrap();
    assert_ne!(next.live_view.session_id.as_deref(), Some(first.as_str()));
    assert!(next.live_view_controls.properties.values().all(|p| p.target.is_none() && p.error.is_none()));
    assert!(camera.configure_live_view(&first, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(3))])).is_err());
    camera.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn stop_or_cancel_during_live_descriptor_read_prevents_late_set_and_writable_publication() {
    for cancel in [false, true] {
        let script = Arc::new(Mutex::new(Script::new()));
        let camera = connected(script.clone());
        let session = camera.start_live_view().unwrap().live_view.session_id.unwrap();
        let (started, receive) = mpsc::channel();
        script.lock().unwrap().stall = Some((GET_PROPERTY_DESC, Duration::from_millis(350), started));
        let worker = camera.clone();
        let result = std::thread::spawn(move || worker.configure_live_view(&session, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(5))])));
        receive.recv_timeout(Duration::from_secs(2)).unwrap();
        if cancel { camera.request_cancel(); } else { camera.stop_live_view().unwrap(); }
        assert!(result.join().unwrap().is_err());
        assert!(script.lock().unwrap().writes.is_empty());
        if cancel { camera.disconnect_confirmed().unwrap(); }
        let health = camera.health();
        assert!(health.live_view_controls.session_id.is_none());
        assert!(health.live_view_controls.properties.values().all(|p| !p.writable && p.options.iter().all(|o| !o.enabled)));
        assert!(camera.pipe_descriptor().is_none());
    }
}
#[cfg(windows)]
#[test]
fn live_control_fresh_state_retires_externally_stopped_session_without_write() {
    let script = Arc::new(Mutex::new(Script::new()));
    let camera = connected(script.clone());
    let first = camera.start_live_view().unwrap().live_view.session_id.unwrap();
    script.lock().unwrap().lv = false;
    assert!(camera.configure_live_view(&first, &BTreeMap::from([("cameraZoom".into(), CameraValue::Integer(5))])).is_err());
    assert!(script.lock().unwrap().writes.is_empty());
    let health = camera.health();
    assert!(health.connected && !health.live_view.active);
    assert!(health.live_view.session_id.is_none() && health.live_view_controls.session_id.is_none());
    assert!(camera.pipe_descriptor().is_none());
    let next = camera.start_live_view().unwrap().live_view.session_id.unwrap();
    assert_ne!(first, next);
    camera.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn stop_during_live_frame_read_does_not_publish_late_frame() {
    let script = Arc::new(Mutex::new(Script::new()));
    let camera = connected(script.clone());
    camera.start_live_view().unwrap();
    let (started, receive) = mpsc::channel();
    script.lock().unwrap().stall = Some((GET_LV, Duration::from_millis(350), started));
    receive.recv_timeout(Duration::from_secs(2)).unwrap();
    camera.stop_live_view().unwrap();
    assert!(!camera.health().live_view.fresh);
    assert!(camera.pipe_descriptor().is_none());
    assert!(camera.health().live_view_controls.session_id.is_none());
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
    use std::io::{Read, Write};
    let a = connected(Arc::new(Mutex::new(Script::new())));
    a.start_live_view().unwrap();
    let descriptor = a.pipe_descriptor().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while a.health().live_view.sequence == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut received = None;
    while Instant::now() < deadline {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&descriptor.endpoint)
        {
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
                    let frame = decode_live_view_frame(&bytes, &descriptor.session_id, 0).unwrap();
                    file.write_all(
                        &encode_live_view_ack(&frame.session_id, frame.sequence).unwrap(),
                    )
                    .unwrap();
                    received = Some(frame);
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
fn open_test_live_view_client(adapter: &CameraAdapter) -> (std::fs::File, LiveViewPipeDescriptor) {
    let descriptor = adapter.pipe_descriptor().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while adapter.health().live_view.sequence == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    loop {
        if let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&descriptor.endpoint)
        {
            return (file, descriptor);
        }
        assert!(
            Instant::now() < deadline,
            "local named pipe client could not connect"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[cfg(windows)]
fn read_test_live_view_payload(
    file: &mut std::fs::File,
    descriptor: &LiveViewPipeDescriptor,
) -> LiveViewFrame {
    use std::io::Read;
    let mut bytes = vec![0; LIVE_VIEW_HEADER_BYTES];
    file.read_exact(&mut bytes).unwrap();
    let len = u32::from_le_bytes(bytes[48..52].try_into().unwrap()) as usize;
    assert!(len <= MAX_LIVE_VIEW_BYTES);
    bytes.resize(LIVE_VIEW_HEADER_BYTES + len, 0);
    file.read_exact(&mut bytes[LIVE_VIEW_HEADER_BYTES..])
        .unwrap();
    decode_live_view_frame(&bytes, &descriptor.session_id, 0).unwrap()
}
#[cfg(windows)]
#[test]
fn slow_named_pipe_consumer_receives_complete_frame_before_exact_ack() {
    use std::io::Write;
    let adapter = connected(Arc::new(Mutex::new(Script::new())));
    adapter.start_live_view().unwrap();
    let (mut file, descriptor) = open_test_live_view_client(&adapter);
    assert_eq!(descriptor.version, 2);
    std::thread::sleep(Duration::from_millis(100));
    let frame = read_test_live_view_payload(&mut file, &descriptor);
    assert_eq!((frame.width, frame.height), (8, 6));
    // Fragmented ACKs are accepted only after the whole fixed envelope arrives.
    let ack = encode_live_view_ack(&frame.session_id, frame.sequence).unwrap();
    file.write_all(&ack[..8]).unwrap();
    file.write_all(&ack[8..]).unwrap();
    drop(file);
    adapter.stop_live_view().unwrap();
}
#[cfg(windows)]
#[test]
fn missing_ack_times_out_and_never_blocks_owner_status_or_stop_revoke() {
    use std::io::Read;
    let adapter = connected(Arc::new(Mutex::new(Script::new())));
    adapter.start_live_view().unwrap();
    let (mut file, descriptor) = open_test_live_view_client(&adapter);
    let frame = read_test_live_view_payload(&mut file, &descriptor);
    std::thread::sleep(Duration::from_millis(320));
    assert!(adapter.health().live_view.sequence > frame.sequence);
    let mut byte = [0];
    assert!(
        !matches!(file.read(&mut byte),Ok(count)if count>0),
        "unacknowledged connection must close at the receipt deadline"
    );
    drop(file);
    let (mut pending, descriptor) = open_test_live_view_client(&adapter);
    let _ = read_test_live_view_payload(&mut pending, &descriptor);
    let started = Instant::now();
    adapter.request_cancel();
    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(adapter.pipe_descriptor().is_none());
    assert!(!adapter.health().live_view.fresh);
    drop(pending);
}
#[test]
fn ack_encoder_binds_protocol_session_and_positive_sequence() {
    let session = "00112233445566778899aabbccddeeff";
    let ack = encode_live_view_ack(session, 7).unwrap();
    assert_eq!(&ack[..8], b"MCTACK02");
    assert_eq!(
        &ack[8..24],
        &[
            0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff
        ]
    );
    assert_eq!(&ack[24..], &7u64.to_le_bytes());
    assert!(encode_live_view_ack(session, 0).is_err());
    assert!(encode_live_view_ack("bad", 7).is_err());
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

#[test]
fn independent_automatic_exposure_iso_white_balance_and_af_mode_keep_host_single_transaction(){
    for mode in [2,3,4] {
        let mut script=Script::new();script.values.insert(0x500e,CameraValue::Integer(mode));script.values.insert(NIKON_PROPERTIES.iter().find(|p|p.0=="autoIso").unwrap().2,CameraValue::Integer(1));script.values.insert(0x5005,CameraValue::Integer(2));script.values.insert(0xd161,CameraValue::Integer(0));
        let shared=Arc::new(Mutex::new(script));let camera=connected(shared.clone());assert!(camera.profile_readback().is_err());assert!(super::nikon::independent_readback(&camera.health()).is_ok());let destination=output("independent-auto-policy");let image=camera.capture_independent_to_path(&destination,&format!("auto-mode-{mode}"),&AtomicBool::new(false)).unwrap();assert!(image.exposure_ended);assert_eq!(image.size,std::fs::metadata(destination).unwrap().len());let script=shared.lock().unwrap();assert!(!script.calls.contains(&SET_PROPERTY));assert!(!script.calls.contains(&AF_DRIVE));assert!(script.calls.contains(&CAPTURE_RAM));
    }
}
#[test]
fn scan_fixed_issues_keep_native_off_semantics_and_fresh_read_does_not_write_core_properties(){
    let shared=Arc::new(Mutex::new(Script::new()));let camera=connected(shared.clone());assert!(crate::capture_profile::scan_camera_issues(&camera.health()).is_empty());
    for (key,code,unsafe_value) in [("whiteBalance",0x5005,2),("activeDLighting",0xd14e,0),("hdrMode",0xd0b0,1),("longExposureNoiseReduction",0xd06b,1),("exposureDelay",0xd16a,0)] {
        // Use the device's described property code, rather than guessing its numeric address.
        let code=camera.health().properties.get(key).map(|p|p.code).unwrap_or(code);
        let original=shared.lock().unwrap().values[&code].clone();shared.lock().unwrap().values.insert(code,CameraValue::Integer(unsafe_value));camera.read_properties().unwrap();let issues=crate::capture_profile::scan_camera_issues(&camera.health());assert!(issues.iter().any(|issue|issue.key==key),"{key}: {issues:?}");assert!(camera.profile_readback().is_err());shared.lock().unwrap().values.insert(code,original);camera.read_properties().unwrap();
    }
    assert!(!shared.lock().unwrap().calls.contains(&SET_PROPERTY));
}

#[test]
fn independent_automatic_output_notifications_are_admitted_but_host_media_drift_is_not(){
    for (key,value,allowed) in [("iso",400,true),("shutter",0x1000a,true),("aperture",560,true),("recordingMedia",0,false),("burstNumber",2,false)] {
        let mut script=Script::new();script.values.insert(0x500e,CameraValue::Integer(2));let auto_iso=NIKON_PROPERTIES.iter().find(|p|p.0=="autoIso").unwrap().2;script.values.insert(auto_iso,CameraValue::Integer(1));let code=NIKON_PROPERTIES.iter().find(|p|p.0==key).unwrap().2;
        script.capture_value_changes.insert(code,CameraValue::Integer(value));script.capture_events.insert(0,vec![NikonEvent{code:0x4006,parameter:code as u32}]);let shared=Arc::new(Mutex::new(script));let camera=connected(shared.clone());let path=output("independent-output-notification");let result=camera.capture_independent_to_path(&path,key,&AtomicBool::new(false));assert_eq!(result.is_ok(),allowed,"{key}: {result:?}");assert_eq!(path.exists(),allowed);assert!(!shared.lock().unwrap().calls.contains(&SET_PROPERTY));
    }
}

#[test]
fn bracket_advertisement_descriptor_and_off_readback_are_required_without_blind_private_reads(){
    for case in ["on","unadvertised","wrongType","readFailure"] {
        let mut script=Script::new();match case {
            "on"=>{script.values.insert(BRACKETING_ENABLE,CameraValue::Integer(1));},
            "unadvertised"=>{script.device_info=Some(device_info_without_property(BRACKETING_ENABLE));},
            "wrongType"=>{script.property_types.insert(BRACKETING_ENABLE,4);},
            _=>{script.property_responses.insert((GET_PROPERTY_DESC,BRACKETING_ENABLE),VecDeque::from([0x200a]));},
        }
        let shared=Arc::new(Mutex::new(script));let camera=connected(shared.clone());if case=="wrongType"{assert!(!camera.health().properties["bracketing"].writable);assert!(camera.apply_properties(&BTreeMap::from([("bracketing".into(),CameraValue::Integer(0))])).is_err());}
        assert!(crate::capture_profile::scan_camera_issues(&camera.health()).iter().any(|issue|issue.key=="bracketing"),"{case}");assert!(camera.profile_readback().is_err());let script=shared.lock().unwrap();assert!(!script.calls.contains(&SET_PROPERTY));if case=="unadvertised"{assert!(!script.requests.iter().any(|(opcode,params)|matches!(*opcode,GET_PROPERTY_DESC|GET_PROPERTY|SET_PROPERTY)&&params.first()==Some(&(BRACKETING_ENABLE as u32))));}
    }
}
#[test]
fn physical_adl_is_required_but_hdr_can_use_documented_nef_exclusion_without_hiding_errors(){
    let shared=Arc::new(Mutex::new(Script::new()));let camera=connected(shared);let health=camera.health();
    for case in ["adlUnadvertised","adlUnknown","hdrUnadvertised","hdrOn","hdrError"]{
        let mut current=health.clone();if case.starts_with("adl"){let p=current.properties.get_mut("activeDLighting").unwrap();p.actual=None;p.confirmed=false;if case=="adlUnadvertised"{p.supported=false;p.readable=false;p.error=None;}else{p.error=Some("fresh read failed".into());}}
        else{let p=current.properties.get_mut("hdrMode").unwrap();match case{"hdrUnadvertised"=>{p.supported=false;p.readable=false;p.actual=None;p.confirmed=false;p.error=None;let info=current.device_info.as_mut().unwrap();info.properties.retain(|code|*code!=0xd130);info.effective_properties.retain(|code|*code!=0xd130);},"hdrOn"=>{p.actual=Some(CameraValue::Integer(1));},_=>{p.supported=false;p.readable=false;p.actual=None;p.confirmed=false;p.error=Some("advertised HDR descriptor failed".into());}}}
        let issues=crate::capture_profile::scan_camera_issues(&current);assert_eq!(issues.is_empty(),case=="hdrUnadvertised","{case}: {issues:?}");if case=="hdrUnadvertised"{let values=profile_readback(&current).unwrap();let profile=crate::capture_profile::CaptureProfile::from_camera_readback(&current,values,None).unwrap();assert_eq!(profile.schema_version,6);assert_eq!(profile.fixed_acquisition_evidence.unwrap().hdr_proof,"nef_exclusion");}
    }
}
#[test]
fn independent_automatic_exposure_keeps_the_existing_host_transaction() {
    let mut script=Script::new();
    script.values.insert(0x500e,CameraValue::Integer(2));
    script.values.insert(0xd054,CameraValue::Integer(1));
    let shared=Arc::new(Mutex::new(script));
    let camera=connected(shared.clone());
    assert!(camera.profile_readback().is_err(),"automatic exposure remains unavailable for fixed scans");
    let path=output("independent-automatic-host-output");
    let image=camera.capture_independent_to_path(&path,"single-host-output",&AtomicBool::new(false)).unwrap();
    assert!(image.exposure_ended);
    assert_eq!(image.size,std::fs::metadata(path).unwrap().len());
    assert!(!shared.lock().unwrap().calls.contains(&SET_PROPERTY));
}
