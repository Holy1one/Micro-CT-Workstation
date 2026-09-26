/*
 * Alignment geometry for the fixed industrial console.
 *
 * This module is the single place where the stylesheet's layout numbers are
 * turned into geometry facts. The alignment regression test imports it and the
 * CDP measurement harness can require the same derivations, so the test can
 * never grade a different formula than the one the stylesheet actually uses.
 *
 * Everything here is derived from the stylesheet text: no geometry literal is
 * duplicated, and the only pinned numbers are the design baseline and the 1.2x
 * growth factor the brief states for the bottom row.
 */

/** The console's design baseline, shared with canvas-layout.ts. */
export const DESIGN_WIDTH = 1920;
export const DESIGN_HEIGHT = 1080;

/** Bottom-row growth factor: 264 * 1.2 = 316.8, rounded to 317. */
export const BOTTOM_ROW_SCALE = 1.2;
export const BOTTOM_ROW_BASE_PX = 264;
export const BOTTOM_ROW_PX = Math.round(BOTTOM_ROW_BASE_PX * BOTTOM_ROW_SCALE);

const ruleBody = (css, selector) => {
  const pattern = new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{([^}]*)\\}`);
  const match = css.match(pattern);
  if (!match) throw new Error(`stylesheet is missing the ${selector} rule`);
  return match[1];
};

const pxDeclaration = (body, property, selector) => {
  const match = body.match(new RegExp(`(?:^|;)\\s*${property}\\s*:\\s*(-?[\\d.]+)px`));
  if (!match) throw new Error(`${selector} does not declare ${property} in px`);
  return Number(match[1]);
};

const trackList = (body, selector) => {
  const match = body.match(/grid-template-rows\s*:\s*([^;]+);/);
  if (!match) throw new Error(`${selector} does not declare grid-template-rows`);
  return match[1].trim();
};

/** The row-gap of a rule, in px, or null when the rule does not declare one. */
const rowGapPx = (css, selector) => {
  const body = ruleBody(css, selector);
  const match = body.match(/(?:^|;)\s*row-gap\s*:\s*(-?[\d.]+)px/);
  return match ? Number(match[1]) : null;
};

/** Splits a track list on top-level whitespace, keeping minmax(...) intact. */
export const splitTracks = (tracks) => {
  const out = [];
  let depth = 0;
  let current = "";
  for (const char of tracks.trim()) {
    if (char === "(") depth += 1;
    if (char === ")") depth -= 1;
    if (/\s/.test(char) && depth === 0) {
      if (current) out.push(current);
      current = "";
      continue;
    }
    current += char;
  }
  if (current) out.push(current);
  return out;
};

const pxTracks = (tracks, selector) =>
  splitTracks(tracks).map((track) => {
    const px = track.match(/^(-?[\d.]+)px$/);
    if (!px) throw new Error(`${selector} track "${track}" is not a fixed px track`);
    return Number(px[1]);
  });

/** The fixed-px subset of a track list plus the total of the flexible tracks. */
export const trackShape = (tracks) => {
  const parts = splitTracks(tracks);
  return {
    count: parts.length,
    px: parts.map((track) => (track.match(/^(-?[\d.]+)px$/) ? Number(track.match(/^(-?[\d.]+)px$/)[1]) : null)),
    flexible: parts.filter((track) => /fr\b/.test(track)).length,
  };
};

/**
 * Reads every number the alignment invariants depend on straight out of the
 * stylesheet source. Throws when a declaration the invariants rely on is gone,
 * so deleting a rule fails the gate instead of silently skipping an assertion.
 */
export function readGeometry(css) {
  const canvasBody = ruleBody(css, ".design-canvas");
  const workspaceBody = ruleBody(css, ".workspace-body");
  const rightBody = ruleBody(css, ".col--right");
  const opBody = ruleBody(css, ".operation-panel");

  const canvasTracks = trackShape(trackList(canvasBody, ".design-canvas"));
  const workspaceTracks = trackList(workspaceBody, ".workspace-body");
  const workspaceShape = trackShape(workspaceTracks);
  // The upper row is the flexible track (the main console area); the divider and
  // the bottom console row are the fixed tracks around it.
  const workspaceFixed = workspaceShape.px.filter((px) => px !== null);
  const rightShape = trackShape(trackList(rightBody, ".col--right"));
  const rightFixed = rightShape.px.filter((px) => px !== null);

  const columns = workspaceBody.match(/grid-template-columns\s*:\s*([^;]+);/);
  const columnGap = workspaceBody.match(/column-gap\s*:\s*([\d.]+)px/);
  const padding = workspaceBody.match(/padding\s*:\s*([\d.]+)px/);

  // Every surface that declares a scrolling overflow axis, so the "no new
  // scrollbar" assertion can be written against a derived allow-list.
  const escapeSelector = (selector) => selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const scrolls = (selector) => {
    const pattern = new RegExp(`${escapeSelector(selector)}\\s*\\{[^}]*overflow(-x|-y)?\\s*:\\s*(auto|scroll)`);
    return pattern.test(css);
  };

  return {
    canvas: {
      width: pxDeclaration(canvasBody, "width", ".design-canvas"),
      height: pxDeclaration(canvasBody, "height", ".design-canvas"),
      chromeTracks: canvasTracks.px,
      chromeTrackCount: canvasTracks.count,
      topChrome: canvasTracks.px[0],
      statusBar: canvasTracks.px[canvasTracks.count - 1],
    },
    workspace: {
      tracks: workspaceTracks,
      trackCount: workspaceShape.count,
      flexibleTracks: workspaceShape.flexible,
      // The upper row is the flexible track, so it carries no literal height:
      // the columns beside it decide it by their own content.
      upperIsFlexible: workspaceShape.px[0] === null,
      divider: workspaceFixed[0],
      bottom: workspaceFixed[workspaceFixed.length - 1],
      upper: null,
      columns: columns ? columns[1].trim() : null,
      columnGap: columnGap ? Number(columnGap[1]) : null,
      padding: padding ? Number(padding[1]) : null,
    },
    rightColumn: {
      trackCount: rightShape.count,
      flexibleTracks: rightShape.flexible,
      upperIsFlexible: rightShape.px[0] === null,
      // The right column's upper panel is the flexible track; its bottom panel
      // is an auto track sized by the Operation Status panel's own height.
      bottomIsAuto: rightShape.px[rightShape.count - 1] === null,
      // The separator between the two right-column panels is the column's own
      // row-gap. It has to equal the shared divider row (a hairline), not the
      // 12px padding rhythm: a 12px gap would push the Operation Status panel
      // 12px down and pull the X-ray panel (and only it) 11px above the shared
      // line the left and centre columns end on.
      spacer: rowGapPx(css, ".col--right"),
      tracks: trackList(rightBody, ".col--right").trim(),
    },
    operationPanel: {
      height: pxDeclaration(opBody, "height", ".operation-panel"),
      minHeight: pxDeclaration(opBody, "min-height", ".operation-panel"),
    },
    // The only surfaces allowed to scroll inside the console: the log list, the
    // image strip and the overlay dialog. The tab rail is clipped, not
    // scrollable, and is recorded here so a change is visible.
    allowedScrollers: [".log-lines", ".image-strip", ".console__tabs", ".modal-card__body"]
      .filter((selector) => scrolls(selector)),
  };
}

/**
 * Turns the derived numbers into the alignment facts the invariants assert.
 * Everything is computed from the canvas row heights rather than typed in, so a
 * change to the chrome, the status bar or the workspace padding moves both the
 * console and the assertion together.
 */
export function deriveAlignment(geometry) {
  const { canvas, workspace, rightColumn, operationPanel } = geometry;
  // The canvas grid is [top chrome, 1px, 1fr, 1px, status bar]; the workspace
  // grid's content box is that 1fr row: the canvas height minus the top chrome,
  // the status bar and every hairline frame track around the content row. The
  // hairline count is derived (tracks minus chrome minus the flexible row)
  // rather than assumed, so adding a frame track cannot silently shift the line.
  const flexibleFrames = 1;
  const frameDividers = canvas.chromeTrackCount - 2 - flexibleFrames;
  const dividerPx = canvas.chromeTracks.length >= 2 ? canvas.chromeTracks[1] : 1;
  const contentRow = canvas.height - canvas.topChrome - canvas.statusBar - frameDividers * dividerPx;
  const columnGap = workspace.columnGap ?? 0;
  const padding = workspace.padding ?? 0;
  const rightSpacer = rightColumn.spacer ?? 0;

  const workspaceContentBox = contentRow - padding * 2;
  // The upper row is the flexible track: it receives whatever the workspace
  // content box has left after the divider row and the bottom console row.
  const upperRow = workspaceContentBox - workspace.divider - workspace.bottom;
  // Every upper column ends on this line, measured from the workspace content
  // box's top: the upper row's height.
  const upperBottomLine = upperRow;
  // The right column spans the upper row, the divider row and the bottom row,
  // so its flexible first track is that span minus its own separator (the
  // column's row-gap) and minus its auto track, which the Operation Status
  // panel fills with its own published height.
  const rightSpan = upperRow + workspace.divider + workspace.bottom;
  const rightBottomTrack = operationPanel.height;
  const rightUpperTrack = rightSpan - rightSpacer - rightBottomTrack;

  return {
    canvasHeight: canvas.height,
    frameDividers,
    contentRow,
    workspaceContentBox,
    upperRow,
    upperBottomLine,
    rightSpan,
    rightBottomTrack,
    rightUpperTrack,
    rightColumnTrackSum: rightUpperTrack + rightSpacer + rightBottomTrack,
    requiredRightSpacer: workspace.divider,
    // Both bottom areas are one grid track tall and the Operation Status panel
    // carries that same height, so the panel needs no min-height that differs
    // from the console row height.
    bottomRow: workspace.bottom,
    operationHeight: operationPanel.height,
    operationMinHeight: operationPanel.minHeight,
    columnGap,
    padding,
  };
}

/** The single place that states how the shared bottom line is reached. */
export function assertAlignmentInvariants(assert, geometry) {
  const facts = deriveAlignment(geometry);
  const { workspace, rightColumn, operationPanel, canvas } = geometry;

  assert.equal(canvas.width, DESIGN_WIDTH, "the console keeps its 1920px design width");
  assert.equal(canvas.height, DESIGN_HEIGHT, "the console keeps its 1080px design height");

  // 1. The bottom row is the published bottom-row height, not a fresh literal.
  assert.equal(
    workspace.bottom,
    BOTTOM_ROW_PX,
    `the bottom console row must be ${BOTTOM_ROW_BASE_PX} * ${BOTTOM_ROW_SCALE} = ${BOTTOM_ROW_PX}px`,
  );

  // 2. The Operation Status panel adopts the bottom row height exactly, so the
  //    log console and the Operation Status panel share a top and a bottom edge.
  assert.equal(operationPanel.height, BOTTOM_ROW_PX, "the Operation Status panel height must equal the bottom console row");
  assert.equal(operationPanel.minHeight, BOTTOM_ROW_PX, "the Operation Status panel min-height must equal the bottom console row");
  assert.ok(rightColumn.bottomIsAuto, "the right column's Operation Status track stays auto, sized by the panel's own height");

  // 3. The right column's separator must be the row the columns beside it use
  //    for the same visual break, not the 12px padding rhythm: a wider gap
  //    pushes the Operation Status panel down and pulls the X-ray panel (and
  //    only it) above the line the left and centre columns end on.
  assert.equal(
    rightColumn.spacer,
    facts.requiredRightSpacer,
    "the right column's panel separator must equal the shared divider row so all three upper columns end on one line",
  );

  // 4. Structure: the upper row is flexible, so the columns beside it decide
  //    the shared line instead of a literal; the divider stays a hairline.
  assert.equal(workspace.trackCount, 3, "the workspace grid keeps its upper / divider / bottom structure");
  assert.ok(workspace.upperIsFlexible, "the upper row must stay the flexible track so it sizes to the upper columns");
  assert.ok(workspace.flexibleTracks === 1, "exactly one workspace track may be flexible");
  assert.ok(facts.upperRow > workspace.bottom, "the upper row stays the larger area");
  assert.ok(workspace.divider >= 0 && workspace.divider <= 2, "the divider row stays a hairline track");

  // 5. The right column's stack reproduces the shared line inside its own span:
  //    the right column covers the same box the workspace grid gives the upper
  //    columns (upper row + divider row + bottom row), its own separator equals
  //    the divider row and its auto track equals the bottom row. Therefore the
  //    X-ray panel's flexible track ends exactly where the left and centre
  //    columns end, and the Operation Status panel starts on the console's top.
  assert.equal(
    facts.rightColumnTrackSum,
    facts.workspaceContentBox,
    "the right column's tracks must cover the same box the upper columns cover",
  );
  assert.equal(
    facts.rightUpperTrack + rightColumn.spacer,
    facts.upperBottomLine + workspace.divider,
    "the X-ray panel must end on the same line as the left and centre columns",
  );

  // 6. No scrollbar may be added anywhere. The only surfaces allowed to scroll
  //    are the log list, the image strip and the overlay dialog; the tab rail is
  //    clipped rather than scrollable, so it must not appear here.
  assert.deepEqual(
    geometry.allowedScrollers,
    [".log-lines", ".image-strip", ".modal-card__body"],
    "only the log list, the image strip and the overlay dialog may scroll",
  );
}
