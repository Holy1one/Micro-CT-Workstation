import * as THREE from "three";
import type { SceneTheme } from "./types";

// The stage: a fabric-covered table under a dome whose shell carries the wall
// paper. One half sphere instead of a wall plus a ceiling, so there is no seam
// to leak through - whichever way the camera looks, including straight up, it is
// looking at the inside of the same shell.
//
// The radius is pinned to the camera, not chosen on its own. `computeFitDistance`
// slides the camera back until every corner of the bench clears the frustum, and
// that distance tracks the viewport: on a wide canvas it lands near 940, but the
// horizontal fov shrinks with the aspect ratio, so a narrow canvas pushes it past
// 1300. The dome has to stay outside the orbit limit or the camera ends up behind
// the BackSide shell, where nothing renders and the background simply disappears.
// MAX_DISTANCE in LiveSceneCanvas is read at build time against this number; the
// two have to move together.
const STAGE_RADIUS = 1500;
const TABLE_Y = -40;

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
  ctx.fillStyle = "rgba(0,0,0,0.12)";
  for (let x = 0; x < size; x += step) ctx.fillRect(x, 0, 2, size);
  ctx.fillStyle = "rgba(255,255,255,0.12)";
  for (let y = 0; y < size; y += step) ctx.fillRect(0, y, size, 2);
  addGrain(ctx, size, 16);
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
  return (
    <group>
      <mesh position={[0, TABLE_Y, 0]} rotation={[-Math.PI / 2, 0, 0]} userData={{ excludeFromFit: true }}>
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
