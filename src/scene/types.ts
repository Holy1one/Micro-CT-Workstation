export type SceneThemeName = "light" | "dark";
export type ViewPreset = "iso" | "front" | "top";
export type SceneFallbackReason = "webgl-unavailable" | "context-lost";

export interface SceneViewModel {
  dataState: "ready" | "scanning" | "paused" | "fault";
  angleDeg: number;
  theme: SceneThemeName;
  beamOn: boolean;
  xrayLatched: boolean;
}

export interface SceneTheme {
  accent: string;
  beam: string;
  sceneLine: string;
  optics: string;
  textPrimary: string;
  panel: string;
  panelHeader: string;
  well: string;
  chassis: string;
  stageTable: string;
  stageWall: string;
  stageGlow: number;
  rayReady: number;
  rayScanning: number;
  rayPaused: number;
  rayFault: number;
}
