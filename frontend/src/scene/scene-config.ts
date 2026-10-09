/**
 * Single geometric source of truth for the visualization optical axis.
 * Positions are display coordinates, not calibration or motion commands; the
 * final assertion catches accidental loss of source/sample/detector collinearity.
 */

import type { ViewPreset } from "./types";

export const OPTICAL_AXIS_Y = 30;
export const SOURCE_Z = -320;
export const SAMPLE_Z = 0;
export const SCINTILLATOR_Z = 205;
export const LENS_Z = 292;

export const RING_TRACK = {
  radius: 405,
  innerRadius: 378,
  outerRadius: 432,
  bottomY: -112,
  baseTopY: -94,
  railTopY: -76,
  raceWidth: 8,
  scaleInnerRadius: 389,
  scaleOuterRadius: 421,
} as const;

export const CARRIAGE = {
  mounts: {
    source: { width: 144, depth: 112, centerX: 0, centerZ: -6 },
    camera: { width: 196, depth: 112, centerX: 9, centerZ: -6 },
  },
  plateThickness: 8,
  mountingY: -42.8,
  lift: {
    source: { baseTopY: -42.8, topY: -1.5 },
    camera: { baseTopY: -68, topY: -42.8 },
    plateThickness: 8,
    armInset: 24,
    sideSpacing: 36,
  },
  guideRadius: 14,
  guideHeight: 16,
  guideY: -85,
  guideHalfSpacing: 58,
  loadRadius: 10,
  loadWidth: 6,
  loadHalfSpacing: 58,
  // The camera's low lift overlaps the wheel height. Put its load wheels beyond
  // the plate ends, where the tyre can turn without entering the lower deck.
  cameraLoadHalfSpacing: 120,
} as const;

export function trackContactPose(radius: number, tangentOffset: number) {
  const radialZ = Math.sqrt(radius * radius - tangentOffset * tangentOffset);
  return {
    x: tangentOffset,
    z: radialZ - RING_TRACK.radius,
    yaw: Math.atan2(tangentOffset, radialZ),
  };
}

export const TURNTABLE = {
  baseRadius: 34,
  baseHeight: 70,
  baseY: RING_TRACK.bottomY + 35,
  spindleRadius: 24,
  platterRadius: 44,
  platterHeight: 9,
  platterY: -16,
} as const;

export const SAMPLE = {
  size: [66, 86, 56] as const,
  center: [0, OPTICAL_AXIS_Y, SAMPLE_Z] as const,
} as const;

export const CAMERA = {
  bodySize: [136, 88, 66] as const,
  bodyCenter: [0, 18, 355] as const,
  lensRadius: 40,
  lensLength: 126,
  lensCenter: [0, OPTICAL_AXIS_Y, 292] as const,
} as const;

export const SOURCE = {
  bodySize: [144, 126, 184] as const,
  bodyCenter: [0, 12, -405] as const,
  focus: [0, OPTICAL_AXIS_Y, SOURCE_Z] as const,
} as const;

export const SCINTILLATOR = {
  size: [132, 112, 7] as const,
  center: [0, OPTICAL_AXIS_Y, SCINTILLATOR_Z] as const,
} as const;

export const CAMERA_CONSTRAINTS = {
  minPolarAngle: (12 * Math.PI) / 180,
  maxPolarAngle: (78 * Math.PI) / 180,
  minZoom: 0.6,
  maxZoom: 2.2,
  target: [0, 20, 0] as const,
} as const;

export const CAMERA_PRESETS: Record<ViewPreset, readonly [number, number, number]> = {
  iso: [760, 430, 760],
  front: [780, 95, 0],
  top: [0, 1050, 0.001],
};

export function assertOpticalAxisConfiguration(): void {
  const points = [SOURCE.focus, SAMPLE.center, SCINTILLATOR.center, CAMERA.lensCenter];
  if (!points.every(([x]) => x === 0)) {
    throw new Error("Scene optical components must share x=0");
  }
  if (!points.every(([, y]) => y === OPTICAL_AXIS_Y)) {
    throw new Error("Scene optical components must share the optical-axis height");
  }
  if (!(SOURCE_Z < SAMPLE_Z && SAMPLE_Z < SCINTILLATOR_Z && SCINTILLATOR_Z < LENS_Z)) {
    throw new Error("Scene optical components must follow source-to-lens Z ordering");
  }
}

assertOpticalAxisConfiguration();
