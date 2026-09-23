/**
 * Client-filling layout acceptance: controls use one scale and the grid adapts
 * to the available width and height, without any letterboxing.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { computeCanvasLayout, DESIGN_HEIGHT, DESIGN_WIDTH } from "../src/canvas-layout.ts";
import { readFileSync } from "node:fs";

test("frontend, CSS and desktop share a 1920 by 1080 baseline", () => {
  assert.equal(DESIGN_WIDTH, 1920);
  assert.equal(DESIGN_HEIGHT, 1080);
  const config = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url)));
  assert.equal(config.app.windows[0].width, DESIGN_WIDTH);
  assert.equal(config.app.windows[0].height, DESIGN_HEIGHT);
  const rust = readFileSync(new URL("../src-tauri/src/main.rs", import.meta.url), "utf8");
  assert.match(rust, new RegExp("const DESIGN_WIDTH: f64 = " + DESIGN_WIDTH + "\\.0"));
  assert.match(rust, new RegExp("const DESIGN_HEIGHT: f64 = " + DESIGN_HEIGHT + "\\.0"));
});

for (const [label, width, height] of [
  ["1080p design", 1920, 1080], ["1080p desktop client", 1920, 1009],
  ["1440p", 2560, 1440], ["4K", 3840, 2160],
  ["1440p at 125% DPI", 2048, 1112], ["4K at 150% DPI", 2560, 1400],
  ["4K at 200% DPI", 1920, 1040], ["window minimum", 1728, 972],
  ["small desktop", 1366, 697], ["ultrawide", 3440, 1440], ["portrait", 1080, 1920],
]) {
  test(label + " fills all four client edges without cropping", () => {
    const layout = computeCanvasLayout(width, height);
    assert.equal(layout.scaledWidth, width);
    assert.equal(layout.scaledHeight, height);
    assert.equal(layout.gutterX, 0);
    assert.equal(layout.gutterY, 0);
    assert.equal(layout.mode, "fill");
    assert.ok(layout.designWidth >= 1920 - 0.001);
    assert.ok(layout.designHeight >= 960 - 0.001);
    assert.ok(layout.scaledWidth <= width + 0.001);
    assert.ok(layout.scaledHeight <= height + 0.001);
    assert.ok(Math.abs(layout.designWidth * layout.zoom - width) < 0.001);
    assert.ok(Math.abs(layout.designHeight * layout.zoom - height) < 0.001);
    assert.ok(Math.abs(layout.gutterX * 2 + layout.scaledWidth - width) < 0.001);
    assert.ok(Math.abs(layout.gutterY * 2 + layout.scaledHeight - height) < 0.001);
  });
}
test("2K and 4K enlarge the entire canvas", () => {
  assert.equal(computeCanvasLayout(2560, 1440).zoom, 4 / 3);
  assert.equal(computeCanvasLayout(3840, 2160).zoom, 2);
});
test("invalid dimensions cannot create negative sizes or scale", () => {
  for (const [width, height] of [[0, 0], [-10, 800], [NaN, 900], [1024, Infinity]]) {
    const layout = computeCanvasLayout(width, height);
    for (const field of ["zoom", "scaledWidth", "scaledHeight", "gutterX", "gutterY"]) {
      assert.ok(Number.isFinite(layout[field]) && layout[field] >= 0);
    }
  }
});
