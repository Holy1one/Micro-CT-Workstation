import type { ViewPreset } from "./types";

export const OPTICAL_AXIS_Y = 30;
export const SOURCE_Z = -320;
export const SAMPLE_Z = 0;
export const SCINTILLATOR_Z = 205;
export const LENS_Z = 292;

export const TURNTABLE = {
  baseRadius: 34,
  baseHeight: 22,
  baseY: -25,
  platterRadius: 44,
  platterHeight: 7,
  platterY: -4,
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
