/**
 * Decorative stage, floor, backdrop, and lighting geometry for the 3D view.
 * All exported behavior is visual. Optical equipment positions remain in
 * scene-config.ts and safety state remains outside this module.
 */

import { useEffect } from "react";
import { useThree } from "@react-three/fiber";
import * as THREE from "three";
import { RING_TRACK } from "./scene-config";
import type { SceneTheme } from "./types";

// The stage: a fabric-covered table under a dome whose shell carries the wall
// paper. One half sphere instead of a wall plus a ceiling, so there is no seam
// to leak through - whichever way the camera looks, including straight up, it is
// looking at the inside of the same shell.
//
const STAGE_RADIUS = 4000;
const GARDEN_RADIUS = 560;
const TABLE_Y = RING_TRACK.bottomY;
const POND_RADIUS = 310;
const POND_Y = TABLE_Y - 27;
const PLINTH_BOTTOM_Y = TABLE_Y - 136;
export const STAGE_SHADOW_Y = PLINTH_BOTTOM_Y - 11;
const DRAIN_ANGLES = [0.36, 1.13] as const;
const DRAIN_HALF_ANGLE = 0.055;
const WALL_ARCS = [
  [0, DRAIN_ANGLES[0] - DRAIN_HALF_ANGLE],
  [DRAIN_ANGLES[0] + DRAIN_HALF_ANGLE, DRAIN_ANGLES[1] - DRAIN_HALF_ANGLE],
  [DRAIN_ANGLES[1] + DRAIN_HALF_ANGLE, Math.PI * 2],
] as const;
// RingGeometry starts at +X; CylinderGeometry starts at +Z. Both arc sets
// therefore describe the same two radial openings in world coordinates.
const DECK_DRAIN_ANGLES = DRAIN_ANGLES.map((angle) => (angle - Math.PI / 2 + Math.PI * 2) % (Math.PI * 2)).sort((a, b) => a - b);
const DECK_ARCS = [
  [0, DECK_DRAIN_ANGLES[0] - DRAIN_HALF_ANGLE],
  [DECK_DRAIN_ANGLES[0] + DRAIN_HALF_ANGLE, DECK_DRAIN_ANGLES[1] - DRAIN_HALF_ANGLE],
  [DECK_DRAIN_ANGLES[1] + DRAIN_HALF_ANGLE, Math.PI * 2],
] as const;

const SHRUB_POSITIONS: readonly [number, number, number][] = [
  [-480, -180, 18], [-500, 20, 21], [-430, 285, 16],
  [430, -285, 20], [500, -20, 19], [480, 180, 17],
  [-230, 470, 15], [230, 470, 18], [-230, -470, 16], [230, -470, 17],
];
const PEBBLE_ANGLES = [-168, -125, -80, -36, 12, 54, 103, 147] as const;
const TREE_POSITIONS: readonly [number, number][] = [[-360, -370], [370, -360], [430, 295]];

function Shrub({ x, z, size, night }: { x: number; z: number; size: number; night: boolean }) {
  return (
    <group position={[x, TABLE_Y, z]}>
      <mesh position={[0, size * 0.42, 0]} castShadow={false}>
        <icosahedronGeometry args={[size, 0]} />
        <meshStandardMaterial color={night ? "#647c77" : "#80a775"} flatShading roughness={1} />
      </mesh>
      <mesh position={[size * 0.62, size * 0.29, size * 0.2]}>
        <icosahedronGeometry args={[size * 0.67, 0]} />
        <meshStandardMaterial color={night ? "#4e6967" : "#6d956c"} flatShading roughness={1} />
      </mesh>
    </group>
  );
}

function MiniatureTree({ x, z, night }: { x: number; z: number; night: boolean }) {
  return (
    <group position={[x, TABLE_Y, z]}>
      <mesh position={[0, 20, 0]}>
        <cylinderGeometry args={[4, 5, 40, 6]} />
        <meshStandardMaterial color={night ? "#594e49" : "#98785d"} roughness={1} />
      </mesh>
      <mesh position={[0, 49, 0]}>
        <coneGeometry args={[22, 51, 7]} />
        <meshStandardMaterial color={night ? "#405f5e" : "#78aa80"} flatShading roughness={1} />
      </mesh>
      <mesh position={[0, 68, 0]}>
        <coneGeometry args={[15, 39, 7]} />
        <meshStandardMaterial color={night ? "#507070" : "#91bd90"} flatShading roughness={1} />
      </mesh>
    </group>
  );
}

