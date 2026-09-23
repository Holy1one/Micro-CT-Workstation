/**
 * Procedural visual model of the X-ray source, sample turntable, detector, and
 * camera. Beam visibility and turntable angle are read from SceneViewModel;
 * geometry here is illustrative and must never be used as hardware telemetry.
 */

import { Line, RoundedBox } from "@react-three/drei";
import { useFrame, useThree } from "@react-three/fiber";
import { useEffect, useMemo, useRef } from "react";
import { FeedbackAngle } from "./angle-feedback";
import * as THREE from "three";
import {
  CAMERA,
  CARRIAGE,
  OPTICAL_AXIS_Y,
  RING_TRACK,
  SAMPLE,
  SOURCE,
  TURNTABLE,
  trackContactPose,
} from "./scene-config";
import { StageSurroundings } from "./stage-surroundings";
import type { SceneTheme, SceneViewModel } from "./types";

// Optical axis: light grey, and deliberately kept out of the theme tokens so it
// stays legible against both the near-white and the near-black stage. Three only
// parses 3- or 6-digit hex - an 8-digit value is rejected outright and leaves the
// material at its default white, which vanishes on the light stage.
const AXIS_GREY = "#575a5e";

// Colour of the optical path when the tube is not emitting: a near-white haze
// that still marks where the beam would run without lighting the scene up. Its
// weight comes from the state tokens below, only the hue is fixed here.
const BEAM_PATH_COLOR = "#eef2f6";

// Nosepiece measurements, all taken forward from the housing front face: a
// 12-unit flange, a 64-unit barrel breaking into a short cone, and a 0.6-unit
// ceramic window sunk into the flat exit face. The beam leaves through that same
// exit plane, so the port is derived once here and the nosepiece and the cone
// both read from it instead of each carrying its own copy of the numbers.
const FLANGE_DEPTH = 12;
const NOSEPIECE_LENGTH = 64;
const WINDOW_DEPTH = 0.6;
const SOURCE_FRONT_Z = SOURCE.bodyCenter[2] + SOURCE.bodySize[2] / 2;
const SOURCE_FLANGE_FRONT_Z = SOURCE_FRONT_Z + FLANGE_DEPTH - 1;
const SOURCE_PORT_Z = SOURCE_FLANGE_FRONT_Z + NOSEPIECE_LENGTH + WINDOW_DEPTH;

// Cone geometry: the beam starts just inside the exit port - anchored back at
// the focal spot it spent its first 80 units buried in the brass and emerged
// through the cone wall already 45 units wide - and spreads from there at a full
// angle of 48 degrees, so the radius at the far end follows from the throw.
const BEAM_PORT_INSET = 5;
const BEAM_PORT_RADIUS = 1.2;
const BEAM_FULL_ANGLE_DEG = 48;

// The cone is drawn four times, nested, and each shell is stated as its own
// full angle in degrees and converted to an axis scale in Beam - angles are what
// one actually judges on screen. Splitting the difference in *scale* is wrong:
// the shell is scaled about the axis, so angle = 2*atan(scale * tan(24 deg)) and
// the midpoint in scale lands far outside the midpoint in angle. Hence the mid
// shell is averaged in angle instead: 48, (48 + 14.7)/2 = 31.4, 14.7. The core
// runs at half the 29 degrees it used to.
//
// Weight ramps in step with the shells: the outer one is a pale wash and each
// step inward is heavier, so the cone reads as a beam with a hot centre rather
// than rings of unrelated brightness. The core keeps the weight it had; the
// outer shell is lightened to a wash and the mid shell is interpolated
// between the two, which is what makes the density climb smoothly.
//
// The halo is the scatter term: a shell ~1.8x the cone, uselessly faint on its
// own, that puts a soft edged bloom around the beam. Stacked with the three
// shells it is what makes the beam read as light travelling through air (the
// Tyndall look) rather than a solid glass cone with three visible rims. It is
// excluded from the camera fit because it is wider than the machine and would
// otherwise push the framing back for a haze nobody is looking at.
const BEAM_CORE_ANGLE_DEG = 14.7;
const BEAM_CORE_WEIGHT = 0.98;
const BEAM_OUTER_WEIGHT = 0.3;
const BEAM_HALO_ANGLE_DEG = 77;
const BEAM_HALO_WEIGHT = 0.12;
const BEAM_LAYERS: { fullAngleDeg: number; weight: number; halo?: boolean }[] = [
  { fullAngleDeg: BEAM_HALO_ANGLE_DEG, weight: BEAM_HALO_WEIGHT, halo: true },
  { fullAngleDeg: BEAM_FULL_ANGLE_DEG, weight: BEAM_OUTER_WEIGHT },
  { fullAngleDeg: (BEAM_FULL_ANGLE_DEG + BEAM_CORE_ANGLE_DEG) / 2, weight: (BEAM_OUTER_WEIGHT + BEAM_CORE_WEIGHT) / 2 },
  { fullAngleDeg: BEAM_CORE_ANGLE_DEG, weight: BEAM_CORE_WEIGHT },
];

// The detector rig is modelled at its true size and then scaled about its front
// lens face: the barrel stays exactly where the beam expects to land and the
// assembly grows backwards along +Z, away from the sample.
const DETECTOR_SCALE = 1.3;
const DETECTOR_FRONT_Z = CAMERA.lensCenter[2] - CAMERA.lensLength / 2;

/** Maps a point of the modelled detector through that same scale-up, so helpers
 *  drawn against the rig - the optical axis rules through it - stay registered. */
function detectorPoint([x, y, z]: readonly [number, number, number]): [number, number, number] {
  return [
    x * DETECTOR_SCALE,
    OPTICAL_AXIS_Y + (y - OPTICAL_AXIS_Y) * DETECTOR_SCALE,
    DETECTOR_FRONT_Z + (z - DETECTOR_FRONT_Z) * DETECTOR_SCALE,
  ];
}

/**
 * Colour and weight of the cone for each machine state. All four states run the
 * same cone - only the weight changes, and it is read from the CSS token of the
 * same name, so the states are directly comparable:
 *   scanning + beam on -> the glow hue at --rayScanning, and only this state
 *                         actually emits: it is the one lit tube
 *   ready / paused / beam off -> the transparent near-white haze at
 *                         --rayReady / --rayPaused: the optical path is marked,
 *                         nothing is being emitted
 *   fault, or the interlock latched -> --rayFault (0), so nothing is drawn
 *
 * The haze matters more than it looks: with no cone at all in the idle states the
 * beam would simply pop into existence, and the operator loses the one cue that
 * says where the beam runs.
 *
 * Fault deliberately keeps the geometry mounted: the camera frames the scene from
 * the bounding box, so unmounting the cone - which is by far the widest thing on
 * the bench - would silently pull the camera in and change the composition for
 * that one state.
 */
function beamFace(view: SceneViewModel, theme: SceneTheme): { color: string; opacity: number } {
  if (view.xrayLatched || view.dataState === "fault") {
    return { color: theme.beam, opacity: theme.rayFault };
  }
  if (view.beamOn) {
    return { color: theme.beam, opacity: theme.rayScanning };
  }
  // The dashed optical axis remains visible; a luminous cone requires beam ON.
  return { color: BEAM_PATH_COLOR, opacity: 0 };
}

