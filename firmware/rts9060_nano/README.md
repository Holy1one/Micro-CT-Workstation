# RTS9060 Nano firmware candidate

This directory contains a recovered **candidate**, not hardware-certified firmware.
The user approved importing the source from
`.cache/tests/turntable/20260929-firmware-recovery/rts9060_nano/rts9060_nano.ino`.
The original imported `.ino` had 32,429 bytes and SHA-256
`470133efbb0f2192c725a0aef223cf6256030ce1f9c0dc541bd3cea4efa8e562`.
The current 2.2.0 / 20261010 candidate adds the unified ordinary-move settle
contract: `MOVE_ABS`/`MOVE_REL` wait `MOVE_SETTLE_MS = 150 ms` measured from the
**last STEP falling edge** before entering `CAPTURE_HOLD` and emitting
`READY_TO_CAPTURE`; a zero-displacement MOVE starts the same 150 ms when the
command is accepted. The wait is independent of angle and projection count and no
longer includes the extra pulse period that `finishPhase` previously added. HOME
phases, the acceleration/cruise ramp, the 20 us pulse width, direction setup,
heartbeat, the STOP path and the same-ID `CAPTURE_HOLD` release are unchanged. The
2.1.1 / 20261009 candidate's `RESTORE_REFERENCE <id> <expected-pos>` capability is
retained. The current source is 34,782 bytes with SHA-256
`811bae5c1d6e08b318c81ac8d077908df127e00d1e95bf185cf090b5ad1069ec`;
`build-config.json` pins that identity, keeps the original source identity and
records the previous 2.1.1 candidate hash. Historical comments
in the candidate are retained as source provenance; they do not establish current
wiring or authorize reading any archived project.

## Offline build and harness

```powershell
node tools/build/build-turntable-firmware.mjs
node tests/firmware/test-turntable-firmware.mjs
cargo test -p ct-engine devices::turntable
```

The AVR build directly follows the installed Arduino AVR core 1.8.8 recipes with
AVR GCC `7.3.0-atmel3.6.1-arduino7`, Nano ATmega328P, 16 MHz and the
`eightanaloginputs` variant. `ARDUINO15` may identify the installed package root.
No previously generated HEX is used. The script builds the sketch and core from
source, links an ELF and generates HEX/BIN. It records every command, compiler
version, source identity, lengths, hashes and memory use in a fresh
`.cache/tests/turntable/<run-id>/` directory. There is no upload or serial command.

The first rebuilt candidate occupied 13,416 bytes of flash and 1,382 bytes of RAM
under the pinned configuration; the 2.2.0 candidate rebuilds to 14,160 bytes of
flash and 1,476 bytes of RAM. These are build observations, not proof of the
currently installed flash. Each run's `result.json` is the artifact evidence.

`host_harness.cpp` directly includes the current `.ino` with its existing
`HOST_HARNESS` branch. `host_arduino_stubs.h` supplies deterministic clock, pins
and ASCII I/O. It executes the actual parser, pulse count and state transitions,
including prescribed negative moves, ABS normalization, REL full circles,
SETTLING/CAPTURE_HOLD, matching capture release, HOME bounded retreat and failure
bounds, STOP reference invalidation, and explicit warning OFF. For the 150 ms
contract it also measures the last falling edge to `READY_TO_CAPTURE` interval for
0.5/1/2/5 degree moves (required inside 150-152 ms, which rejects the previous
extra-tail-period behavior), the zero-displacement acceptance anchor, that STOP
before 150 ms never exposes a `READY_TO_CAPTURE`, that a hold emits READY exactly
once, and that `micros`/`millis` wrap during the wait cannot shorten it. The host
compiler is MSVC on Windows or `CXX`/`c++` on other platforms. It models the
branch's software falling edge and does not model AVR interrupt scheduling.

## Preserved behavior and hardware gate

- Ordinary MOVE follows `HOME_DIRECTION=-1`; only HOME internally retreats in the
  opposite direction (release bound 20 degrees, clearance 0.25 degrees, final
  same-direction slow approach bound 12 degrees).
- Default microsteps are 8 and pulses per revolution are 96,000. The application
  rejects a different pulse scale and does not expose speed or microstep controls.
- MOVE_ABS normalizes one-circle phase and cannot automatically cross zero from
  a previous negative turn. MOVE_REL retains signed displacement. The Rust phase
  planner sends bounded negative relative moves from confirmed unwrapped pulses.
- A move ends in CAPTURE_HOLD, released only by the same move ID's CAPTURE_DONE.
  STOP invalidates reference/homed/rearmed. Warning remains set until explicit OFF.
- The 150 ms settle is a firmware reservation, not proof that the load has
  physically settled. Load stability, image vibration, repeatability and the real
  last-pulse-to-exposure timing still require authorized hardware acceptance; the
  PC adds no fixed or angle-derived sleep and advances only on matched READY
  evidence.
- RESTORE_REFERENCE restores the previous calibrated reference only after the PC
  verifies an unchanged-position operator confirmation and frozen checkpoint.
  The firmware requires idle/rearmed/fresh heartbeat, warning OFF and an exact
  expected/current/target counter match. It does not change counters or emit STEP;
  wrong positions, active motion, FAULT and a pressed STOP remain rejected. This
  is reference recovery, not a newly executed HOME. It requires separate upload
  authorization and hardware acceptance before use on a production controller.
- The potential shared-flag/ISR ordering race in `stepPulseStart` remains
  unmodified and unproven; host tests cannot validate this timing.

**BLOCKED without separately authorized hardware acceptance:** byte comparison
against the device's application flash, physical direction, load-side angle and
HOME accuracy, ISR pulse timing, and full linked acquisition. VERSION/BUILD text
cannot substitute for flash comparison. This task neither reads flash nor uploads
firmware, opens a real serial port, or operates a turntable.