/** A thin falling sheet. Its small colour map moves down with gravity while
 * the fixed ribs and outlet foam describe the steady shape of the flow. */
function Cascade({ angle, index, night }: { angle: number; index: number; night: boolean }) {
  const radius = GARDEN_RADIUS;
  const x = Math.sin(angle);
  const z = Math.cos(angle);
  const water = night ? "#528b9b" : "#73bac5";
  const glint = night ? "#a4cbd0" : "#e0f8ee";
  const fallHeight = POND_Y - PLINTH_BOTTOM_Y;
  return (
    <group>
      <group position={[x * 435, POND_Y - 1, z * 435]} rotation={[0, angle, 0]}>
        <mesh>
          <boxGeometry args={[42, 2.5, 254]} />
          <meshStandardMaterial color={water} roughness={0.25} metalness={0} />
        </mesh>
        <mesh position={[-12, 1.6, 0]}>
          <boxGeometry args={[4, 0.8, 248]} />
          <meshBasicMaterial color={glint} transparent opacity={0.7} />
        </mesh>
      </group>
      <group position={[x * (radius + 2), (POND_Y + PLINTH_BOTTOM_Y) / 2, z * (radius + 2)]} rotation={[0, angle, 0]}>
        <mesh>
          <planeGeometry args={[53, fallHeight]} />
          <meshStandardMaterial color={water} map={CASCADE_FLOW_MAPS[index]} side={THREE.DoubleSide} roughness={0.22} metalness={0} />
        </mesh>
        {[-17, -3, 13].map((offset, index) => (
          <mesh key={offset} position={[offset, 0, 0.8]}>
            <planeGeometry args={[index === 1 ? 3 : 5, fallHeight - 5]} />
            <meshBasicMaterial color={glint} side={THREE.DoubleSide} transparent opacity={index === 1 ? 0.5 : 0.28} depthWrite={false} />
          </mesh>
        ))}
      </group>
      <mesh position={[x * (radius + 27), PLINTH_BOTTOM_Y + 4, z * (radius + 27)]} rotation={[-Math.PI / 2, 0, 0]}>
        <circleGeometry args={[31, 16]} />
        <meshStandardMaterial color={water} roughness={0.3} metalness={0} />
      </mesh>
      <mesh position={[x * (radius + 27), PLINTH_BOTTOM_Y + 5, z * (radius + 27)]} rotation={[-Math.PI / 2, 0, 0]}>
        <ringGeometry args={[21, 25, 20]} />
        <meshBasicMaterial color={glint} transparent opacity={0.65} depthWrite={false} />
      </mesh>
    </group>
  );
}

/** Small, deterministic textures stay resident for the lifetime of the scene.
 * Only their UV offsets change; animation never uploads a new image. */
function waterTexture(size: number, pixel: (u: number, v: number) => [number, number, number], repeat: [number, number], color: boolean): THREE.DataTexture {
  const data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const [r, g, b] = pixel(x / size, y / size);
      const offset = (y * size + x) * 4;
      data[offset] = Math.round(r * 255);
      data[offset + 1] = Math.round(g * 255);
      data[offset + 2] = Math.round(b * 255);
      data[offset + 3] = 255;
    }
  }
  const texture = new THREE.DataTexture(data, size, size, THREE.RGBAFormat);
  texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
  texture.repeat.set(...repeat);
  texture.magFilter = THREE.LinearFilter;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  if (color) texture.colorSpace = THREE.SRGBColorSpace;
  texture.needsUpdate = true;
  return texture;
}

