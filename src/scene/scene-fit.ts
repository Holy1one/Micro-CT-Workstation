import {
  Box3,
  Vector3,
  type Camera,
  type Object3D,
  type OrthographicCamera,
} from "three";

export const FRUSTUM_HEIGHT = 1200;
export const FIT_WIDTH = 0.88;
export const FIT_HEIGHT = 0.82;

/** Keep the aspect comparison away from a divide-by-zero on a collapsed box. */
const MIN_EXTENT = 1e-6;

/**
 * Rewrite an orthographic camera's frustum so that
 * `(right - left) / (top - bottom)` equals `viewWidth / viewHeight` exactly,
 * while the vertical extent stays pinned at FRUSTUM_HEIGHT.
 *
 * The vertical extent is the anchor, not the vertical extent *of the canvas*:
 * the rig deliberately keeps a constant vertical world-height and widens only
 * horizontally, so a wide canvas reveals more of the bench instead of shrinking
 * it. Whatever the horizontal half-width comes out as, the pair must share the
 * canvas aspect ratio or the image is stretched; that invariant is what this
 * helper exists to restore.
 *
 * Returns true when it changed the frustum, so callers can avoid re-uploading
 * an identical projection matrix.
 */
export function applyFrustumHeight(
  camera: OrthographicCamera,
  viewWidth: number,
  viewHeight: number,
): boolean {
  // The renderer reports fractional CSS sizes, so a collapsed or nonsensical box
  // must not be allowed to poison the projection with Infinity/NaN.
  if (!(viewWidth > 0) || !(viewHeight > 0)) return false;

  const halfHeight = FRUSTUM_HEIGHT / 2;
  const halfWidth = FRUSTUM_HEIGHT * (viewWidth / viewHeight) / 2;
  if (!Number.isFinite(halfWidth) || !Number.isFinite(halfHeight)) return false;

  const unchanged = camera.left === -halfWidth
    && camera.right === halfWidth
    && camera.bottom === -halfHeight
    && camera.top === halfHeight;
  if (unchanged) return false;

  camera.left = -halfWidth;
  camera.right = halfWidth;
  camera.bottom = -halfHeight;
  camera.top = halfHeight;
  camera.updateProjectionMatrix();
  return true;
}

/** Aspect ratio of the live projection, or null when it cannot be measured. */
export function measuredFrustumAspect(camera: OrthographicCamera): number | null {
  const width = camera.right - camera.left;
  const height = camera.top - camera.bottom;
  if (!Number.isFinite(width) || !Number.isFinite(height)) return null;
  if (Math.abs(height) < MIN_EXTENT || Math.abs(width) < MIN_EXTENT) return null;
  return width / height;
}

/** Collect visible mesh bounds, ignoring helpers excluded by this scene. */
export function collectSceneBounds(root: Object3D): Box3 {
  const bounds = new Box3();
  const scratch = new Box3();
  root.traverse((object) => {
    if (!(object as { isMesh?: boolean }).isMesh) return;

    let excluded = false;
    let ancestor: Object3D | null = object;
    while (ancestor) {
      if (!ancestor.visible) return;
      if (ancestor.userData?.excludeFromFit) excluded = true;
      ancestor = ancestor.parent;
    }
    if (excluded && !object.userData?.includeInFit) return;

    scratch.setFromObject(object);
    if (!scratch.isEmpty()) bounds.union(scratch);
  });
  return bounds;
}

/** Fit the projected bounds rather than world axes: each preset has a different screen extent. */
export function computeFitZoom(
  root: Object3D,
  camera: Camera,
  viewWidth: number,
  viewHeight: number,
): number | null {
  const bounds = collectSceneBounds(root);
  if (bounds.isEmpty() || viewWidth <= 0 || viewHeight <= 0) return null;

  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (const x of [bounds.min.x, bounds.max.x]) {
    for (const y of [bounds.min.y, bounds.max.y]) {
      for (const z of [bounds.min.z, bounds.max.z]) {
        const local = new Vector3(x, y, z).applyMatrix4(camera.matrixWorldInverse);
        minX = Math.min(minX, local.x);
        maxX = Math.max(maxX, local.x);
        minY = Math.min(minY, local.y);
        maxY = Math.max(maxY, local.y);
      }
    }
  }
  const aspect = viewWidth / viewHeight;
  return Math.min(
    FRUSTUM_HEIGHT * aspect * FIT_WIDTH / Math.max(maxX - minX, 1),
    FRUSTUM_HEIGHT * FIT_HEIGHT / Math.max(maxY - minY, 1),
  );
}
