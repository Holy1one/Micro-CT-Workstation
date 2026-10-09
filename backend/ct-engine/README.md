# ct-engine

`ct-engine` is the production domain core and sidecar process. It is the only owner of physical-device connections, safety gates, scan state, and projection commit progress.

## Source layout

- `src/main.rs`: bounded JSONL stdin/stdout process loop and deterministic shutdown on parent EOF.
- `src/lib.rs`: public request/response DTOs, domain commands, safety checks, complete snapshots, and top-level engine state.
- `src/scan.rs`: transactional multi-device projection workflow and manifest persistence.
- `src/operations.rs` / `device_controls.rs`: cancellable background operations, independent device entry points and priority all-device cleanup.
- `src/software_update.rs`: engine-issued maintenance ownership and correlated device cleanup reports; updater consumers must receive the current ready report before EOF and normal engine exit.
- `src/capture_profile.rs` / `presets.rs` / `task_registry.rs`: native readback evidence, camera-only preset JSON and persistent unfinished task locks across restarts.
- `src/reconstruction_preprocess.rs`: offline reference variance, noise censoring, support masks, normalized smoothing and confidence-gated conjugate-ray axis estimation; no device access.
- `src/devices/`: physical-device communication implementations, separated by device role.

The executable is started and monitored by Tauri. React never imports this crate directly; it communicates through Tauri and the versioned JSONL envelope.

## Development commands

Run these commands from the repository root. Inside `backend/`, omit
`--manifest-path backend/Cargo.toml`; both use the same workspace and cache.

```powershell
cargo test --manifest-path backend/Cargo.toml -p ct-engine
cargo test --manifest-path backend/Cargo.toml --workspace
```

Tests are offline. Passing tests do not prove real movement, camera capture, X-ray output, or hardware interlocks.

The production-coordinator regression group `cargo test --manifest-path backend/Cargo.toml -p ct-engine scan::tests::r0_ -- --nocapture`
runs all three real device owners with scripted physical transports. It covers projection-time
camera loss, interlock, Stop during transfer, unconfirmed OFF, manifest persistence failure,
and pause/resume without duplicate files. Each case retains its own evidence under
`.cache/tests/ct-engine/`. The [R0 coverage matrix](../../docs/validation/README.md)
records the assertions, remaining scope and required hardware acceptance.

The native WPD/PTP camera and message-owned device runtime are an offline candidate. Camera JPEG streams use a random user-ACL named pipe; stdio carries protocol-v2 control/snapshot envelopes. Timed tick, monitoring and EOF do not require UI polling. `MICRO_CT_ENGINE_DATA_DIR` isolates offline test state; normal state lives under `LOCALAPPDATA/MicroCTWorkstation`.
# Offline reconstruction

`src/reconstruction.rs` consumes only a sealed scan manifest and hash-verified NEF files. It does not control CT hardware. FDK runs on CPU or on a compatible GPU through `src/fdk.wgsl`: the first view checks GPU output against CPU output and measures the expected runtime; GPU is selected only when the estimate is at least 15% faster. GPU failures restart the full FDK on CPU. A bounded 64³, five-iteration CPU SIRT is available for small scans; larger SIRT jobs and CGLS remain disabled. The science volume is stored as signed float32 and the display preview as uint8 with separate window metadata. Numerical tests use synthetic geometry; D7100 NEF and GPU driver behavior still require validation with actual data and the target machine.

## Preparation and acquisition admission

`set_capture_root {savePath}` edits the sole `Parameters.savePath` root without requiring a scan ID. `prepare_scan {taskId,projectionCount,maxXraySec,geometry}` validates the complete existing geometry and commits it atomically, using only the confirmed native camera shutter for exposure. Neither preparation nor its cancellation creates a task lock or acquisition files. New-task Preflight requires this preparation; recovery retains its frozen task configuration.

Only explicit camera Apply writes acquisition properties. Preflight, start and recovery read fresh values and refuse active Live View or mismatches. Current profile version 6 freezes fixed white balance, advertised UINT8 bracketing `0xD0C0` OFF, Active D-Lighting OFF (native 4), HDR OFF or documented NEF exclusion, and the existing M/Auto ISO OFF/MF/single RAM NEF requirements. `cameraScanReadiness.issues` explains actual values, requirements and corrections. Versions 1–5 keep their original hashes and read-only reconstruction semantics; they cannot append projections.

Independent captures use a separate admission path: verified P/A/S and Auto ISO/WB/AF mode are allowed, while RAM/NEF/single host-output transaction, bounded native events, associated C102 completion and complete non-overwriting file/hash evidence remain mandatory. Automatic exposure outputs may vary only according to their confirmed exposure mode. No implicit autofocus is sent. Successful captures are registered by `captureId` in `deviceControls.camera.lastCapturedImage`; the internal descriptor command accepts that ID, never a renderer path.

Reconstruction worker events carry sequence and timestamp and enter the aggregate console once. Software update preparation rejects activities, Live View and retained task locks, obtains concurrent STOP/OFF and camera cleanup evidence, and keeps maintenance ownership until shutdown or matching explicit cancellation. An unknown or failed report cannot release the lock or authorize an update. A later ordinary Stop invalidates a previously ready report without clearing maintenance ownership.

## Acceptance evidence scope

The accepted P0 camera package has real D7100 independent RAM NEF and UI Live View evidence: [result](../../.cache/tests/desktop/20261005-004700-0070-nef/result.json) and [correlated native events/file](../../.cache/tests/desktop/20261005-004700-0070-nef/captured-nef.json). EXE SHA-256 is `0070808c9fb206d4745ba039bcd1eb4c13d19e0a12fef3d164be93797637862c`; embedded engine SHA-256 is `f8f074d75518e4cf0304e630c683ba643e8f5f23be882b00687ee03e3b191ffa`. The recorded C101/C102 and exposure-end association produced a complete 23,476,423-byte NEF with hash, preserved/restored original parameters, and delivered 30.1096 seconds of UI Live View without missing frames after the first or saved live pixel files. App and engine both exited normally with strict cleanup. An explicit pre-close LV-stop terminal snapshot was not retained.

This is package-specific P0 acceptance, with no X-ray or Nano motion. It does not certify the subsequent profile-v4/automatic-photo/maintenance implementation, linked radiation scanning, lens-drive qualification, mechanical accuracy or reconstruction fidelity. Current domain regression is [offline PASS](../../.cache/tests/ct-engine/20261005-025200-domain-final/result.json); browser parity is [simulation-only PASS](../../.cache/tests/frontend/20261005-042300-preview-final/result.json). These local `tmp/` evidence files are ignored by Git; hardware certification remains separate from software or installer signatures.
