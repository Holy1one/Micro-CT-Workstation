# Prototype Instructions

Run the local server yourself and open the preview in the browser available to this environment. Do not give the user server-start instructions when you can run it.

Before making substantial visual changes, use the Product Design plugin's `get-context` skill when the visual source is unclear or no longer matches the current goal. When the user gives durable prototype-specific design feedback, preferences, or decisions, record them in `AGENTS.md`.

When implementing from a selected generated mock, treat that image as the source of truth for layout, component anatomy, density, spacing, color, typography, visible content, and hierarchy.

Build app UI in `src/`. Keep `.openai/hosting.json`, `worker/index.js`, `scripts/prepare-sites-build.mjs`, and `tests/sites-worker.test.mjs` intact so the same local prototype can be handed to Sites. Before a Sites handoff, run `npm run build` and `npm run test:sites`; the build must leave `dist/client/index.html`, `dist/server/index.js`, and `dist/.openai/hosting.json`.

The desktop UI is a fixed, no-page-scroll 16:9 industrial workstation. Keep device status and scan parameters on the left, the equipment scene and hover-expanding four-button control dock in the center, X-ray controls and operation status on the right, and aggregated logs plus current-round image preview in the fixed bottom rail. `前端/首页.png` and `前端/按钮说明.png` are the behavioral/layout source of truth; generated mockups are visual references only.

Durable workstation UI rules:

- Keep the native Windows title bar; do not draw a second application title/header below it. The content begins with the compact `File / Edit / Tools / Help` menu row.
- The visible application language is English only, including phases, device details, errors and log messages.
- The center scene must not crowd out the side columns. At 1600 × 900 the side columns are approximately 24% each; use readable 11–13 px workstation text rather than miniature dashboard typography.
- Each device row has its own connection indicator and refresh control. Do not replace these with one undifferentiated global status row.
- The fixed bottom rail has five selectors: aggregated, X-ray, turntable and camera logs, then Image Preview. Image Preview is hidden until its selector is active and replaces the log content; it is never a permanent right-bottom panel.
- Logs use terminal-like consecutive lines but retain the light industrial palette. Timestamps include the full local date and time to seconds.
- The right-bottom panel contains operation progress, process state and the current scan configuration summary—not Image Preview.
- Passing layout QA requires both page and every main panel to satisfy `scrollWidth <= clientWidth` and `scrollHeight <= clientHeight` at 1600 × 900 and the 125% equivalent 1280 × 720 viewport. Hiding overflow is not evidence that content fits.
