/**
 * Alignment regression gate for the fixed industrial console.
 *
 * The three upper columns end on one line. The Log and Operation Status
 * panels both start 12px below that line and share the same bottom edge.
 * Neither upper panel nor Operation Status moves.
 * These facts are derived from `src/styles.css` by `layout-geometry.mjs` and asserted
 * here, so the gate fails as soon as the numbers drift apart again — it does
 * not pin fresh literals where a derivation is possible.
 *
 * Reachability note: `test:sites` runs `node --test tests/*.test.mjs`, so this
 * file is named `*.test.mjs` on purpose. `tests/layout-geometry.mjs` holds the
 * derivations and is intentionally not collected on its own.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  BOTTOM_ROW_PX,
  DESIGN_HEIGHT,
  DESIGN_WIDTH,
  assertAlignmentInvariants,
  deriveAlignment,
  readGeometry,
} from "./layout-geometry.mjs";

const STYLESHEET = new URL("../src/styles.css", import.meta.url);
const stylesheet = readFileSync(STYLESHEET, "utf8");

test("the bottom panels use the published 306px design height", () => {
  assert.equal(BOTTOM_ROW_PX, 306);
});

test("the stylesheet numbers satisfy every alignment invariant", () => {
  const geometry = readGeometry(stylesheet);
  assertAlignmentInvariants(assert, geometry);
});

test("the log alone moves down to align with Operation Status", () => {
  const geometry = readGeometry(stylesheet);
  const facts = deriveAlignment(geometry);
  assert.equal(geometry.operationPanel.height, geometry.workspace.bottom);
  assert.equal(geometry.operationPanel.minHeight, geometry.workspace.bottom);
  assert.equal(facts.rightUpperTrack, facts.upperBottomLine);
  assert.equal(facts.operationTop, facts.logTop);
  assert.equal(facts.operationBottom, facts.logBottom);
  assert.equal(facts.operationTop - facts.upperBottomLine, geometry.workspace.columnGap);
});

test("the right panel gap matches the shared 12px panel rhythm", () => {
  const { workspace, rightColumn, operationPanel } = readGeometry(stylesheet);
  assert.equal(rightColumn.spacer, workspace.columnGap);
  assert.equal(workspace.divider, workspace.columnGap);
  assert.equal(rightColumn.bottomTrack, operationPanel.height);
  assert.match(rightColumn.tracks, /^minmax\(0, 1fr\) \d+px$/);
});

test("the three upper columns and the bottom console stay inside the canvas", () => {
  const geometry = readGeometry(stylesheet);
  const facts = deriveAlignment(geometry);
  // Workspace grid content box = canvas height - chrome rows - hairline frames.
  assert.equal(
    facts.contentRow,
    DESIGN_HEIGHT - geometry.canvas.topChrome - geometry.canvas.statusBar - facts.frameDividers * geometry.canvas.chromeTracks[1],
  );
  assert.equal(facts.workspaceContentBox, facts.contentRow - geometry.workspace.padding * 2);
  // Upper row + divider row + bottom row are the whole workspace content box.
  assert.equal(
    facts.upperRow + geometry.workspace.divider + geometry.workspace.bottom,
    facts.workspaceContentBox,
  );
  assert.equal(geometry.canvas.width, DESIGN_WIDTH);
});

test("upper panels and log are separated by empty space, not an overlay line", () => {
  const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
  const workspace = app.slice(app.indexOf('<section className="workspace-body">'), app.indexOf('<BottomConsole ws={ws}'));
  assert.ok(workspace.length > 0);
  assert.doesNotMatch(workspace, /<div className="app-divider"\s*\/>/);
  assert.doesNotMatch(stylesheet, /\.workspace-body\s*>\s*\.app-divider\s*\{/);
});

test("the 3D viewport joins the toolbar and follows the outer bottom curve", () => {
  const tokens = readFileSync(new URL("../src/tokens.css", import.meta.url), "utf8");
  const scene = tokens.match(/\.live-scene\s*\{([^}]*)\}/)?.[1];
  assert.ok(scene, "the viewport must declare its clipping geometry");
  assert.match(scene, /border-radius:\s*0 0 calc\(var\(--radiusCardCompact\) - 1px\) calc\(var\(--radiusCardCompact\) - 1px\)/);
  assert.match(scene, /overflow:\s*hidden/);
  assert.match(stylesheet, /\.panel\s*\{[^}]*border:\s*1px solid var\(--line\);[^}]*border-radius:\s*var\(--radiusCardCompact\)/);
});

test("no column or panel gained an in-column scrollbar", () => {
  const { allowedScrollers } = readGeometry(stylesheet);
  // Exactly three surfaces may scroll: the log list, the image strip and the
  // overlay dialog. The tab rail is clipped, not scrollable.
  assert.deepEqual(allowedScrollers, [".log-lines", ".image-strip", ".modal-card__body"]);
  assert.match(stylesheet, /\.console__tabs \{[^}]*overflow: hidden;/);
  assert.match(stylesheet, /\.col \{[^}]*overflow: hidden;/);
  assert.doesNotMatch(stylesheet, /\.col \{[^}]*overflow-y: auto/);
  assert.doesNotMatch(stylesheet, /\.(panel|col)[a-z_-]* \{[^}]*overflow(-x|-y)?: auto/);
  // The panel rule itself must keep clipping rather than scrolling.
  assert.match(stylesheet, /\.panel \{[\s\S]*overflow: hidden;/);
});
