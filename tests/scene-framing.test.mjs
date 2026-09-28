import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { Box3, BoxGeometry, Group, Mesh, OrthographicCamera, Vector3 } from "three";
import { applyFrustumHeight, collectSceneBounds, computeFitZoom, FIT_HEIGHT, FIT_WIDTH, FRUSTUM_HEIGHT, measuredFrustumAspect } from "../src/scene/scene-fit.ts";

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

/* ------------------------------------------------------------------ *
 * Camera frustum must track the canvas aspect ratio.
 *
 * Regression for the distortion defect: @react-three/fiber resets an
 * orthographic camera's four sides to +/-size/2 on every size change (it does
 * so unless `camera.manual` is set), which rewrote top/bottom to the CANVAS
 * height while the rig only ever corrected left/right. The projection aspect
 * then became FRUSTUM_HEIGHT*w/h^2 instead of w/h, i.e. a relative error of
 * |FRUSTUM_HEIGHT/h - 1| - measured live as 74.9% at a 999x686 buffer and 60.3%
 * at 998x3019. The vertical extent must stay pinned at FRUSTUM_HEIGHT and the
 * horizontal extent must follow from the canvas aspect.
 * ------------------------------------------------------------------ */
const FRUSTUM_CASES = [
  ["first-visit layout", 1000, 687],
  ["wide 1280x720", 999, 686],
  ["wide 2560x1440", 1001, 688],
  ["tall portrait 1080x1920", 998, 3019],
  ["small window", 666, 457.375],
  ["sub-unit canvas", 1, 3],
];

/**
 * The defect this file now guards against has two halves, and both are
 * reconstructed here so the assertions below cannot pass vacuously.
 *
 * 1. `@react-three/fiber`'s resize handler rewrites an orthographic camera to
 *    `left/right = +/-size.width/2`, `top/bottom = +/-size.height/2` on every
 *    size change (it skips that only when `camera.manual` is set). That happens
 *    AFTER the rig's own effect, and it is the write that broke the projection.
 * 2. The rig then only corrected `left/right` from the canvas aspect and never
 *    re-asserted `top/bottom`, so the vertical extent stayed at the canvas
 *    height. The projection aspect became `FRUSTUM_HEIGHT * w / h^2` instead of
 *    `w / h` - a relative error of `|FRUSTUM_HEIGHT / h - 1|`, measured live as
 *    74.94% at a 999x686 buffer and 60.27% at 998x3019.
 */

/** Exactly what the renderer's resize handler does to an un-manual camera. */
function rendererResizesCamera(camera, width, height) {
  camera.left = width / -2;
  camera.right = width / 2;
  camera.top = height / 2;
  camera.bottom = height / -2;
  camera.updateProjectionMatrix();
}

/** The pre-fix rig: re-fit position, but correct the frustum horizontally only. */
function preFixRigWrites(camera, width, height) {
  const halfWidth = FRUSTUM_HEIGHT * width / Math.max(height, 1) / 2;
  camera.left = -halfWidth;
  camera.right = halfWidth;
  camera.updateProjectionMatrix();
}

/** The post-fix rig: the projection is corrected as a whole, from the canvas box. */
function postFixRigWrites(camera, width, height) {
  applyFrustumHeight(camera, width, height);
}

/** Drives the "user orbited, then the window was resized" sequence. */
function driveResize(rigWrites, width, height) {
  const camera = new OrthographicCamera();
  rigWrites(camera, VIEW_WIDTH, VIEW_HEIGHT);
  rendererResizesCamera(camera, width, height);
  rigWrites(camera, width, height);
  return camera;
}

/**
 * The contract: after the renderer has stomped the camera and the rig has run
 * again, the projection must share the canvas aspect ratio and keep its vertical
 * anchor. Returns the measurements so callers can assert on them.
 */