function Beam({ view, theme }: { view: SceneViewModel; theme: SceneTheme }) {
  const face = beamFace(view, theme);
  // The cone begins a hair inside the exit port and, with the scintillator gone,
  // terminates on the camera's front element; its radius at either end follows
  // from the throw and the full angle.
  const beamStartZ = SOURCE_PORT_Z - BEAM_PORT_INSET;
  const beamEndZ = CAMERA.lensCenter[2] - CAMERA.lensLength / 2;
  const length = beamEndZ - beamStartZ;
  const centerZ = beamStartZ + length / 2;
  const outerHalfTan = Math.tan(THREE.MathUtils.degToRad(BEAM_FULL_ANGLE_DEG / 2));
  const endRadius = BEAM_PORT_RADIUS + length * outerHalfTan;
  const { color, opacity } = face;
  return (
    // Rotation of +90 degrees about X swings the cylinder's top face onto +Z, so
    // the cone starts at the pinhole (radius 1.2 just inside the port at
    // z = -238.9) and opens along +Z onto the camera at 48 degrees full angle.
    // Hidden rather than unmounted at zero weight, so the framing stays put.
    <group position={[0, OPTICAL_AXIS_Y, centerZ]} rotation={[Math.PI / 2, 0, 0]} visible={opacity > 0}>
      {BEAM_LAYERS.map(({ fullAngleDeg, weight, halo }) => {
        // endRadius already encodes the outer shell's half angle, so a shell's
        // scale is just the ratio of the two half-angle tangents.
        const scale = Math.tan(THREE.MathUtils.degToRad(fullAngleDeg / 2)) / outerHalfTan;
        return (
          <mesh
            key={fullAngleDeg}
            scale={[scale, 1, scale]}
            userData={halo ? { excludeFromFit: true } : undefined}
          >
            <cylinderGeometry args={[endRadius, BEAM_PORT_RADIUS, length, 28, 1, true]} />
            <meshBasicMaterial
              color={color}
              transparent
              opacity={opacity * weight}
              blending={THREE.AdditiveBlending}
              depthWrite={false}
              side={THREE.DoubleSide}
            />
          </mesh>
        );
      })}
    </group>
  );
}

/**
 * Radiation trefoil as stencilled on the housing panels: a yellow disc carrying
 * three 60-degree black blades spaced 120 degrees apart, plus a black hub.
 *
 * The mark is modelled in the XY plane facing +Z, so a caller only has to
 * rotate the wrapping group onto the target face. `radius` is the yellow disc.
 */
function RadiationMark({ radius }: { radius: number }) {
  const bladeInner = radius * 0.3;
  const bladeOuter = radius * 0.84;
  const bladeAngle = Math.PI / 3;
  const ink = { color: "#191512", roughness: 0.62, metalness: 0.04 };

  return (
    <group>
      <mesh>
        <circleGeometry args={[radius, 48]} />
        <meshStandardMaterial color="#e5c02a" roughness={0.58} metalness={0.06} side={THREE.DoubleSide} />
      </mesh>
      {[60, 180, 300].map((thetaStart) => (
        <mesh key={thetaStart} position={[0, 0, 0.05]}>
          <ringGeometry
            args={[bladeInner, bladeOuter, 24, 1, THREE.MathUtils.degToRad(thetaStart), bladeAngle]}
          />
          <meshStandardMaterial {...ink} side={THREE.DoubleSide} />
        </mesh>
      ))}
      <mesh position={[0, 0, 0.05]}>
        <circleGeometry args={[radius * 0.18, 24]} />
        <meshStandardMaterial {...ink} side={THREE.DoubleSide} />
      </mesh>
    </group>
  );
}

/**
 * Micro-focus X-ray source, modelled after the reference hardware photograph.
 *
 * Reading the flat elevation into 3D: the optical axis runs along +Z (beam
 * travels towards the sample at z = 0), so the brass nosepiece must point along
 * +Z while the machined aluminium housing trails behind it.
 *
 * The housing is a low-profile box: half the configured SOURCE height and
 * re-centred on the optical axis, so the brass flange can span exactly the same
 * Y range and stay in full contact with the housing face.
 *
 *   housing (aluminium)  z: [-497, -313]   x: [-72, 72]   y: [-1.5, 61.5]
 *   flange (brass)       z: [-314, -302]   84 x 63 plate (height == housing)
 *   barrel (brass)       z: [-302, -262]   constant r 26, a plain cylinder
 *   nose shoulder        z: [-262, -238]   short, steep frustum r 26 -> 11
 *   exit face            flush at z = -238: brass rim, pale ceramic insert,
 *                                          pinhole
 *   X-ray focal spot     z: SOURCE.focus[2] = -320 (inside the barrel)
 *
 * The nosepiece keeps its full radius almost to the tip before breaking into the
 * cone, so the shoulder falls steeply in the front third of the barrel rather
 * than tapering gently from the flange.
 */
