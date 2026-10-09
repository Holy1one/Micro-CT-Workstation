# Tauri desktop shell

`backend/src-tauri/` is the thin Windows host around React and `ct-engine`.

- `src/main.rs`: Tauri commands, native known-folder resolution, dialogs, window sizing, and lifecycle hooks.
- `src/windows_icons.rs`: loads the EXE's embedded ICO through Windows at the window DPI for both large and small icons; refreshes after DPI changes and releases owned handles. Windows selects DPI frames and decodes their transparency/masks.
- `src/engine_client.rs`: verifies and starts the sidecar, sends versioned JSONL requests, checks request/sequence identity, and enforces timeouts.
- `src/updater.rs`: restricted anonymous release transport, data/global/version
  signature validation, common offline ZIP verification and early apply helper.
  A shared data-directory lease is acquired before engine bootstrap; applying
  holds it exclusively. The helper never bootstraps a control engine or WebView.
- `update-public-key.pub`: the compiled release trust root. Private key/password
  material is kept outside the repository and release packages.
- `tauri.conf.json`: window, security, bundle, and sidecar configuration.
- `capabilities/`: explicit Tauri permission declarations.
- `../../assets/app-*`: maintained bundle icons shared with the repository assets.
- `gen/schemas/`: generated Tauri schemas; do not edit manually.
- `binaries/`: regenerated sidecar copies created by `npm.cmd run engine:build`; ignored by Git.

The shell does not own hardware or scan state. IPC failures are surfaced and never replaced with browser simulation.

The Cargo workspace and lockfile are in `backend/`. Run npm commands from the
repository root; `tools/build/run-tauri.mjs` supplies the desktop and npm-hook
paths explicitly. Generated sidecars and schemas remain inside this crate.

`camera_capture_preview` accepts only a registered capture ID, resolves the
engine descriptor, bounds the NEF read, verifies size/hash and returns embedded
JPEG bytes in memory. The descriptor/session is checked again after the read;
the renderer never supplies a file path or receives a preview-file write path.

Installation confirmation requests an engine maintenance barrier. Only the
matching production engine session and completed operation, locked ready report,
device cleanup/never-owned evidence and no transactions permit EOF. Owned engine
exit zero and its original process-handle creation time are then required before
the verified helper is dispatched and this app exits. The helper re-verifies the
package, intent, owner identity and exclusive target lease; process exit alone
cannot stand in for device cleanup. Normal checking/downloading does not stop devices.

Update state exposes `notice` separately from `error`. A structured successful
receipt must agree with the actual running target/version/kind and required
successful installer exit before it becomes a completion notice. Failed or incomplete
installer results remain errors even when the app version happens to match.
New check/import/cancel actions clear stale feedback; the React dialog renders
completion as status and actual failures as alerts.

`reconstruction_preview` accepts a method ID after the engine reports a completed reconstruction. It reads only the engine-registered cache inside the scan output directory, verifies the bounded uint8 volume and SHA-256 recorded in `preview.json`, and returns voxel bytes plus shape, spacing and display-window metadata. It does not start computation or own reconstruction state.

The MSI installs ct-engine next to ct-workstation. Its SHA-256 is compiled into the desktop host and verified before launch. Missing or modified engines require Repair. No engine executable is extracted to the user data directory.

The update dialog checks GitHub’s latest stable release, offers Download when a higher signed version is available, and offers Install after verification. Channels, transport URLs and offline import are not exposed in the normal menu.
