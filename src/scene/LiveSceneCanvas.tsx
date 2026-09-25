/**
 * Three.js canvas host and bounded camera controller for the equipment scene.
 * It fits the visual model, applies view presets, and reports context loss to
 * the fallback hook without sending any device command.
 */

import { OrbitControls, OrthographicCamera } from "@react-three/drei";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { useEffect, useRef, useState } from "react";
import { OrthographicCamera as OrthographicCameraImpl, PCFSoftShadowMap, Vector3 } from "three";
import { EquipmentScene } from "./EquipmentScene";
import { CAMERA_CONSTRAINTS, CAMERA_PRESETS } from "./scene-config";
import { computeFitZoom, FRUSTUM_HEIGHT } from "./scene-fit";
import { useSceneTheme } from "./theme-three";
import type { SceneViewModel, ViewPreset } from "./types";

/** Leave room at the top for the centered view dock and at the edges for the ring. */
const VIEW_CENTER_SHIFT = 0.05;

/** The subset of OrbitControls this rig touches, kept local so the store's
 *  untyped `controls` slot stays honest at the call sites. */
type OrbitLike = {
  target?: { set: (x: number, y: number, z: number) => void };
  minZoom?: number;
  maxZoom?: number;
  update?: () => void;
  addEventListener?: (type: string, listener: () => void) => void;
  removeEventListener?: (type: string, listener: () => void) => void;
};

type SceneFeedback = {
  state: string;
  detail: string;
};

/** Keep the browser's reduced-motion preference in sync with Three.js. */
function usePrefersReducedMotion(): boolean {
  const [reducedMotion, setReducedMotion] = useState(() =>
    typeof window !== "undefined"
      && window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );

  useEffect(() => {
    const preference = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = (): void => setReducedMotion(preference.matches);
    update();
    if (typeof preference.addEventListener === "function") {
      preference.addEventListener("change", update);
      return () => preference.removeEventListener("change", update);
    }
    preference.addListener(update);
    return () => preference.removeListener(update);
  }, []);

  return reducedMotion;
}

/** Build a single static shadow map; the track and carriages do not move. */
function SceneCanvasSettings({ reducedMotion }: { reducedMotion: boolean }) {
  const { gl, invalidate } = useThree();
  useEffect(() => {
    gl.shadowMap.enabled = true;
    gl.shadowMap.type = PCFSoftShadowMap;
    gl.shadowMap.autoUpdate = false;
    gl.shadowMap.needsUpdate = true;
    gl.domElement.dataset.prefersReducedMotion = String(reducedMotion);
    gl.domElement.dataset.orbitDamping = String(!reducedMotion);
    gl.domElement.dataset.sceneShadowMap = String(gl.shadowMap.enabled);
    invalidate();
  }, [gl, invalidate, reducedMotion]);
  return null;
}

