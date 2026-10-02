# Offline contract tests

- `canvas-layout.test.mjs`: fixed-canvas scale and viewport invariants.
- `desktop-path-contract.test.mjs`: frontend/Tauri path, picker, save, and window contracts.
- `menu-wiring-contract.test.mjs`: verifies that every visible menu entry has an implementation.
- `native-device-controls.test.mjs`: executes native property draft validation,
  exact shutter matching, snapshot session ordering, priority Stop scheduling,
  occupation/unfinished locks and explicitly simulated independent camera/preset flows.
- `preview-scan-lifecycle.test.mjs`: negative direction, unwrapped phase planning,
  matching ticket release, stop/pause and final-angle retention through the actual
  browser-preview coordinator. This is UI evidence, not production-device evidence.
- `sites-worker.test.mjs`: hosted-preview asset and packaging behavior.

Run all frontend gates with:

```powershell
npm.cmd test
```

These tests do not connect to the Nano, D7100, or Moxtek source and cannot establish real-hardware acceptance.
