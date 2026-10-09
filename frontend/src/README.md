# React frontend

`src/` is the presentation layer. It renders complete engine snapshots, collects operator input, and sends domain commands. It does not own production device truth or scan progress.

## Top-level files

- `main.tsx`: mounts React and imports the global stylesheet.
- `App.tsx`: composes the workstation panels, menus, controls, dialogs, logs, and image area.
- `CameraPanel.tsx` / `TurntablePanel.tsx`: permanent independent device connections
  and controls on the full-height left column. `camera-draft.ts` shares one
  session-associated parameter draft with `CameraSettingsDialog.tsx`; only explicit
  Apply writes and confirmed operation results replace readback.
- Live View controls have their own `CameraHealth.liveViewControls` session and
  two-property catalog: `cameraZoom` (D1A3), `focusMode` (D061). They do not share
  the photographic Apply/preset/profile draft. Typed `camera_live_view_read` and
  `camera_live_view_configure` require the current LV session; only these two
  advertised, freshly described UINT8 properties are accepted by the engine.
- `ScanPreparationDialog.tsx` / `TaskRecoveryDialog.tsx`: atomic new task/geometry
  preparation and frozen-data recovery. The camera storage root and confirmed
  native shutter are the sole scan path/exposure sources.
- `WorkstationMedia.tsx`: equipment, verified result, registered independent photo
  and memory-only Live View share the central content area. Safety controls remain
  available; late reads cannot reopen a closed display.
- `ReconstructionConsole.tsx`: compact algorithm controls; real progress/events
  enter the engine log aggregation instead of a second local event list.
- `UpdateDialog.tsx`: signed online/offline release state and explicit installation
  confirmation. Package transport/verification belongs to the desktop host; device
  maintenance authority belongs to the engine.
- `FailureBanner.tsx`: the one closable failure surface, portalled above every page
  and overlay. Closing it hides a notice only: cleanup stays unconfirmed, no engine
  lock is cleared and the safety Stop stays clickable. `TroubleshootingPane.tsx` is
  the Help → Troubleshooting page (symptom → engine code → exact next action) and
  consumes the same single advice table as the banner, so the two cannot drift.
- `DeviceConsoles.tsx`: auxiliary isolated-turntable bench confirmation dialog;
  it does not create a second camera draft or independent device owner.
  The 39px title bar, 209.375px aligned bottom panels and 12px gaps share the
  client-filling grid. Only overlays and the log/image console scroll.
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
  It also owns the single failure-advice table and the `deriveFailureReport`
  derivation (stable dedup keys; a cancelled operation is not a failure by itself).
- `native-labels.ts`: the only presentation mapping point for engine text. ct-engine
  still returns Chinese labels for 15 of the 34 catalog keys and for enum options and
  disabled reasons; this module renders them in English and degrades an unmapped
  Chinese message to one neutral sentence. Raw codes, raw messages, the diagnostics
  JSON and the exported log keep the engine's original text.
- `workstationAdapter.ts`: browser-only preview implementation.
- `rts9060/`: in-memory preview protocol, transport, device models, and workflow. It never accesses physical hardware and is not firmware evidence.

Production snapshots expose fixed-scan issues, complete capture ID/session/op/
size/hash registration and software-update maintenance reports. Prepare/preflight/
start/recovery compare fresh camera evidence without silently repairing shutter,
RAM/single settings or stopping Live View. Independent single capture may use
automatic photographic parameters; scanning still requires the fixed policy.

`platform/cameraCaptures.ts` accepts only an engine capture ID. The host resolves
the registration, performs bounded size/hash checks and returns embedded JPEG
bytes without writing a preview file. `platform/updater.ts` exposes restricted
check/status/cancel/native-import/confirm operations, never renderer URLs, keys
or arbitrary install paths.

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
View uses the controlled native frame endpoint. Missing or rejected new frames
retain the last decoded in-memory image with a STALE label; only fresh accepted
frames are labelled LIVE. Session retirement clears the image and rejects late
results. Closing the console requests Stop Live
View. Capture stays disabled until Live View is explicitly stopped. Unknown focus
position is displayed and never stored as a preset coordinate.

Each inactive-to-start Live View transition creates a fresh 32-hex nonce, separate
from the stable camera connection and capture registration session. Frame metadata,
pipe endpoint and `liveViewControls.sessionId` must agree; Stop and retirement revoke
both display and controls. Late reads/writes cannot target a new LV session. Ordinary
photographic Apply remains blocked during LV. The 34 photographic controls and two
LV controls remain separate; renderer display zoom is not native D1A3 zoom.

Read-only runtime evidence now includes lensSort D0E1, photo/movie selector D1A6
and activeSlot D1F2, preserving descriptor nativeWritable without exposing setters.
Unadvertised or failed fields stay UNKNOWN. LensSort=1 alone is not a motor capability:
LV AF needs fresh photo selector=0, D061 mode=0 and advertised operations; bounded
focus step permits modes0/4, AF-F2 blocks bounded motor and physical-MF3 blocks writes
and motor. The UI consumes the engine's capability/disabled reasons.

Preset comparison labels unconfirmed, unreadable, unsupported or failed values
UNKNOWN even if a retained value equals the preset. `WorkstationDialog` focuses on
mount and uses current callback refs; ordinary snapshot updates do not steal typing
focus. [Focused browser evidence](../.cache/tests/frontend/20261005-163257-dialog-focus-fix/result.json)
is UI_ONLY_NO_HARDWARE. These new source changes prepare a later rc3 candidate and
are only offline verified; the signed rc2 package and its acceptance remain separate.
Current fixed scans consume profile 6 readiness and fresh parameter readback. AF-region9205, Bulb/video/JPEG transactions and full Picture Control editing
remain outside the implemented subset.

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

`cameraLiveView.ts` keeps at most the current decoded frame and its previous image
until the React DOM presents the replacement. JPEG data URLs avoid browser Blob
storage and are released from the feed/DOM on replacement and stop. Live View has
no frame saving, file logging or frame cache on disk.