function SourceAssembly({ theme }: { theme: SceneTheme }) {
  const axisY = OPTICAL_AXIS_Y;
  const bodyCenter: [number, number, number] = [SOURCE.bodyCenter[0], axisY, SOURCE.bodyCenter[2]];
  const bodySize: [number, number, number] = [
    SOURCE.bodySize[0],
    SOURCE.bodySize[1] / 2,
    SOURCE.bodySize[2],
  ];
  const bodyX = bodyCenter[0];
  const bodyZ = bodyCenter[2];
  const bodyTop = bodyCenter[1] + bodySize[1] / 2;
  const frontZ = bodyZ + bodySize[2] / 2;
  const lidTop = bodyTop + 8.7;

  // Brass nosepiece, measured forward from the housing front face. The flange is
  // sunk 1 unit into the housing and its Y size matches the housing height.
  const flangeWidth = 84;
  const flangeHeight = bodySize[1];
  const flangeDepth = FLANGE_DEPTH;
  const flangeFrontZ = SOURCE_FLANGE_FRONT_Z;

  // Straight barrel, then a steep shoulder in the front third: the barrel holds
  // its full radius for 40 units and the cone covers the remaining 24, so the
  // taper reads as a machined shoulder instead of a gently narrowing horn.
  // Barrel radii stay below the flange half-height so nothing pokes out of it.
  const barrelR = 26;
  const coneEndZ = flangeFrontZ + NOSEPIECE_LENGTH;
  const shoulderZ = coneEndZ - 24;
  const coneTipR = 11;

  // Flat exit face at the cone tip: brass rim, pale ceramic window insert sunk
  // flush with it, and the pinhole the beam leaves through.
  const windowR = 6.2;
  const windowDepth = WINDOW_DEPTH;
  const noseFaceZ = SOURCE_PORT_Z;

  // Without an environment map metals only pick up specular highlights, so the
  // metalness values stay moderate to keep the surfaces readable.
  const aluBody = { color: theme.chassis, roughness: 0.5, metalness: 0.32 };
  const aluPlate = { color: theme.panelHeader, roughness: 0.45, metalness: 0.35 };
  const brass = { color: "#cba558", roughness: 0.3, metalness: 0.62 };
  const darkPart = { color: theme.well, roughness: 0.7, metalness: 0.22 };
  const steel = { color: theme.sceneLine, roughness: 0.32, metalness: 0.5 };
  const ceramic = { color: "#ddd7c6", roughness: 0.52, metalness: 0.04 };

  const lidBoltPositions: [number, number, number][] = [
    [-54, lidTop + 0.2, bodyZ - 72],
    [54, lidTop + 0.2, bodyZ - 72],
    [-54, lidTop + 0.2, bodyZ + 72],
    [54, lidTop + 0.2, bodyZ + 72],
  ];
  const faceBoltPositions: [number, number, number][] = [
    [-54, axisY - 16, frontZ + 1.4],
    [54, axisY - 16, frontZ + 1.4],
    [-54, axisY + 16, frontZ + 1.4],
    [54, axisY + 16, frontZ + 1.4],
  ];
  const flangeBoltPositions: [number, number, number][] = [
    [-32, axisY - 12, flangeFrontZ + 0.4],
    [32, axisY - 12, flangeFrontZ + 0.4],
    [-32, axisY + 12, flangeFrontZ + 0.4],
    [32, axisY + 12, flangeFrontZ + 0.4],
  ];

  // Radiation trefoil on all five housing faces except the beam exit, each one
  // the largest disc that still stays on its panel and clears everything else:
  //   top    -> sits on the lid plate (the housing top itself hides beneath it),
  //             pulled back from the fan shroud (r 29.6) and the lid screws
  //   bottom -> bare 144 x 184 underside, capped by the corner fillets at r 66
  //   sides  -> 184 x 63 panels, so their 63 height caps the radius at 25
  //             (also pushed rearward to clear the vent slots and connector)
  //   rear   -> bare 144 x 63 panel, capped the same way
  const radiationDecals: {
    key: string;
    position: [number, number, number];
    rotation: [number, number, number];
    radius: number;
  }[] = [
    {
      key: "top",
      position: [bodyX, lidTop + 0.12, bodyZ + 55],
      rotation: [-Math.PI / 2, 0, 0],
      radius: 23,
    },
    {
      key: "bottom",
      position: [bodyX, bodyCenter[1] - bodySize[1] / 2 - 0.12, bodyZ],
      rotation: [Math.PI / 2, 0, 0],
      radius: 64,
    },
    {
      key: "right",
      position: [bodyX + bodySize[0] / 2 + 0.12, axisY, bodyZ - 55],
      rotation: [0, Math.PI / 2, 0],
      radius: 25,
    },
    {
      key: "left",
      position: [bodyX - bodySize[0] / 2 - 0.12, axisY, bodyZ - 55],
      rotation: [0, -Math.PI / 2, 0],
      radius: 25,
    },
    {
      key: "rear",
      position: [bodyX, axisY, bodyZ - bodySize[2] / 2 - 0.12],
      rotation: [0, Math.PI, 0],
      radius: 25,
    },
  ];

  /** Socket-head cap screw; `axis` orients the head along the fastener normal. */
  const bolt = (position: [number, number, number], axis: "y" | "z", key: string) => (
    <group key={key} position={position} rotation={axis === "z" ? [Math.PI / 2, 0, 0] : [0, 0, 0]}>
      <mesh>
        <cylinderGeometry args={[5.6, 5.6, 3, 18]} />
        <meshStandardMaterial {...steel} />
      </mesh>
      <mesh position={[0, 1.7, 0]}>
        <cylinderGeometry args={[3.2, 3.2, 1.2, 18]} />
        <meshStandardMaterial {...darkPart} />
      </mesh>
    </group>
  );

  return (
    <group>
      {/* bead-blasted aluminium housing */}
      <RoundedBox args={bodySize} radius={6} smoothness={4} position={bodyCenter}>
        <meshStandardMaterial {...aluBody} />
      </RoundedBox>

      {/* stepped lid plate */}
      <RoundedBox args={[126, 9, 166]} radius={3} smoothness={3} position={[bodyX, bodyTop + 4.2, bodyZ]}>
        <meshStandardMaterial {...aluPlate} />
      </RoundedBox>
      {lidBoltPositions.map((position, index) => bolt(position, "y", `lid-${index}`))}

      {/* axial cooling fan sunk into the lid opening */}
      <mesh position={[bodyX, lidTop + 0.9, bodyZ]}>
        <cylinderGeometry args={[28, 28, 2, 40]} />
        <meshStandardMaterial {...darkPart} />
      </mesh>
      <mesh position={[bodyX, lidTop + 2.4, bodyZ]} rotation={[-Math.PI / 2, 0, 0]}>
        <torusGeometry args={[27, 2.6, 12, 44]} />
        <meshStandardMaterial {...steel} />
      </mesh>
      {Array.from({ length: 13 }, (_, index) => (
        <group
          key={`blade-${index}`}
          position={[bodyX, lidTop + 3.2, bodyZ]}
          rotation={[0, (index / 13) * Math.PI * 2, 0]}
        >
          <mesh position={[0, 0, 16.5]} rotation={[0, 0, 0.52]}>
            <boxGeometry args={[17, 1.4, 6]} />
            <meshStandardMaterial {...darkPart} />
          </mesh>
        </group>
      ))}
      <mesh position={[bodyX, lidTop + 4.2, bodyZ]}>
        <cylinderGeometry args={[9.5, 10.5, 8, 28]} />
        <meshStandardMaterial {...aluPlate} />
      </mesh>
      <mesh position={[bodyX, lidTop + 8.4, bodyZ]}>
        <cylinderGeometry args={[6, 6, 1.6, 24]} />
        <meshStandardMaterial {...steel} />
      </mesh>

      {/* side connector and vent slots */}
      <mesh position={[bodyX + 73, axisY - 6, bodyZ + 60]} rotation={[0, 0, Math.PI / 2]}>
        <cylinderGeometry args={[9, 9, 3, 26]} />
        <meshStandardMaterial {...aluPlate} />
      </mesh>
      <mesh position={[bodyX + 72.6, axisY - 6, bodyZ + 60]} rotation={[0, 0, Math.PI / 2]}>
        <cylinderGeometry args={[5.4, 5.4, 3.6, 26]} />
        <meshStandardMaterial {...darkPart} />
      </mesh>
      {[18, 62].map((offset, index) => (
        <mesh key={`vent-${index}`} position={[bodyX - 72.4, axisY - 10, bodyZ + offset]}>
          <boxGeometry args={[2, 6, 26]} />
          <meshStandardMaterial {...darkPart} />
        </mesh>
      ))}

      {/* housing face bolts framing the nosepiece */}
      {faceBoltPositions.map((position, index) => bolt(position, "z", `face-${index}`))}

      {/* brass mounting flange: height matches the housing and sits on the
          optical axis, so it drops flush onto the housing front face */}
      <RoundedBox
        args={[flangeWidth, flangeHeight, flangeDepth]}
        radius={2.5}
        smoothness={3}
        position={[bodyX, axisY, frontZ + flangeDepth / 2 - 1]}
      >
        <meshStandardMaterial {...brass} />
      </RoundedBox>
      {flangeBoltPositions.map((position, index) => bolt(position, "z", `flange-${index}`))}

      {/* brass barrel: a plain cylinder that holds its full radius right up to
          the shoulder, as on the reference hardware */}
      <mesh position={[bodyX, axisY, (flangeFrontZ + shoulderZ) / 2]} rotation={[Math.PI / 2, 0, 0]}>
        <cylinderGeometry args={[barrelR, barrelR, shoulderZ - flangeFrontZ, 44]} />
        <meshStandardMaterial {...brass} />
      </mesh>

      {/* nose: short steep frustum dropping to the flat exit face, rooted at the
          barrel radius so the two meet in a crisp shoulder, not a long taper */}
      <mesh position={[bodyX, axisY, (shoulderZ + coneEndZ) / 2]} rotation={[Math.PI / 2, 0, 0]}>
        <cylinderGeometry args={[coneTipR, barrelR, coneEndZ - shoulderZ, 44]} />
        <meshStandardMaterial {...brass} />
      </mesh>

      {/* pale ceramic window insert sitting flush in the tip face, with the
          pinhole the beam leaves through */}
      <mesh position={[bodyX, axisY, coneEndZ + windowDepth / 2]} rotation={[Math.PI / 2, 0, 0]}>
        <cylinderGeometry args={[windowR, windowR, windowDepth, 40]} />
        <meshStandardMaterial {...ceramic} />
      </mesh>
      <mesh position={[bodyX, axisY, noseFaceZ + 0.03]}>
        <circleGeometry args={[1.7, 24]} />
        <meshStandardMaterial color="#3a4048" roughness={0.72} metalness={0.18} />
      </mesh>
      <mesh position={[bodyX, axisY, noseFaceZ + 0.06]}>
        <circleGeometry args={[1.2, 20]} />
        <meshBasicMaterial color={theme.beam} transparent opacity={0.45} />
      </mesh>

      {/* trefoil stencilled on the remaining five faces of the housing */}
      {radiationDecals.map(({ key, position, rotation, radius }) => (
        <group key={key} position={position} rotation={rotation}>
          <RadiationMark radius={radius} />
        </group>
      ))}

      {/* focal-spot glow: pushed off-axis so it grazes the cone wall instead of
          blowing out the exit window */}
      <pointLight
        position={[bodyX - 52, axisY + 26, coneEndZ + 6]}
        color={theme.beam}
        intensity={700}
        distance={250}
        decay={2}
      />
    </group>
  );
}

/**
 * DSLR-shaped detector camera, rebuilt from the reference photographs.
 *
 * Reading the two views into 3D: the optical axis still runs along Z and the
 * lens points back down it at -Z (towards the source at z = -320), so the camera's own
 * right hand - grip, shutter release, rear control pad, thumb dial - lands on
 * +X while its left hand - mode dial, strap lug - lands on -X. The body reads as
 * a stack of volumes sitting on the shell:
 *
 *   grip (rubber)   x [56, 82]   y [-26, 58]   z [321, 375]
 *   shell           x [-68, 68]  y [-26, 62]   z [322, 388]
 *   top deck        y [52, 68]   inset plate carrying the pentaprism hump
 *                                (y 66 -> 92) with its hot shoe, and the eyecup
 *                                hanging off the hump's rear face
 *   lens barrel     z [229, 324] stepped rings, 0.78 -> 1.0 of the configured
 *                                lens radius, widest at the front collar
 *   rear face       LCD bezel nudged off the grip side, control pad on +X
 *
 * Detail stops where the scene stops paying for it: chamfered boxes for the
 * shell and grip, plain cylinders for the dials and buttons, ring/circle discs
 * for the lens glass, and a rubber eyecup over a dark recess and eye lens.
 *
 * Finishes carry the read: matte grey shell under a satin metal deck, mid grey
 * rubber on the grip and the knurled rings, light grey anodised barrel, chrome
 * trim and dial faces. The optical parts are real glass - `meshPhysicalMaterial`
 * with `transmission`, domed so there is something to refract - and the LCD
 * keeps a clearcoat cover instead. Nothing is black: the ramp bottoms out at the
 * mid grey of the LCD cover and lifts proportionally to the chrome of `trim`.
 */
