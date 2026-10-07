# Build and verification scripts

- `prepare-engine-sidecar.mjs`: builds `ct-engine` and copies it to Tauri's target-specific sidecar name.
- `build-portable.mjs`: runs a fresh native Windows Tauri release build; `--offline-candidate` publishes to `release/native-device-controls-offline/` and verifies that the existing formal portable EXE stays unchanged.
- `build-turntable-firmware.mjs`: verifies the imported candidate source hash and rebuilds sketch/core from the pinned Arduino AVR source/toolchain, without serial discovery or upload.
- `test-turntable-firmware.mjs`: compiles the unchanged candidate's actual HOST_HARNESS branch and runs its deterministic parser/state/pulse checks.
- `collect-candidate-licenses.mjs`: collects locked Cargo/npm license declarations and available notices into the offline candidate package.
- `build-update-release.mjs`: builds the current version under `release/<version>/`; saves the unmarked portable EXE before independently bundling NSIS/MSI and preserves the formal portable entry.
- `inspect-msi.ps1`: reads actual MSI Property/Summary tables without installing or invoking custom actions.
- `record-bundle-identities.mjs`: compares actual unpacked/installed NSIS/MSI application bytes and bundle markers with the portable build.
- `update-sign.ps1` / `update-keygen-child.mjs`: generate or recover the stable encrypted key under the current user's LocalAppData with restrictive ACL and DPAPI password; sign only final version release files with official CLI 2.12.1, keeping secrets out of process argv/output.
- `sign-update-release.mjs`: enforces MSI ledger and bundle identity evidence, signs final artifacts and manifest with an explicit application version, and verifies each ZIP_STORED offline package using the application's early verifier. It never publishes or uploads.
- `msi-version-ledger.json`: fixes the UpgradeCode and allocates strictly increasing numeric installer versions separately from application SemVer.
- `lib/release-version.mjs`: checks canonical SemVer, strictly increasing app/MSI ledger entries, Cargo/package/Tauri agreement and the explicit installer downgrade block; rc1 remains `0.9.1`, rc2 maps to `0.9.2`.
- `prepare-sites-build.mjs`: assembles the optional hosted-preview output.
- `process-logo.mjs`: explicit one-time logo asset transformation.
- `gen-module-graph.mjs`: generates the human-readable module graph.
- `verify-module-map.mjs`: checks unique file ownership and dependency references.
- `impact.mjs`: maps Git changes to owning modules, dependants, hidden edges, and required gates.
- `lib/module-map.mjs`: dependency-free shared path/glob helpers for the three module-map commands.

These scripts are offline build/development tooling and must not connect to real hardware.
