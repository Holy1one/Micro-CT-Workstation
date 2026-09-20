# Offline contract tests

- `canvas-layout.test.mjs`: fixed-canvas scale and viewport invariants.
- `desktop-path-contract.test.mjs`: frontend/Tauri path, picker, save, and window contracts.
- `menu-wiring-contract.test.mjs`: verifies that every visible menu entry has an implementation.
- `sites-worker.test.mjs`: hosted-preview asset and packaging behavior.

Run all frontend gates with:

```powershell
npm.cmd test
```

These tests do not connect to the Nano, D7100, or Moxtek source and cannot establish real-hardware acceptance.