function CameraAssembly() {
  const [bodyW, bodyH, bodyD] = CAMERA.bodySize;
  const [, bodyY, bodyZ] = CAMERA.bodyCenter;
  const axisY = OPTICAL_AXIS_Y;

  const halfW = bodyW / 2;
  const topY = bodyY + bodyH / 2;
  const frontZ = bodyZ - bodyD / 2;
  const rearZ = bodyZ + bodyD / 2;

  const lensR = CAMERA.lensRadius;
  const lensFrontZ = CAMERA.lensCenter[2] - CAMERA.lensLength / 2;

  // Barrel rings as offsets behind the front face: the front collar carries the
  // glass and is the widest part, and the mount collar runs 2 units into the
  // shell so its end cap never shows.
  const barrelRings: { start: number; end: number; scale: number }[] = [
    { start: 0, end: 12, scale: 1 },
    { start: 12, end: 17, scale: 0.82 },
    { start: 17, end: 35, scale: 0.95 },
    { start: 35, end: 40, scale: 0.8 },
    { start: 40, end: 60, scale: 0.9 },
    { start: 60, end: 65, scale: 0.78 },
    { start: 65, end: frontZ + 2 - lensFrontZ, scale: 0.85 },
  ];

  // Knurled bands riding a touch proud of the focus and zoom rings.
  const barrelRibs: { offset: number; radius: number }[] = [
    { offset: 21, radius: lensR * 0.96 },
    { offset: 26, radius: lensR * 0.96 },
    { offset: 31, radius: lensR * 0.96 },
    { offset: 45, radius: lensR * 0.91 },
    { offset: 50, radius: lensR * 0.91 },
    { offset: 55, radius: lensR * 0.91 },
  ];

  // Elements as shallow spherical caps instead of flat discs: a cap of half-angle
  // `theta` has its rim at radius R sin(theta) and bulges R(1 - cos theta) out of
  // that rim, so the glass gets the volume `thickness`/`ior` need to bend rays.
  const domeTheta = 0.32;
  const domeRadius = (rim: number) => rim / Math.sin(domeTheta);
  const outerRim = lensR * 0.82;
  const coreRim = lensR * 0.38;
  const outerDome = domeRadius(outerRim);
  const coreDome = domeRadius(coreRim);

  // A fixed finish chart rather than theme tokens: the reference is a grey-bodied
  // DSLR, and the light theme's whites collapse the grip, deck and barrel into
  // one silhouette - so the camera carries its own materials the way the source
  // carries its brass and ceramic. The ramp starts at a mid grey and lifts every
  // step proportionally from there to chrome: the darkest finish on the camera
  // is now the LCD cover, and the rubbers and dials sit above it.
  const shell = { color: "#c6c8ca", roughness: 0.78, metalness: 0.05 };
  const deck = { color: "#dfe1e3", roughness: 0.36, metalness: 0.45 };
  const rubber = { color: "#8a8c8e", roughness: 0.92, metalness: 0.03 };
  const barrel = { color: "#d1d3d5", roughness: 0.3, metalness: 0.5 };
  const trim = { color: "#eef0f1", roughness: 0.2, metalness: 0.5 };
  const dial = { color: "#e5e7e9", roughness: 0.28, metalness: 0.5 };
  const knurl = { color: "#7d7f81", roughness: 0.88, metalness: 0.04 };
  const faceDark = { color: "#838587", roughness: 0.6, metalness: 0.18 };

  // The optical parts are glass rather than tinted plastic: `transmission`
  // refracts whatever sits behind the element, so it only reads when the element
  // has depth - hence the shallow domes below - and `clearcoat` carries the
  // highlight. Nothing is black here either, but nothing is blue-tinted either:
  // the greys are neutral and the glass attenuation is a warm neutral rather
  // than a coating blue, because with no environment map every faint tint turns
  // into a visible cast on the surrounding chrome.
  const coverGlass = { color: "#7b7d7f", roughness: 0.12, metalness: 0.3, clearcoat: 1, clearcoatRoughness: 0.06 };
  const viewGlass = {
    color: "#dee1e2",
    transmission: 0.88,
    thickness: 2.4,
    ior: 1.5,
    roughness: 0.05,
    metalness: 0,
    clearcoat: 1,
    clearcoatRoughness: 0.05,
    attenuationColor: new THREE.Color("#b3b7b9"),
    attenuationDistance: 60,
    transparent: true,
    opacity: 1,
  };
  const lensGlass = {
    color: "#eaedee",
    transmission: 0.94,
    thickness: 14,
    ior: 1.52,
    roughness: 0.04,
    metalness: 0,
    clearcoat: 1,
    clearcoatRoughness: 0.04,
    iridescence: 0.1,
    iridescenceIOR: 1.32,
    attenuationColor: new THREE.Color("#b8bcbf"),
    attenuationDistance: 140,
    transparent: true,
    opacity: 1,
  };
  // The inner group sits deeper, so a heavier tint and higher index read as a
  // second piece of glass behind the first instead of a second highlight.
  const lensGlassDeep = {
    ...lensGlass,
    color: "#ced2d5",
    transmission: 0.72,
    thickness: 22,
    ior: 1.62,
    roughness: 0.06,
    iridescence: 0.08,
    attenuationDistance: 70,
  };
  // The ring step is the barrel mouth rather than glass: mid grey, barely
  // translucent, enough contrast to stop the front element reading as a hole.
  const aperture = {
    color: "#939597",
    transmission: 0.35,
    thickness: 1.5,
    ior: 1.4,
    roughness: 0.28,
    metalness: 0.15,
    transparent: true,
    opacity: 1,
  };

  type Finish = { color: string; roughness: number; metalness: number };

  /** Round control sitting proud of a face; `axis` is the fastener normal. */
  const knob = (
    position: [number, number, number],
    radius: number,
    height: number,
    finish: Finish,
    axis: "x" | "y" | "z",
    key: string,
  ) => (
    <mesh
      key={key}
      position={position}
      rotation={axis === "z" ? [Math.PI / 2, 0, 0] : axis === "x" ? [0, 0, Math.PI / 2] : [0, 0, 0]}
    >
      <cylinderGeometry args={[radius, radius, height, 32]} />
      <meshStandardMaterial {...finish} />
    </mesh>
  );

  return (
    <group>
      {/* rubber grip wrapping the +X flank of the shell */}
      <RoundedBox args={[26, 84, 54]} radius={12} smoothness={3} position={[69, 16, 348]}>
        <meshStandardMaterial {...rubber} />
      </RoundedBox>

      {/* body shell */}
      <RoundedBox args={[bodyW, bodyH, bodyD]} radius={9} smoothness={3} position={[0, bodyY, bodyZ]}>
        <meshStandardMaterial {...shell} />
      </RoundedBox>

      {/* stepped top plate: inset on both sides so the shoulder line reads */}
      <RoundedBox args={[128, 16, 62]} radius={5} smoothness={3} position={[0, topY - 2, bodyZ + 2]}>
        <meshStandardMaterial {...deck} />
      </RoundedBox>

      {/* pentaprism hump: a four-sided frustum, so it rakes back on all sides
          and still presents a flat roof for the hot shoe */}
      <group position={[0, topY + 4, bodyZ - 4]} scale={[35, 1, 27]}>
        <mesh position={[0, 13, 0]} rotation={[0, Math.PI / 4, 0]}>
          <cylinderGeometry args={[Math.SQRT2 * 0.62, Math.SQRT2, 26, 4, 1]} />
          <meshStandardMaterial {...deck} flatShading />
        </mesh>
      </group>

      {/* hot shoe: base plate, two contact rails, centre pad */}
      <mesh position={[0, 92.5, bodyZ - 4]}>
        <boxGeometry args={[44, 5, 38]} />
        <meshStandardMaterial {...trim} />
      </mesh>
      {[-15, 15].map((x) => (
        <mesh key={`rail-${x}`} position={[x, 96, bodyZ - 4]}>
          <boxGeometry args={[7, 5, 34]} />
          <meshStandardMaterial {...deck} />
        </mesh>
      ))}
      <mesh position={[0, 95.4, bodyZ - 4]}>
        <boxGeometry args={[11, 1.6, 26]} />
        <meshStandardMaterial {...faceDark} />
      </mesh>

      {/* viewfinder eyecup: rubber frame sunk into the back of the hump so it
          never floats, with the dark recess and eye lens stepping out of it */}
      <RoundedBox args={[50, 28, 26]} radius={6} smoothness={3} position={[0, 75, 375]}>
        <meshStandardMaterial {...rubber} />
      </RoundedBox>
      <mesh position={[0, 75, 388.4]}>
        <boxGeometry args={[34, 18, 2]} />
        <meshStandardMaterial {...faceDark} />
      </mesh>
      <mesh position={[0, 75, 389.8]}>
        <boxGeometry args={[22, 11, 1]} />
        <meshPhysicalMaterial {...viewGlass} />
      </mesh>

      {/* mode dial on the camera's left shoulder, with its marker cap */}
      {knob([-50, topY + 7, bodyZ - 13], 15, 13, deck, "y", "mode-dial")}
      {knob([-50, topY + 14, bodyZ - 13], 13, 3, dial, "y", "mode-cap")}

      {/* shutter release and the pair of top-plate buttons on the grip side */}
      {knob([58, topY + 6, frontZ + 14], 9.5, 7, trim, "y", "shutter-collar")}
      {knob([58, topY + 10.5, frontZ + 14], 7.5, 4, dial, "y", "shutter")}
      {knob([46, topY + 7, bodyZ + 15], 5, 5, trim, "y", "top-button-a")}
      {knob([46, topY + 7, bodyZ + 29], 5, 5, trim, "y", "top-button-b")}

      {/* command dials: front one on the grip shoulder, rear thumb dial */}
      {knob([70, topY - 2, frontZ + 8], 12, 12, dial, "x", "front-dial")}
      {knob([62, topY - 2, bodyZ + 13], 11, 12, dial, "x", "thumb-dial")}

      {/* rear LCD on a raised bezel, nudged off the grip side */}
      <RoundedBox args={[92, 70, 5]} radius={2} smoothness={2} position={[-18, 10, rearZ + 1.4]}>
        <meshStandardMaterial {...deck} />
      </RoundedBox>
      <mesh position={[-18, 10, rearZ + 4.1]}>
        <planeGeometry args={[82, 50]} />
        <meshPhysicalMaterial {...coverGlass} side={THREE.DoubleSide} />
      </mesh>

      {/* rear control pad with its ring, plus two round buttons */}
      {knob([44, 4, rearZ + 3], 14, 5, deck, "z", "rear-pad")}
      <mesh position={[44, 4, rearZ + 5.8]}>
        <ringGeometry args={[8, 12.5, 32]} />
        <meshStandardMaterial {...trim} side={THREE.DoubleSide} />
      </mesh>
      <mesh position={[44, 4, rearZ + 6]}>
        <circleGeometry args={[5.6, 24]} />
        <meshStandardMaterial {...faceDark} side={THREE.DoubleSide} />
      </mesh>
      {knob([52, 28, rearZ + 3], 6.5, 5, trim, "z", "rear-button-a")}
      {knob([52, -18, rearZ + 3], 6.5, 5, trim, "z", "rear-button-b")}

      {/* strap lug on the camera's left flank - on the right it would sit
          buried inside the grip, exactly as the references show it */}
      <RoundedBox args={[12, 20, 18]} radius={4} smoothness={2} position={[-(halfW + 3), 48, frontZ + 6]}>
        <meshStandardMaterial {...shell} />
      </RoundedBox>
      {knob([44, 22, frontZ - 1.5], 5.5, 5, trim, "z", "lens-release")}

      {/* chrome mount ring where the barrel leaves the shell */}
      <mesh position={[0, axisY, frontZ - 1]} rotation={[Math.PI / 2, 0, 0]}>
        <cylinderGeometry args={[36, 36, 4, 48]} />
        <meshStandardMaterial {...trim} />
      </mesh>

      {/* lens barrel: stepped rings opening up towards the front element */}
      {barrelRings.map(({ start, end, scale }, index) => (
        <mesh
          key={`barrel-${index}`}
          position={[0, axisY, lensFrontZ + (start + end) / 2]}
          rotation={[Math.PI / 2, 0, 0]}
        >
          <cylinderGeometry args={[lensR * scale, lensR * scale, end - start, 48]} />
          <meshStandardMaterial {...barrel} />
        </mesh>
      ))}
      {barrelRibs.map(({ offset, radius }) => (
        <mesh key={`rib-${offset}`} position={[0, axisY, lensFrontZ + offset]} rotation={[Math.PI / 2, 0, 0]}>
          <cylinderGeometry args={[radius, radius, 1.6, 48]} />
          <meshStandardMaterial {...knurl} />
        </mesh>
      ))}

      {/* front element: outer glass dome over the barrel mouth, inner group
          behind it - each centred so its rim lands on the old disc plane */}
      <mesh position={[0, axisY, lensFrontZ - 0.1 + outerDome * Math.cos(domeTheta)]} rotation={[-Math.PI / 2, 0, 0]}>
        <sphereGeometry args={[outerDome, 48, 24, 0, Math.PI * 2, 0, domeTheta]} />
        <meshPhysicalMaterial {...lensGlass} side={THREE.DoubleSide} />
      </mesh>
      <mesh position={[0, axisY, lensFrontZ - 0.2]}>
        <ringGeometry args={[lensR * 0.45, lensR * 0.65, 48]} />
        <meshPhysicalMaterial {...aperture} side={THREE.DoubleSide} />
      </mesh>
      <mesh position={[0, axisY, lensFrontZ - 0.3 + coreDome * Math.cos(domeTheta)]} rotation={[-Math.PI / 2, 0, 0]}>
        <sphereGeometry args={[coreDome, 40, 20, 0, Math.PI * 2, 0, domeTheta]} />
        <meshPhysicalMaterial {...lensGlassDeep} side={THREE.DoubleSide} />
      </mesh>
    </group>
  );
}