function frustumContractError(rigWrites, width, height) {
  const camera = driveResize(rigWrites, width, height);
  const canvasAspect = width / height;
  const frustumAspect = measuredFrustumAspect(camera);
  return {
    canvasAspect,
    frustumAspect,
    relErr: frustumAspect === null ? Infinity : Math.abs(frustumAspect - canvasAspect) / canvasAspect,
    verticalExtent: camera.top - camera.bottom,
  };
}

test("the frustum contract rejects the original defect", () => {
  // Non-vacuity control. Without this the next test could pass because the
  // harness is too weak, rather than because the fix works.
  for (const [label, width, height] of [["wide", 999, 686], ["tall", 998, 3019]]) {
    const control = frustumContractError(preFixRigWrites, width, height);
    assert.ok(control.relErr > 0.5,
      `${label}: the pre-fix rig must be rejected, but its error was only ${(control.relErr * 100).toFixed(3)}%`);
    assert.ok(Math.abs(control.verticalExtent - height) < 1e-9,
      `${label}: the pre-fix rig should leave the vertical extent at the canvas height`);
  }
});

test("a resize keeps the orthographic projection matched to the canvas at any size", () => {
  for (const [label, width, height] of FRUSTUM_CASES) {
    const actual = frustumContractError(postFixRigWrites, width, height);
    assert.ok(actual.frustumAspect !== null, `${label}: frustum aspect should be measurable`);
    assert.ok(actual.relErr < 0.01,
      `${label}: frustum aspect ${actual.frustumAspect} vs canvas ${actual.canvasAspect} (err ${(actual.relErr * 100).toFixed(3)}%)`);
    // The vertical anchor is FRUSTUM_HEIGHT, not the canvas height: a wide canvas
    // must reveal more of the bench rather than rescale it.
    assert.ok(Math.abs(actual.verticalExtent - FRUSTUM_HEIGHT) < 1e-9,
      `${label}: vertical extent ${actual.verticalExtent} must stay ${FRUSTUM_HEIGHT}`);

    // One correction per size, not one per frame.
    const camera = new OrthographicCamera();
    assert.equal(applyFrustumHeight(camera, width, height), true,
      `${label}: helper should report the first correction`);
    assert.equal(applyFrustumHeight(camera, width, height), false,
      `${label}: a repeat call must be a no-op so the matrix is not re-uploaded per frame`);
  }
});

/* ------------------------------------------------------------------ *
 * The wiring, not just the arithmetic.
 *
 * The defect lived in where the correction was APPLIED, not in the formula:
 * the old rig corrected `left/right` only inside an effect that is skipped once
 * the user orbits, and it never re-asserted `top/bottom` at all. So the check
 * below reads the shipped rig and requires the projection to be re-established
 * from the live canvas in the per-frame callback itself, BEFORE the diagnostics
 * are published. That ordering is what makes the correction independent of the
 * `userOrbited` early return and of who resized the camera last.
 * ------------------------------------------------------------------ */
test("the shipped rig corrects the projection every frame, before publishing diagnostics", () => {
  const source = readFileSync(
    new URL("../src/scene/LiveSceneCanvas.tsx", import.meta.url),
    "utf8",
  );

  // Isolate the per-frame callback: it is the one that publishes cameraPosition.
  const frameMarker = "gl.domElement.dataset.cameraPosition";
  const frameAt = source.indexOf(frameMarker);
  assert.ok(frameAt > 0, "the rig should still publish data-camera-position from a frame callback");
  const frameStart = source.lastIndexOf("useFrame(", frameAt);
  assert.ok(frameStart > 0, "the cameraPosition write should live inside a useFrame callback");
  const frameBody = source.slice(frameStart, frameAt);

  assert.ok(frameBody.includes("applyFrustumHeight"),
    "the per-frame callback must re-assert the frustum: correcting it only inside the "
    + "size effect leaves the projection stale whenever that effect is skipped or runs first");
  assert.ok(frameBody.includes("measuredFrustumAspect"),
    "the per-frame callback must measure the current projection so it only rewrites a stale one");
  assert.ok(frameBody.includes("invalidate"),
    "a corrected projection must invalidate the demand-mode frame so it is actually drawn");
});
