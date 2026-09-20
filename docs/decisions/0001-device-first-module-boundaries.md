# Agent Note: Device-first production module boundaries

Status: implemented

## Problem

The production drivers were flat files under `ct-engine/src/adapters`. That layout described a technical pattern but did not give X-ray, camera, and turntable work independent ownership. It also provided no clear place for a future controller-managed topology alongside today's PC-direct connections.

## Decision

Production device code is organized by physical role under `ct-engine/src/devices/{xray,camera,turntable}`. Each directory exposes a small `mod.rs` surface and keeps its current vendor-specific implementation private. Cross-device transaction order remains in `scan.rs`, and authoritative safety and scan state remain in `ct-engine`.

The current topology is PC-direct. A future Arduino-centered implementation will be added as an explicit controller-managed backend behind the same domain behavior. It will not replace the current backend silently and will not create a second uncoordinated scan authority.

## Alternatives considered

### Keep a flat `adapters` directory

This minimized the immediate move, but every deep device branch would continue to share one namespace and broad directory rules. The name also hid whether an adapter represented a physical device, an IPC layer, or a preview implementation.

### Create one Rust crate per device immediately

Separate crates offer stronger compile-time boundaries, but today the adapters share engine DTOs and scanning assumptions that have not yet stabilized as public crate contracts. Splitting crates now would add manifests and versioning without reducing the most dangerous hidden protocol dependencies. The directory boundaries can later become crates when their public ports stabilize.

### Design the Arduino protocol first

There is no active controller firmware source or verified unified protocol. Inventing it in the PC repository would turn assumptions into false contracts. The architecture therefore reserves a backend seam while deferring protocol fields until firmware and hardware evidence exist.

## Consequences

- X-ray, camera, and turntable changes have distinct paths, rules, tests, and module-map ownership.
- The scan coordinator remains the only place that orders multiple devices.
- Existing Rust type names and JSONL behavior remain compatible after the move.
- Future controller work must begin with an active firmware source and a versioned protocol decision.
- The large engine and preview files still need incremental decomposition; this decision prevents new device-specific code from increasing that coupling.