/**
 * One AirPod Pro, read off the product photograph.
 *
 * The bud is a small shell of volumes around its own local frame - origin on the
 * platter, +Y up the stem, +Z out of the nozzle:
 *
 *   stem     rounded slab 11.4 x 34 x 9.6 with radius 4.5, which is what turns
 *            the rectangular section into the product's flattened oval
 *   collar   satin metal band closing the stem's lower end, microphone port on
 *            its +Z face
 *   shell    ellipsoid centred at y 40 - 23.2 wide, 24.8 tall, 27.6 deep - in
 *            gloss white under a clearcoat
 *   nozzle   short barrel along +Z dressed with the silicone tip, open ended so
 *            the dark bore shows through the mouth ring
 *
 * The black reads are not decals and not sunk slabs: they are concentric
 * spherical caps sharing the shell's centre and sitting 0.9% proud of it, so the
 * acoustic mesh on the -X flank and the vent on the crown follow the curvature
 * exactly and can never poke through it at the edges. The force sensor is the one
 * flat inset, kept to the stem's -X face where the section is nearly planar.
 */
const EARBUD = {
  stemWidth: 11.4,
  stemHeight: 34,
  stemDepth: 9.6,
  collarHeight: 3.4,
  shellRadius: 13.5,
  shellCenterY: 40,
  shellCenterZ: -1,
  shellScale: [0.86, 0.92, 1.02] as const,
  nozzleY: 37,
  tipMouthZ: 24.8,
} as const;

