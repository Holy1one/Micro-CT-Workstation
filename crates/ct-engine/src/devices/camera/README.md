# Native camera adapter (offline candidate)

This module owns Windows WPD/PTP communication, identity and capability discovery,
property write/readback, Nikon D7100 host RAM NEF capture, Live View, and host file
confirmation. `ct-engine` owns all device occupancy and cross-device safety rules.
No external camera program, extra bridge process, USB driver replacement, camera
card fallback, or production simulation is used.

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
| `capture.rs` | Correlated SDRAM events, NEF structural completeness, SHA-256 and atomic no-overwrite commit |
| `live_view.rs` | Strict JPEG validation, binary envelope, latest frame slot and current-user-only named pipe |
| `tests.rs` | Scripted transport through the same production actor and capture implementation |

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

## Parameters and profile

Properties carry native numeric codes or strings, their described PTP type,
enumeration/range, writable flag, requested target and actual readback. Apply
rejects every unsupported key/value before writing. It serializes writes and
reads each actual value back. Partial failures retain actual or unknown state;
they do not manufacture rollback or a successful preflight.

D7100 shutter is the exact packed rational integer
`numerator << 16 | denominator`, in seconds. For example, 200 ms is `0x00010005`.
`set_exposure_ms` selects only an exactly equal native enumerated shutter and
never rounds. Bulb, Time and flash-sync sentinel values are not timed shutters.
`set_exposure_target` changes only the shared cached target, preserving actual
readback and confirmation; explicit Apply or preflight performs the write.

`profile_readback` requires confirmed shutter, ISO, aperture, exposure mode,
Auto ISO, NEF format, RAW bit depth/compression, crop, focus mode, drive mode,
exposure delay and long-exposure noise reduction, and freezes other supported
readable parameters. Scans require M, Auto ISO off, NEF-only, MF, single shot,
delay off and long-exposure noise reduction off. Physical focus position remains
unknown and is never invented or saved.

## Capture and cancellation

`capture_to_path(path, operation_id, cancel, optional_event_sender)` returns
`CapturedImage` with its camera session, operation ID, object handle, final size,
NEF format and SHA-256. The domain chooses the independent single-shot or scan
directory; this module does not update scan counts, manifests or progress.

The actor confirms current properties and Live View off, confirms DeviceReady,
and requires an empty event queue before triggering `0x90C0` into SDRAM without
AF. One capture is in flight. SDRAM `0xC102` produces `ExposureEnded`; `0xC101`
identifies its RAM object. Standard/card, duplicate, unexpected and unresolved
events are refused. A completion event is never inferred from elapsed exposure,
temporary files, process exit, object appearance, or the end of transfer.

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
An idle owner checks connection/identity every 250 ms under the same independent
two-second driver deadline and reports `lastSeenAt`. Camera property-change
events invalidate the affected actual readback until it is explicitly refreshed.
Every capture reads properties again and rejects drift from its previously
confirmed profile before sending the RAM trigger. Owner panics publish isolated,
unconfirmed cleanup before any potentially blocked COM release; stale health is
never retained as evidence that the worker is alive.

## Live View binary channel

`0x9201` starts, `0x9202` stops and `0x9203` retrieves D7100 Live View. Start/stop
read `0xD1A2` back. JPEG starts at byte 384; length, decoded dimensions and header
dimensions must agree. Frame requests run at most 10 fps, with no fixed-rate
promise. One latest-frame slot replaces older frames. Capture is refused while
Live View is active; it never stops preview automatically to shoot.

Frame data is never in JSONL. The engine creates a cryptographically random
`\\.\pipe\microct-lv-*` endpoint with an ACL granting only the current user's
SID, rejects remote clients, and writes from a separate bounded pipe worker.
Slow or absent consumers cannot block device control. `pipe_descriptor()` is
an internal desktop capability and is not an arbitrary UI-supplied path.

| Byte offset | Value |
| --- | --- |
| 0 | 8-byte `MCTLV001` magic |
| 8 | LE u16 protocol 1 |
| 10 | LE u16 codec 1 (JPEG) |
| 12 | LE u32 header length 64 |
| 16 | 16 random session bytes (health session is their lowercase hex) |
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

## Offline verification

```powershell
cargo test -p ct-engine devices::camera --offline -- --test-threads=1
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

Generated evidence belongs in `tmp/tests/camera/<run-id>/`. Test TIFFs are
synthetic integrity fixtures, not acquired NEF or scientific calibration data.
