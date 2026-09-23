# React frontend

`src/` is the presentation layer. It renders complete engine snapshots, collects operator input, and sends domain commands. It does not own production device truth or scan progress.

## Top-level files

- `main.tsx`: mounts React and imports the global stylesheet.
- `App.tsx`: composes the workstation panels, menus, controls, dialogs, logs, and image area.
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
- `workstationAdapter.ts`: browser-only preview implementation.
- `rts9060/`: in-memory preview protocol, transport, device models, and workflow. It never accesses physical hardware and is not firmware evidence.

## `scene/`

Read-only Three.js visualization. `scene-config.ts` owns display geometry, `EquipmentScene.tsx` builds the model, and `LiveSceneCanvas.tsx` owns camera interaction. WebGL fallback and theme conversion remain display-only.

`angle-feedback.ts` linearly follows confirmed angle samples over 300 ms without extrapolation. The turntable platter, marker and sample share one animated transform; the camera and source stay fixed. Unknown feedback freezes the displayed pose; paused/stopped/completed snapshots settle at the confirmed angle. Long gaps and task changes re-anchor instead of replaying unobserved motion. Renderer diagnostics expose the actual mesh angle as a canvas data attribute for offline QA; they never feed device state back into the engine.

## `platform/`

Small frontend wrappers around Tauri paths, pickers, directory reveal, and log export. Browser preview receives safe no-hardware behavior.
