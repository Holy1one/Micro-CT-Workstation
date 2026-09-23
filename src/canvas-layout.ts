/**
 * Edge-to-edge workstation layout. CSS pixels already account
 * for Windows DPI; multiplying by devicePixelRatio again would double-scale.
 * One control scale is retained; the grid absorbs the actual client aspect ratio.
 * There is never letterboxing. Desktop restrictions live in Tauri.
 */
export const DESIGN_WIDTH = 1920;
export const DESIGN_HEIGHT = 1080;
export const MIN_CANVAS_SCALE = 0.9;
export const MIN_DESIGN_HEIGHT = 960;

export interface CanvasLayout {
  zoom: number;
  designWidth: number;
  designHeight: number;
  scaledWidth: number;
  scaledHeight: number;
  gutterX: number;
  gutterY: number;
  mode: "fill";
}

export function computeCanvasLayout(viewportWidth: number, viewportHeight: number): CanvasLayout {
  const width = Number.isFinite(viewportWidth) ? Math.max(0, viewportWidth) : 0;
  const height = Number.isFinite(viewportHeight) ? Math.max(0, viewportHeight) : 0;
  const zoom = Math.min(width / DESIGN_WIDTH, height / MIN_DESIGN_HEIGHT);
  return {
    zoom,
    designWidth: zoom > 0 ? width / zoom : DESIGN_WIDTH,
    designHeight: zoom > 0 ? height / zoom : DESIGN_HEIGHT,
    scaledWidth: width, scaledHeight: height, gutterX: 0, gutterY: 0, mode: "fill",
  };
}
