# Native camera adapter (offline candidate)

This module owns Windows WPD/PTP communication, identity and capability discovery,
property write/readback, Nikon D7100 host RAM NEF capture, Live View, and host file
confirmation. `ct-engine` owns all device occupancy and cross-device safety rules.
No external camera program, extra bridge process, USB driver replacement, camera
card fallback, or production simulation is used.

D7100 photographic controls also follow the official manual's P/S/A/M, RAW/JPEG,
white-balance, HDR/bracketing and flash dependencies. Explicit HDR/bracketing OFF
requests precede incompatible mode changes, with fresh descriptors before each
dependent write. Native descriptors remain mandatory; no setting is changed
implicitly. See the [manual dependency matrix](../../../../../docs/architecture/native-camera-capability-matrix.md#手册功能关系与机身检查).
Multiple exposure has no current host control or machine admission field. The
operator checks the camera body before preflight; USB connection, a disabled menu
and a single host NEF do not prove that this setting is OFF.

## Sources and acceptance status

- The WPD transport is independently implemented against Microsoft's
  [MTP extension interface](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/supporting-mtp-extensions)
  and [IPortableDevice interface](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Devices/PortableDevices/struct.IPortableDevice.html).
- Necessary Nikon operation/property codes, packed rational shutter semantics,
  SDRAM event layout, and D7100's 384-byte Live View header are adapted from the
  MIT-licensed `CameraControl.Devices/Nikon/NikonBase.cs`, `NikonD600Base.cs`,
  `NikonD7100.cs`, and `BaseMTPCamera.cs`. Attribution and permission are in
  [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
- No `PortableDeviceLib` implementation, complete external camera framework, or
  closed vendor DLL has been copied or linked.
- **仅离线验证。** Real D7100 WPD behavior, event ordering and exposure semantics,
  host RAM transfer, lens behavior, temperature/device-loss behavior, and NEF
  acquisition remain **BLOCKED** until separately authorized hardware acceptance.
  `hardwareCertified` always remains false in this candidate.

## Files and boundary

| File | Responsibility |
| --- | --- |
| `types.rs` | Stable serializable identity, capabilities, properties, health and capture evidence |
| `actor.rs` | One COM owner, bounded messages, immediate cached state, cancellation generations and independent deadline watchdog |
| `wpd.rs` | In-process WPD COM creation, explicit discovery/open and bounded MTP data phases |
| `ptp.rs` | Strict PTP identity/property/object/event datasets and monotonic shared deadlines |
| `nikon.rs` | D7100 capability negotiation, exact native properties and frozen profile readback |
| `properties.rs` | Descriptor-backed photographic option labels, contextual effectiveness, dependency ordering and capture readiness |
| `capture.rs` | Correlated SDRAM events, NEF structural completeness, SHA-256 and atomic no-overwrite commit |
| `storage.rs` | Strict camera storage datasets: GetStorageIDs/GetStorageInfo/GetObjectHandles parsing, positional id binding, 64-bit capacity and offline writable-card planning |
| `card_capture.rs` | Card-capture transaction and pure association state: one standard InitiateCapture, exactly one ObjectAdded/completion pair, StoreFull/StoreRemoved handling, metadata-only reads (no image transfer, no local file, no hash) |
| `card_import.rs` | Authorized USB import of a retained `acquired_pending_import` task: fresh storage/handle/ObjectInfo enumeration through the single camera owner, record-to-object binding by storage id + source name + size + capture date, bounded GetObject with streaming SHA-256 and complete-NEF validation, task-private staging with an atomic no-overwrite commit into `frames/` and `references/pre-dark\|pre-flat/`, resume/cancel, path-escape refusal and the atomic schema-4 seal |
| `live_view.rs` | Strict JPEG validation, binary envelope, latest frame slot and current-user-only named pipe |
| `tests.rs` | Scripted transport through the same production actor and capture implementation |

The verified D7100 NEF-only RAM ObjectInfo path accepts the measured `0x3800`
image format and date fields with contiguous trailing NUL padding. Filename,
keywords, identity and property strings remain strict; nonzero date padding is
rejected. Complete NEF validation and associated C102 completion remain required.

`CameraAdapter` is a cloneable Send + Sync message proxy; it contains no COM
interface. The transport factory runs inside the dedicated camera thread.
Windows COM objects are created, used and released there. No unsafe Send
implementation is added to a COM object.

`new()`/`discover()` start an idle owner and do not enumerate or connect devices.
`discover_list()` discovers explicitly, and `connect_selected(device_id)` opens
one explicit identity. The legacy-shaped no-argument `connect()` refuses implicit
selection. Unrecognized identities receive only described standard properties;
no Nikon private operation is sent. Host capture is disabled when its identity,
RAM-only trigger, and event capability set are not established.

`VendorExtensionID` is a PTP/MTP namespace, not a USB manufacturer ID. D7100
can report Microsoft/MTP `0x00000006` rather than Nikon `0x0000000A`.
Selection keeps the reported value intact: exact Nikon manufacturer and D7100
model are required; Microsoft/MTP namespaces additionally require the known
USB VID `04B0` / PID `0430` binding or an explicit `nikon.com` extension.
Capabilities still require the returned operation/property/event lists. Idle
monitoring falls back to standard GetDeviceInfo when DeviceReady/GetEvent are
not advertised; no private capability is fabricated or probed.

Health `deviceInfo` exposes the original standard/extension versions, extension
ID/description, functional mode, device version, operations, events, properties,
capture/image formats, dataset length/SHA-256 and adapter-selection evidence.
These come from the existing read-only `0x1001` response and permit diagnosis
without another device command or an image in JSONL. For further read-only
driver inspection, Microsoft documents
[vendor-extension description](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/wpd-command-mtp-ext-get-vendor-extension-description)
and the [MTP extension interface](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/supporting-mtp-extensions).

The MIT camera manager's exact D7100 model mapping remains the adaptation source.
The [gphoto project namespace definitions](https://github.com/gphoto/libgphoto2/blob/master/camlibs/ptp2/ptp.h)
and [Nikon USB identity/namespace observations](https://github.com/gphoto/libgphoto2/blob/master/camlibs/ptp2/library.c)
are factual corroboration only; no LGPL implementation is copied or linked.

For an identified D7100 advertising `0x90CA`, the owner reads GetVendorPropCodes
with no parameters. The bounded payload is a UINT32 count and exactly that many
UINT16 property codes. The original GetDeviceInfo `properties` stays intact;
`vendorPropertyCodes` records the response/list/hash and `effectiveProperties`
is the actual base/extended union used for reads and Live View capability checks.
Unadvertised discovery and unknown camera identities are never probed.

Standard `0x4008` DeviceInfoChanged requests new DeviceInfo/capability evidence
(PIMA 15740:2000 sections 5.5.1 and 12.5.8; the original standard is
[available as a PDF](https://people.ece.cornell.edu/land/courses/ece4760/FinalProjects/f2012/jmv87/site/files/pima15740-2000.pdf)).
[Microsoft's WPD event contract](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/event-constants)
also requires clients to requery changed capabilities. The selected idle camera
is refreshed within its existing shared two-second deadline: GetDeviceInfo,
exact manufacturer/model/serial and adapter-binding validation, advertised
vendor property discovery, capability negotiation and fresh properties/LV OFF.
No reopen, write, autofocus, capture or automatic wake occurs. Sleep, reserved
or unimplemented vendor functional modes cannot reuse the old qualification.
Nikon's fixed event-parameter slot has no standard DeviceInfoChanged meaning;
it is retained verbatim and never treated as an object handle. Health
`lastDeviceInfoChangedDiagnostic` retains the original event batch across later
empty polls. Identity drift or failed revalidation retires the session. A mixed
batch containing old capture/card/reset/unknown events is rejected before
refresh. At a capture boundary, during capture or after transfer, this event
refuses trigger/commit; an associated C102 in the same batch still preserves
physical exposure-end evidence. Engine preflight/profile is invalidated when
identity, DeviceInfo/effective properties, native descriptors or capabilities
change, even when parameter actual values are unchanged.

## Parameters and profile

Properties carry native numeric codes or strings, their described PTP type,
enumeration/range, writable flag, requested target and actual readback. Apply
rejects every unsupported key/value before writing. It serializes writes and
reads each actual value back. Partial failures retain actual or unknown state;
they do not manufacture rollback or a successful preflight.

The D7100 photographic catalog has 34 controls: exposure mode, Auto ISO,
NEF/JPEG format, release mode, burst count, recording media, focus mode,
exposure delay, RAW bit depth/compression, crop, shutter, ISO, aperture,
white balance, exposure compensation, long-exposure noise reduction,
color space, Picture Control, metering mode, JPEG dimensions, high-ISO noise
reduction, fluorescent white balance, color temperature, Auto ISO maximum,
center-weighted area, flash mode/compensation/sync, bracketing, Active D-Lighting and
HDR mode/exposure difference/strength. A catalog entry is never device support
evidence: an unadvertised property remains visible with `supported: false` and
an explicit reason, and receives no property command. The original descriptor
`supportedValues` and `range` remain intact. Known option labels are intersected
with that descriptor; unsupported choices are visible and disabled. Numeric
ranges keep their complete native step domain and are not replaced by the
catalog's representative labels.

Each property exposes `supported`, raw `nativeWritable`, contextual `effective`
and `writable`, `disabledReason`, and `options` with `value`, `label`, `supported`,
`enabled`, and `reason`. Confirmed actual parent values determine applicability:
M/S enables shutter, M/A enables aperture, fluorescent/Kelvin white balance
enables its matching subparameter, Auto ISO enables its maximum, and central
weighted metering enables its area. Auto ISO retains a writable base ISO and
labels it explicitly; it never claims the base value is measured exposure ISO.
JPEG dimensions are ineffective in NEF-only mode and affect the JPEG component
of NEF+JPEG. RAW recording controls are ineffective in JPEG-only mode. NEF
disables HDR ON choices while HDR OFF remains selectable. Live View disables
parameter Apply with reasons on both properties and options. Unknown or failed
parent readbacks never enable dependent controls. These Nikon-specific rules
are scoped to the identified D7100; other identities keep standard descriptors.

Apply accepts up to 64 described properties. It skips exact unchanged,
confirmed, error-free actual values, including read-only or inactive values
in a whole preset. Changing mode, Auto ISO, format, crop, white balance,
metering or HDR precedes dependent writes. A parent's successful write/readback
is followed by fresh descriptor/actual reads and renewed admission of every
child. A changed enum or rejected parent stops the child; the result retains
partial actual evidence. Current native read-only/unsupported descriptors
cannot be bypassed by predicting that a later mode change might enable them.
The physical MF selector lock also cannot be bypassed with a projected draft.

`capabilities.disabledReasons.captureReady` reports current confirmed profile,
Live View and diagnostic admission failures without changing the structural
`hostCapture` or `scanNef` capabilities. An advertised HDR property must be
confirmed OFF for deterministic NEF capture; a camera that never advertised
that optional property is not rejected for its absence. Detailed source
coverage and remaining unsupported operations are recorded in
[the capability matrix](../../../../../docs/architecture/native-camera-capability-matrix.md).

D7100 shutter decoding follows its advertised native property code. `0xD100`
uses `numerator << 16 | denominator`, in seconds (200 ms = `0x00010005`);
standard `0x500D` uses 0.1 ms units (20000 = 2000 ms). Both require a UINT32
descriptor. No model/brand inference substitutes one encoding for another.
`set_exposure_ms` selects only an exactly equal native enumerated shutter and
never rounds. Bulb, Time and flash-sync sentinel values are not timed shutters.
`set_exposure_target` changes only the shared cached target, preserving actual
readback and confirmation; only explicit Apply performs the write.

`profile_readback` requires confirmed shutter, ISO, aperture, exposure mode,
Auto ISO, NEF format, RAW bit depth/compression, crop, focus mode, drive mode,
single-image burst number, SDRAM recording media, exposure delay and long-exposure noise reduction, and freezes other supported
readable parameters. Scans require M, Auto ISO off, NEF-only, physical MF (3)
or software MF (4), single shot,
delay off and long-exposure noise reduction off. Physical focus position remains
unknown and is never invented or saved.
Physical MF (3) makes the mode selector read-only; capture never changes it to
software MF (4). AF modes 0/1/2 remain ineligible for scan capture; independent photographs use their separate verified admission path. The MIT source
proves the physical-MF selector lock but does not prove lens-motor eligibility
in that state; AF/step commands are therefore disabled with an explicit
unverified-capability reason until that eligibility has evidence.

## Capture and cancellation

`capture_to_path(path, operation_id, cancel, optional_event_sender)` returns
`CapturedImage` with its camera session, operation ID, object handle, final size,
NEF format and SHA-256. The domain derives normal independent photographs from the sole image root plus `CameraCaptures`; only the diagnostic `camera_test_capture` command may use an explicitly supplied isolated directory. The domain chooses the scan
directory; this module does not update scan counts, manifests or progress.

Explicit `configure_host_capture` reads the UINT8 `0xD10B` recording-media and
UINT16 `0x5018` burst-number descriptors/current values, writes SDRAM=1 and
single-image count=1 when needed, and confirms their readback before `0x90C0`
with parameter `0xFFFFFFFF`. Capture and restore only validate the frozen
profile; they never repair a changed acquisition setting implicitly.
Read-only `0xD1B2` information-display status is corroborated by the
[gphoto property configuration at fixed commit 3ee2ff3](https://github.com/gphoto/libgphoto2/blob/3ee2ff306582da982a053803dc42c4ee9607774e/camlibs/ptp2/config.c),
as factual protocol evidence without copied implementation.
Its descriptor/value is exposed separately in health `runtimeProperties` as
`infoDisplayErrorStatus`, outside the profile and presets, with
`semanticStatus: "unknown"`. Raw UINT8 values, including 0 and 1, remain opaque
diagnostics. The earlier zero-only capture gate was an unsupported inference
from the property name; the public property table and
[Nikon capture implementation](https://github.com/gphoto/libgphoto2/blob/3ee2ff306582da982a053803dc42c4ee9607774e/camlibs/ptp2/library.c)
provide no documented clear/fault interpretation or D1B2 capture gate.
Only advertised, read-only UINT8 with confirmed fresh raw readback and
confirmed SDRAM=1 can explain its property-change notification during capture
or the bounded post-transfer drain. Failed reads, unsupported types, writable
descriptors, acquisition-property and other notifications refuse commit;
post-transfer still requires an empty queue. These diagnostics provide no
device-health or radiation-safety verdict and never authorize X-ray output.

`runtimeProperties.liveViewProhibitCondition` separately records advertised,
read-only UINT32 `0xD1A4`. The MIT Live View prohibition reader includes shooting
in progress, shooting-command processing and an image pending in SDRAM among
its conditions; the [fixed gphoto property definition](https://github.com/gphoto/libgphoto2/blob/3ee2ff306582da982a053803dc42c4ee9607774e/camlibs/ptp2/config.c)
corroborates its type and read-only role. Its raw mask remains semantically
unknown for global health and never proves exposure completion. An exact D1A4
notification during capture/transfer causes fresh diagnostic, frozen-parameter,
RAM/single-image and Live View OFF validation. Any mismatch, unknown read,
unexpected type/writability or other acquisition notification refuses commit.
DeviceBusy reads retry only within the current finite operation deadline.

The exact read-only runtime catalog also includes exposure display `0xD1B0`
(UINT8), meter indication `0xD1B1` (INT8), exposure indication light `0xD1B3`,
and continuous-shooting count `0xD1B4` (UINT8). The latter is a status counter,
distinct from writable `0x5018` BurstNumber. Nikon namespace definitions and
the fixed gphoto property table establish these roles; D1B3's native integer
type and read-only flag must come from this device's descriptor. No adjacent
unknown property is admitted. Every accepted notification still requires fresh
frozen capture values, RAM/single-image settings and Live View OFF. All raw
runtime values remain semantically unknown and provide no completion evidence.

The actor confirms current properties and Live View off, confirms DeviceReady,
and requires an empty event queue before triggering `0x90C0` into SDRAM without
AF. One capture is in flight. SDRAM `0xC102` produces `ExposureEnded`; `0xC101`
identifies its RAM object. Standard/card, duplicate, unexpected and unresolved
events are refused. A completion event is never inferred from elapsed exposure,
temporary files, process exit, object appearance, or the end of transfer.
The MIT Nikon event path reads ObjectInfo when C101 arrives and processes C102
independently. This adapter likewise downloads the associated bounded object
into private memory on C101, then continues waiting for C102. No file is staged
or committed until reliable completion, validated NEF data and an empty event
queue are all confirmed. This separates transfer order from exposure evidence.

GetObjectInfo length and format, the complete GetObject response, TIFF directory
extents, D7100 identity and CFA raw extents are validated. This is file-completeness
validation. WPD unknown totals use a one-byte EOF probe at the object-size cap;
only zero bytes followed by a successful end-transfer response complete the read.
Any extra byte, cancellation, expired deadline or failed end response is refused.
This preserves the independent ObjectInfo versus final file-length check.
Scientific
NEF decoding belongs to the unchanged reconstruction
pipeline. File write, flush and hash finish before atomic no-overwrite rename.
Only then is separate `FileComplete` evidence returned. An unresolved capture
poisons the session rather than rebinding a late event to another frame.

The Nikon polled event dataset has no independent host operation ID. Association
uses one exclusive selected session, one in-flight RAM operation, DeviceReady,
fresh event draining, exact event/handle validation, and retirement on uncertainty.
This evidence policy is implemented and tested; its real device semantics remain
an explicit hardware acceptance gate.

Every operation shares one monotonic deadline. `request_cancel()` invalidates the
generation and revokes frames immediately, independently of COM and the ordinary
queue. Hardware cleanup is a later cached state. A blocked driver cannot block
status or the engine's independent OFF path. Its late return is checked against
the cancelled generation before side effects. Unconfirmed capture termination,
Live View shutdown or close remains isolated; it is never reported as successful
cancellation. The independent watchdog also isolates a stalled Live View call
without UI polling.
Pending-capture cleanup uses a separate finite I/O context on the original
physical session, so repeated Stop generations cannot cancel its C102 consumer.
Its bound is the trigger's frozen timed exposure plus two seconds; time is only
a waiting limit. Exactly one associated C102 after the drained trigger is still
required to clear pending exposure. Cancelled objects are not downloaded or
committed. Busy polling retries within this bound, and Close follows the wait.
An attempted Close retires the interface even when Close fails; subsequent Stop
does not poll that dead interface or reinterpret a no-op Close as confirmation.
`lastCleanupDiagnostic` retains the first attempt's session/operation, bounded
wait, completion evidence and close outcome. Same-session physical cleanup
evidence can be published after a newer Stop generation, while old capture
business results remain invalid. Missing C102 or failed cleanup remains unknown.
Cancellation, timeout and channel-loss from a caller invalidate only that
caller's still-current generation, checked under the commit gate. A late old
waiter cannot revoke a newer Stop's confirmed cleanup or overwrite an existing
unconfirmed Close diagnostic. Explicit operator Stop still invalidates the
current generation and revokes frames immediately; actual failure stays isolated.
An idle owner checks connection/identity every 250 ms under the same independent
two-second driver deadline and reports `lastSeenAt`. DeviceReady retries only
the known DeviceBusy response at 10 ms intervals within that shared, cancellable
deadline; other errors and expiry still retire the session. Camera property-change
events for tracked parameters trigger a bounded read by that same owner. Actual
readback remains confirmed, and a value differing from its preserved target
retains `property.error` and invalidates preflight. Scan capture-time acquisition
property changes remain rejected; the exact validated read-only catalog
runtime diagnostics use fresh frozen-parameter validation. Independent captures also admit only ISO/shutter/aperture output notifications governed by the confirmed automatic mode, with a fresh comparison of every fixed transaction property. Cleanup keeps the triggering device error even when close
succeeds; unknown event codes remain fail-closed and include code/parameter.
`lastEventDiagnostic` stores only the latest response code, length/hash, first
64 payload bytes and first 16 event records for diagnosis. A new successful
connection clears this previous-session error evidence.
`lastCaptureEventDiagnostic` independently retains the last nonempty capture
batch with its session/operation, batch sequence/counts, bounded raw evidence
and exposure-ended flag. Capture and cleanup diagnostics also keep `eventBatches`,
the last 16 nonempty batches, with monotonic sequence and bounded response
metadata. Empty polls cannot erase this history; no image bytes are recorded.
Idle and cleanup polling cannot overwrite the capture diagnostic. A batch
containing exactly one associated RAM completion first records ExposureEnded;
later property/unknown-event failures still refuse file commit. Duplicate RAM
completion, old queues and unknown events remain rejected. A property-change
error includes its native code and whether it maps to a frozen property.
Every capture reads properties again and rejects drift from its previously
confirmed profile before sending the RAM trigger. Owner panics publish isolated,
unconfirmed cleanup before any potentially blocked COM release; stale health is
never retained as evidence that the worker is alive.

## Live View binary channel

`0x9201` starts, `0x9202` stops and `0x9203` retrieves D7100 Live View. Start/stop
read `0xD1A2` back. D7100 header width/height at bytes 8/10 are big-endian UINT16;
JPEG starts at byte 384. Length, decoded dimensions and header dimensions must
agree. The internal desktop pipe below uses little-endian fields. Frame requests run at most 10 fps, with no fixed-rate
promise. One latest-frame slot replaces older frames. Capture is refused while
Live View is active; it never stops preview automatically to shoot.

Each successful Start establishes a new random 32-hex Live View session. The
pipe descriptor, every frame, `liveView.sessionId` and
`liveViewControls.sessionId` share that identity. The stable camera connection
`sessionId` remains the capture identity. Stop, external confirmed LV OFF,
disconnect and isolation revoke the LV identity and all LV write permissions;
restarting cannot accept old controls or late frames.

`camera_live_view_read { sessionId }` and
`camera_live_view_configure { sessionId, controls }` operate only on the current
active session. `controls` accepts one or both of `cameraZoom` (`0xD1A3`) and
`focusMode` (`0xD061`), separately from the 34 photographic properties and from
scan profiles/presets. Ordinary photographic Apply remains forbidden in LV.
Read refreshes DeviceInfo/advertisement and confirmed LV ON without reopening
or changing photographic properties. Each write additionally requires a fresh
advertised integer descriptor, current readback, enum/range permission and native
writability. `cameraZoom` accepts scalar integer types 1–8 and encodes GET/SET
using the freshly described native width; values and described bounds remain
within 0–255. `focusMode` still requires UINT8 and one byte. A descriptor type
change during SET cannot confirm the target, even if the numeric readback agrees.
The actual D7100 D1A3 descriptor type remains pending hardware verification;
digiCamControl's fixed eight-byte write is not substituted for native evidence.
Unsupported types and unadvertised properties stay UNKNOWN and receive no SET;
unadvertised properties receive no speculative GET either.

Partial writes retain target and actual evidence and do not claim rollback or
completion. An explicit subsequent Apply can declare a new target after fresh
descriptor/readback admission. Read preserves an unresolved mismatch; Stop and
Start establish a new control session with cleared targets. Errors explain
this recovery path. Queued work checks the LV generation again after reads and
before writes or motor commands, so Stop/cancellation cannot restore permissions.

Advertised `lensSort` (`0xD0E1`), `liveViewSelector` (`0xD1A6`) and `activeSlot`
(`0xD1F2`) are read-only display diagnostics in `runtimeProperties`; actual
`nativeWritable` evidence is retained. No card labels are guessed from a generic
Nikon mapping, and an unadvertised slot remains UNKNOWN. D1A6 selects photo/movie
LV and is not an AF-mode property. Bounded LV AF requires fresh CPU-lens category
1, photo selector 0, D061 AF-S 0, and advertised AF/DeviceReady operations.
Manual lens step requires the same lens/selector evidence and D061 0 or software
MF 4 plus advertised step/DeviceReady operations. Physical MF 3 is read-only and
cannot drive either motor; AF-F 2 can be selected only when described, but does
not enable these bounded motor actions. Unknown or failed evidence disables
motor controls. Outside LV, AF additionally requires normal D161 mode 0/1/2;
manual step always requires LV. Lens category alone does not prove motor support.

Frame data is never in JSONL. The engine creates a cryptographically random
`\\.\pipe\microct-lv-*` endpoint with an ACL granting only the current user's
SID, rejects remote clients, and writes from a separate bounded pipe worker.
Slow or absent consumers cannot block device control. `pipe_descriptor()` is
an internal desktop capability and is not an arbitrary UI-supplied path.

| Byte offset | Value |
| --- | --- |
| 0 | 8-byte `MCTLV001` magic |
| 8 | LE u16 protocol 2 |
| 10 | LE u16 codec 1 (JPEG) |
| 12 | LE u32 header length 64 |
| 16 | 16 random LV session bytes (`liveView.sessionId` is their lowercase hex) |
| 32 | LE u64 monotonic sequence |
| 40 / 44 | LE u32 width / height |
| 48 | LE u32 JPEG length, at most 4 MiB |
| 52 | LE u32 reserved zero |
| 56 | LE u64 capture timestamp in Unix milliseconds |
| 64 | JPEG bytes, exact declared length |

The desktop forwarder uses `decode_live_view_frame` with the engine-issued
expected session and last accepted sequence. It refuses foreign, duplicate,
old, truncated, oversized, undecodable or mismatched frames. Within a camera
session, sequence remains monotonic across preview restarts. Stop/disconnect,
session change and faults revoke real-time frame identity. Frames older than
one second are not served as fresh.

The protocol 2 pipe is duplex. After reading and validating the complete header
and JPEG, the client writes an exact 32-byte receipt ACK using
`encode_live_view_ack(session_id, sequence)`: ASCII `MCTACK02` (8 bytes), the
16 raw session bytes, and the LE u64 frame sequence. The producer keeps its
handle until this exact receipt arrives, preventing DisconnectNamedPipe from
discarding unread payload. Frame write and ACK share one monotonic 250 ms
deadline. Missing, partial, foreign or incorrect ACKs expire/close the instance;
they never wait on the device owner or Stop. No arbitrary sleep or unbounded
FlushFileBuffers is used as evidence of delivery. Old protocol 1 clients are
not silently accepted.

## Offline verification

```powershell
cargo test --manifest-path backend/Cargo.toml -p ct-engine devices::camera --offline -- --test-threads=1
```

Tests instantiate only a scripted transport on the real production owner thread.
They cover capture/event/file separation, no timed exposure guessing, wrong or
duplicate events, truncation, no overwrites, partial parameter application,
native shutter equality, unknown brands, blocked transfer cancellation, state
access, background watchdog isolation, JPEG envelopes, frame revocation, and
actual local named-pipe bytes. They never enumerate WPD or contact a camera.
`scripted_camera(ScriptedCameraConfig)` is available only under `cfg(test)` to
exercise the same scan coordinator with configurable exposure/transfer delays,
missing exposure events and truncated synthetic TIFF payloads.

Generated evidence belongs in `.cache/tests/camera/<run-id>/`. Test TIFFs are
synthetic integrity fixtures, not acquired NEF or scientific calibration data.

### Current fixed scan policy and independent photographs

Scan admission additionally requires confirmed fixed WB, bracketing OFF from a currently advertised `0xD0C0` UINT8 descriptor and fresh readback, and ADL OFF=4. Missing or failed ADL evidence blocks D7100 scanning. HDR requires native OFF=0 when advertised; only an unadvertised HDR property can use confirmed NEF incompatibility as equivalent exclusion. Profile v6 freezes these proofs and the confirmed acquisition parameters. Profiles 1–5 remain read-only and retain their original canonical hashes; retired fields are decoded solely to verify those historical hashes.

`capture_to_path` remains the fixed scan transaction path. `capture_independent_to_path` allows the confirmed photographic exposure modes, Auto ISO, auto WB and supported AF mode without implicitly triggering AF. Only ISO/shutter/aperture outputs governed by their confirmed automatic mode may vary; identity, session, RAM, format, single host-output count and fixed transaction conditions stay checked. Both paths explicitly refuse active Live View and retain associated C102 completion, complete NEF length/hash, cancellation and atomic no-replace commit. The independent path produces one validated host NEF. Independent captures never infer completion from shutter duration; their native event wait remains bounded.

Preflight/start/recovery issue reads only for core camera configuration; they never stop Live View or invoke `configure_host_capture`/shutter writes. Explicit Apply owns all configuration writes. Live View cleanup and photographs keep their existing ownership and cancellation evidence.