function AirPod({
  position,
  rotationDeg,
}: {
  position: [number, number, number];
  rotationDeg: number;
}) {
  const finish = {
    shell: { color: "#f2f3f4", roughness: 0.18, metalness: 0.02, clearcoat: 1, clearcoatRoughness: 0.12 },
    silicone: { color: "#e9eaeb", roughness: 0.74, metalness: 0 },
    ink: { color: "#191b1e", roughness: 0.54, metalness: 0.08 },
    collar: { color: "#cbcdce", roughness: 0.28, metalness: 0.5 },
    inset: { color: "#d1d3d4", roughness: 0.5, metalness: 0.05 },
  };
  const degrees = THREE.MathUtils.degToRad;
  const {
    stemWidth,
    stemHeight,
    stemDepth,
    collarHeight,
    shellRadius,
    shellCenterY,
    shellCenterZ,
    shellScale,
    nozzleY,
    tipMouthZ,
  } = EARBUD;
  const shell: [number, number, number] = [0, shellCenterY, shellCenterZ];
  const capRadius = shellRadius * 1.009;

  return (
    <group position={position} rotation={[0, degrees(rotationDeg), 0]}>
      {/* stem */}
      <RoundedBox
        args={[stemWidth, stemHeight, stemDepth]}
        radius={4.5}
        smoothness={4}
        position={[0, stemHeight / 2, 0]}
      >
        <meshPhysicalMaterial {...finish.shell} />
      </RoundedBox>

      {/* satin collar, with the microphone port on its forward face */}
      <mesh position={[0, collarHeight / 2, 0]} scale={[1, 1, 0.8]}>
        <cylinderGeometry args={[5.5, 5.5, collarHeight, 40]} />
        <meshStandardMaterial {...finish.collar} />
      </mesh>
      <mesh position={[0, collarHeight / 2, 4.5]}>
        <circleGeometry args={[1.45, 20]} />
        <meshStandardMaterial {...finish.ink} />
      </mesh>

      {/* force sensor */}
      <RoundedBox args={[1.2, 12, 3.6]} radius={0.55} smoothness={3} position={[-5.45, 22, 0]}>
        <meshStandardMaterial {...finish.inset} />
      </RoundedBox>

      {/* shell */}
      <mesh position={shell} scale={shellScale}>
        <sphereGeometry args={[shellRadius, 40, 28]} />
        <meshPhysicalMaterial {...finish.shell} />
      </mesh>

      {/* acoustic mesh, wrapped onto the shell's -X flank */}
      <mesh position={shell} scale={shellScale}>
        <sphereGeometry
          args={[capRadius, 40, 28, degrees(-15), degrees(30), degrees(80), degrees(40)]}
        />
        <meshStandardMaterial {...finish.ink} side={THREE.DoubleSide} />
      </mesh>

      {/* vent oval on the crown, biased towards the shell's rear */}
      <mesh position={shell} scale={shellScale}>
        <sphereGeometry
          args={[capRadius, 40, 28, degrees(-98), degrees(46), degrees(8), degrees(16)]}
        />
        <meshStandardMaterial {...finish.ink} side={THREE.DoubleSide} />
      </mesh>

      {/* nozzle barrel and silicone tip */}
      <mesh position={[0, nozzleY, 12.5]} rotation={[Math.PI / 2, 0, 0]}>
        <cylinderGeometry args={[6.6, 7.2, 9, 32]} />
        <meshStandardMaterial {...finish.shell} />
      </mesh>
      <mesh position={[0, nozzleY, 19.5]} rotation={[Math.PI / 2, 0, 0]}>
        <cylinderGeometry args={[6, 8.8, 11, 32, 1, true]} />
        <meshStandardMaterial {...finish.silicone} side={THREE.DoubleSide} />
      </mesh>
      <mesh position={[0, nozzleY, tipMouthZ - 0.1]}>
        <torusGeometry args={[4.6, 1.5, 14, 32]} />
        <meshStandardMaterial {...finish.silicone} />
      </mesh>
      <mesh position={[0, nozzleY, tipMouthZ - 0.5]}>
        <circleGeometry args={[3.1, 28]} />
        <meshStandardMaterial {...finish.ink} />
      </mesh>
    </group>
  );
}

function AnnularSolid({ inner, outer, bottom, top, color }: {
  inner: number; outer: number; bottom: number; top: number; color: string;
}) {
  const profile = useMemo(() => [
    new THREE.Vector2(inner, bottom), new THREE.Vector2(outer, bottom),
    new THREE.Vector2(outer, top), new THREE.Vector2(inner, top),
    new THREE.Vector2(inner, bottom),
  ], [inner, outer, bottom, top]);
  return (
    <mesh>
      <latheGeometry args={[profile, 256]} />
      <meshStandardMaterial color={color} roughness={0.32} metalness={0.48} />
    </mesh>
  );
}

function RingTrack() {
  const [scaleTexture] = useMemo(() => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 2048;
    const ctx = canvas.getContext("2d");
    if (ctx) {
      const unit = canvas.width / (2 * RING_TRACK.scaleOuterRadius);
      ctx.translate(canvas.width / 2, canvas.height / 2);
      ctx.scale(unit, unit);
      ctx.fillStyle = "#c9cbcd";
      ctx.fillRect(-432, -432, 864, 864);
      ctx.strokeStyle = ctx.fillStyle = "#25282b";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.font = "600 11px Arial";
      for (let degree = 0; degree < 360; degree += 1) {
        ctx.save();
        ctx.rotate(degree * Math.PI / 180);
        const length = degree % 30 === 0 ? 12 : degree % 10 === 0 ? 9 : degree % 5 === 0 ? 6 : 3;
        ctx.lineWidth = degree % 10 === 0 ? 0.8 : 0.4;
        ctx.beginPath();
        ctx.moveTo(0, -RING_TRACK.scaleInnerRadius);
        ctx.lineTo(0, -RING_TRACK.scaleInnerRadius - length);
        ctx.stroke();
        if (degree % 30 === 0) ctx.fillText(String(degree), 0, -412);
        ctx.restore();
      }
    }
    const texture = new THREE.CanvasTexture(canvas);
    texture.colorSpace = THREE.SRGBColorSpace;
    texture.anisotropy = 8;
    return [texture];
  }, []);
  useEffect(() => () => scaleTexture.dispose(), [scaleTexture]);
  const rail = RING_TRACK;
  return (
    <group name="stationary-circular-track">
      <AnnularSolid inner={rail.innerRadius} outer={rail.outerRadius} bottom={rail.bottomY} top={rail.baseTopY} color="#a7abad" />
      <AnnularSolid inner={rail.innerRadius} outer={rail.innerRadius + rail.raceWidth} bottom={rail.baseTopY} top={rail.railTopY} color="#d3d5d6" />
      <AnnularSolid inner={rail.outerRadius - rail.raceWidth} outer={rail.outerRadius} bottom={rail.baseTopY} top={rail.railTopY} color="#d3d5d6" />
      <AnnularSolid inner={rail.scaleInnerRadius} outer={rail.scaleOuterRadius} bottom={rail.baseTopY} top={rail.railTopY - 2} color="#b9bdc0" />
      <mesh position={[0, rail.railTopY - 1.9, 0]} rotation={[-Math.PI / 2, 0, 0]}>
        <ringGeometry args={[rail.scaleInnerRadius, rail.scaleOuterRadius, 256]} />
        <meshStandardMaterial map={scaleTexture} roughness={0.53} metalness={0.22} />
      </mesh>
      {[rail.innerRadius + 1, rail.outerRadius - 1].map((radius) => (
        <mesh key={radius} position={[0, rail.bottomY + 5, 0]} rotation={[-Math.PI / 2, 0, 0]}>
          <torusGeometry args={[radius, 1.1, 8, 256]} />
          <meshStandardMaterial color="#64696d" roughness={0.4} metalness={0.45} />
        </mesh>
      ))}
    </group>
  );
}

