# Console Redesign + RTS9060 Device Link Migration (v0.2.0-alpha)

Date: 2026-09-16. Scope: presentation-layer rebuild per `前端/重构交接说明.md` +
device-side adaptation ported from the proven Python host in `kernal/software/host/`.

## What changed

### Presentation layer (per handoff spec, acceptance list §8)

- Apple-style industrial console, Light default / Dark night mode, all colors from
  `src/tokens.css` (copied verbatim from `前端/tokens.css`); zero hardcoded hex in
  components. Theme flips `<html data-theme>` only, persisted in localStorage, no
  remount, no scan-state reset.
- Page grid `32 / 1 / 1fr / 1 / 220 / 1 / 30`; three columns `352 / 1fr / 376`
  (320/340 below 1440px), gap + padding 14.
- Menu bar: system menu left, theme segmented control + DEVELOPER PREVIEW badge +
  ENGINE ONLINE capsule right; no brand mark.
- Live Scene: theme × turntable-angle scene render (`3D-scene-{light|dark}[-144].png`,
  Ready = 0°, Scanning/Paused/Fault = 144°), three frosted status floats (X-RAY /
  CAMERA / SAMPLE), LIVE RENDER indicator, turntable-angle readout, safety bar, and
  the frosted-glass control dock (4 round keys, icons only, labels expand on hover).
- Four-state machine READY / SCANNING / PAUSED / FAULT on `<html data-state>`;
  five-color button semantics per spec (green start / amber pause / light-blue
  resume / deep-blue home / red E-stop; violet restore).
- Output console: 178px tab rail (Log Aggregation + X-ray / Turntable / Camera
  sub-logs + Image Preview), newest-first logs, 110px monospaced timestamp column,
  level colors PASS blue / INFO·OK green / WARN amber / ERR red / ACTION blue.
- Numeric rules: kV/µA always one decimal (30.2 kV / 101.0 µA); angle two decimals
  on the `0 / 72 / 144 / 216 / 288` sequence at 5 views; the angle readout, ANGLE
  stat, turntable POS line and log angles are driven by one value.

### Device link layer (ported from `kernal/software/host`)

New `src/engine/rts9060/` module; the command stream is byte-identical to the
serial line protocol of `rts9060_nano` (115200 8N1):

- `protocol.ts` — line codec and command builders. Command names unchanged:
  HEARTBEAT / PING / STATUS / SET_MICROSTEPS / REARM / HOME / MOVE_ABS / MOVE_REL /
  CAPTURE_DONE / STOP / GET_HALL / INFO / XRAY_WARNING. Mechanics: 96000 pulses/rev
  (1.8° × 60:1 × 8 µstep), 19200 pulses per 72° view.
- `transport.ts` — `NanoTransport` interface + `FirmwareTransport`, an in-process
  executor of the exact firmware state machine (boot banner, HOME phases,
  ACK → motion → READY_TO_CAPTURE handshake, CAPTURE_HOLD/IDLE release,
  STOP → STOPPED POSITION_UNKNOWN). The same interface is the future attach point
  for a physical CH340 serial bridge; nothing above it changes.
- `devices.ts` — Moxtek 12 W controller model (from `rts9060_xray.py`: set kV/µA,
  beam on/off, fail-closed latch, 4–70 kV / 0–1000 µA / 12 W limits) and the Nikon
  D7100 capture model (host-only storage, frame-%04d naming, per-frame validation
  before commit).
- `workflow.ts` — the scan orchestrator from `rts9060_workflow.py`: boot handshake
  → REARM → HOME → 8-check pre-inspection → per-view MOVE_ABS → settle → beam on →
  exposure window → capture → CAPTURE_DONE → commit. Pause lands at the commit
  boundary with the pulse counter retained; E-STOP cuts beam + motion in parallel
  and latches FAULT (home reference lost) until the operator releases it, re-homes
  and re-passes pre-inspection. Checkpoints persist per committed view and power
  the Restore key (resume semantics → light-blue Start).
- `workstationAdapter.ts` — bridges the workflow to the existing `EngineAdapter`
  contract, so `useEngine` and the data flow are unchanged; the console renders
  from `snapshot.workstation`.

### Untouched (acceptance #14)

- `src-tauri/` zero changes; `crates/ct-engine` zero changes; serial command names
  unchanged. The adapter factory now selects the RTS9060 link layer for every
  runtime, so the desktop shell presents the same console as the dev server.

## Verification

- `npm run typecheck`, `npm run build`, `npm run test:sites` — see docs/CURRENT-PROGRESS.md.
- Interactive chain exercised end to end: boot → pre-inspection 8/8 → start →
  per-view MOVE_ABS/exposure/capture → pause → resume → E-STOP → release →
  re-home → re-inspection → restore.

## Camera / source naming rules

D7100 everywhere; X-ray source link is USB; no legacy model names or bus labels
in UI copy or logs.
