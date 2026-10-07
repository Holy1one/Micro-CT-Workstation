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
  | "finishing"
  | "stopping"
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

/** Geometry is recorded with the acquisition, not inferred from the 3D view. */
export interface ReconstructionGeometry {
  sodMm: number;
  objectToDetectorMm: number;
  detectorWidthMm: number;
  centerOffsetXMm: number;
  centerOffsetYMm: number;
  rotationDirection: "clockwise" | "counterclockwise";
  mirrorX: boolean;
  measurement: "measured" | "estimated";
}

export type ScanPromptStage = "geometry" | "preReferences" | "placeSample";

export interface ScanFlowView {
  stage: "idle" | "geometry" | "preDark" | "preFlat" | "placeSample" |
    "projections" |
    "finalizing" | "persisting_completed" | "completed" | "stopped" | "fault";
  prompt: null | {
    stage: ScanPromptStage;
    title: string;
    message: string;
    confirmLabel: string;
    cancelLabel: string;
  };
  references: {
    preDark: number;
    preFlat: number;
    postFlat: number;
    postDark: number;
    required: number;
  };
  geometry: ReconstructionGeometry | null;
  checkpointPath: string | null;
}

export type ReconstructionMethodId = "fdk" | "sirt" | "cgls";

export interface ReconstructionView {
  unlocked: boolean;
  methods: Array<{
    id: ReconstructionMethodId;
    label: string;
    enabled: boolean;
    reason: string | null;
  }>;
  selectedMethod: ReconstructionMethodId | null;
  status: "idle" | "running" | "completed" | "failed";
  percent: number;
  message: string;
  cachePath: string | null;
}

export interface LogEntry {
  id: string;
  timestamp: string;
  level: "info" | "success" | "warning" | "error";
  /** Engine log source. ct-engine emits system | preflight | operator | xray | nano | camera |
   *  reconstruction | devices; the console filters on those literals and the browser preview
   *  maps its own source keys onto the same values, so a new engine source still renders. */
  source: string;
  message: string;
}