function ScissorLift({ side }: { side: "source" | "camera" }) {
  const mount = CARRIAGE.mounts[side];
  const lift = CARRIAGE.lift;
  const { baseTopY, topY } = lift[side];
  const lowerY = baseTopY + 1.5;
  const upperY = topY - lift.plateThickness - 1.5;
  const halfSpan = mount.width / 2 - lift.armInset;
  const armLength = Math.hypot(2 * halfSpan, upperY - lowerY);
  const armAngle = Math.atan2(upperY - lowerY, 2 * halfSpan);
  const screwY = baseTopY + 8;
  const screwEndX = mount.centerX + mount.width / 2 + 17;
  const steel = { color: "#aeb4b8", metalness: 0.55, roughness: 0.34 };
  const black = { color: "#303539", metalness: 0.32, roughness: 0.62 };
  return (
    <group name={`${side}-fixed-scissor-lift`}>
      {[baseTopY - lift.plateThickness / 2, topY - lift.plateThickness / 2].map((y, index) => (
        <RoundedBox key={index} args={[mount.width, lift.plateThickness, mount.depth]} radius={2} smoothness={2} position={[mount.centerX, y, mount.centerZ]}>
          <meshStandardMaterial {...black} />
        </RoundedBox>
      ))}
      {[-lift.sideSpacing, lift.sideSpacing].map((z) => (
        <group key={z} position={[mount.centerX, 0, mount.centerZ + z]}>
          {[1, -1].map((direction) => (
            <mesh key={direction} position={[0, (lowerY + upperY) / 2, 0]} rotation={[0, 0, direction * armAngle]}>
              <boxGeometry args={[armLength, 5, 5]} />
              <meshStandardMaterial {...steel} />
            </mesh>
          ))}
          {[-halfSpan, 0, halfSpan].flatMap((x) => (x === 0 ? [(lowerY + upperY) / 2] : [lowerY, upperY]).map((y) => (
            <mesh key={`${x}-${y}`} position={[x, y, 0]} rotation={[Math.PI / 2, 0, 0]}>
              <cylinderGeometry args={[3.6, 3.6, 9, 20]} />
              <meshStandardMaterial color="#dde0e1" metalness={0.72} roughness={0.2} />
            </mesh>
          )))}
        </group>
      ))}
      <mesh position={[mount.centerX, screwY, mount.centerZ]}>
        <boxGeometry args={[14, 10, lift.sideSpacing * 2 + 12]} />
        <meshStandardMaterial {...steel} />
      </mesh>
      <mesh position={[(mount.centerX + screwEndX) / 2, screwY, mount.centerZ]} rotation={[0, 0, Math.PI / 2]}>
        <cylinderGeometry args={[2.4, 2.4, screwEndX - mount.centerX, 24]} />
        <meshStandardMaterial color="#c4c8ca" metalness={0.7} roughness={0.27} />
      </mesh>
      <mesh position={[screwEndX - 9, screwY, mount.centerZ]}>
        <boxGeometry args={[12, 13, 18]} />
        <meshStandardMaterial {...steel} />
      </mesh>
      <mesh position={[screwEndX + 1, screwY, mount.centerZ]} rotation={[0, Math.PI / 2, 0]}>
        <torusGeometry args={[12, 2.5, 10, 40]} />
        <meshStandardMaterial {...black} />
      </mesh>
      <mesh position={[screwEndX + 1, screwY, mount.centerZ]} rotation={[0, 0, Math.PI / 2]}>
        <cylinderGeometry args={[4, 4, 5, 20]} />
        <meshStandardMaterial {...steel} />
      </mesh>
      {[0, Math.PI / 2, Math.PI, 3 * Math.PI / 2].map((angle) => (
        <mesh key={angle} position={[screwEndX + 1, screwY + Math.cos(angle) * 6, mount.centerZ + Math.sin(angle) * 6]} rotation={[angle, 0, 0]}>
          <boxGeometry args={[2, 12, 2]} />
          <meshStandardMaterial {...steel} />
        </mesh>
      ))}
    </group>
  );
}

function RailCarriage({ side }: { side: "source" | "camera" }) {
  const c = CARRIAGE;
  const mount = c.mounts[side];
  const plateBottom = c.lift[side].baseTopY - c.lift.plateThickness;
  const loadHalfSpacing = side === "camera" ? c.cameraLoadHalfSpacing : c.loadHalfSpacing;
  const loadY = RING_TRACK.railTopY + c.loadRadius;
  const guideRadii = [RING_TRACK.innerRadius - c.guideRadius, RING_TRACK.outerRadius + c.guideRadius];
  const loadRadii = [RING_TRACK.innerRadius + RING_TRACK.raceWidth / 2, RING_TRACK.outerRadius - RING_TRACK.raceWidth / 2];
  return (
    <group name={`${side}-rail-carriage`}>
      <ScissorLift side={side} />
      {guideRadii.flatMap((radius) => [-c.guideHalfSpacing, c.guideHalfSpacing].map((offset) => {
        const pose = trackContactPose(radius, offset);
        return (
          <group key={`guide-${radius}-${offset}`} name="side-guide-roller" position={[pose.x, 0, pose.z]} rotation={[0, pose.yaw, 0]}>
            <mesh position={[0, c.guideY, 0]}>
              <cylinderGeometry args={[c.guideRadius, c.guideRadius, c.guideHeight, 40]} />
              <meshStandardMaterial color="#bec3c6" roughness={0.25} metalness={0.58} />
            </mesh>
            {[-5, 5].map((y) => (
              <mesh key={y} position={[0, c.guideY + y, 0]} rotation={[-Math.PI / 2, 0, 0]}>
                <torusGeometry args={[c.guideRadius - 0.4, 0.4, 8, 40]} />
                <meshStandardMaterial color="#4e5357" metalness={0.35} roughness={0.45} />
              </mesh>
            ))}
            <mesh position={[0, (c.guideY + plateBottom) / 2, 0]}>
              <cylinderGeometry args={[4.5, 4.5, plateBottom - c.guideY, 24]} />
              <meshStandardMaterial color="#777e83" metalness={0.55} roughness={0.3} />
            </mesh>
            <mesh position={[0, plateBottom - 3, 0]}>
              <cylinderGeometry args={[9, 9, 6, 24]} />
              <meshStandardMaterial color="#979da1" metalness={0.4} roughness={0.4} />
            </mesh>
          </group>
        );
      }))}
      {loadRadii.flatMap((radius) => [-loadHalfSpacing, loadHalfSpacing].map((offset) => {
        const pose = trackContactPose(radius, offset);
        const plateEdgeX = mount.centerX + Math.sign(offset) * mount.width / 2;
        return (
          <group key={`load-${radius}-${offset}`} name="radial-axle-load-roller" position={[pose.x, loadY, pose.z]} rotation={[0, pose.yaw, 0]}>
            <mesh rotation={[Math.PI / 2, 0, 0]}>
              <cylinderGeometry args={[c.loadRadius, c.loadRadius, c.loadWidth, 36]} />
              <meshStandardMaterial color="#666c70" roughness={0.4} metalness={0.45} />
            </mesh>
            <mesh rotation={[Math.PI / 2, 0, 0]}>
              <cylinderGeometry args={[3, 3, 16, 24]} />
              <meshStandardMaterial color="#d3d6d8" roughness={0.22} metalness={0.55} />
            </mesh>
            {[-6, 6].map((z) => (
              <mesh key={z} position={[0, (plateBottom - loadY) / 2, z]}>
                <boxGeometry args={[7, Math.abs(plateBottom - loadY), 3]} />
                <meshStandardMaterial color="#aeb4b8" roughness={0.38} metalness={0.42} />
              </mesh>
            ))}
            {side === "camera" && (
              <mesh position={[(plateEdgeX - pose.x) / 2, plateBottom - loadY + 2, 0]}>
                <boxGeometry args={[Math.abs(plateEdgeX - pose.x) + 10, 4, 18]} />
                <meshStandardMaterial color="#aeb4b8" roughness={0.38} metalness={0.42} />
              </mesh>
            )}
          </group>
        );
      }))}
    </group>
  );
}

function MountedEquipment({ side, theme }: { side: "source" | "camera"; theme: SceneTheme }) {
  const source = side === "source";
  return (
    <group name={`${side}-track-mount`} rotation={[0, source ? Math.PI : 0, 0]}>
      <group position={[0, 0, RING_TRACK.radius]}>
        <RailCarriage side={side} />
        {source ? (
          <group rotation={[0, Math.PI, 0]} position={[0, 0, -RING_TRACK.radius]}>
            <SourceAssembly theme={theme} />
          </group>
        ) : (
          <group position={[0, OPTICAL_AXIS_Y * (1 - DETECTOR_SCALE), DETECTOR_FRONT_Z * (1 - DETECTOR_SCALE) - RING_TRACK.radius]} scale={DETECTOR_SCALE}>
            <CameraAssembly />
          </group>
        )}
      </group>
    </group>
  );
}