function CameraRig({ preset, presetRevision }: { preset: ViewPreset; presetRevision: number }) {
  const { camera, controls, invalidate, scene, size, gl } = useThree();
  useFrame(() => { gl.domElement.dataset.cameraPosition = camera.position.toArray().join(","); });
  /** Rounded viewport key. The live panel is a flex child whose neighbours keep
   *  changing height, and the whole layout is CSS-zoomed, so the ResizeObserver
   *  reports sub-pixel size changes constantly. Rounding collapses that jitter:
   *  re-framing on every report yanked the camera back to the preset mid-orbit. */
  const fitKey = `${Math.round(size.width)}x${Math.round(size.height)}`;
  /** Once the user orbits, the camera is theirs until they pick a preset again. */
  const userOrbited = useRef(false);
  const lastPreset = useRef<string | null>(null);

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
    const presetKey = preset + ":" + presetRevision;
    const presetChanged = lastPreset.current !== presetKey;
    lastPreset.current = presetKey;
    if (presetChanged) {
      // A preset click is an explicit request to re-frame, so it also clears the
      // manual-orbit lock and re-enables auto-fit on later resizes.
      userOrbited.current = false;
    } else if (userOrbited.current) {
      // Resize (or any size report) must not fight the user's current view.
      return;
    }

    const orthographic = camera as OrthographicCameraImpl;
    const target = new Vector3(...CAMERA_CONSTRAINTS.target);
    const eyeDirection = new Vector3(...CAMERA_PRESETS[preset]).sub(target);

    let cancelled = false;
    let retried = false;

    const apply = () => {
      if (cancelled) return;
      const halfWidth = FRUSTUM_HEIGHT * size.width / Math.max(size.height, 1) / 2;
      orthographic.left = -halfWidth;
      orthographic.right = halfWidth;
      orthographic.position.copy(target).add(eyeDirection);
      orthographic.lookAt(target);
      orthographic.updateMatrixWorld();

      const fit = computeFitZoom(scene, orthographic, size.width, size.height);
      // The bench may not be attached to the scene graph on the first pass, and
      // framing an empty box would drop the camera onto the preset distance and
      // leave it there. Give it one more frame before trusting the result.
      if (fit === null && !retried) {
        retried = true;
        requestAnimationFrame(apply);
        return;
      }
      if (fit === null) return;

      orthographic.zoom = fit;
      // Moving the camera and orbit target upward puts the equipment below the
      // dock while retaining the same preset direction and orbit pivot.
      const screenUp = new Vector3(0, 1, 0).applyQuaternion(orthographic.quaternion);
      const offset = screenUp.multiplyScalar(FRUSTUM_HEIGHT * VIEW_CENTER_SHIFT / fit);
      orthographic.position.add(offset);
      target.add(offset);
      orthographic.updateProjectionMatrix();
      orthographic.updateMatrixWorld();
      const orbit = controls as OrbitLike | null;
      if (orbit?.target) {
        orbit.minZoom = fit * CAMERA_CONSTRAINTS.minZoom;
        orbit.maxZoom = fit * CAMERA_CONSTRAINTS.maxZoom;
        orbit.target.set(target.x, target.y, target.z);
        orbit.update?.();
      }
      invalidate();
    };

    apply();
    return () => {
      cancelled = true;
    };
  }, [camera, controls, invalidate, preset, presetRevision, scene, fitKey]);
  return null;
}

export function LiveSceneCanvas({
  view,
  status,
  preset,
  presetRevision = 0,
  onContextLost,
}: {
  view: SceneViewModel;
  status: SceneFeedback;
  preset: ViewPreset;
  presetRevision?: number;
  onContextLost: () => void;
}) {
  const sceneTheme = useSceneTheme();
  const reducedMotion = usePrefersReducedMotion();
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
      <SceneCanvasSettings reducedMotion={reducedMotion} />
      <OrthographicCamera
        makeDefault
        near={-9000}
        far={9000}
        top={FRUSTUM_HEIGHT / 2}
        bottom={-FRUSTUM_HEIGHT / 2}
        left={-FRUSTUM_HEIGHT / 2}
        right={FRUSTUM_HEIGHT / 2}
        position={CAMERA_PRESETS.iso}
      />
      <OrbitControls
        makeDefault
        enableDamping={!reducedMotion}
        dampingFactor={0.08}
        enablePan={false}
        minPolarAngle={CAMERA_CONSTRAINTS.minPolarAngle}
        maxPolarAngle={CAMERA_CONSTRAINTS.maxPolarAngle}
        minZoom={0.1}
        maxZoom={5}
        target={CAMERA_CONSTRAINTS.target}
      />
      {/* After the bench: the rig measures the scene, so it has to run once the
          geometry is in the graph. */}
      <EquipmentScene view={view} status={status} reducedMotion={reducedMotion} theme={sceneTheme} />
      <CameraRig preset={preset} presetRevision={presetRevision} />
    </Canvas>
  );
}
