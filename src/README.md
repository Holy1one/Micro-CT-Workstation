# React frontend

`src/` is the presentation layer. It renders complete engine snapshots, collects operator input, and sends domain commands. It does not own production device truth or scan progress.

## Top-level files

- `main.tsx`: mounts React and imports the global stylesheet.
- `App.tsx`: composes the workstation panels, menus, controls, dialogs, logs, and image area.
- `DeviceConsoles.tsx`: Devices → Camera/Turntable overlays; drafts, confirmed readback,
  camera-only presets, independent capture, Live View and prescribed-direction motion.
  Only overlay bodies scroll; the 39px branded title bar and 306px bottom panels retain their layout.
- `tokens.css`: semantic color and state tokens for light/dark and safety states.
- `styles.css`: workstation layout and component styling.
- `canvas-layout.ts`: pure client-filling grid dimensions and uniform control scale; never letterboxes.
- `scan-input.ts`: validates projection/exposure drafts and converts minute input to the engine's seconds.
- `menuActions.ts`: menu vocabulary and command availability.

## `engine/`

- `types.ts`: shared `EngineCommand`, `EngineSnapshot`, and workstation view-model contract.
- `adapter.ts`: selects Tauri production mode or browser preview once at startup.
- `tauriAdapter.ts`: production `engine_snapshot` / `engine_command` calls.
- `useEngine.ts`: React hook that stores the latest complete snapshot.
- `deviceControls.ts`: pure draft/capability gates, exact native shutter fractions,
  engine session/sequence ordering and a command scheduler that gives Stop priority.
- `workstationAdapter.ts`: browser-only preview implementation.
- `rts9060/`: in-memory preview protocol, transport, device models, and workflow. It never accesses physical hardware and is not firmware evidence.

JSONL v2 snapshots carry engine session and snapshot sequence, operation IDs and
stages, native device identities/capabilities/readback, and persistent unfinished-task
locks. Polling continues while commands and device operations are pending. Accepted
commands acknowledge enqueue; only engine operation/readback evidence reports
completion. Stop bypasses ordinary queued calls and invalidates unstarted UI requests.
Tauri errors never select the preview backend.

Unfinished tasks retain parameter/focus/motion locks while allowing independent
connection repair and property reads. Their overlay includes an explicit controlled
Restore preflight confirmation for frozen task settings and USB Auto Shut Down
release. Recovery HOME remains the original scan control; this confirmation neither
unlocks ordinary controls nor abandons the task.

The camera console requires explicit selection from discovery. Presets only load
drafts; Apply writes native integer/string values and verifies readback. Shutter and
scan Exposure share the exact native fraction target. Independent capture uses its
own directory and operation identity and never modifies scan frames or paths. Live
View uses the controlled native frame endpoint; invalid, old, unrelated, stale or
undecodable frames lose their live display. Closing the console requests Stop Live
View. Capture stays disabled until Live View is explicitly stopped. Unknown focus
position is displayed and never stored as a preset coordinate.

Turntable UI angles are positive along the prescribed direction and the preview
protocol uses negative relative displacement from confirmed unwrapped pulses. A
matching capture ticket must release to IDLE; scans retain the last angle and do
not return to zero after completion. Bench isolation is an explicit confirmation
and leaves the unmonitored source UNKNOWN with beam commands prohibited. The engine
remains responsible for OFF-before-HOME and all safety decisions.

## `scene/`

Read-only Three.js visualization. `scene-config.ts` owns display geometry, `EquipmentScene.tsx` builds the model, and `LiveSceneCanvas.tsx` owns camera interaction. WebGL fallback and theme conversion remain display-only.

`ReconstructionPreview.tsx` displays three linked slices and a rotatable volume from the engine-registered, hash-verified preview. It never sends device commands.

Reconstruction starts without replacing the equipment view. The UI preloads and validates completed preview data before automatically showing it once; independent Equipment/Result controls switch the loaded view without starting computation or requiring a live device connection.

`angle-feedback.ts` linearly follows confirmed angle samples over 300 ms without extrapolation. The turntable platter, marker and sample share one animated transform; the camera and source stay fixed. Unknown feedback freezes the displayed pose; paused/stopped/completed snapshots settle at the confirmed angle. Long gaps and task changes re-anchor instead of replaying unobserved motion. Renderer diagnostics expose the actual mesh angle as a canvas data attribute for offline QA; they never feed device state back into the engine.

## `platform/`

Small frontend wrappers around Tauri paths, pickers, directory reveal, and log export. Browser preview receives safe no-hardware behavior.

`cameraControls.ts` forwards only afterSeq to the engine-owned Live View endpoint;
no arbitrary pipe/path is exposed. It also wraps native camera capture-directory
selection and preset import/export. All candidate controls state hardware acceptance
is pending; browser-only data is explicitly marked NO HARDWARE.