const POND_NORMAL_MAP = waterTexture(128, (u, v) => {
  // The normal is the derivative of three low-amplitude travelling waves.
  const first = Math.PI * 2 * (2 * u + v);
  const second = Math.PI * 2 * (u - 3 * v);
  const third = Math.PI * 2 * (5 * u - 2 * v);
  const slopeU = Math.PI * 2 * (0.008 * 2 * Math.cos(first) + 0.004 * Math.cos(second) + 0.002 * 5 * Math.cos(third));
  const slopeV = Math.PI * 2 * (0.008 * Math.cos(first) - 0.004 * 3 * Math.cos(second) - 0.002 * 2 * Math.cos(third));
  const inverseLength = 1 / Math.hypot(slopeU, slopeV, 1);
  return [(1 - slopeU * inverseLength) / 2, (1 - slopeV * inverseLength) / 2, (1 + inverseLength) / 2];
}, [1, 1], false);

const POND_DEPTH_MAP = waterTexture(64, (u, v) => {
  const radius = Math.min(1, Math.hypot(u - 0.5, v - 0.5) * 2);
  const brightness = 0.68 + 0.28 * radius ** 1.5;
  return [brightness, brightness, brightness];
}, [1, 1], true);
POND_DEPTH_MAP.wrapS = POND_DEPTH_MAP.wrapT = THREE.ClampToEdgeWrapping;

const CASCADE_FLOW_MAPS = [0, 1].map(() => waterTexture(64, (u, v) => {
  const streak = 0.5 + 0.5 * Math.cos(Math.PI * 2 * (6 * u + 0.35 * Math.sin(Math.PI * 4 * v)));
  const brokenFoam = 0.5 + 0.5 * Math.sin(Math.PI * 2 * (7 * v + 2 * u));
  const brightness = 0.56 + 0.3 * streak + 0.14 * brokenFoam;
  return [brightness, brightness, brightness];
}, [1, 2], true));

/** Builds a tiling greyscale grain map. Grey-only on purpose: the surface takes
 *  its colour from `material.color`, so the same map survives a theme switch and
 *  only darkens or lightens with it. It is served as both a roughness map and a
 *  bump map, which is why the mean has to sit near 1 - otherwise every surface
 *  inherits a roughness offset from the texture instead of its own setting. */
function createGrainMap(pattern: (ctx: CanvasRenderingContext2D, size: number) => void, repeat: [number, number]): THREE.Texture {
  const size = 256;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (ctx) pattern(ctx, size);
  const texture = new THREE.CanvasTexture(canvas);
  texture.wrapS = THREE.RepeatWrapping;
  texture.wrapT = THREE.RepeatWrapping;
  texture.repeat.set(repeat[0], repeat[1]);
  texture.anisotropy = 4;
  return texture;
}

/** Per-pixel jitter. Applied last so it sits under the pattern. */
function addGrain(ctx: CanvasRenderingContext2D, size: number, amount: number): void {
  const image = ctx.getImageData(0, 0, size, size);
  for (let index = 0; index < image.data.length; index += 4) {
    const jitter = (Math.random() - 0.5) * amount;
    image.data[index] += jitter;
    image.data[index + 1] += jitter;
    image.data[index + 2] += jitter;
  }
  ctx.putImageData(image, 0, 0);
}

// Cloth: warp and weft drawn as alternating stripes, one dark and one light, so
// the weave reads as relief once the same map is used as a bump map rather than
// as a flat print. Eight tiles across a 3000-unit table puts a thread roughly
// every 6 units - visible from the presets, not readable as stripes.
const CLOTH_MAP = createGrainMap((ctx, size) => {
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, size, size);
  const step = 4;
  ctx.fillStyle = "rgba(0,0,0,0.035)";
  for (let x = 0; x < size; x += step) ctx.fillRect(x, 0, 2, size);
  ctx.fillStyle = "rgba(255,255,255,0.035)";
  for (let y = 0; y < size; y += step) ctx.fillRect(0, y, size, 2);
  addGrain(ctx, size, 5);
}, [8, 8]);

