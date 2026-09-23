# Production device modules

This directory contains the current PC-direct hardware implementations. Each child directory owns one physical role and exposes its public Rust surface from `mod.rs`.

## `xray/`

- `moxtek.rs` discovers and validates the Moxtek source.
- Owns binary serial framing, requested setpoints, measured readback, beam transitions, USB auto-shutdown, and fail-closed disconnect.
- Does not decide when a scan may expose; that policy belongs to the engine and scan coordinator.

## `camera/`

- `digicam_control.rs` integrates the Nikon D7100 through DigiCamControl.
- Owns discovery, exposure conversion, capture invocation, timeout handling, task-id validation, and host-side file confirmation.
- A successful tool exit is insufficient without the expected output-file evidence.
- Timed D7100 exposures use fractional milliseconds (rated 0.125–30000 ms, excluding Bulb). On connection, query `list shutterspeed` for actual bounds; configure using an exact enumerated token and confirm with `get shutterspeed`. Unsupported values are errors, never rounded to another exposure.

## `turntable/`

- `nano.rs` implements the RTS9060 Nano serial protocol.
- Owns device identity/capability validation, heartbeat, HOME, motion, warning output, `CAPTURE_DONE`, native STOP, and recovery evidence after hardware faults.
- Protocol commands are external contracts and cannot be changed from assumptions.

Device modules do not call one another. `scan.rs` is the only place that orders turntable, X-ray, and camera actions into one projection transaction. A future Arduino-centered topology must be an explicit alternative backend rather than a second independent scan authority.
