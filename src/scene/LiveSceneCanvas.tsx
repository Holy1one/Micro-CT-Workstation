import { OrbitControls, PerspectiveCamera } from "@react-three/drei";
import { Canvas, useThree } from "@react-three/fiber";
import { useEffect, useRef } from "react";
import { Box3, MathUtils, PerspectiveCamera as PerspectiveCameraImpl, Vector3, type Mesh, type Object3D } from "three";
import { EquipmentScene } from "./EquipmentScene";
import { CAMERA_CONSTRAINTS, CAMERA_PRESETS } from "./scene-config";
import { useSceneTheme } from "./theme-three";
import type { SceneViewModel, ViewPreset } from "./types";

/** Fraction of the viewport the optical bench is fitted into. */
const FIT_MARGIN = 0.96;
/** Camera framing: a 40 degree vertical field gives readable perspective on a
 *  bench this long without the wide-angle distortion of a 60 degree lens. */
const FIT_FOV = 40;
const PRESET_ZOOM: Record<ViewPreset, number> = { iso: 1, front: 1.05, top: 1.15 };
/** Orbit limits in world units - perspective cameras frame by distance, not zoom.
 *  The upper bound is locked inside the dome: STAGE_RADIUS is 1500 in
 *  stage-surroundings.tsx, and orbiting out past it would leave the camera behind
 *  the BackSide shell, where nothing renders - the table and background would
 *  simply vanish. The gap to leave here is driven by `computeFitDistance`, whose
 *  result depends on the viewport aspect; it can reach ~1300 in a narrow canvas,
 *  so anything above that point starts clipping the bench to stay in the dome. */
const MIN_DISTANCE = 320;
const MAX_DISTANCE = 1350;

/** Bounding box of everything that must stay visible, ignoring fit-excluded helpers. */
function sceneBounds(root: Object3D): Box3 {
  const bounds = new Box3();
  const scratch = new Box3();
  root.traverse((object) => {
    if (object.userData?.excludeFromFit) return;
    if (!(object as Mesh).isMesh) return;
    scratch.setFromObject(object);
    if (!scratch.isEmpty()) bounds.union(scratch);
  });
  return bounds;
}

/** Distance from the orbit target at which the whole optical bench fits inside
 *  the current canvas. The camera only ever slides along its own view direction,
 *  so the corners keep their offsets in camera space and the distance each one
 *  needs follows from the frustum half angles - the nearest corner (in the sense
 *  of the one demanding the most room) decides. Null when there is nothing to
 *  frame yet, so the caller can retry instead of fitting to an empty box. */
function computeFitDistance(
  root: Object3D,
  camera: PerspectiveCameraImpl,
  target: Vector3,
  viewWidth: number,
  viewHeight: number,
): number | null {
  const bounds = sceneBounds(root);
  if (bounds.isEmpty() || viewWidth <= 0 || viewHeight <= 0) return null;
  const tanV = Math.tan(MathUtils.degToRad(camera.fov) / 2) * FIT_MARGIN;
  const tanH = tanV * (viewWidth / viewHeight);
  const corners: Vector3[] = [];
  for (const x of [bounds.min.x, bounds.max.x]) {
    for (const y of [bounds.min.y, bounds.max.y]) {
      for (const z of [bounds.min.z, bounds.max.z]) corners.push(new Vector3(x, y, z));
    }
  }
  // How much further back the camera has to sit so every corner clears the
  // frustum; negative means the preset is already wide enough and can move in.
  let extra = -Infinity;
  for (const corner of corners) {
    const local = corner.applyMatrix4(camera.matrixWorldInverse);
    const depth = -local.z;
    extra = Math.max(extra, Math.abs(local.x) / tanH - depth, Math.abs(local.y) / tanV - depth);
  }
  return camera.position.distanceTo(target) + extra;
}

/** The subset of OrbitControls this rig touches, kept local so the store's
 *  untyped `controls` slot stays honest at the call sites. */
type OrbitLike = {
  target?: { set: (x: number, y: number, z: number) => void };
  update?: () => void;
  addEventListener?: (type: string, listener: () => void) => void;
  removeEventListener?: (type: string, listener: () => void) => void;
};

