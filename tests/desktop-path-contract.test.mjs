/**
 * Cross-layer contract tests for desktop path and dialog commands.
 * The tests verify that frontend invoke names and Rust handlers remain aligned
 * without opening native dialogs or touching user files.
 */

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);
const read = async (path) => readFile(new URL(path, root), "utf8");

test("desktop path resolver anchors Camera Roll to the configured Pictures folder", async () => {
  const source = await read("src-tauri/src/main.rs");
  assert.match(source, /fn resolve_default_image_directory/);
  assert.match(source, /FOLDERID_CameraRoll/);
  assert.match(source, /FOLDERID_Pictures/);
  assert.match(source, /SHGetKnownFolderPath\(folder_id, 0,/);
  assert.match(source, /path\.is_dir\(\)/);
  assert.match(source, /fn is_descendant_path/);
  assert.match(source, /fn select_image_directory/);
  assert.match(source, /existing_known_folder\(&FOLDERID_Pictures\)/);
  assert.match(source, /select_image_directory\([\s\S]*existing_known_folder\(&FOLDERID_Pictures\),[\s\S]*existing_known_folder\(&FOLDERID_CameraRoll\),[\s\S]*\)\?/);
  assert.ok(source.includes('r"E:\\Main\\Pictures"'));
  assert.ok(source.includes('r"C:\\Users\\x\\Pictures\\Camera Roll"'));
  assert.match(source, /image_directory_selection_rejects_legacy_camera_roll_on_another_drive/);
  assert.match(source, /image_directory_selection_prefers_camera_roll_below_current_pictures/);
  assert.match(source, /image_directory_selection_uses_pictures_without_camera_roll/);
  assert.match(source, /image_directory_selection_uses_camera_roll_when_pictures_is_missing/);
  assert.match(source, /image_directory_selection_reports_both_lookup_failures/);
  assert.doesNotMatch(source, /USERPROFILE|create_dir|KF_FLAG_CREATE/);
});

test("directory picker has dialog-only permission and preview remains fail-closed", async () => {
  const [capability, desktopPaths, workflow, adapter] = await Promise.all([
    read("src-tauri/capabilities/default.json"),
    read("src/platform/desktopPaths.ts"),
    read("src/engine/rts9060/workflow.ts"),
    read("src/engine/workstationAdapter.ts"),
  ]);
  assert.match(capability, /dialog:allow-open/);
  assert.doesNotMatch(capability, /fs:|shell:/);
  assert.match(desktopPaths, /directory: true/);
  assert.match(desktopPaths, /multiple: false/);
  assert.match(workflow, /savePath: ""/);
  assert.match(workflow, /taskId: ""/);
  assert.match(workflow, /projectionCount: 0/);
  assert.match(workflow, /exposureMs: 0/);
  assert.match(workflow, /maxXraySec: 0/);
  assert.match(workflow, /validateCompleteParams\(this\.params\)/);
  assert.match(workflow, /Select a non-empty Save Path before pre-inspection/);
  assert.match(adapter, /savePath: wf\.params\.savePath/);
  assert.doesNotMatch(`${workflow}\n${adapter}`, /D:\\\\CT/);
});

test("scan setup patches stay partial and are validated atomically by the sidecar", async () => {
  const [types, app, engine] = await Promise.all([
    read("src/engine/types.ts"),
    read("src/App.tsx"),
    read("crates/ct-engine/src/lib.rs"),
  ]);
  assert.match(types, /type ScanSetupUpdate = Partial</);
  assert.match(app, /setup: \{ \[field\]: parsed \}/);
  assert.match(engine, /save_path: Option<String>/);
  assert.match(engine, /projection_count: Option<u32>/);
  assert.match(engine, /setup\.apply\(&self\.parameters, self\.max_xray_sec\)/);
  assert.match(engine, /task_id: String::new\(\)/);
  assert.match(engine, /projection_count: 0/);
  assert.match(engine, /max_xray_sec: 0/);
  assert.match(engine, /!\(1\.\.=600\)\.contains\(&self\.max_xray_sec\)/);
  assert.match(engine, /deny_unknown_fields/);
});

// "maximized": true in tauri.conf.json creates the window maximized, which skips
// the restored state: Windows then has no restore bounds and the shell stays at
// maximized size with IsZoomed false. Startup therefore applies the readable
// minimum while restored and maximizes from setup instead.
test("window keeps browser arguments, maximizes from restored startup, and applies dynamic minimum size", async () => {
  const [config, source] = await Promise.all([
    read("src-tauri/tauri.conf.json"),
    read("src-tauri/src/main.rs"),
  ]);
  assert.match(config, /msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-gpu-sandbox/);
  assert.match(source, /current_monitor\(\)/);
  assert.match(source, /work_area\(\)/);
  assert.match(source, /set_min_size/);
  assert.match(source, /window\.outer_size\(\)/);
  assert.match(source, /window\.inner_size\(\)/);
  assert.match(source, /const MIN_READABLE_SCALE: f64 = 0\.9/);
  assert.match(source, /fn calculate_min_inner_size/);
  assert.match(source, /DESIGN_WIDTH \* MIN_READABLE_SCALE/);
  assert.match(source, /DESIGN_HEIGHT \* MIN_READABLE_SCALE/);
  assert.match(source, /target_width\.min\(available_width\)\.max\(1\.0\)/);
  assert.match(source, /target_height\.min\(available_height\)\.max\(1\.0\)/);
  assert.doesNotMatch(config, /"minWidth"|"minHeight"/);
  assert.doesNotMatch(config, /"maximized": true/);
  assert.match(source, /window\.is_maximized\(\)\.unwrap_or\(false\)/);
  assert.match(source, /apply_dynamic_min_size\(window\)\?;\s*\n\s*window\.maximize\(\)/);
  assert.match(source, /WindowEvent::Moved/);
  assert.match(source, /WindowEvent::Resized/);
  assert.match(source, /WindowEvent::ScaleFactorChanged/);
  assert.match(source, /schedule_dynamic_min_size/);
  assert.match(source, /run_on_main_thread/);
  assert.match(source, /bootstrap_engine\(app\.app_handle\(\)\.clone\(\)\)/);
});

test("fixed design canvas scales as one unit and side columns never scroll", async () => {
  const [app, styles, tokens] = await Promise.all([
    read("src/App.tsx"),
    read("src/styles.css"),
    read("src/tokens.css"),
  ]);
  assert.match(styles, /\.design-canvas[\s\S]*width: 1920px;[\s\S]*height: 1080px;/);
  assert.match(app, /computeCanvasLayout\(window\.innerWidth, window\.innerHeight\)/);
  assert.match(app, /zoom: canvasLayout\.zoom/);
  assert.match(app, /canvasLayout\.designHeight/);
  assert.match(styles, /\.menu-dropdown \{[\s\S]*position: absolute;/);
  assert.doesNotMatch(app, /Math\.min\(window\.innerWidth \/ 1600/);
  assert.doesNotMatch(styles, /\.design-canvas\s*\{[^}]*transform:/);
  assert.match(styles, /grid-template-rows: 34px 1px minmax\(0, 1fr\) 1px 206px 1px 30px/);
  assert.match(styles, /grid-template-columns: 420px minmax\(0, 1fr\) 450px/);
  assert.match(styles, /grid-template-rows: 34px 1px minmax\(0, 1fr\) 1px 206px 1px 30px/);
  assert.match(styles, /\.viewport-shell \{[^}]*var\(--viewportGutter\)/);
  assert.match(styles, /\.design-canvas \{[\s\S]*var\(--canvasBorder\)/);
  assert.ok((tokens.match(/--viewportGutter:/g) ?? []).length >= 2);
  assert.ok((tokens.match(/--canvasBorder:/g) ?? []).length >= 2);
  assert.match(styles, /\.col \{[^}]*overflow: hidden;/);
  assert.doesNotMatch(styles, /\.col \{[^}]*overflow-y: auto/);
  assert.match(styles, /\.console__tabs \{[^}]*overflow: hidden;/);
  assert.match(styles, /\.log-lines\s*\{[^}]*overflow-y: auto;/);
  assert.match(styles, /\.image-strip\s*\{[^}]*overflow-x: auto;/);
});
