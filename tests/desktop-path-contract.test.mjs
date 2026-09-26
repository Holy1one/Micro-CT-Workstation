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

/* ---------- shared layout facts read out of src/styles.css ---------- */

/** The panel class the Operation Status section actually carries in App.tsx
 *  (`<section className="panel operation-panel">`). There is no `.op-status`
 *  rule, so any selector using it silently matches nothing. */
export const OPERATION_PANEL_CLASS = "operation-panel";

/** Bottom console row: 264 design px * 1.2 = 316.8, rounded to 317. The same
 *  number is the Operation Status panel's own height; the two must never be
 *  allowed to drift apart, so a change on either side has to update both. */
export const BOTTOM_LOG_ROW_PX = 317;

/** The right column's panel separator is the shared 1px hairline, not the
 *  12px panel rhythm: a 12px gap pushes the Operation Status panel down and
 *  pulls the X-ray panel above the line the other two columns end on. */
export const RIGHT_COLUMN_SPACER_PX = 1;

/** The first client-area row's design height, in design px. `.design-canvas`
 *  spends row 1 on the custom title bar (`header.menu-bar`: compact menus,
 *  brand, window controls) and the last row on the 30px status bar. This is a
 *  specified design value, documented in src/AGENTS.md; the stylesheet is
 *  graded against the spec, never the other way round. */
export const TOP_CHROME_ROW_PX = 39;

/** The design canvas grid: top chrome, hairline, content, hairline, status bar. */
export const CHROME_ROW_TRACKS = /grid-template-rows: (\d+)px 1px minmax\(0, 1fr\) 1px (\d+)px/;

const escapeForRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * Every declaration body of one concrete class selector. Derived from the
 * class name actually rendered by App.tsx, so renaming the panel in both places
 * keeps working while a selector that matches nothing fails instead of skipping.
 */
export const classRuleBodies = (css, className) => [
  ...css.matchAll(new RegExp(`\\.${escapeForRegExp(className)}\\s*\\{([^}]*)\\}`, "g")),
].map((match) => match[1]);

/**
 * The bottom console row height declared by `.workspace-body`, or null when the
 * rule is gone. Callers must treat null as a failure: the row is the anchor of
 * the shared bottom edge, not something to fall back to a range for.
 */
export const bottomLogRowPx = (css) => {
  const match = css.match(
    /\.workspace-body\s*\{[^}]*grid-template-rows:\s*minmax\(0,\s*1fr\)\s+1px\s+(\d+)px/,
  );
  return match ? Number(match[1]) : null;
};

/** The `min-height` values the Operation Status panel declares, in source order. */
export const operationPanelMinHeightsPx = (css, className = OPERATION_PANEL_CLASS) =>
  classRuleBodies(css, className)
    .flatMap((body) => [...body.matchAll(/min-height:\s*(\d+)px/g)])
    .map((match) => Number(match[1]));

/** The `height` values the Operation Status panel declares, in source order. */
export const operationPanelHeightsPx = (css, className = OPERATION_PANEL_CLASS) =>
  classRuleBodies(css, className)
    .flatMap((body) => [...body.matchAll(/(?<!min-)height:\s*(\d+)px/g)])
    .map((match) => Number(match[1]));

/** The separator the right status column puts between its two panels, or null. */
export const rightColumnSpacerPx = (css) => {
  const bodies = classRuleBodies(css, "col--right");
  for (const body of bodies) {
    const match = body.match(/(?:^|;)\s*(?:row-)?gap\s*:\s*(\d+)px/);
    if (match) return Number(match[1]);
  }
  return null;
};

/**
 * Turns the stylesheet text into the number the bottom-log-row assertion is
 * graded against: the Operation Status panel's published height. There is no
 * fallback path on purpose - a stylesheet where the panel no longer publishes
 * its height (wrong class, deleted rule, non-px value) throws here, and the
 * test fails rather than quietly passing a looser check.
 */