function CameraRig({ preset }: { preset: ViewPreset }) {
  const { camera, controls, invalidate, scene, size } = useThree();
  /** Rounded viewport key. The live panel is a flex child whose neighbours keep
   *  changing height, and the whole layout is CSS-zoomed, so the ResizeObserver
   *  reports sub-pixel size changes constantly. Rounding collapses that jitter:
   *  re-framing on every report yanked the camera back to the preset mid-orbit. */
  const fitKey = `${Math.round(size.width)}x${Math.round(size.height)}`;
  /** Once the user orbits, the camera is theirs until they pick a preset again. */
  const userOrbited = useRef(false);
  const lastPreset = useRef<ViewPreset | null>(null);

  useEffect(() => {
    const orbit = controls as OrbitLike | null;
    if (!orbit?.addEventListener) return;
    const markOrbited = () => {
      userOrbited.current = true;
    };
    orbit.addEventListener("start", markOrbited);
    return () => orbit.removeEventListener?.("start", markOrbited);
  }, [controls]);

  useEffect(() => {
    const presetChanged = lastPreset.current !== preset;
    lastPreset.current = preset;
    if (presetChanged) {
      // A preset click is an explicit request to re-frame, so it also clears the
      // manual-orbit lock and re-enables auto-fit on later resizes.
      userOrbited.current = false;
    } else if (userOrbited.current) {
      // Resize (or any size report) must not fight the user's current view.
      return;
    }

    const perspective = camera as PerspectiveCameraImpl;
    const target = new Vector3(...CAMERA_CONSTRAINTS.target);
    const eyeDirection = new Vector3(...CAMERA_PRESETS[preset]).sub(target);
    const eyeUnit = eyeDirection.clone().normalize();

    let cancelled = false;
    let retried = false;

    const apply = () => {
      if (cancelled) return;
      perspective.position.copy(target).add(eyeDirection);
      perspective.lookAt(target);
      // Camera.updateMatrixWorld also refreshes matrixWorldInverse, which the fit
      // below reads to place the corners in camera space.
      perspective.updateMatrixWorld();

      const fit = computeFitDistance(scene, perspective, target, size.width, size.height);
      // The bench may not be attached to the scene graph on the first pass, and
      // framing an empty box would drop the camera onto the preset distance and
      // leave it there. Give it one more frame before trusting the result.
      if (fit === null && !retried) {
        retried = true;
        requestAnimationFrame(apply);
        return;
      }
      if (fit === null) return;

      const distance = MathUtils.clamp(fit / PRESET_ZOOM[preset], MIN_DISTANCE, MAX_DISTANCE);
      perspective.position.copy(target).addScaledVector(eyeUnit, distance);
      perspective.lookAt(target);
      perspective.updateProjectionMatrix();
      perspective.updateMatrixWorld();
      const orbit = controls as OrbitLike | null;
      if (orbit?.target) {
        orbit.target.set(...CAMERA_CONSTRAINTS.target);
        orbit.update?.();
      }
      invalidate();
    };

    apply();
    return () => {
      cancelled = true;
    };
  }, [camera, controls, invalidate, preset, scene, fitKey]);
  return null;
}

export function LiveSceneCanvas({
  view,
  preset,
  onContextLost,
}: {
  view: SceneViewModel;
  preset: ViewPreset;
  onContextLost: () => void;
}) {
  const sceneTheme = useSceneTheme();
  return (
    <Canvas
      className="scene-canvas"
      resize={{ offsetSize: true }}
      frameloop="demand"
      dpr={[1, 1.75]}
      gl={{ alpha: true, antialias: false, powerPreference: "high-performance" }}
      onCreated={({ gl, invalidate }) => {
        gl.setClearColor(0x000000, 0);
        gl.domElement.addEventListener(
          "webglcontextlost",
          (event: Event) => {
            event.preventDefault();
            onContextLost();
          },
          { once: true },
        );
        invalidate();
      }}
    >
      <PerspectiveCamera
        makeDefault
        fov={FIT_FOV}
        near={20}
        far={6000}
        position={CAMERA_PRESETS.iso}
      />
      <OrbitControls
        makeDefault
        enableDamping
        dampingFactor={0.08}
        enablePan={false}
        minPolarAngle={CAMERA_CONSTRAINTS.minPolarAngle}
        maxPolarAngle={CAMERA_CONSTRAINTS.maxPolarAngle}
        minDistance={MIN_DISTANCE}
        maxDistance={MAX_DISTANCE}
        target={CAMERA_CONSTRAINTS.target}
      />
      {/* After the bench: the rig measures the scene, so it has to run once the
          geometry is in the graph. */}
      <EquipmentScene view={view} theme={sceneTheme} />
      <CameraRig preset={preset} />
    </Canvas>
  );
}
