# Tauri desktop shell

`src-tauri/` is the thin Windows host around React and `ct-engine`.

- `src/main.rs`: Tauri commands, native known-folder resolution, dialogs, window sizing, and lifecycle hooks.
- `src/engine_client.rs`: verifies and starts the sidecar, sends versioned JSONL requests, checks request/sequence identity, and enforces timeouts.
- `tauri.conf.json`: window, security, bundle, and sidecar configuration.
- `capabilities/`: explicit Tauri permission declarations.
- `icons/`: generated desktop/mobile icon assets.
- `gen/schemas/`: generated Tauri schemas; do not edit manually.
- `binaries/`: regenerated sidecar copies created by `npm.cmd run engine:build`; ignored by Git.

The shell does not own hardware or scan state. IPC failures are surfaced and never replaced with browser simulation.

`reconstruction_preview` accepts a method ID after the engine reports a completed reconstruction. It reads only the engine-registered cache inside the scan output directory, verifies the bounded uint8 volume and SHA-256 recorded in `preview.json`, and returns voxel bytes plus shape, spacing and display-window metadata. It does not start computation or own reconstruction state.