function TurntableAndSample({ view, theme }: { view: SceneViewModel; theme: SceneTheme }) {
  const movingGroup = useRef<THREE.Group>(null);
  const motion = useRef(new FeedbackAngle());
  const { invalidate, gl } = useThree();
  const applyRotation = (now: number): void => {
    if (!movingGroup.current) return;
    movingGroup.current.rotation.y = -motion.current.value(now) * Math.PI / 180;
    // Read-only renderer diagnostics expose the actual model transform for QA.
    gl.domElement.dataset.turntableAngle = String(-movingGroup.current.rotation.y * 180 / Math.PI);
    gl.domElement.dataset.turntableFeedbackAngle = String(view.angleDeg);
    gl.domElement.dataset.turntableRenderedAt = String(now);
    gl.domElement.dataset.turntableFeedbackKnown = String(view.feedbackValid);
  };
  useEffect(() => {
    const now = performance.now();
    motion.current.accept({
      id: view.feedbackId, taskId: view.taskId, angleDeg: view.angleDeg,
      direction: view.rotationDirection, valid: view.feedbackValid,
      running: view.dataState === "scanning",
    }, now);
    applyRotation(now);
    invalidate();
  }, [view.feedbackId, view.taskId, view.angleDeg, view.rotationDirection, view.feedbackValid, view.dataState, invalidate]);
  useFrame(() => {
    const now = performance.now();
    applyRotation(now);
    if (motion.current.animating(now)) invalidate();
  });
  const markerPosition = useMemo<readonly [number, number, number]>(
    () => [0, TURNTABLE.platterY + TURNTABLE.platterHeight / 2 + 1.4, TURNTABLE.platterRadius - 7],
    [],
  );
  return (
    <group>
      <mesh position={[0, TURNTABLE.baseY, 0]}>
        <cylinderGeometry args={[TURNTABLE.baseRadius, TURNTABLE.baseRadius, TURNTABLE.baseHeight, 48]} />
        <meshStandardMaterial color={theme.chassis} roughness={0.4} metalness={0.45} />
      </mesh>
      <group ref={movingGroup} name="feedback-turntable">
        <mesh position={[0, (TURNTABLE.baseY + TURNTABLE.baseHeight / 2 + TURNTABLE.platterY - TURNTABLE.platterHeight / 2) / 2, 0]}>
          <cylinderGeometry args={[TURNTABLE.spindleRadius, TURNTABLE.spindleRadius, TURNTABLE.platterY - TURNTABLE.platterHeight / 2 - TURNTABLE.baseY - TURNTABLE.baseHeight / 2, 48]} />
          <meshStandardMaterial color="#b8bdc1" roughness={0.3} metalness={0.5} />
        </mesh>
        <mesh position={[0, TURNTABLE.platterY, 0]}>
          <cylinderGeometry args={[TURNTABLE.platterRadius, TURNTABLE.platterRadius, TURNTABLE.platterHeight, 64]} />
          {/* No environment map is bound, and a near-mirror face without one
              takes its colour straight from whatever light hits it - at 0.75
              metalness the blue fill used to paint half the platter. Mid
              metalness plus a coarser surface keeps the satin look without the
              cast. */}
          <meshStandardMaterial color={theme.panelHeader} roughness={0.42} metalness={0.4} />
        </mesh>
        <mesh position={markerPosition} rotation={[-Math.PI / 2, 0, 0]}>
          <circleGeometry args={[4.2, 20]} />
          <meshBasicMaterial color={theme.accent} />
        </mesh>
        {/* Sample: one earbud standing on the platter, centred on the rotation
            axis so it turns in place under the beam. The platter face is the
            bud's y = 0, hence the platter offset below. */}
        <group position={[0, TURNTABLE.platterY + TURNTABLE.platterHeight / 2, 0]}>
          <AirPod position={[0, 0, 0]} rotationDeg={25} />
        </group>
      </group>
    </group>
  );
}

export function EquipmentScene({ view, theme }: { view: SceneViewModel; theme: SceneTheme }) {
  // Light colours are fixed rather than taken from the theme tokens. The light
  // theme's tokens are near-white so they happen to work, but the dark theme's
  // are near-black (#212834) - and a light whose colour is near-black emits
  // almost nothing no matter how high its intensity, which is exactly why the
  // dark scene collapsed to a black silhouettes-and-a-beam picture.
  //
  // Light theme: a bright, even room - sky/ground hemisphere plus a key light
  // and a neutral rim light, so every finish reads against the near-white stage.
  //
  // Dark theme: the room lights are off and a single warm-white lamp hangs on
  // the turntable axis, straight above it. Because it is directly overhead it is
  // never inside the camera's view, and pointing straight down means all three
  // models - source, platter/sample, detector - sit the same distance from it
  // and so come out evenly lit rather than one end glowing and the other black.
  // The cone is wide (0.6 rad, fully feathered) so the bench is washed, not
  // spotlit into a hard circle.
  //
  // The blue accent fill is gone from both themes. With no environment map a
  // metal surface is a mirror for the lights and nothing else, so a tinted light
  // paints every chrome part in its colour - that was the blue showing up on
  // each object, not just the platter.
  //
  // The dark theme's surface tokens are also replaced for the bench. They are
  // tuned for the UI panels, and as surface colours they sit at around #1a2029 -
  // a value that stays black however much light is thrown at it. Raising the
  // lamp to compensate would only blow out the white sample, so instead the
  // bench gets the same value structure on a neutral ramp that can actually take
  // the light: no blue cast, and it reads as dark grey rather than as a
  // silhouette.
  const dark = view.theme === "dark";
  const surfaces: SceneTheme = dark
    ? { ...theme, chassis: "#4a4f56", panelHeader: "#565c63", well: "#33373c", sceneLine: "#666c73" }
    : theme;
  return (
    <group>
      <hemisphereLight color={dark ? "#eef0f2" : theme.panelHeader} groundColor={surfaces.well} intensity={0.85} />
      {dark ? (
        <>
          {/* Straight above the turntable axis (x = z = 0), pointing down at the
              origin. ~900 units up, hence the large candela figure: three decays
              a spot light by distance squared. At this geometry the source, the
              platter and the detector all sit 870-960 units away, so they land
              within 1.0-1.25 lux of each other - evenly lit, and well short of
              the light theme's brightness. */}
          <spotLight
            position={[0, 900, 0]}
            intensity={950000}
            color="#fff2dc"
            angle={0.6}
            penumbra={0.9}
            decay={2}
            distance={3600}
          />
          <directionalLight position={[620, 520, 780]} intensity={0.65} color="#e8e9ea" />
          <directionalLight position={[-420, 260, -360]} intensity={0.3} color="#e6e8ea" />
        </>
      ) : (
        <>
          <directionalLight position={[420, 650, 360]} intensity={0.78} color={theme.panelHeader} />
          <directionalLight position={[-420, 260, -360]} intensity={0.22} color="#e6e8ea" />
        </>
      )}
      {/* The room first, so the shells of the beam composite over it. */}
      <StageSurroundings theme={theme} />
      <RingTrack />
      <MountedEquipment side="source" theme={surfaces} />
      <TurntableAndSample view={view} theme={surfaces} />
      <MountedEquipment side="camera" theme={surfaces} />
      <Beam view={view} theme={theme} />
      {/* optical axis as a long-dashed light grey rule - a construction line
          rather than a coloured spine, so it reads the same on either theme */}
      <Line
        points={[
          SOURCE.focus,
          SAMPLE.center,
          detectorPoint(CAMERA.lensCenter),
        ]}
        color={AXIS_GREY}
        lineWidth={1}
        dashed
        dashSize={16}
        gapSize={10}
        transparent
        opacity={0.9}
      />
      <mesh position={[0, RING_TRACK.bottomY + 0.1, 0]} rotation={[-Math.PI / 2, 0, 0]} userData={{ excludeFromFit: true }}>
        <planeGeometry args={[1000, 1000]} />
        <shadowMaterial color={theme.sceneLine} transparent opacity={0.18} />
      </mesh>
    </group>
  );
}
