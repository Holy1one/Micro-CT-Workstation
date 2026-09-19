# Prototype Instructions

Run the local server yourself and open the preview in the browser available to this environment. Do not give the user server-start instructions when you can run it.

Before making substantial visual changes, use the Product Design plugin's `get-context` skill when the visual source is unclear or no longer matches the current goal. When the user gives durable prototype-specific design feedback, preferences, or decisions, record them in `AGENTS.md`.

When implementing from a selected generated mock, treat that image as the source of truth for layout, component anatomy, density, spacing, color, typography, visible content, and hierarchy.

Build app UI in `src/`. Keep `.openai/hosting.json`, `worker/index.js`, `scripts/prepare-sites-build.mjs`, and `tests/sites-worker.test.mjs` intact so the same local prototype can be handed to Sites. Before a Sites handoff, run `npm run build` and `npm run test:sites`; the build must leave `dist/client/index.html`, `dist/server/index.js`, and `dist/.openai/hosting.json`.

The desktop UI is a fixed, no-page-scroll 16:9 industrial workstation. Keep device status and scan parameters on the left, the equipment scene and hover-expanding four-button control dock in the center, X-ray controls and operation status on the right, and aggregated logs plus current-round image preview in the fixed bottom rail. `前端/首页.png` and `前端/按钮说明.png` are the behavioral/layout source of truth; generated mockups are visual references only.

## Release & push（每次发版必走）

版本发布必须同时推送分支与标签，标签不会随分支自动上传。

```bash
# 1. 同步版本号：Cargo.toml（workspace.package）、src-tauri/tauri.conf.json、package.json
# 2. 门禁
cargo test --workspace
npm run typecheck
npm run test:sites
# 3. 提交 + 标签 + 推送
git add -A
git commit -m "vX.Y.Z: <summary>"
git tag -a vX.Y.Z -m "vX.Y.Z: <summary>"
git push origin main
git push origin vX.Y.Z
```

- 标签名 = `v` + `package.json` 的版本号，只打在发布提交上。
- 每次 `git push origin main` 之后补一次标签推送；等价简写 `git push --follow-tags origin main`。
- 免安装包随发布提交更新：先 `npx tauri build --no-bundle`，再把 `target/release/ct-workstation.exe` 覆盖到 `portable-release/micro-ct-workstation-portable.exe`。
- 本机走本地代理时，两条 push 前加 `git -c http.proxy=http://127.0.0.1:7897`（端口以当前代理为准）。

Durable workstation UI rules:

- The console follows `前端/重构交接说明.md` + `前端/tokens.css` (design baseline 1600×1000): page grid `32/1/1fr/1/220/1/30`, columns `352/1fr/376`, Apple-style light theme by default with a dark night mode on `<html data-theme>`, run state on `<html data-state>` (ready/scanning/paused/fault). All colors come from tokens.css variables — never hardcode hex in components.
- Keep the native Windows title bar; the content begins with the compact `File / Edit / Tools / Help` menu row; no brand mark in the menu bar (theme segmented control + DEVELOPER PREVIEW + ENGINE ONLINE on the right).
- The visible application language is English only, including phases, device details, errors and log messages. Camera is always D7100; the X-ray source link is always USB; kV/µA render with one decimal, angles with two.
- The device/task owner is the RTS9060 link layer in `src/engine/rts9060/` (ported from `kernal/software/host`), bridged by `src/engine/workstationAdapter.ts` to the unchanged `EngineAdapter` contract. Serial command names are contractual: HEARTBEAT/PING/STATUS/SET_MICROSTEPS/REARM/HOME/MOVE_ABS/MOVE_REL/CAPTURE_DONE/STOP/GET_HALL. Mechanics: 96000 pulses/rev; N views per rev → 360/N degrees and 96000/N pulses per view. `src-tauri/` and `crates/ct-engine` stay untouched.
- Safety semantics are highest priority: outside SCANNING the scene safety bar, status bar safety text and the Xray switch must all show the closed state at the same time; FAULT forces all three into the red latched state simultaneously. E-STOP release requires re-home + re-inspection before Start.
- Logs: newest first, 110px monospaced timestamp column, level colors PASS blue / INFO·OK green / WARN amber / ERR red / ACTION blue; every scan milestone (MOVE_ABS, exposure window, capture, CAPTURE_DONE, pause/resume/home/E-stop, pre-inspection) must land in the log.
- Passing layout QA requires both page and every main panel to satisfy `scrollWidth <= clientWidth` and `scrollHeight <= clientHeight` at 1600 × 900 and the 125% equivalent 1280 × 720 viewport. Hiding overflow is not evidence that content fits.
