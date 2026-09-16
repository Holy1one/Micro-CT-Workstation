import type {
  DeviceStatus,
  EngineAdapter,
  EngineCommand,
  EnginePhase,
  EngineSnapshot,
  LogEntry,
  ScanParameters,
} from "./types";

const MODE_LABEL = "Developer Preview / No Real Hardware";
const LOCK_REASON = "The V1 X-ray channel is not implemented; production remains fail-closed";

const defaultParameters: ScanParameters = {
  taskId: "demo-001",
  savePath: "~/MicroCT/runs/demo-001",
  projectionCount: 120,
  angleStepDeg: 3,
  exposureMs: 180,
};

const phaseLabels: Record<EnginePhase, string> = {
  idle: "Idle", ready_for_home: "Ready for Home", ready: "Ready", running: "Running",
  paused: "Paused", stopped: "Stopped", completed: "Completed", fault: "Fault",
};

function now(): string {
  return new Date().toISOString();
}

function makeDevices(state: "offline" | "connected" | "ready" | "busy"): DeviceStatus[] {
  return [
    {
      id: "turntable",
      label: "Turntable / Nano",
      state,
      detail:
        state === "offline"
          ? "Waiting for connection"
          : state === "ready"
            ? "Online · ready"
            : state === "busy"
              ? "Motion in progress"
              : "Online · awaiting command",
    },
    {
      id: "camera",
      label: "Camera / Nikon D7100",
      state: state === "busy" ? "busy" : state,
      detail:
        state === "offline"
          ? "Waiting for connection"
          : state === "ready"
            ? "Online · host storage"
            : state === "busy"
              ? "Awaiting capture"
              : "Online · host storage",
    },
    {
      id: "xray",
      label: "X-ray / Moxtek 12 W",
      state: "locked",
      detail: "V1 interlocked · no output",
    },
  ];
}

function makeInitialSnapshot(): EngineSnapshot {
  return {
    mode: "developer_preview",
    modeLabel: MODE_LABEL,
    connectionState: "disconnected",
    adapterLabel: "Developer Preview Adapter",
    phase: "idle",
    phaseLabel: phaseLabels.idle,
    preflightPassed: false,
    homed: false,
    requiresPreflight: true,
    requiresHome: true,
    safety: {
      xrayAvailable: false,
      xrayEnabled: false,
      interlockOk: false,
      lockReason: LOCK_REASON,
    },
    devices: makeDevices("offline"),
    parameters: defaultParameters,
    progress: {
      current: 0,
      total: defaultParameters.projectionCount,
      percent: 0,
      angleDeg: 0,
      etaSeconds: null,
    },
    imageCount: 0,
    logs: [
      {
        id: "boot",
        timestamp: now(),
        level: "info",
        source: "系统",
        message: "ct-engine started; waiting for an explicit preview connection",
      },
      {
        id: "safe",
        timestamp: now(),
        level: "warning",
        source: "射线",
        message: "Production is fail-closed; V1 does not enable real X-ray output",
      },
    ],
    lastError: null,
    updatedAt: now(),
  };
}

function appendLog(snapshot: EngineSnapshot, level: LogEntry["level"], source: LogEntry["source"], message: string): void {
  snapshot.logs = [
    {
      id: `${Date.now()}-${snapshot.logs.length}`,
      timestamp: now(),
      level,
      source,
      message,
    },
    ...snapshot.logs,
  ].slice(0, 32);
}

function validateParameters(parameters: ScanParameters): void {
  if (!parameters.taskId.trim()) throw new Error("Task ID is required");
  if (!parameters.savePath.trim()) throw new Error("Save path is required");
  if (!Number.isInteger(parameters.projectionCount) || parameters.projectionCount < 4 || parameters.projectionCount > 3600) {
    throw new Error("Projection count must be between 4 and 3600");
  }
  if (!Number.isFinite(parameters.angleStepDeg) || parameters.angleStepDeg <= 0 || parameters.angleStepDeg > 360) {
    throw new Error("Angle step must be between 0 and 360 degrees");
  }
  if (!Number.isInteger(parameters.exposureMs) || parameters.exposureMs < 1 || parameters.exposureMs > 10000) {
    throw new Error("Exposure must be between 1 and 10000 ms");
  }
}

