# Documentation map

## Current references

- `architecture/README.md`: primary software architecture entry.
- `architecture/directory-map.md`: responsibility of every active directory and important subdirectory.
- `architecture/execution-flows.md`: startup, command, scan, failure, and shutdown flows.
- `architecture/dual-control-topology.md`: current PC-direct topology and the planned Arduino-managed backend boundary.
- `architecture/module-graph.md`: generated module dependency view; never edit by hand.
- `WINDOWS-INTEGRATION.md`: current Windows process, device-module, and hardware acceptance boundary.
- `3d-model-and-camera-guide.md`: current procedural 3D scene maintenance guide.

## Decisions

`decisions/` records durable choices, rejected alternatives, and consequences. Update a decision only when the shipped facts change; create a new decision when replacing its rationale.

## Historical material

- `CURRENT-PROGRESS.md`, `CONSOLE-REDESIGN.md`, and `system_design.md` are dated implementation snapshots.
- `PHASE-1-3-IMPLEMENTATION-PLAN.md` is a superseded plan.
- `WINDOWS-CODEX-PROMPT.md` is an archived handoff prompt with an obsolete machine path.
- `HARDWARE-V3.md` records a retired hardware-planning location.

Historical documents explain why the repository evolved but are not authority for current paths or behavior. Source code, tests, `module-map/`, and `docs/architecture/` take precedence.

## Verification evidence

`shots/` contains screenshots, probe scripts, JSON measurements, and transient QA logs from earlier acceptance work. These files are evidence, not product code or reusable architecture. Python caches and other regenerated output do not belong in design truth.