// Paper: soft blotches rather than lines. Radial gradients instead of hard
// circles - a wall covering has no visible repeat, and at this scale a regular
// pattern would turn into moire long before it read as texture.
const PAPER_MAP = createGrainMap((ctx, size) => {
  ctx.fillStyle = "#f2f2f2";
  ctx.fillRect(0, 0, size, size);
  for (let index = 0; index < 180; index += 1) {
    const x = Math.random() * size;
    const y = Math.random() * size;
    const radius = 20 + Math.random() * 70;
    const tint = Math.random() > 0.5 ? "255,255,255" : "0,0,0";
    const gradient = ctx.createRadialGradient(x, y, 0, x, y, radius);
    gradient.addColorStop(0, `rgba(${tint},0.05)`);
    gradient.addColorStop(1, `rgba(${tint},0)`);
    ctx.fillStyle = gradient;
    ctx.beginPath();
    ctx.arc(x, y, radius, 0, Math.PI * 2);
    ctx.fill();
  }
  addGrain(ctx, size, 10);
}, [10, 3]);

/** A quiet high-to-low wash. Canvas values stay close to white so this map
 *  modulates the token colour without tinting it or competing with the scene. */
const BACKDROP_GRADIENT_MAP = createGrainMap((ctx, size) => {
  const gradient = ctx.createLinearGradient(0, 0, 0, size);
  gradient.addColorStop(0, "#d8d8d8");
  // SphereGeometry's upper half uses texture V=1 at its pole and V=0.5 at the
  // horizon. Finish the wash over that used half instead of wasting it below.
  gradient.addColorStop(0.5, "#ffffff");
  gradient.addColorStop(1, "#ffffff");
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, size, size);
  addGrain(ctx, size, 1.5);
}, [1, 1]);

/** The oversized floor gets a broad centre lift and gentle edge falloff. It is
 *  an emissive multiplier only: the existing fabric albedo and relief remain
 *  responsible for the ground's material character. */