export class DevPreviewAdapter implements EngineAdapter {
  readonly kind = "developer_preview" as const;
  private snapshot: EngineSnapshot = makeInitialSnapshot();
  private lastTick = Date.now();

  async getSnapshot(): Promise<EngineSnapshot> {
    this.tick();
    return structuredClone(this.snapshot);
  }

  async dispatch(command: EngineCommand): Promise<EngineSnapshot> {
    this.tick();
    this.snapshot.lastError = null;

    switch (command.type) {
      case "connect":
        if (command.adapter === "real_hardware") {
          throw new Error("V1 only permits the developer preview adapter; real hardware remains locked");
        }
        this.snapshot.connectionState = "connected";
        this.snapshot.adapterLabel = "Developer Preview Adapter · Simulated State Machine";
        this.snapshot.devices = makeDevices("connected");
        this.snapshot.phase = "idle";
        this.snapshot.phaseLabel = phaseLabels.idle;
        this.snapshot.preflightPassed = false;
        this.snapshot.homed = false;
        this.snapshot.requiresPreflight = true;
        this.snapshot.requiresHome = true;
        appendLog(this.snapshot, "success", "系统", "Developer preview connected; no serial, camera or X-ray access");
        break;
      case "disconnect":
        this.snapshot = makeInitialSnapshot();
        appendLog(this.snapshot, "warning", "系统", "Preview adapter disconnected; safety state reset");
        break;
      case "preflight":
        this.ensureConnected();
        this.snapshot.preflightPassed = true;
        this.snapshot.requiresPreflight = false;
        this.snapshot.phase = "ready_for_home";
        this.snapshot.phaseLabel = phaseLabels.ready_for_home;
        this.snapshot.devices = makeDevices("ready");
        appendLog(this.snapshot, "success", "系统", "Preview preflight passed; turntable and camera are simulated only");
        break;
      case "home":
        this.ensureConnected();
        if (!this.snapshot.preflightPassed) throw new Error("Run preflight first");
        if (this.snapshot.phase === "running" || this.snapshot.phase === "paused") throw new Error("Homing is unavailable during a scan");
        this.snapshot.homed = true;
        this.snapshot.requiresHome = false;
        this.snapshot.progress.angleDeg = 0;
        this.snapshot.phase = "ready";
        this.snapshot.phaseLabel = phaseLabels.ready;
        appendLog(this.snapshot, "success", "转台", "Preview homing complete; current position 0.00 degrees");
        break;
      case "set_parameters":
        validateParameters(command.parameters);
        if (this.snapshot.phase === "running" || this.snapshot.phase === "paused") throw new Error("Parameters cannot change during a scan");
        this.snapshot.parameters = structuredClone(command.parameters);
        this.snapshot.progress.total = command.parameters.projectionCount;
        this.snapshot.progress.current = Math.min(this.snapshot.progress.current, command.parameters.projectionCount);
        this.snapshot.progress.percent = Math.round((this.snapshot.progress.current / command.parameters.projectionCount) * 100);
        appendLog(this.snapshot, "info", "系统", "Scan parameters applied to developer preview");
        break;
      case "start_scan":
        this.ensureReady();
        this.snapshot.phase = "running";
        this.snapshot.phaseLabel = phaseLabels.running;
        this.snapshot.progress = {
          current: 0,
          total: this.snapshot.parameters.projectionCount,
          percent: 0,
          angleDeg: 0,
          etaSeconds: Math.ceil(this.snapshot.parameters.projectionCount * 0.85),
        };
        this.snapshot.imageCount = 0;
        this.snapshot.devices = makeDevices("busy");
        appendLog(this.snapshot, "info", "系统", "Developer preview started: motion workflow only, no X-ray output");
        break;
      case "pause":
        if (this.snapshot.phase !== "running") throw new Error("No running scan can be paused");
        this.snapshot.phase = "paused";
        this.snapshot.phaseLabel = phaseLabels.paused;
        this.snapshot.devices = makeDevices("connected");
        appendLog(this.snapshot, "warning", "系统", `Preview paused at position ${this.snapshot.progress.current}`);
        break;
      case "resume":
        if (this.snapshot.phase !== "paused") throw new Error("No paused scan can be resumed");
        this.snapshot.phase = "running";
        this.snapshot.phaseLabel = phaseLabels.running;
        this.snapshot.devices = makeDevices("busy");
        appendLog(this.snapshot, "info", "系统", "Preview resumed");
        break;
      case "restore_previous":
        this.ensureConnected();
        if (this.snapshot.phase === "running" || this.snapshot.phase === "paused") throw new Error("Progress cannot be restored while a workflow is active");
        this.snapshot.preflightPassed = true;
        this.snapshot.homed = true;
        this.snapshot.requiresPreflight = false;
        this.snapshot.requiresHome = false;
        this.snapshot.progress.current = Math.max(1, Math.floor(this.snapshot.parameters.projectionCount / 3));
        this.snapshot.progress.total = this.snapshot.parameters.projectionCount;
        this.snapshot.progress.percent = Math.round((this.snapshot.progress.current / this.snapshot.progress.total) * 100);
        this.snapshot.progress.angleDeg = Number((this.snapshot.progress.current * this.snapshot.parameters.angleStepDeg).toFixed(2));
        this.snapshot.progress.etaSeconds = Math.ceil((this.snapshot.progress.total - this.snapshot.progress.current) * 0.85);
        this.snapshot.phase = "paused";
        this.snapshot.phaseLabel = phaseLabels.paused;
        this.snapshot.devices = makeDevices("connected");
        appendLog(this.snapshot, "warning", "系统", `Developer preview progress restored: ${this.snapshot.progress.current} / ${this.snapshot.progress.total}; no real task file was read`);
        break;
      case "stop":
        this.ensureConnected();
        this.snapshot.phase = "stopped";
        this.snapshot.phaseLabel = phaseLabels.stopped;
        this.snapshot.devices = makeDevices("connected");
        this.snapshot.preflightPassed = false;
        this.snapshot.homed = false;
        this.snapshot.requiresPreflight = true;
        this.snapshot.requiresHome = true;
        appendLog(this.snapshot, "error", "系统", "Emergency stop executed; preflight and home are invalidated");
        break;
    }
    this.snapshot.updatedAt = now();
    return structuredClone(this.snapshot);
  }

