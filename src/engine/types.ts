export type EngineMode = "production_locked" | "developer_preview";

export type EnginePhase =
  | "idle"
  | "ready_for_home"
  | "ready"
  | "running"
  | "paused"
  | "stopped"
  | "completed"
  | "fault";

export type DeviceState = "offline" | "connected" | "ready" | "busy" | "locked" | "fault";

export type DeviceId = "turntable" | "camera" | "xray";

export type ConnectionState = "disconnected" | "connected" | "degraded" | "lost";

export interface DeviceStatus {
  id: DeviceId;
  label: string;
  state: DeviceState;
  detail: string;
}

export interface SafetyState {
  xrayAvailable: boolean;
  xrayEnabled: boolean;
  interlockOk: boolean;
  lockReason: string;
}

export interface ScanParameters {
  taskId: string;
  savePath: string;
  projectionCount: number;
  angleStepDeg: number;
  exposureMs: number;
}

export interface ScanProgress {
  current: number;
  total: number;
  percent: number;
  angleDeg: number;
  etaSeconds: number | null;
}

export interface LogEntry {
  id: string;
  timestamp: string;
  level: "info" | "success" | "warning" | "error";
  source: "系统" | "转台" | "相机" | "射线";
  message: string;
}

export interface EngineSnapshot {
  mode: EngineMode;
  modeLabel: string;
  connectionState: ConnectionState;
  adapterLabel: string;
  phase: EnginePhase;
  phaseLabel: string;
  preflightPassed: boolean;
  homed: boolean;
  requiresPreflight: boolean;
  requiresHome: boolean;
  safety: SafetyState;
  devices: DeviceStatus[];
  parameters: ScanParameters;
  progress: ScanProgress;
  imageCount: number;
  logs: LogEntry[];
  lastError: string | null;
  updatedAt: string;
}

export type EngineCommand =
  | { type: "connect"; adapter: "developer_preview" | "real_hardware" }
  | { type: "disconnect" }
  | { type: "preflight" }
  | { type: "home" }
  | { type: "start_scan" }
  | { type: "pause" }
  | { type: "resume" }
  | { type: "restore_previous" }
  | { type: "stop" }
  | { type: "set_parameters"; parameters: ScanParameters };

export interface EngineAdapter {
  readonly kind: "developer_preview" | "tauri";
  getSnapshot(): Promise<EngineSnapshot>;
  dispatch(command: EngineCommand): Promise<EngineSnapshot>;
}
