/**
 * Pure layout mathematics for fitting the fixed industrial console canvas.
 * The function accepts browser client dimensions and returns scale, offsets,
 * and design height without reading the DOM, which keeps it easy to test.
 */

export const DESIGN_WIDTH = 1920;
export const DESIGN_HEIGHT = 1080;
/** Smallest allowed uniform scale of the design canvas (1920 x 1080 -> 1728 x 972). */
export const MIN_CANVAS_SCALE = 0.9;
/** Design-canvas height window that may absorb the real client aspect ratio. */
export const MIN_DESIGN_HEIGHT = DESIGN_HEIGHT * MIN_CANVAS_SCALE;
export const MAX_DESIGN_HEIGHT = 1400;

export interface CanvasLayout {
  zoom: number;
  /** Design-canvas height in design pixels: it absorbs the client aspect ratio. */
  designHeight: number;
  scaledWidth: number;
  scaledHeight: number;
  gutterX: number;
  gutterY: number;
  /** "fill" keeps both axes flush; "contain" falls back to symmetric chassis gutter. */
  mode: "fill" | "contain";
}

/**
 * Fits the fixed 1920-wide design canvas into the client area without distortion.
 *
 * Maximised windows are rarely exactly 16:9 because the taskbar removes height,
 * so the canvas height absorbs the remaining difference while the width always
 * fills the window. Only extreme aspect ratios fall back to a symmetric gutter.
 */
export function computeCanvasLayout(viewportWidth: number, viewportHeight: number): CanvasLayout {
  const safeWidth = Number.isFinite(viewportWidth) ? Math.max(0, viewportWidth) : 0;
  const safeHeight = Number.isFinite(viewportHeight) ? Math.max(0, viewportHeight) : 0;

  const fillZoom = safeWidth > 0 ? safeWidth / DESIGN_WIDTH : 0;
  const fillHeight = fillZoom > 0 ? safeHeight / fillZoom : 0;
  const canFill =
    fillZoom > 0 &&
    fillHeight >= MIN_DESIGN_HEIGHT &&
    fillHeight <= MAX_DESIGN_HEIGHT;

  const zoom = canFill
    ? fillZoom
    : Math.min(
        safeWidth > 0 ? safeWidth / DESIGN_WIDTH : 0,
        safeHeight > 0 ? safeHeight / DESIGN_HEIGHT : 0,
      );
  const designHeight = canFill ? fillHeight : DESIGN_HEIGHT;
  const scaledWidth = DESIGN_WIDTH * zoom;
  const scaledHeight = designHeight * zoom;

  return {
    zoom,
    designHeight: designHeight > 0 ? designHeight : DESIGN_HEIGHT,
    scaledWidth,
    scaledHeight,
    gutterX: Math.max(0, (safeWidth - scaledWidth) / 2),
    gutterY: Math.max(0, (safeHeight - scaledHeight) / 2),
    mode: canFill ? "fill" : "contain",
  };
}