  private ensureConnected(): void {
    if (this.snapshot.connectionState !== "connected") throw new Error("Connect the developer preview adapter first");
  }

  private ensureReady(): void {
    this.ensureConnected();
    if (!this.snapshot.preflightPassed) throw new Error("Run preflight first");
    if (!this.snapshot.homed) throw new Error("Home the turntable first");
    if (!this.snapshot.safety.xrayAvailable) {
      // A preview scan is intentionally allowed: this is motion/process demonstration only.
      appendLog(this.snapshot, "warning", "射线", "X-ray is unavailable; running a no-output workflow preview");
    }
  }

  private tick(): void {
    const currentTime = Date.now();
    const elapsed = currentTime - this.lastTick;
    this.lastTick = currentTime;
    if (this.snapshot.phase !== "running" || elapsed < 500) {
      this.snapshot.updatedAt = now();
      return;
    }
    const increments = Math.max(1, Math.floor(elapsed / 850));
    const next = Math.min(this.snapshot.progress.current + increments, this.snapshot.progress.total);
    this.snapshot.progress.current = next;
    this.snapshot.progress.percent = Math.round((next / this.snapshot.progress.total) * 100);
    this.snapshot.progress.angleDeg = Number(((next * this.snapshot.parameters.angleStepDeg) % 360).toFixed(2));
    this.snapshot.progress.etaSeconds = next >= this.snapshot.progress.total ? 0 : Math.ceil((this.snapshot.progress.total - next) * 0.85);
    // Preview motion positions are not image files. Keep this at zero until a
    // real camera adapter reports host-validated captures.
    this.snapshot.imageCount = 0;
    this.snapshot.updatedAt = now();
    if (next >= this.snapshot.progress.total) {
      this.snapshot.phase = "completed";
      this.snapshot.phaseLabel = phaseLabels.completed;
      this.snapshot.devices = makeDevices("ready");
      this.snapshot.preflightPassed = true;
      this.snapshot.homed = true;
      this.snapshot.requiresPreflight = false;
      this.snapshot.requiresHome = false;
      appendLog(this.snapshot, "success", "系统", "Developer preview completed; no real projections were created");
    }
  }
}
