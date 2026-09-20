# Build and verification scripts

- `prepare-engine-sidecar.mjs`: builds `ct-engine` and copies it to Tauri's target-specific sidecar name.
- `prepare-sites-build.mjs`: assembles the optional hosted-preview output.
- `process-logo.mjs`: explicit one-time logo asset transformation.
- `gen-module-graph.mjs`: generates the human-readable module graph.
- `verify-module-map.mjs`: checks unique file ownership and dependency references.
- `impact.mjs`: maps Git changes to owning modules, dependants, hidden edges, and required gates.
- `lib/module-map.mjs`: dependency-free shared path/glob helpers for the three module-map commands.

These scripts are offline build/development tooling and must not connect to real hardware.
