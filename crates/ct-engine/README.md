# ct-engine

`ct-engine` is the production domain core and sidecar process. It is the only owner of physical-device connections, safety gates, scan state, and projection commit progress.

## Source layout

- `src/main.rs`: bounded JSONL stdin/stdout process loop and deterministic shutdown on parent EOF.
- `src/lib.rs`: public request/response DTOs, domain commands, safety checks, complete snapshots, and top-level engine state.
- `src/scan.rs`: transactional multi-device projection workflow and manifest persistence.
- `src/devices/`: physical-device communication implementations, separated by device role.

The executable is started and monitored by Tauri. React never imports this crate directly; it communicates through Tauri and the versioned JSONL envelope.

## Development commands

```powershell
cargo test -p ct-engine
cargo test --workspace
```

Tests are offline. Passing tests do not prove real movement, camera capture, X-ray output, or hardware interlocks.