export interface EngineSnapshot {
  engineSessionId: string;
  snapshotSequence: number;
  operationId: string | null;
  operations: DeviceOperation[];
  deviceControls: DeviceControls;
  cameraScanReadiness?: CameraScanReadiness;
  softwareUpdatePreparation?: SoftwareUpdatePreparation | null;
  cameraCaptureDescriptor?: IndependentCapturedImage | null;
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

export interface ScanPreparation {
  taskId: string;
  projectionCount: number;
  maxXraySec: number;
  geometry: ReconstructionGeometry;
}

export interface CameraScanIssue {
  key: string;
  actual: CameraValue | null;
  requirement: string;
  remedy: string;
}
export interface CameraScanReadiness { ready: boolean; issues: CameraScanIssue[] }
export interface IndependentCapturedImage {
  captureId: string;
  sessionId: string;
  operationId: string;
  path: string;
  bytes: number;
  sha256: string;
  format: "NEF";
  exposureEnded: boolean;
}
export type ShutdownDeviceEvidence = "confirmed" | "never-owned" | "unknown";
export interface SoftwareUpdatePreparation {
  sessionId: string;
  operationId: string;
  status: "preparing" | "ready" | "blocked" | "cancelled";
  ready: boolean;
  maintenanceLocked: boolean;
  createdAt: string;
  completedAt: string | null;
  turntableStop: ShutdownDeviceEvidence;
  xrayOff: ShutdownDeviceEvidence;
  cameraCleanup: ShutdownDeviceEvidence;
  noActiveTransactions: boolean;
  error: string | null;
}

export type EngineCommand =
  | ({ type: "prepare_scan" } & ScanPreparation)
  | { type: "set_capture_root"; savePath: string }
  | { type: "camera_capture_descriptor"; captureId: string }
  | { type: "prepare_software_update" }
  | { type: "cancel_software_update_prepare"; operationId: string }
  | { type: "camera_connect"; deviceId?: string; acknowledgeUnconfirmedCleanup?: boolean }
  | { type: "camera_disconnect" | "camera_status" | "camera_identity" | "camera_read_properties" | "camera_live_view_start" | "camera_live_view_stop" | "camera_autofocus" }
  | { type: "camera_apply_properties"; properties: CameraProperties; presetId?: string }
  | { type: "camera_live_view_read"; sessionId: string }
  | { type: "camera_live_view_configure"; sessionId: string; controls: CameraProperties }
  | { type: "camera_capture" | "camera_test_capture"; directory?: string }
  | { type: "camera_focus"; step: number }
  | { type: "turntable_connect" | "turntable_disconnect" | "turntable_status" | "turntable_identity" | "turntable_home" }
  | { type: "turntable_relative" | "turntable_to_angle"; angleDeg: number }
  | { type: "turntable_bench_mode"; enabled: boolean; physicallyIsolated: boolean }
  | { type: "preset_list" | "preset_export" | "abandon_scan" }
  | { type: "preset_load" | "preset_compare" | "preset_delete"; id: string }
  | { type: "preset_save"; name: string; description?: string; source?: string }
  | { type: "preset_import"; jsonText: string }
  | { type: "xray_connect" | "xray_status" | "xray_identity" | "xray_off" }
  | { type: "connect"; adapter: "developer_preview" | "real_hardware" }
  | { type: "disconnect" }
  | { type: "preflight"; restore?: boolean; confirmUsbAutoShutdownRelease?: boolean }
  /** Operator attestation that the camera body multiple-exposure control is OFF for this session. */
  | { type: "camera_confirm_multiple_exposure_off"; confirmed: boolean; operator?: string }
  | { type: "home" }
  | { type: "start_scan" }
  | { type: "confirm_scan_stage"; stage: ScanPromptStage; geometry?: ReconstructionGeometry }
  | { type: "cancel_scan_stage"; stage: ScanPromptStage }
  | { type: "start_reconstruction"; method: ReconstructionMethodId }
  | { type: "pause" }
  | { type: "resume" }
  | { type: "restore_previous" }
  | { type: "stop" }
  | { type: "set_parameters"; parameters: ScanParameters }
  | { type: "retry_device"; device: DeviceId }
  | { type: "xray_disconnect" }
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

/** Native WPD/PTP evidence; numeric values are exact integers, never display labels. */
export type CameraValue = number | string;
export type CameraProperties = Record<string, CameraValue>;
export interface CameraIdentity { deviceId: string; manufacturer: string; model: string; serial: string | null; backend: string }
export interface CameraDeviceInfo {
  standardVersion: number; vendorExtensionId: number; vendorExtensionVersion: number;
  vendorExtensionDescription: string; functionalMode: number; deviceVersion: string;
  operations: number[]; events: number[]; properties: number[]; captureFormats: number[]; imageFormats: number[];
  datasetSize: number; datasetSha256: string; adapterSelection: string;
  effectiveProperties?: number[];
  vendorPropertyCodes?: { operationCode: number; responseCode: number; properties: number[];
    datasetSize: number; datasetSha256: string; error: string | null } | null;
}
export interface CameraEventDiagnostic {
  operationCode: number; responseCode: number | null; datasetSize: number; datasetSha256: string;
  dataPrefixHex: string; events: { code: number; parameter: number }[]; error: string | null;
}
export interface CameraEventBatchDiagnostic extends CameraEventDiagnostic {
  sequence: number; eventCount: number;
}
export interface CameraCaptureEventDiagnostic extends CameraEventDiagnostic {
  sessionId: string; operationId: string; batchSequence: number; eventCount: number;
  captureCompleteCount: number; exposureEndedObserved: boolean;
  eventBatches?: CameraEventBatchDiagnostic[];
}
export interface CameraCleanupDiagnostic {
  sessionId: string | null; operationId: string | null; pendingAtStart: boolean;
  exposureEndedObserved: boolean; pollCount: number; waitLimitMs: number; elapsedMs: number;
  ioCancelConfirmed: boolean; closeConfirmed: boolean; error: string | null;
  lastEventDiagnostic: CameraEventDiagnostic | null;
  eventBatches?: CameraEventBatchDiagnostic[];
}
export type CameraRuntimeProperty = CameraProperty & { semanticStatus: "unknown" };
export interface CameraProperty {
  key: string; label: string; code: number; dataType: number; readable: boolean; writable: boolean;
  supported: boolean; nativeWritable: boolean; effective: boolean; disabledReason: string | null;
  options: { value: CameraValue; label: string; supported: boolean; enabled: boolean; reason: string | null }[];
  supportedValues: CameraValue[]; range: [number, number, number] | null;
  actual: CameraValue | null; target: CameraValue | null; confirmed: boolean; error: string | null;
}
export interface CameraCapabilities {
  properties: boolean; hostCapture: boolean; liveView: boolean; autofocus: boolean; focusStep: boolean;
  scanNef: boolean; hardwareCertified: boolean; disabledReasons: Record<string, string>;
}
export interface CameraLiveView {
  active: boolean; sessionId: string | null; sequence: number; lastFrameAt: string | null;
  fresh: boolean; width: number | null; height: number | null; fpsLimit: number; error: string | null;
}
/** Transient camera controls belong to one Live View instance, outside scan profiles and presets. */
export interface CameraLiveViewControls {
  sessionId: string | null;
  properties: Record<string, CameraProperty>;
}
export interface MultipleExposureEvidence {
  physicalApplicable: boolean; observable: boolean; status: "unknown" | "off" | "on" | "not_applicable";
  source: string; reason: string | null;
}
/**
 * Operator attestation kept strictly separate from machine evidence. The engine
 * never rewrites `multipleExposureEvidence`; it publishes this record with its
 * live validity so a stale session cannot be shown as a confirmed OFF state.
 */
export interface MultipleExposureOperatorEvidence {
  state: "none" | "confirmed" | "stale";
  attested: boolean; valid: boolean;
  confirmedAt: string | null; operator: string | null; deviceId: string | null; sessionId: string | null;
  policyVersion: number | null; evidenceFingerprint: string | null;
  machineStatus: MultipleExposureEvidence["status"]; machineObservable: boolean;
}
export interface CameraHealth {
  connected: boolean; backend: string; serial: string | null; transferPolicy: string | null;
  lastCapture: string | null; lastError: string | null; exposureMinMs: number | null; exposureMaxMs: number | null;
  lastCapturedImage?: IndependentCapturedImage | null;
  identity: CameraIdentity | null; deviceInfo?: CameraDeviceInfo | null; discovered: CameraIdentity[]; sessionId: string | null;
  lastEventDiagnostic?: CameraEventDiagnostic | null;
  lastDeviceInfoChangedDiagnostic?: CameraEventDiagnostic | null;
  lastCleanupDiagnostic?: CameraCleanupDiagnostic | null;
  lastCaptureEventDiagnostic?: CameraCaptureEventDiagnostic | null;
  runtimeProperties?: Record<string, CameraRuntimeProperty>;
  multipleExposureEvidence?: MultipleExposureEvidence;
  /** Session-bound operator OFF attestation; machine evidence stays UNKNOWN. */
  multipleExposureOperator?: MultipleExposureOperatorEvidence;
  capabilities: CameraCapabilities; properties: Record<string, CameraProperty>; liveView: CameraLiveView;
  liveViewControls?: CameraLiveViewControls;
  isolated: boolean; cleanupConfirmed: boolean; busy: boolean; operationId: string | null;
  phase: string; focusPosition: number | null;
  /** Engine-side "camera worker last seen" timestamp (RFC3339) or null; separate from session/connection evidence. */
  lastSeenAt?: string | null;
}
export interface NanoStatus {
  state: string; positionPulses: number; targetPulses: number; microsteps: number; pulsesPerRev: number;
  referenceValid: boolean; homed: boolean; rearmed: boolean; hallActive: boolean; captureId: number;
}
export interface CameraPreset { id: string; name: string; description: string; models: string[]; parameters: CameraProperties; source: string; savedAt: string }
export interface DeviceOperation {
  operationId: string; deviceSessionId: string; device: string; kind: string; stage: string;
  cancelRequested: boolean; status: "queued" | "running" | "cancelling" | "completed" | "cancelled" | "failed" | "cleanup_unconfirmed" | "rejected";
  error: string | null; startedAt: string; updatedAt: string;
}
/** One record in `DeviceControls.taskLock.retained` (engine `TaskRecord`, camelCase). */
export interface RetainedTaskRecord {
  parameters: { angleStepDeg: number; exposureMs: number; projectionCount: number; savePath: string; taskId: string };
  status: string;
  registeredAt: string;
  updatedAt: string;
}
export interface DeviceControls {
  camera: CameraHealth | null;
  turntable: { connected: boolean; identity: { device: string; version: string; protocol: number; build: string; buzzerInstalled: boolean; capabilities: string[] } | null; status: NanoStatus | null; confirmedAngleDeg: number | null; benchMode: boolean; reason: string | null; hardwareCertified: false };
  /** `retained` is the engine's list of retained task records, never a boolean:
   * an empty array is still truthy in JS, so callers must test its length. */
  taskLock: { locked: boolean; reason: string | null; taskId: string | null; savePath: string | null; status: string; retained: RetainedTaskRecord[] };
  presets: { schemaVersion: number; presets: CameraPreset[]; error: string | null };
  exportJson?: string | null;
  cameraDraft?: CameraProperties | null;
  presetComparison?: unknown;
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
  source: "system" | "xray" | "nano" | "camera" | "preflight" | "operator" | "reconstruction";
  message: string;
}

export interface ConsoleFrame {
  index: number;
  angleDeg: number;
  exposureMs: number;
  fileName: string;
  path?: string;
  bytes?: number;
  sha256?: string;
}

export interface WorkstationView {
  scanFlow: ScanFlowView;
  reconstruction: ReconstructionView;
  cameraExposure: { minMs: number; maxMs: number; known: boolean };
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
  scene: { angleDeg: number; rotated: boolean; angleKnown: boolean; rotationDirection: -1 | 1 };
  xray: {
    connected: boolean;
    setKv: number;
    setUa: number;
    monKv: number | null;
    monUa: number | null;
    powerW: number | null;
    tempC: number | null;
    beamState: "on" | "off" | "unknown";
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
    stop: boolean;
    playMode: "start" | "pause" | "resume" | "disabled";
    homeReason: string;
    restoreReason: string;
    playReason: string;
  };
  scanSetup: ScanSetup;
  consoleLogs: ConsoleLogLine[];
  frames: ConsoleFrame[];
  checkpointAvailable: boolean;
}
