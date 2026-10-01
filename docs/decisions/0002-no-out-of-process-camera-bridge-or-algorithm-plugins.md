# Agent Note: No out-of-process camera bridge or algorithm plugin system

Status: implemented

## Problem

Two extension seams existed only as design drafts while carrying tracked files
and module-map ownership: a Windows-only .NET `digicam-bridge.exe` process for
the Nikon D7100 (`bridges/digicam/README.md`, protocol `digicam.bridge.v1`) and a
Python algorithm plugin system with four package contracts and an
`AlgorithmSupervisor` running `python -m ct_worker` (`algorithm-plugins/README.md`).
No build, test, or release path produced either one. In reality the camera was
implemented in-process and reconstruction methods were implemented inside
`ct-engine`.

## Decision

Neither seam is adopted. Both draft directories and their module-map ownership
were removed, and the replacement facts are recorded here.

- The camera is driven in-process from `crates/ct-engine/src/devices/camera/`
  against DigiCamControl on Windows. There is no second camera process, no bridge
  handshake, and no bridge-side scan state.
- Reconstruction algorithms are compiled into `ct-engine` and declared by
  `ReconstructionMethod`. Adding a method means adding a Rust implementation,
  offline tests, and a versioned release.

## Alternatives considered

### Keep the drafts as unimplemented contracts

Keeping them cost nothing at build time, but a directory-policy entry plus
module-map ownership made an unbuilt plan read as an owned subsystem; two
reviews already treated `bridges/` as stale. The drafts stay recoverable from git
history, and this note records why they were dropped.

### Implement the .NET bridge

It would have isolated .NET/SDK incompatibility from the Rust engine and kept a
non-Windows host option open. The product is now a single Windows host (Tauri,
WebView2, DigiCamControl), so that isolation buys no compatibility while adding a
second owner of camera state that must be reconciled with `ct-engine`.

### Implement the algorithm plugin system

It would let algorithm authors add methods without rebuilding the application,
and this project does add reconstruction algorithms over time. It also requires a
second runtime (a Python environment), a package and source-trust model, dataset
compatibility checks, resource limits, cancellation, and a new execution path
that consumes scan data. `algorithm-plugins/README.md` itself listed unresolved
blocking questions and shipped no executable schema. Adding the same method in
Rust is cheaper and auditable: the `ReconstructionMethod` seam, offline tests,
and a release tag keep one truth source.

## Consequences

- `bridges/` and `algorithm-plugins/` no longer exist. `module-map/modules.json`
  and `module-map/directory-policy.json` no longer register them, and the
  `extensions` module now owns only the Sites static preview worker.
- Adding a reconstruction algorithm is a source change in `ct-engine` carried by
  a release tag. Exporting scientific artifacts stays the supported way to
  experiment with external algorithms outside the application.
- Historical documents that still describe the two seams as options
  (`docs/ARCHITECTURE.md` selection rationale, `CURRENT-PROGRESS.md`,
  `WINDOWS-INTEGRATION.md`, `WINDOWS-CODEX-PROMPT.md`,
  `PHASE-1-3-IMPLEMENTATION-PLAN.md`) keep their text as history; they are not
  authority for current paths or behavior.
- Revisit only when several external authors must add algorithms without a
  repository build, or when a non-Windows host must drive the D7100 again.
