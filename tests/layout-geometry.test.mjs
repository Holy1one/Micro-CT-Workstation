/**
 * Alignment regression gate for the fixed industrial console.
 *
 * The three upper columns must end on one line, and the bottom log console and
 * the Operation Status panel must share both a top and a bottom edge. Those
 * facts are derived from `src/styles.css` by `layout-geometry.mjs` and asserted
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
  BOTTOM_ROW_BASE_PX,
  BOTTOM_ROW_SCALE,
  DESIGN_HEIGHT,
  DESIGN_WIDTH,
  assertAlignmentInvariants,
  deriveAlignment,
  readGeometry,
} from "./layout-geometry.mjs";

const STYLESHEET = new URL("../src/styles.css", import.meta.url);
const stylesheet = readFileSync(STYLESHEET, "utf8");

test("the bottom row grows from the published bottom-area height", () => {
  assert.equal(BOTTOM_ROW_PX, Math.round(BOTTOM_ROW_BASE_PX * BOTTOM_ROW_SCALE));
  assert.equal(BOTTOM_ROW_PX, 317, "264 * 1.2 = 316.8 rounds to 317 design px");
});

test("the stylesheet numbers satisfy every alignment invariant", () => {
  const geometry = readGeometry(stylesheet);
  assertAlignmentInvariants(assert, geometry);
});

test("the operation status panel and the log console share one height and one row", () => {
  const { workspace, operationPanel } = readGeometry(stylesheet);
  assert.equal(operationPanel.height, workspace.bottom);
  assert.equal(operationPanel.minHeight, workspace.bottom);
  // A mismatch here is exactly the regression that made the bottom-right area
  // look cramped: the panel was clamped to a row it was taller than.
  assert.equal(operationPanel.height, BOTTOM_ROW_PX);
});

test("the right column's separator matches the shared divider row", () => {
  const { workspace, rightColumn } = readGeometry(stylesheet);
  assert.equal(
    rightColumn.spacer,
    workspace.divider,
    "a wider separator would leave only the X-ray panel short of the shared bottom line",
  );
  // The contract also pins the decoupled right column stack; the separator must
  // therefore come from the row-gap, not from a fixed middle track.
  assert.match(rightColumn.tracks, /^minmax\(0, 1fr\) auto$/);
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
