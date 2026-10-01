# ct-engine

`ct-engine` is the production domain core and sidecar process. It is the only owner of physical-device connections, safety gates, scan state, and projection commit progress.

## Source layout

- `src/main.rs`: bounded JSONL stdin/stdout process loop and deterministic shutdown on parent EOF.
- `src/lib.rs`: public request/response DTOs, domain commands, safety checks, complete snapshots, and top-level engine state.
- `src/scan.rs`: transactional multi-device projection workflow and manifest persistence.
- `src/reconstruction_preprocess.rs`: offline reference variance, noise censoring, support masks, normalized smoothing and confidence-gated conjugate-ray axis estimation; no device access.
- `src/devices/`: physical-device communication implementations, separated by device role.

The executable is started and monitored by Tauri. React never imports this crate directly; it communicates through Tauri and the versioned JSONL envelope.

## Development commands

```powershell
cargo test -p ct-engine
cargo test --workspace
```

Tests are offline. Passing tests do not prove real movement, camera capture, X-ray output, or hardware interlocks.
# Offline reconstruction

`src/reconstruction.rs` consumes only a sealed scan manifest and hash-verified NEF files. It does not control CT hardware. FDK runs on CPU or on a compatible GPU through `src/fdk.wgsl`: the first view checks GPU output against CPU output and measures the expected runtime; GPU is selected only when the estimate is at least 15% faster. GPU failures restart the full FDK on CPU. A bounded 64³, five-iteration CPU SIRT is available for small scans; larger SIRT jobs and CGLS remain disabled. The science volume is stored as signed float32 and the display preview as uint8 with separate window metadata. Numerical tests use synthetic geometry; D7100 NEF and GPU driver behavior still require validation with actual data and the target machine.
