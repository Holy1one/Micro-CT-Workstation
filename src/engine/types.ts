/**
 * Shared frontend contract for commands, complete engine snapshots, and the
 * derived workstation view-model. Changes here cross production Tauri IPC and
 * the browser preview, so every consumer must be updated in the same change.
 */

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
  workstation?: WorkstationView;
}

export interface ScanSetup {
  savePath: string;
  taskId: string;
  projectionCount: number;
  angleStepDeg: number;
  exposureMs: number;
  maxXraySec: number;
}

export type ScanSetupUpdate = Partial<Omit<ScanSetup, "angleStepDeg">>;

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
  | { type: "set_parameters"; parameters: ScanParameters }
  | { type: "estop_release" }
  | { type: "retry_device"; device: DeviceId }
  | { type: "xray_disconnect" }
  | { type: "camera_test_capture" }
  | { type: "xray_toggle" }
  | { type: "timer_toggle" }
  | { type: "usb_auto_shut_down_toggle" }
  | { type: "set_usb_shutdown_delay"; delay: number }
  | { type: "send_voltage"; kv: number }
  | { type: "send_current"; ua: number }
  | { type: "update_scan_setup"; setup: ScanSetupUpdate };

export type AdapterKind = "developer_preview" | "tauri";

export interface EngineAdapter {
  readonly kind: AdapterKind;
  getSnapshot(): Promise<EngineSnapshot>;
  dispatch(command: EngineCommand): Promise<EngineSnapshot>;
  close?(): void;
}

/* ------------------------------------------------------------------ *
 * Workstation console view-model (RTS9060 link layer).                 *
 * Emitted by the workstation adapter as `EngineSnapshot.workstation`;  *
 * the console renders exclusively from this block when present.        *
 * ------------------------------------------------------------------ */

export type ConsoleDataState = "ready" | "scanning" | "paused" | "fault";
export type ConsoleTone = "ok" | "warn" | "accent" | "danger" | "muted";

export interface ConsoleDeviceView {
  id: DeviceId;
  name: string;
  word: string;
  tone: ConsoleTone;
  spec: string;
}

export interface ConsoleFloatView {
  key: "X-RAY" | "CAMERA" | "SAMPLE";
  text: string;
  tone: ConsoleTone;
}

export interface ConsoleLogLine {
  id: string;
  timestamp: string;
  level: "PASS" | "INFO" | "OK" | "WARN" | "ERR" | "ACTION";
  source: "system" | "xray" | "nano" | "camera" | "preflight" | "operator";
  message: string;
}

export interface ConsoleFrame {
  index: number;
  angleDeg: number;
  exposureMs: number;
  fileName: string;
}

export interface WorkstationView {
  dataState: ConsoleDataState;
  phaseWord: string;
  phaseTone: "accent" | "warn" | "danger";
  devices: ConsoleDeviceView[];
  onlineSummary: string;
  preflight: {
    word: string;
    percent: number;
    tone: "pass" | "warn" | "fail" | "running";
    subline: string;
  };
  floats: ConsoleFloatView[];
  safetyBar: { text: string; tone: "muted" | "danger" | "dangerBold" };
  scene: { angleDeg: number; rotated: boolean };
  xray: {
    connected: boolean;
    setKv: number;
    setUa: number;
    monKv: number;
    monUa: number;
    powerW: number;
    tempC: number;
    beamOn: boolean;
    latched: boolean;
    onSec: number;
    offSec: number;
    timerOn: boolean;
    usbAutoShutDown: boolean;
    usbAutoShutDownKnown: boolean;
    usbShutdownDelay: number | null;
    manualControlsEnabled: boolean;
    timerControlsEnabled: boolean;
    setpointControlsEnabled: boolean;
    voltageConfirmed: boolean;
    currentConfirmed: boolean;
    setpointConfirmed: boolean;
  };
  progress: {
    captured: number;
    total: number;
    percent: number;
    angleDeg: number;
    etaText: string;
    barTone: "accent" | "warn" | "danger";
    barLabel: string;
  };
  summary: { savePath: string; acquisition: string; output: string };
  statusbar: { left: string; right: string; dotTone: ConsoleTone };
  dock: {
    home: boolean;
    play: boolean;
    restore: boolean;
    estop: boolean;
    playMode: "start" | "pause" | "resume" | "disabled";
    homeReason: string;
    playReason: string;
  };
  scanSetup: ScanSetup;
  consoleLogs: ConsoleLogLine[];
  frames: ConsoleFrame[];
  checkpointAvailable: boolean;
}