export function readOperationPanelHeight(css, className = OPERATION_PANEL_CLASS) {
  const bodies = classRuleBodies(css, className);
  if (bodies.length === 0) {
    throw new Error(`stylesheet has no .${className} rule to read the panel height from`);
  }
  const heights = operationPanelHeightsPx(css, className);
  const minHeights = operationPanelMinHeightsPx(css, className);
  if (heights.length === 0 && minHeights.length === 0) {
    throw new Error(`.${className} publishes no height: / min-height: in px`);
  }
  return [...heights, ...minHeights][0];
}

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
  assert.match(workflow, /maxXraySec: 600/);
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
  assert.match(engine, /max_xray_sec: 600/);
  assert.match(engine, /exposure_ms: Option<f64>/);
  assert.match(engine, /1\.\.=3600/);
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
  assert.match(source, /requires_maximized_window/);
  assert.match(source, /SetWindowSubclass/);
  assert.match(source, /command == SC_MOVE \|\| command == SC_SIZE/);
  assert.match(source, /command == SC_RESTORE && IsIconic\(hwnd\) == 0/);
  assert.doesNotMatch(source, /set_resizable\(false\)|set_resizable\(!maximize_only\)/);
  assert.match(source, /set_maximizable\(!maximize_only\)/);
  assert.match(source, /window\.is_minimized\(\)/);
  assert.match(source, /window\.maximize\(\)/);
  assert.match(source, /window\.show\(\)/);
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
  // The first row is the single top chrome row (custom title bar: brand +
  // menus + window controls) and the last is the status bar. The four-track
  // structure and both pinned heights must not change.
  const chromeRows = styles.match(CHROME_ROW_TRACKS);
  assert.ok(chromeRows, "the design canvas must declare top-row / content / status-bar tracks");
  const topRowPx = Number(chromeRows[1]);
  const statusRowPx = Number(chromeRows[2]);
  // Exactly 39, with the spec authority named so this is not self-referential:
  // the height is a documented design value, not merely whatever the
  // stylesheet happens to say. The previous check accepted anything in
  // [26, 64] - a range 2.5x the width of the value, which silently tolerated
  // the 13px drift between the 26px spec and the shipped 39px track while the
  // status bar beside it was pinned to exactly 30. All 26..64 values, the
  // stale 26 included, now fail.
  assert.equal(
    topRowPx,
    TOP_CHROME_ROW_PX,
    `the custom title-bar row must stay exactly ${TOP_CHROME_ROW_PX} design px, ` +
      `the value specified in src/AGENTS.md (client-area first row design height); got ${topRowPx}px`,
  );
  // The range 26 <= topRowPx <= 64 was deliberately deleted here. It is not a
  // regression gate: it accepted 26 through 64, so the drift it was meant to
  // catch passed through it untouched. Do not reintroduce a tolerance band.
  assert.equal(statusRowPx, 30, "the status bar row must stay 30px");
  // Cross-check against the written spec so the pinned literal is anchored to a
  // document rather than to this test. This is deliberately the inverse of the
  // defect that created it: the range assertion let src/AGENTS.md keep saying
  // 26px while src/styles.css shipped 39px. Whichever of the two drifts, one of
  // the two assertions now fails.
  const spec = await read("src/AGENTS.md");
  assert.match(
    spec,
    new RegExp(`设计高度\\s*${TOP_CHROME_ROW_PX}px`),
    `src/AGENTS.md must document the first client-area row's design height as ${TOP_CHROME_ROW_PX}px`,
  );
  assert.doesNotMatch(
    spec,
    /设计高度\s*26px/,
    "src/AGENTS.md must not keep the stale 26px first-row design height",
  );
  assert.match(styles, /grid-template-columns: 420px minmax\(0, 1fr\) 450px/);
  // The bottom console row must match the Operation Status panel's height, so
  // the two bottom areas line up instead of the log column looking cramped.
  // Both numbers are read out of the stylesheet and must be EQUAL: the row
  // height and the panel's own published height cannot drift apart. An earlier
  // version of this check read a `.op-status` rule that does not exist, so the
  // capture was always null and the equality was never asserted at all; the
  // panel is `.operation-panel` in both App.tsx and styles.css. There is no
  // fallback branch any more - if either number cannot be read, or the two
  // disagree, this test fails.
  const logRowPx = bottomLogRowPx(styles);
  assert.ok(
    logRowPx !== null,
    "src/styles.css must declare the bottom console row in .workspace-body's grid-template-rows",
  );
  const operationPanelHeightPx = readOperationPanelHeight(styles);
  assert.equal(
    logRowPx,
    operationPanelHeightPx,
    "the bottom log row must equal the Operation Status panel height",
  );
  assert.equal(
    logRowPx,
    BOTTOM_LOG_ROW_PX,
    `the bottom log row stays ${BOTTOM_LOG_ROW_PX} design px (264 * 1.2, rounded)`,
  );
  assert.equal(
    operationPanelHeightPx,
    BOTTOM_LOG_ROW_PX,
    `the Operation Status panel stays ${BOTTOM_LOG_ROW_PX} design px so it shares the log row's edges`,
  );
  assert.deepEqual(
    operationPanelMinHeightsPx(styles),
    [BOTTOM_LOG_ROW_PX],
    "every Operation Status min-height must equal the bottom log row",
  );
  assert.match(app, /className="panel operation-panel"/);
  // The right status column's own separator has to be the shared 1px hairline,
  // not the 12px panel rhythm: 12px would push the Operation Status panel 12px
  // down and pull the X-ray panel above the line the other columns end on.
  assert.equal(
    rightColumnSpacerPx(styles),
    RIGHT_COLUMN_SPACER_PX,
    "the right column's panel separator must be the 1px hairline, not the 12px panel gap",
  );
  // The bottom console stays full width over the left + centre columns only,
  // and the right status column runs through to the bottom of the workspace.
  assert.match(styles, /\.workspace-body > \.console \{[\s\S]*grid-column: 1 \/ 3;/);
  assert.match(styles, /\.col--right \{[\s\S]*grid-row: 1 \/ 4;/);
  assert.match(styles, /\.col--right \{[\s\S]*grid-template-rows: minmax\(0, 1fr\) auto;/);
  assert.match(styles, /\.viewport-shell \{[^}]*var\(--appBg\)/);
  assert.match(app, /width: `\$\{canvasLayout\.designWidth\}px`/);
  assert.ok((tokens.match(/--viewportGutter:/g) ?? []).length >= 2);
  assert.ok((tokens.match(/--canvasBorder:/g) ?? []).length >= 2);
  assert.match(styles, /\.col \{[^}]*overflow: hidden;/);
  assert.doesNotMatch(styles, /\.col \{[^}]*overflow-y: auto/);
  // Industrial console rule: no in-panel scrolling. Only the bottom log
  // console (log lines + image strip) and the modal overlay may scroll.
  assert.match(styles, /\.panel \{[\s\S]*overflow: hidden;/);
  assert.doesNotMatch(styles, /\.(panel|col)[a-z_-]* \{[^}]*overflow(-x|-y)?: auto/);
  assert.doesNotMatch(styles, /\.xray-panel[^{]*\{[^}]*overflow(-y)?: auto/);
  assert.match(styles, /\.console__tabs \{[^}]*overflow: hidden;/);
  assert.match(styles, /\.log-lines\s*\{[^}]*overflow-y: auto;/);
  assert.match(styles, /\.image-strip\s*\{[^}]*overflow-x: auto;/);
});

test("engineering menus and numeric scan fields stay compact and free of branding", async () => {
  const [app, styles] = await Promise.all([read("src/App.tsx"), read("src/styles.css")]);
  const menu = app.slice(app.indexOf("function MenuBar("), app.indexOf("function DevicePanel("));
  assert.doesNotMatch(menu, /app-identity|MICRO|Production|badge|theme-toggle/);
  assert.doesNotMatch(app, /Production workstation|parseHms|formatHms|hh:mm:ss|setup-note|panel__footnote|dock-guidance/);
  const scan = app.slice(app.indexOf("function ScanParamsPanel("), app.indexOf("function DockIcon("));
  assert.doesNotMatch(scan, /<small>1[–-]/);
  assert.match(scan, /minutesToSeconds\(maxXray\)/);
  assert.match(scan, /secondsToMinutes\(setup\.maxXraySec \|\| 600\)/);
  assert.match(app, /className="help-tip" title=\{text\}/);
  assert.match(styles, /button\.help-tip[^}]*border-radius: 50%/);
});
