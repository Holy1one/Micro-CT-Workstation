/**
 * Decorative stage, floor, backdrop, and lighting geometry for the 3D view.
 * All exported behavior is visual. Optical equipment positions remain in
 * scene-config.ts and safety state remains outside this module.
 */

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
export function StageSurroundings({ theme }: { theme: SceneTheme }) {
  const night = theme.stageGlow < 0.3;
  return (
    <group>
      <mesh position={[0, TABLE_Y - 3, 0]} rotation={[-Math.PI / 2, 0, 0]} receiveShadow userData={{ excludeFromFit: true }}>
        <circleGeometry args={[STAGE_RADIUS, 96]} />
        <meshStandardMaterial
          color={theme.stageTable}
          map={CLOTH_MAP}
          emissive={theme.stageTable}
          emissiveMap={CLOTH_MAP}
          emissiveIntensity={theme.stageGlow}
          roughness={0.95}
          metalness={0}
          roughnessMap={CLOTH_MAP}
          bumpMap={CLOTH_MAP}
          bumpScale={1.5}
        />
      </mesh>
      {/* The finite tray participates in camera fit; only the room shell is
          excluded. The rail's 432-unit outer edge remains clear. */}
      <group>
        <mesh position={[0, TABLE_Y - 6, 0]}>
          <cylinderGeometry args={[GARDEN_RADIUS + 10, GARDEN_RADIUS + 4, 12, 64]} />
          <meshStandardMaterial color={night ? "#394e57" : "#eee4d1"} roughness={0.95} />
        </mesh>
        <mesh position={[0, TABLE_Y + 0.5, 0]} rotation={[-Math.PI / 2, 0, 0]}>
          <ringGeometry args={[465, GARDEN_RADIUS, 64]} />
          <meshStandardMaterial color={night ? "#485b61" : "#b7c69c"} roughness={1} />
        </mesh>
        <mesh position={[0, TABLE_Y + 1, 0]} rotation={[-Math.PI / 2, 0, 0]}>
          <ringGeometry args={[GARDEN_RADIUS - 23, GARDEN_RADIUS + 10, 64]} />
          <meshStandardMaterial color={night ? "#82919a" : "#efe5cf"} roughness={0.9} />
        </mesh>
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
          emissive={theme.stageWall}
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
