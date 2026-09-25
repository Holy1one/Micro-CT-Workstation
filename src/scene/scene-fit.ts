import {
  Box3,
  Vector3,
  type Camera,
  type Object3D,
} from "three";

export const FRUSTUM_HEIGHT = 1200;
export const FIT_WIDTH = 0.88;
export const FIT_HEIGHT = 0.82;

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
