import assert from "node:assert/strict";
import test from "node:test";

import {
  computeCanvasLayout,
  DESIGN_HEIGHT,
  DESIGN_WIDTH,
  MIN_CANVAS_SCALE,
} from "../src/canvas-layout.ts";

const bothAxesFlush = (layout, viewportWidth, viewportHeight) =>
  Math.abs(viewportWidth - layout.scaledWidth) <= 0.001 &&
  Math.abs(viewportHeight - layout.scaledHeight) <= 0.001;

test("design canvas is authored at the fixed 16:9 baseline", () => {
  assert.equal(DESIGN_WIDTH, 1920);
  assert.equal(DESIGN_HEIGHT, 1080);
  assert.equal(MIN_CANVAS_SCALE, 0.9);
});

test("1080p viewport fills both axes without any gutter", () => {
  const layout = computeCanvasLayout(1920, 1080);
  assert.equal(layout.mode, "fill");
  assert.equal(layout.zoom, 1);
  assert.equal(layout.designHeight, 1080);
  assert.ok(bothAxesFlush(layout, 1920, 1080));
  assert.equal(layout.gutterX, 0);
  assert.equal(layout.gutterY, 0);
});

test("1K maximised client area leaves no side gutter", () => {
  // 1920 x 1009 is what a 1080p screen leaves after the taskbar.
  const layout = computeCanvasLayout(1920, 1009);
  assert.equal(layout.mode, "fill");
  assert.ok(Math.abs(layout.zoom - 1) < 1e-9);
  assert.ok(Math.abs(layout.designHeight - 1009) < 1e-6);
  assert.ok(bothAxesFlush(layout, 1920, 1009));
  assert.equal(layout.gutterX, 0);
  assert.equal(layout.gutterY, 0);
});

test("reported 1011 x 695 client area remains flush on both axes", () => {
  const layout = computeCanvasLayout(1011, 695);
  assert.equal(layout.mode, "fill");
  assert.ok(bothAxesFlush(layout, 1011, 695));
  assert.equal(layout.gutterX, 0);
  assert.equal(layout.gutterY, 0);
});

test("smallest allowed window is 0.9 of the design canvas and still fills", () => {
  const layout = computeCanvasLayout(1728, 972);
  assert.equal(layout.mode, "fill");
  assert.ok(Math.abs(layout.zoom - MIN_CANVAS_SCALE) < 1e-9);
  assert.ok(Math.abs(layout.designHeight - 1080) < 1e-6);
  assert.ok(bothAxesFlush(layout, 1728, 972));
});

test("extreme aspect ratios fall back to symmetric contain", () => {
  // Ultrawide 3440 x 1440 would need a design height below the readable floor.
  const layout = computeCanvasLayout(3440, 1440);
  assert.equal(layout.mode, "contain");
  assert.ok(Math.abs(layout.zoom - 1440 / 1080) < 1e-9);
  assert.ok(Math.abs(layout.gutterX - (3440 - 1920 * (1440 / 1080)) / 2) < 1e-6);
  assert.equal(layout.gutterY, 0);
});

test("degenerate viewports never produce negative sizes or gutters", () => {
  for (const [width, height] of [
    [0, 0],
    [-10, 800],
    [Number.NaN, 900],
    [1024, Number.POSITIVE_INFINITY],
  ]) {
    const layout = computeCanvasLayout(width, height);
    assert.ok(layout.zoom >= 0);
    assert.ok(layout.designHeight > 0);
    assert.ok(layout.scaledWidth >= 0);
    assert.ok(layout.scaledHeight >= 0);
    assert.ok(layout.gutterX >= 0);
    assert.ok(layout.gutterY >= 0);
  }
});