function createGroundFalloffMap(edge: string): THREE.Texture {
  const size = 512;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  if (ctx) {
    const gradient = ctx.createRadialGradient(size / 2, size / 2, size * 0.02, size / 2, size / 2, size * 0.26);
    gradient.addColorStop(0, "#ffffff");
    gradient.addColorStop(0.38, "#fafafa");
    gradient.addColorStop(1, edge);
    ctx.fillStyle = gradient;
    ctx.fillRect(0, 0, size, size);
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.wrapS = THREE.ClampToEdgeWrapping;
  texture.wrapT = THREE.ClampToEdgeWrapping;
  texture.anisotropy = 4;
  return texture;
}

const GROUND_FALLOFF_LIGHT = createGroundFalloffMap("#e6e6e6");
const GROUND_FALLOFF_DARK = createGroundFalloffMap("#ededed");

/** The room the bench stands in: cloth table under a papered dome. Nothing here
 *  participates in the camera fit (see `sceneBounds`) - it is an order of
 *  magnitude wider than the bench, and fitting to it would frame the shell
 *  instead of the machine.
 *
 *  Emissive carries most of the brightness here, and that is not a shortcut - it
 *  is the only way these surfaces can be white at all. Physically lit, a wall
 *  takes the hemisphere light's ground colour (its inward normal points down: the
 *  hemisphere term is a 50/50 sky/ground mix), and three divides directional
 *  contributions by PI, so the light theme lands somewhere near 0.3 linear no
 *  matter how bright the albedo is. Pushing colour towards white to compensate
 *  clips the surface flat instead. The glow adds its light directly and touches
 *  nothing else, which a real light could not do - a light in the scene lights
 *  the machine too.
 *
 *  The same grain map serves as map, roughnessMap, bumpMap and emissiveMap, so
 *  the cloth and the paper still read once the surface is this bright: the weave
 *  is carried by the colour it multiplies as well as by the relief. */
export function StageSurroundings({ theme, reducedMotion }: { theme: SceneTheme; reducedMotion: boolean }) {
  const night = theme.stageGlow < 0.3;
  const { scene, invalidate } = useThree();
  useEffect(() => {
    const previousFog = scene.fog;
    const fog = new THREE.FogExp2(theme.stageWall, night ? 0.00003 : 0.00004);
    scene.fog = fog;
    invalidate();
    return () => {
      if (scene.fog === fog) scene.fog = previousFog;
      invalidate();
    };
  }, [invalidate, night, scene, theme.stageWall]);

  useEffect(() => {
    if (reducedMotion) return;
    let timer: number | null = null;
    let elapsed = 0;
    let previous = performance.now();
    const tick = () => {
      const now = performance.now();
      elapsed += Math.min((now - previous) / 1000, 0.1);
      previous = now;
      POND_NORMAL_MAP.offset.set((elapsed * 0.018) % 1, (elapsed * 0.011) % 1);
      CASCADE_FLOW_MAPS.forEach((map, index) => { map.offset.y = (elapsed * 0.8 + index * 0.37) % 1; });
      invalidate();
    };
    const syncVisibility = () => {
      if (document.visibilityState === "visible") {
        if (timer === null) {
          previous = performance.now();
          timer = window.setInterval(tick, 50);
        }
      } else if (timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
    syncVisibility();
    document.addEventListener("visibilitychange", syncVisibility);
    return () => {
      document.removeEventListener("visibilitychange", syncVisibility);
      if (timer !== null) window.clearInterval(timer);
    };
  }, [invalidate, reducedMotion]);

  return (
    <group>
      <mesh position={[0, STAGE_SHADOW_Y - 1, 0]} rotation={[-Math.PI / 2, 0, 0]} receiveShadow userData={{ excludeFromFit: true }}>
        <circleGeometry args={[STAGE_RADIUS, 96]} />
        <meshStandardMaterial
          color={theme.stageTable}
          map={CLOTH_MAP}
          emissive={theme.stageTable}
          emissiveMap={night ? GROUND_FALLOFF_DARK : GROUND_FALLOFF_LIGHT}
          emissiveIntensity={theme.stageGlow}
          roughness={0.95}
          metalness={0}
          roughnessMap={CLOTH_MAP}
          bumpMap={CLOTH_MAP}
          bumpScale={1.5}
        />
      </mesh>
      {/* The raised miniature landscape participates in camera fit. The rail
          and optical equipment keep their original coordinates above it. */}
      <group>
        <mesh position={[0, PLINTH_BOTTOM_Y - 5, 0]}>
          <cylinderGeometry args={[GARDEN_RADIUS + 38, GARDEN_RADIUS + 41, 10, 48]} />
          <meshStandardMaterial color={night ? "#303f47" : "#b4aa93"} roughness={1} flatShading />
        </mesh>
        {WALL_ARCS.map(([start, end]) => (
          <mesh key={start} position={[0, (TABLE_Y + PLINTH_BOTTOM_Y) / 2, 0]} receiveShadow>
            <cylinderGeometry args={[GARDEN_RADIUS, GARDEN_RADIUS + 8, TABLE_Y - PLINTH_BOTTOM_Y, 48, 1, true, start, end - start]} />
            <meshStandardMaterial color={night ? "#526063" : "#c7baa3"} roughness={1} flatShading side={THREE.DoubleSide} />
          </mesh>
        ))}
        {DECK_ARCS.map(([start, end]) => (
          <group key={start}>
            <mesh position={[0, TABLE_Y + 0.3, 0]} rotation={[-Math.PI / 2, 0, 0]} receiveShadow>
              <ringGeometry args={[POND_RADIUS, GARDEN_RADIUS, 64, 1, start, end - start]} />
              <meshStandardMaterial color={night ? "#586966" : "#c6cbb0"} roughness={1} flatShading />
            </mesh>
            <mesh position={[0, TABLE_Y + 0.8, 0]} rotation={[-Math.PI / 2, 0, 0]}>
              <ringGeometry args={[455, GARDEN_RADIUS - 12, 64, 1, start, end - start]} />
              <meshStandardMaterial color={night ? "#617a6c" : "#97ae7e"} roughness={1} flatShading />
            </mesh>
          </group>
        ))}
        {WALL_ARCS.map(([start, end]) => (
          <mesh key={start} position={[0, (TABLE_Y + POND_Y) / 2, 0]}>
            <cylinderGeometry args={[POND_RADIUS, POND_RADIUS - 12, TABLE_Y - POND_Y, 48, 1, true, start, end - start]} />
            <meshStandardMaterial color={night ? "#66736f" : "#adac91"} roughness={1} flatShading side={THREE.DoubleSide} />
          </mesh>
        ))}
        <mesh position={[0, POND_Y, 0]} rotation={[-Math.PI / 2, 0, 0]}>
          <circleGeometry args={[POND_RADIUS - 3, 64]} />
          <meshStandardMaterial color={night ? "#376b76" : "#669ea8"} map={POND_DEPTH_MAP} normalMap={POND_NORMAL_MAP} normalScale={new THREE.Vector2(0.18, 0.18)} roughness={0.38} metalness={0} />
        </mesh>
        <mesh position={[0, (TABLE_Y + POND_Y) / 2, 0]}>
          <cylinderGeometry args={[55, 67, TABLE_Y - POND_Y, 16]} />
          <meshStandardMaterial color={night ? "#6a746b" : "#c5bea3"} roughness={1} flatShading />
        </mesh>
        <mesh position={[0, TABLE_Y + 0.5, 0]} rotation={[-Math.PI / 2, 0, 0]}>
          <circleGeometry args={[55, 16]} />
          <meshStandardMaterial color={night ? "#7d8474" : "#d4ceb5"} roughness={1} />
        </mesh>
        {DRAIN_ANGLES.map((angle, index) => <Cascade key={angle} angle={angle} index={index} night={night} />)}
        {SHRUB_POSITIONS.map(([x, z, size]) => (
          <Shrub key={`${x}:${z}`} x={x} z={z} size={size} night={night} />
        ))}
        {TREE_POSITIONS.map(([x, z]) => (
          <MiniatureTree key={`${x}:${z}`} x={x} z={z} night={night} />
        ))}
        {PEBBLE_ANGLES.map((degrees) => {
          const radians = degrees * Math.PI / 180;
          return (
            <mesh key={degrees} position={[Math.sin(radians) * 514, TABLE_Y + 1.1, Math.cos(radians) * 514]}>
              <cylinderGeometry args={[8, 10, 2, 6]} />
              <meshStandardMaterial color={night ? "#89928e" : "#f8efe0"} roughness={1} />
            </mesh>
          );
        })}
        {/* The lamp is outside the rail and illuminates only a small garden
            patch; it never stands in the source/sample/detector corridor. */}
        <group position={[-510, TABLE_Y, -140]}>
          <mesh position={[0, 73, 0]}>
            <cylinderGeometry args={[3, 5, 146, 8]} />
            <meshStandardMaterial color={night ? "#59636d" : "#66717a"} roughness={0.75} />
          </mesh>
          <mesh position={[0, 148, 0]}>
            <cylinderGeometry args={[15, 10, 8, 8]} />
            <meshStandardMaterial color={night ? "#a98564" : "#8b8274"} roughness={0.8} />
          </mesh>
          <mesh position={[0, 140, 0]}>
            <sphereGeometry args={[9, 8, 6]} />
            <meshStandardMaterial color={night ? "#ffdda1" : "#f6e2b9"} emissive={night ? "#ffc879" : "#000000"} emissiveIntensity={night ? 0.75 : 0} />
          </mesh>
          {night && (
            <>
              <mesh position={[0, 1.6, 0]} rotation={[-Math.PI / 2, 0, 0]}>
                <circleGeometry args={[82, 24]} />
                <meshBasicMaterial color="#e9b87c" transparent opacity={0.11} depthWrite={false} />
              </mesh>
              <pointLight position={[0, 136, 0]} color="#ffc58a" intensity={1.9} distance={270} decay={2} />
            </>
          )}
        </group>
      </group>
      {/* Upper half only: theta sweep of PI/2 from the pole down to the rim,
          which lands the rim exactly on the table edge. */}
      <mesh position={[0, TABLE_Y, 0]} userData={{ excludeFromFit: true }}>
        <sphereGeometry args={[STAGE_RADIUS, 96, 48, 0, Math.PI * 2, 0, Math.PI / 2]} />
        <meshStandardMaterial
          color={theme.stageWall}
          map={BACKDROP_GRADIENT_MAP}
          emissive={theme.stageWall}
          emissiveMap={BACKDROP_GRADIENT_MAP}
          emissiveIntensity={theme.stageGlow}
          roughness={1}
          metalness={0}
          roughnessMap={PAPER_MAP}
          bumpMap={PAPER_MAP}
          bumpScale={0.6}
          side={THREE.BackSide}
        />
      </mesh>
    </group>
  );
}
