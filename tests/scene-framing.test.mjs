import assert from "node:assert/strict";
import test from "node:test";
import { Box3, BoxGeometry, Group, Mesh, OrthographicCamera, Vector3 } from "three";
import { collectSceneBounds, computeFitZoom, FIT_HEIGHT, FIT_WIDTH, FRUSTUM_HEIGHT } from "../src/scene/scene-fit.ts";

const VIEW_WIDTH = 991;
const VIEW_HEIGHT = 595;
const CENTER_TARGET = new Vector3(0, 20, 0);
const PRESET_CAMERAS = {
  ISO: [745.1, 485.2, 745.1],
  FRONT: [762.8, 221.6, 0],
  TOP: [0, 1027.5, 144.6],
};

function syntheticSandbox() {
  const root = new Group();
  const track = new Mesh(new BoxGeometry(864, 36, 864));
  track.name = "synthetic-track-bounds";
  track.position.set(0, -94, 0);
  root.add(track);

  const excludedDecorationGroup = new Group();
  excludedDecorationGroup.userData.excludeFromFit = true;
  const decoration = new Mesh(new BoxGeometry(1120, 87, 1120));
  decoration.name = "visible-decoration-omitted-by-legacy-collector";
  decoration.position.set(0, -68.5, 0);
  decoration.userData.includeInFit = true;
  excludedDecorationGroup.add(decoration);
  root.add(excludedDecorationGroup);
  root.updateMatrixWorld(true);

  const intendedBounds = new Box3()
    .setFromObject(track)
    .union(new Box3().setFromObject(decoration));
  return { root, intendedBounds };
}

function projectedExtent(bounds, camera) {
  const extent = { minX: Infinity, maxX: -Infinity, minY: Infinity, maxY: -Infinity };
  for (const x of [bounds.min.x, bounds.max.x]) {
    for (const y of [bounds.min.y, bounds.max.y]) {
      for (const z of [bounds.min.z, bounds.max.z]) {
        const point = new Vector3(x, y, z).project(camera);
        const px = (point.x + 1) * VIEW_WIDTH / 2;
        const py = (1 - point.y) * VIEW_HEIGHT / 2;
        extent.minX = Math.min(extent.minX, px);
        extent.maxX = Math.max(extent.maxX, px);
        extent.minY = Math.min(extent.minY, py);
        extent.maxY = Math.max(extent.maxY, py);
      }
    }
  }
  extent.width = extent.maxX - extent.minX;
  extent.height = extent.maxY - extent.minY;
  return extent;
}

test("ISO, FRONT and TOP fit the track and visible decoration within the orthographic margins", (context) => {
  for (const [preset, cameraPosition] of Object.entries(PRESET_CAMERAS)) {
    const { root, intendedBounds } = syntheticSandbox();
    const aspect = VIEW_WIDTH / VIEW_HEIGHT;
    const camera = new OrthographicCamera(
      -FRUSTUM_HEIGHT * aspect / 2,
      FRUSTUM_HEIGHT * aspect / 2,
      FRUSTUM_HEIGHT / 2,
      -FRUSTUM_HEIGHT / 2,
      -9000,
      9000,
    );
    camera.position.set(...cameraPosition);
    camera.lookAt(CENTER_TARGET);
    camera.updateMatrixWorld(true);

    const fit = computeFitZoom(root, camera, VIEW_WIDTH, VIEW_HEIGHT);
    assert.ok(fit !== null && fit > 0, `${preset} should return a positive fit zoom`);
    camera.zoom = fit;
    camera.updateProjectionMatrix();
    const screenUp = new Vector3(0, 1, 0).applyQuaternion(camera.quaternion);
    camera.position.add(screenUp.multiplyScalar(FRUSTUM_HEIGHT * 0.05 / fit));
    camera.updateMatrixWorld(true);

    const extent = projectedExtent(intendedBounds, camera);
    assert.ok(extent.width <= VIEW_WIDTH * FIT_WIDTH + 1, `${preset} width ${extent.width.toFixed(1)}px exceeds ${FIT_WIDTH * 100}% fit margin`);
    assert.ok(extent.height <= VIEW_HEIGHT * FIT_HEIGHT + 1, `${preset} height ${extent.height.toFixed(1)}px exceeds ${FIT_HEIGHT * 100}% fit margin`);
    assert.ok(extent.minX >= 0 && extent.maxX <= VIEW_WIDTH, `${preset} horizontal bounds overflow: ${JSON.stringify(extent)}`);
    assert.ok(extent.minY >= 0 && extent.maxY <= VIEW_HEIGHT, `${preset} vertical bounds overflow: ${JSON.stringify(extent)}`);

    context.diagnostic(`${preset}: ${extent.width.toFixed(1)} x ${extent.height.toFixed(1)} px of ${VIEW_WIDTH} x ${VIEW_HEIGHT}; zoom=${fit.toFixed(4)}; overflow=0/0/0/0`);

    const collectedBounds = collectSceneBounds(root);
    assert.ok(collectedBounds.min.x <= intendedBounds.min.x && collectedBounds.max.x >= intendedBounds.max.x, `${preset} fit collection omitted the visible decoration bounds`);
    assert.ok(collectedBounds.min.z <= intendedBounds.min.z && collectedBounds.max.z >= intendedBounds.max.z, `${preset} fit collection omitted the visible decoration bounds`);
  }
});
