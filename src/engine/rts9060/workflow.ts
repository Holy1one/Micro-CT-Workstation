/**
 * Browser-only scan workflow simulator retained for UI development.
 * Historical Python-host references explain its origin but are not an active
 * protocol source; production behavior is authoritative in the Rust engine.
 *
 * Pipeline per projection:
 *   MOVE_ABS -> READY_TO_CAPTURE -> settle -> beam on -> exposure window ->
 *   D7100 capture -> frame validation -> CAPTURE_DONE -> commit -> next view
 *
 * Pause takes effect at the projection commit boundary with completed frames
 * retained. A normal stop ends the current scan and invalidates HOME; device
 * faults latch the preview output until preflight and HOME re-establish state.
 */

import { CameraD7100, delay, XraySource12W, type CapturedFrame } from "./devices";
import { cmd, degreesForPulses, FIRMWARE_VERSION, milliDeg, parseLine, pulsesForDegrees, type NanoStatus, type ParsedLine, parseStatus, PULSES_PER_REV } from "./protocol";
import { FirmwareTransport, type NanoTransport } from "./transport";

export type WorkflowPhase = "booting" | "ready" | "scanning" | "paused" | "finishing" | "stopping" | "fault" | "stopped" | "completed";
export type ConsoleLogLevel = "PASS" | "INFO" | "OK" | "WARN" | "ERR" | "ACTION";
export type ConsoleLogSource = "system" | "xray" | "nano" | "camera" | "preflight" | "operator";

export interface ConsoleLogEntry {
  id: string;
  timestamp: string;
  level: ConsoleLogLevel;
  source: ConsoleLogSource;
  message: string;
}

export interface ScanParams {
  savePath: string;
  taskId: string;
  projectionCount: number;
  exposureMs: number;
  maxXraySec: number;
}

interface Checkpoint {
  taskId: string;
  savePath: string;
  projectionCount: number;
  exposureMs: number;
  view: number;
  angleDeg: number;
  ts: string;
}

const CHECKPOINT_KEY = "micro-ct-workstation.checkpoint";
const XRAY_COOLDOWN_MS = 300_000;
const MAX_LOGS = 64;

class AbortView extends Error {
  constructor() {
    super("view aborted");
  }
}

/** Command transactor over a NanoTransport: id allocation + terminal-line matching. */
class NanoLink {
  private nextId = 1;
  private seq = 1;
  private waiters: Array<{ id: number; terminals: string[]; resolve: (line: ParsedLine) => void; reject: (err: Error) => void; timer: ReturnType<typeof setTimeout> }> = [];
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private bannerResolve: (() => void) | null = null;
  readonly banner: Promise<void>;
  lastStatus: NanoStatus | null = null;
  private transport: NanoTransport;
  private onRx?: (line: ParsedLine) => void;

  constructor(transport: NanoTransport, onRx?: (line: ParsedLine) => void) {
    this.transport = transport;
    this.onRx = onRx;
    this.banner = new Promise((resolve) => {
      this.bannerResolve = resolve;
    });
    transport.onLine((raw) => this.receive(raw));
    this.heartbeatTimer = setInterval(() => {
      this.transport.send(cmd.heartbeat(this.seq++));
    }, 500);
  }

  private receive(raw: string): void {
    const line = parseLine(raw);
    if (line.token === "STATUS") this.lastStatus = parseStatus(line);
    if (line.token === "READY" && this.bannerResolve) {
      this.bannerResolve();
      this.bannerResolve = null;
    }
    if (line.token === "ERR") {
      const waiter = this.take(line.id);
      if (waiter) waiter.reject(new Error(`firmware error: ${line.rest.join(" ") || line.raw}`));
    } else {
      const waiter = this.waiters.find((w) => w.id === line.id && w.terminals.includes(line.token));
      if (waiter) {
        this.take(line.id);
        waiter.resolve(line);
      }
    }
    if (line.token === "STOPPED") {
      const interruptedIds = this.waiters.filter((waiter) => waiter.id !== line.id).map((waiter) => waiter.id);
      for (const id of interruptedIds) this.take(id)?.reject(new Error("operation interrupted by turntable STOP"));
    }
    this.onRx?.(line);
  }

  private take(id: number | null) {
    const index = this.waiters.findIndex((w) => w.id === id);
    if (index < 0) return null;
    const [waiter] = this.waiters.splice(index, 1);
    clearTimeout(waiter.timer);
    return waiter;
  }

  exec(line: string, terminals: string[], timeoutMs = 4000): Promise<ParsedLine> {
    const id = this.nextId++;
    const wire = line.replace("{id}", String(id));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.take(id);
        reject(new Error(`timeout waiting for ${terminals.join("/")} (id ${id})`));
      }, timeoutMs);
      this.waiters.push({ id, terminals, resolve, reject, timer });
      this.transport.send(wire);
    });
  }

  close(): void {
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    for (const waiter of [...this.waiters]) this.take(waiter.id)?.reject(new Error("preview link closed"));
    this.transport.close();
  }
}

function nowIso(): string {
  return new Date().toISOString();
}

function formatMmSs(totalSeconds: number): string {
  const s = Math.max(0, Math.round(totalSeconds));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

export class ScanWorkflow {
  readonly source = new XraySource12W();
  readonly camera = new CameraD7100();
  private link: NanoLink;
  private transport: NanoTransport;

  phase: WorkflowPhase = "booting";
  params: ScanParams = {
    savePath: "",
    taskId: "",
    projectionCount: 0,
    exposureMs: 0,
    maxXraySec: 600,
  };
  preflightPassed = false;
  preflightChecks = 0;
  preflightRunning = false;
  homed = false;
  captured = 0;
  angleDeg = 0;
  frames: CapturedFrame[] = [];
  logs: ConsoleLogEntry[] = [];
  timerOn = false;
  usbAutoShutDown = true;
  usbShutdownDelay = 5;
  checkpointAvailable = false;

  private pauseRequested = false;
  private stopRequested = false;
  private scanRunning = false;
  private scanTask: Promise<void> | null = null;
  private stopTask: Promise<void> | null = null;
  private stopSignal: Promise<void> = new Promise(() => undefined);
  private resolveStopSignal: (() => void) | null = null;
  private logCounter = 0;
  private projectionSamplesMs: number[] = [];
  private beamSamplesMs: number[] = [];
  private beamStartedAt: number | null = null;
  private cooldownUntil: number | null = null;
  private onChange: () => void;

  constructor(onChange: () => void, transport?: NanoTransport) {
    this.onChange = onChange;
    this.transport = transport ?? new FirmwareTransport();
    this.link = new NanoLink(this.transport);
    this.checkpointAvailable = this.readCheckpoint() !== null;
    void this.boot();
  }

  get angleStepDeg(): number {
    return this.params.projectionCount > 0 ? 360 / this.params.projectionCount : 0;
  }

  get pulsesPerView(): number {
    return pulsesForDegrees(this.angleStepDeg);
  }

  get etaText(): string {
    if (this.phase === "paused" && this.cooldownUntil === null) return "held";
    const seconds = this.etaSeconds;
    return seconds === null ? "—" : formatMmSs(seconds);
  }

  get etaSeconds(): number | null {
    if (this.phase !== "scanning" || this.projectionSamplesMs.length === 0) return null;
    const remaining = this.params.projectionCount - this.captured;
    if (remaining <= 0) return 0;
    const average = (values: number[]) => values.reduce((sum, value) => sum + value, 0) / values.length;
    const projectionMs = average(this.projectionSamplesMs);
    const beamMs = average(this.beamSamplesMs);
    const limitMs = this.params.maxXraySec * 1000;
    const coolingMs = this.cooldownUntil === null ? 0 : Math.max(0, this.cooldownUntil - Date.now());
    const beamBudget = this.beamStartedAt === null ? 0 : Math.max(0, Date.now() - this.beamStartedAt);
    const futureCoolingCount = Math.floor((beamBudget + Math.max(0, remaining - 1) * beamMs) / limitMs);
    return Math.ceil((remaining * projectionMs + coolingMs + futureCoolingCount * XRAY_COOLDOWN_MS) / 1000);
  }

  get manualControlsEnabled(): boolean {
    if (!this.source.isConnected || this.source.isLatched || this.preflightRunning) return false;
    if (this.phase === "scanning") return true;
    if (["booting", "paused", "finishing", "stopping", "fault"].includes(this.phase)) return false;
    if (this.source.beamOn) return true;
    return (this.phase === "ready" || this.phase === "completed") && this.preflightPassed && this.homed;
  }

  get setpointControlsEnabled(): boolean {
    return (
      this.source.isConnected &&
      !this.source.isLatched &&
      !this.preflightRunning &&
      !["booting", "scanning", "paused", "finishing", "stopping", "fault"].includes(this.phase)
    );
  }

  get timerControlsEnabled(): boolean {
    return !this.preflightRunning && ["ready", "stopped", "completed"].includes(this.phase);
  }

  // ------------------------------------------------------------- lifecycle

  private async boot(): Promise<void> {
    try {
      await this.link.banner;
      await this.link.exec(cmd.status("{id}"), ["STATUS"]);
      await this.link.exec(cmd.setMicrosteps("{id}", 8), ["OK"]);
      this.log("INFO", "system", `DEVELOPER PREVIEW engine ready · NO REAL HARDWARE · NO DEVICE I/O · build ${FIRMWARE_VERSION}`);
      await delay(140);
      this.log("INFO", "xray", `DEVELOPER PREVIEW X-ray · PREVIEW DATA ${this.source.readback().setKv.toFixed(1)} kV / ${this.source.readback().setUa.toFixed(1)} µA · NO DEVICE I/O`);
      this.camera.configure(this.params.savePath);
      await this.link.exec(cmd.rearm("{id}"), ["OK"]);
      await this.link.exec(cmd.home("{id}"), ["HOME_DONE"], 6000);
      this.homed = true;
      this.log("INFO", "nano", "turntable homed · Hall LOW at 0.00°");
      await delay(120);
      this.log("INFO", "camera", `DEVELOPER PREVIEW camera · PREVIEW DATA ${this.params.projectionCount} views · ${this.params.exposureMs} ms · NO DEVICE I/O`);
      if (
        this.params.taskId.trim() &&
        this.params.savePath.trim() &&
        this.params.projectionCount > 0 &&
        this.params.exposureMs > 0 &&
        this.params.maxXraySec > 0
      ) {
        await this.runPreflight();
      } else {
        this.log("WARN", "preflight", "Scan setup is incomplete · pre-inspection remains pending");
      }
      if (this.phase === "booting") this.phase = "ready";
      this.emit();
    } catch (err) {
      this.source.latchOff();
      this.preflightPassed = false;
      this.homed = false;
      this.log("ERR", "system", `boot sequence failed · ${err instanceof Error ? err.message : String(err)}`);
      this.phase = "fault";
      this.emit();
    }
  }

  async runPreflight(): Promise<void> {
    if (this.preflightRunning) return;
    if (this.scanRunning || this.stopTask || ["scanning", "finishing", "stopping"].includes(this.phase)) {
      throw new Error("Preflight is unavailable while a scan is active");
    }
    this.validateCompleteParams(this.params);
    const recovering = this.phase === "fault" || this.phase === "stopped";
    this.preflightRunning = true;
    this.preflightPassed = false;
    this.preflightChecks = 0;
    this.emit();
    try {
      const checks = 8;
      for (let i = 1; i <= checks; i++) {
        await delay(130);
        this.preflightChecks = i;
        this.emit();
      }
      // REARM is part of a successful preview preflight, never an operator
      // supplied latch-release command. HOME remains a separate requirement.
      await this.link.exec(cmd.rearm("{id}"), ["OK"]);
      this.source.rearm();
      this.preflightPassed = true;
      if (recovering) {
        this.homed = false;
        this.phase = "stopped";
      }
      this.log("PASS", "preflight", "8/8 DEVELOPER PREVIEW checks passed · NO REAL HARDWARE · NO DEVICE I/O");
    } catch (err) {
      if (recovering) this.phase = "fault";
      this.log("ERR", "preflight", `preview preflight failed · ${err instanceof Error ? err.message : String(err)}`);
      throw err;
    } finally {
      this.preflightRunning = false;
      this.emit();
    }
  }

  // --------------------------------------------------------------- commands

  async home(): Promise<void> {
    this.ensureNotScanning("HOME");
    if (this.phase === "fault") throw new Error("Run preflight before HOME to recover from the device fault");
    if (!this.preflightPassed) throw new Error("Run preflight before HOME");
    if (this.phase === "paused") {
      this.captured = 0;
      this.frames = [];
      this.clearCheckpoint();
      this.log("WARN", "system", "HOME during paused scan · buffered progress cleared");
    }
    if (!this.link.lastStatus?.rearmed) {
      await this.link.exec(cmd.rearm("{id}"), ["OK"]);
      this.log("INFO", "nano", "REARM · motion chain re-armed");
    }
    this.log("ACTION", "operator", "HOME requested · seeking Hall reference");
    await this.link.exec(cmd.home("{id}"), ["HOME_DONE"], 6000);
    this.homed = true;
    this.angleDeg = 0;
    // Re-referencing the turntable starts a fresh acquisition queue; the
    // on-disk checkpoint is kept so Restore can still resume the aborted run.
    this.captured = 0;
    this.frames = [];
    this.log("INFO", "nano", "turntable homed · Hall LOW at 0.00°");
    if (this.phase !== "booting") this.phase = "ready";
    this.emit();
  }

  startScan(): void {
    if (this.scanRunning || this.stopTask) throw new Error("A scan is already active or stopping");
    if (this.phase === "fault") throw new Error("Device fault latched · run preflight and HOME to recover");
    if (!["ready", "completed", "paused"].includes(this.phase)) throw new Error("Scan is unavailable until the current operation has ended");
    if (!this.preflightPassed) throw new Error("Run pre-inspection first");
    if (!this.homed) throw new Error("Home the turntable first");
    const resumeFrom = this.phase === "paused" ? this.captured : 0;
    if (resumeFrom === 0) {
      this.frames = [];
      this.angleDeg = 0;
      this.projectionSamplesMs = [];
      this.beamSamplesMs = [];
      this.beamStartedAt = null;
      this.cooldownUntil = null;
    }
    this.log("ACTION", "operator", resumeFrom > 0 ? `resume requested · continuing from view ${resumeFrom + 1}` : "start requested · acquisition begins at view 1");
    this.pauseRequested = false;
    this.stopRequested = false;
    this.stopSignal = new Promise<void>((resolve) => { this.resolveStopSignal = resolve; });
    this.phase = "scanning";
    this.scanRunning = true;
    this.emit();
    const task = this.scanLoop(resumeFrom + 1).finally(() => {
      this.scanRunning = false;
      this.scanTask = null;
      this.resolveStopSignal = null;
      this.emit();
    });
    this.scanTask = task;
  }

  pause(): void {
    if (this.phase !== "scanning") return;
    this.pauseRequested = true;
    this.log("ACTION", "operator", "pause requested · applying after the active projection commits");
    this.emit();
  }

  stop(): Promise<void> {
    if (this.stopTask) return this.stopTask;
    if (!["scanning", "paused", "finishing"].includes(this.phase)) return Promise.resolve();
    const task = this.performStop();
    this.stopTask = task.finally(() => { this.stopTask = null; });
    return this.stopTask;
  }

  private async performStop(): Promise<void> {
    this.stopRequested = true;
    this.pauseRequested = false;
    this.phase = "stopping";
    this.source.disable();
    this.resolveStopSignal?.();
    this.log("ACTION", "operator", "scan stop requested · ending the active scan");
    this.emit();

    const scanTask = this.scanTask;
    let stopError: unknown = null;
    try {
      await this.link.exec(cmd.stop("{id}"), ["STOPPED"], 2500);
    } catch (err) {
      stopError = err;
      this.failClosed(`turntable STOP was not confirmed: ${err instanceof Error ? err.message : String(err)}`);
    }
    try {
      await this.link.exec(cmd.xrayWarning("{id}", false), ["OK"], 1500);
    } catch (err) {
      stopError ??= err;
      this.failClosed(`XRAY_WARNING OFF was not confirmed: ${err instanceof Error ? err.message : String(err)}`);
    }
    if (scanTask) {
      try {
        await this.withTimeout(scanTask, 3000, "scan task did not terminate after STOP");
      } catch (err) {
        stopError ??= err;
        this.failClosed(err instanceof Error ? err.message : String(err));
      }
    }
    if (stopError) throw stopError;

    this.preflightPassed = false;
    this.homed = false;
    this.phase = "stopped";
    this.log("PASS", "system", `scan stopped · ${this.captured} / ${this.params.projectionCount} committed · run preflight and HOME before another scan`);
    this.emit();
  }

  async restore(): Promise<void> {
    this.ensureNotScanning("restore");
    if (this.phase === "fault") {
      throw new Error("Device fault latched · run preflight and HOME before restoring");
    }
    if (this.phase === "stopped" || !this.preflightPassed || !this.homed) {
      throw new Error("Recovery required · repeat pre-inspection and HOME before restore");
    }
    const checkpoint = this.readCheckpoint();
    if (!checkpoint) {
      this.log("WARN", "operator", "no previous scan progress on disk · nothing to restore");
      this.emit();
      return;
    }
    this.log("ACTION", "operator", `load previous progress · ${checkpoint.taskId} view ${checkpoint.view} / ${checkpoint.projectionCount}`);
    this.params = {
      ...this.params,
      taskId: checkpoint.taskId,
      savePath: checkpoint.savePath,
      projectionCount: checkpoint.projectionCount,
      exposureMs: checkpoint.exposureMs,
    };
    this.camera.configure(this.params.savePath);
    if (!this.link.lastStatus?.rearmed) await this.link.exec(cmd.rearm("{id}"), ["OK"]);
    await this.link.exec(cmd.moveAbs("{id}", milliDeg(checkpoint.angleDeg)), ["READY_TO_CAPTURE"], 8000);
    await this.link.exec(cmd.captureDone("{id}"), ["IDLE"]);
    this.captured = checkpoint.view;
    this.angleDeg = checkpoint.angleDeg;
    this.frames = Array.from({ length: checkpoint.view }, (_, i) => ({
      index: i + 1,
      angleDeg: i * this.angleStepDeg,
      exposureMs: this.params.exposureMs,
      fileName: `frame-${String(i + 1).padStart(4, "0")}.nef`,
      path: `${this.params.savePath}/frame-${String(i + 1).padStart(4, "0")}.nef`,
      shaOk: true,
    }));
    this.phase = "paused";
    this.pauseRequested = true;
    this.log("INFO", "system", `progress restored · ${this.captured} / ${this.params.projectionCount} at ${this.angleDeg.toFixed(2)}° · press resume to continue`);
    this.emit();
  }

  async retryDevice(id: "xray" | "turntable" | "camera"): Promise<void> {
    if (this.phase === "fault") {
      this.log("WARN", "system", `${id} retry deferred · run preflight and HOME to recover from the device fault`);
      this.emit();
      return;
    }
    const source: ConsoleLogSource = id === "turntable" ? "nano" : id;
    await delay(260);
    if (id === "xray") this.source.connect();
    if (id === "turntable") await this.link.exec(cmd.ping("{id}"), ["PONG"]).catch(() => undefined);
    this.log("INFO", source, `${id === "turntable" ? "turntable" : id} retry · link OK`);
    this.emit();
  }

  xrayDisconnect(): void {
    if (["scanning", "paused", "finishing", "stopping"].includes(this.phase)) return;
    this.source.disconnect();
    this.log("ACTION", "xray", "preview X-ray disconnected · NO DEVICE I/O");
    this.emit();
  }

  setParams(partial: Partial<ScanParams>): void {
    if (["scanning", "paused", "finishing", "stopping"].includes(this.phase)) return;
    if (Object.keys(partial).length === 0) throw new Error("Scan setup patch is empty");

    const next = { ...this.params, ...partial };
    if ("savePath" in partial && !next.savePath.trim()) {
      throw new Error("Save path must not be empty");
    }
    if ("taskId" in partial && !next.taskId.trim()) {
      throw new Error("Task ID must not be empty");
    }
    if (
      "projectionCount" in partial &&
      (!Number.isInteger(next.projectionCount) || next.projectionCount < 1 || next.projectionCount > 3600)
    ) {
      throw new Error("Projection count must be between 1 and 3600");
    }
    if (
      "exposureMs" in partial &&
      (!Number.isFinite(next.exposureMs) || next.exposureMs < 0.125 || next.exposureMs > 30000)
    ) {
      throw new Error("Exposure must be between 0.125 and 30000 ms");
    }
    if (
      "maxXraySec" in partial &&
      (!Number.isInteger(next.maxXraySec) || next.maxXraySec < 1 || next.maxXraySec > 600)
    ) {
      throw new Error("Maximum continuous X-ray duration must be between 1 and 600 seconds");
    }

    this.params = next;
    this.preflightPassed = false;
    this.homed = false;
    if (this.phase !== "fault" && this.phase !== "booting" && this.phase !== "stopped") this.phase = "ready";
    this.camera.configure(this.params.savePath);
    this.log("INFO", "system", `parameters updated · ${this.params.projectionCount} views · ${this.angleStepDeg.toFixed(2)}°/view · ${this.params.exposureMs} ms`);
    this.emit();
  }

  sendVoltage(kv: number): void {
    if (!this.setpointControlsEnabled) throw new Error("X-ray setpoint controls are unavailable in the current preview phase");
    const before = this.source.readback();
    this.source.setVoltage(kv);
    const applied = this.source.readback();
    if (applied.setUa !== before.setUa) {
      this.log("WARN", "xray", `SEND V requested ${kv.toFixed(1)} kV · current auto-adjusted from ${before.setUa.toFixed(1)} µA to ${applied.setUa.toFixed(1)} µA · 12 W limit`);
    }
    this.log("INFO", "xray", `SEND V applied · final ${applied.setKv.toFixed(1)} kV / ${applied.setUa.toFixed(1)} µA`);
    this.emit();
  }

  sendCurrent(ua: number): void {
    if (!this.setpointControlsEnabled) throw new Error("X-ray setpoint controls are unavailable in the current preview phase");
    const before = this.source.readback();
    this.source.setCurrent(ua);
    const applied = this.source.readback();
    if (applied.setKv !== before.setKv) {
      this.log("WARN", "xray", `SEND I requested ${ua.toFixed(1)} µA · voltage auto-adjusted from ${before.setKv.toFixed(1)} kV to ${applied.setKv.toFixed(1)} kV · 12 W limit`);
    }
    this.log("INFO", "xray", `SEND I applied · final ${applied.setKv.toFixed(1)} kV / ${applied.setUa.toFixed(1)} µA`);
    this.emit();
  }

  async xrayToggle(): Promise<void> {
    if (this.phase === "scanning") {
      this.log("ACTION", "operator", "Xray Disable requested during scan · pausing at commit boundary");
      this.pause();
      return;
    }
    if (["paused", "finishing", "stopping", "fault"].includes(this.phase)) return;
    if (this.source.beamOn) {
      this.source.disable();
      this.log("ACTION", "operator", "Xray Disable · tube off");
      this.log("INFO", "xray", "beam off · output disabled");
    } else {
      if (!this.preflightPassed || !this.homed || this.preflightRunning) {
        throw new Error("Run preflight and HOME before enabling preview X-ray output");
      }
      this.source.enable();
      this.log("ACTION", "operator", "Xray Enable · tube on at setpoint");
      this.log("WARN", "xray", `beam on · ${this.source.readback().setKv.toFixed(1)} kV / ${this.source.readback().setUa.toFixed(1)} µA · interlock closed`);
    }
    this.emit();
  }

  timerToggle(): void {
    if (!this.timerControlsEnabled) return;
    this.timerOn = !this.timerOn;
    this.log("INFO", "xray", this.timerOn ? "exposure timer on · hardware window armed" : "exposure timer off");
    this.emit();
  }

  usbAutoShutDownToggle(): void {
    this.usbAutoShutDown = !this.usbAutoShutDown;
    this.log("INFO", "xray", this.usbAutoShutDown ? "USB auto shut down armed" : "USB auto shut down cleared");
    this.emit();
  }

  setUsbShutdownDelay(delay: number): void {
    if (!Number.isFinite(delay) || delay <= 0) return;
    this.usbShutdownDelay = Math.round(delay);
    this.log("INFO", "xray", `USB shutdown delay set to ${this.usbShutdownDelay}`);
    this.emit();
  }

  private validateCompleteParams(params: ScanParams, requireText = true): void {
    if (requireText && !params.taskId.trim()) {
      throw new Error("Enter a non-empty Task ID before pre-inspection");
    }
    if (requireText && !params.savePath.trim()) {
      throw new Error("Select a non-empty Save Path before pre-inspection");
    }
    if (!Number.isInteger(params.projectionCount) || params.projectionCount < 1 || params.projectionCount > 3600) {
      throw new Error("Projection count must be between 1 and 3600");
    }
    if (!Number.isFinite(params.exposureMs) || params.exposureMs < 0.125 || params.exposureMs > 30000) {
      throw new Error("Exposure must be between 0.125 and 30000 ms");
    }
    if (!Number.isInteger(params.maxXraySec) || params.maxXraySec < 1 || params.maxXraySec > 600) {
      throw new Error("Maximum continuous X-ray duration must be between 1 and 600 seconds");
    }
  }

  // -------------------------------------------------------------- scan loop

  private async scanLoop(startView: number): Promise<void> {
    const total = this.params.projectionCount;
    try {
      for (let view = startView; view <= total; view++) {
        if (this.stopRequested) return;
        if (this.cooldownUntil !== null) {
          this.log("WARN", "xray", "Continuous X-ray limit reached · cooling for 300 seconds before resuming");
          let cooldownTimer: ReturnType<typeof setTimeout> | null = null;
          try {
            await Promise.race([
              new Promise<void>((resolve) => { cooldownTimer = setTimeout(resolve, Math.max(0, this.cooldownUntil! - Date.now())); }),
              this.stopSignal.then(() => { throw new AbortView(); }),
            ]);
          } finally {
            if (cooldownTimer !== null) clearTimeout(cooldownTimer);
          }
          this.cooldownUntil = null;
          this.emit();
        }
        const projectionStartedAt = Date.now();
        const angle = (view - 1) * this.angleStepDeg;
        await this.awaitOrStop(this.link.exec(cmd.moveAbs("{id}", milliDeg(angle)), ["READY_TO_CAPTURE"], 8000));
        this.angleDeg = angle;
        this.log("INFO", "nano", `MOVE_ABS view ${view} → ${angle.toFixed(2)}° · ACK`);
        this.emit();
        if (this.stopRequested) return;
        if (!this.source.beamOn) {
          this.source.enable();
          this.beamStartedAt = Date.now();
          await this.link.exec(cmd.xrayWarning("{id}", true), ["OK"]);
        }
        const rb = this.source.readback();
        this.log("INFO", "xray", `DEVELOPER PREVIEW exposure · PREVIEW DATA ${this.params.exposureMs} ms · ${rb.setKv.toFixed(1)} kV / ${rb.setUa.toFixed(1)} µA · NO DEVICE I/O`);
        await this.awaitOrStop(delay(Math.max(this.params.exposureMs, 240)));
        const frame = await this.awaitOrStop(this.camera.capture(view, angle, this.params.exposureMs));
        if (this.stopRequested) return;
        if (!frame.shaOk) throw new Error(`frame ${frame.fileName} failed validation`);
        await this.awaitOrStop(this.link.exec(cmd.captureDone("{id}"), ["IDLE"]));
        this.frames = [...this.frames, frame];
        this.captured = view;
        this.projectionSamplesMs.push(Math.max(1, Date.now() - projectionStartedAt));
        this.beamSamplesMs.push(Math.max(1, Date.now() - Math.max(projectionStartedAt, this.beamStartedAt ?? projectionStartedAt)));
        if (this.beamStartedAt !== null && Date.now() - this.beamStartedAt >= this.params.maxXraySec * 1000 && view < total) {
          await this.closeOutputAtBoundary("Continuous X-ray limit reached after completed exposure");
          this.beamStartedAt = null;
          this.cooldownUntil = Date.now() + XRAY_COOLDOWN_MS;
        }
        this.log("OK", "camera", `captured view ${view} · stored ${this.captured} / ${total}`);
        this.log("INFO", "nano", "CAPTURE_DONE sent · awaiting next move");
        this.writeCheckpoint(view, angle);
        if (view < total) {
          const nextAngle = view * this.angleStepDeg;
          this.log("INFO", "system", `view ${view + 1} queued → ${nextAngle.toFixed(2)}° · ETA ${this.etaText}`);
        }
        this.emit();
        if (this.stopRequested) return;
        if (this.pauseRequested) {
          await this.closeOutputAtBoundary("Pause boundary");
          this.pauseRequested = false;
          this.phase = "paused";
          this.log("PASS", "system", `scan paused at a committed projection · ${this.captured} / ${total} retained`);
          this.emit();
          return;
        }
      }
      if (this.stopRequested) return;
      if (this.pauseRequested) {
        await this.closeOutputAtBoundary("Pause before finalization");
        this.pauseRequested = false;
        this.phase = "paused";
        this.emit();
        return;
      }
      await this.finishScan(total);
    } catch (err) {
      if (this.stopRequested || (err instanceof AbortView && this.stopRequested)) return;
      await this.failScan(err);
    }
  }

  private async finishScan(total: number): Promise<void> {
    this.source.disable();
    await this.closeOutputAtBoundary("All projections committed");
    if (this.stopRequested) return;
    this.phase = "finishing";
    this.log("ACTION", "nano", "returning to zero through the simulated forward-only 360.000° target");
    this.emit();

    await this.awaitOrStop(this.link.exec(cmd.moveAbs("{id}", milliDeg(360)), ["READY_TO_CAPTURE"], 8000));
    if (this.stopRequested) return;
    const statusLine = await this.awaitOrStop(this.link.exec(cmd.status("{id}"), ["STATUS"]));
    const status = parseStatus(statusLine);
    const expectedPulses = pulsesForDegrees(360);
    if (status.state !== "CAPTURE_HOLD" || status.pos !== expectedPulses || !status.homed || !status.rearmed) {
      throw new Error(`preview final zero check failed: state=${status.state} pos=${status.pos} expected=${expectedPulses} homed=${status.homed} rearmed=${status.rearmed}`);
    }
    await this.awaitOrStop(this.link.exec(cmd.captureDone("{id}"), ["IDLE"]));
    if (this.stopRequested) return;
    this.angleDeg = 0;
    this.phase = "completed";
    this.clearCheckpoint();
    this.log("PASS", "nano", "preview turntable returned to the zero orientation · full forward turn confirmed");
    this.log("PASS", "system", `scan complete · ${total} / ${total} views committed · ${this.params.savePath}`);
    this.emit();
  }

  private async closeOutputAtBoundary(reason: string): Promise<void> {
    this.source.disable();
    await this.link.exec(cmd.xrayWarning("{id}", false), ["OK"]);
    this.log("INFO", "xray", `${reason} · preview beam and warning output disabled`);
  }

  private async failScan(reason: unknown): Promise<void> {
    const message = reason instanceof Error ? reason.message : String(reason);
    this.source.latchOff();
    this.preflightPassed = false;
    this.homed = false;
    this.pauseRequested = false;
    this.phase = "fault";
    this.log("ERR", "nano", message);
    this.log("ERR", "system", "device fault latched · preview X-ray output hard-disabled");
    this.emit();
    try { await this.link.exec(cmd.xrayWarning("{id}", false), ["OK"], 1500); } catch { /* keep the fault latched */ }
    try { await this.link.exec(cmd.stop("{id}"), ["STOPPED"], 2500); } catch { /* keep the fault latched */ }
  }

  private failClosed(message: string): void {
    this.source.latchOff();
    this.preflightPassed = false;
    this.homed = false;
    this.phase = "fault";
    this.log("ERR", "system", `preview stop failed closed · ${message}`);
    this.emit();
  }

  private async awaitOrStop<T>(operation: Promise<T>): Promise<T> {
    if (this.stopRequested) throw new AbortView();
    return Promise.race([
      operation,
      this.stopSignal.then(() => { throw new AbortView(); }),
    ]);
  }

  private async withTimeout<T>(operation: Promise<T>, timeoutMs: number, message: string): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | null = null;
    try {
      return await Promise.race([
        operation,
        new Promise<T>((_resolve, reject) => {
          timer = setTimeout(() => reject(new Error(message)), timeoutMs);
        }),
      ]);
    } finally {
      if (timer) clearTimeout(timer);
    }
  }

  private ensureNotScanning(what: string): void {
    if (this.scanRunning || this.stopTask || ["scanning", "finishing", "stopping"].includes(this.phase)) {
      throw new Error(`${what} is unavailable during a scan`);
    }
  }

  // -------------------------------------------------------------- checkpoint

  private writeCheckpoint(view: number, angleDeg: number): void {
    const checkpoint: Checkpoint = {
      taskId: this.params.taskId,
      savePath: this.params.savePath,
      projectionCount: this.params.projectionCount,
      exposureMs: this.params.exposureMs,
      view,
      angleDeg,
      ts: nowIso(),
    };
    try {
      window.localStorage.setItem(CHECKPOINT_KEY, JSON.stringify(checkpoint));
    } catch {
      /* storage unavailable; restore will report empty */
    }
    this.checkpointAvailable = true;
  }

  private readCheckpoint(): Checkpoint | null {
    try {
      const raw = window.localStorage.getItem(CHECKPOINT_KEY);
      return raw ? (JSON.parse(raw) as Checkpoint) : null;
    } catch {
      return null;
    }
  }

  private clearCheckpoint(): void {
    try {
      window.localStorage.removeItem(CHECKPOINT_KEY);
    } catch {
      /* ignore */
    }
    this.checkpointAvailable = false;
  }

  // ------------------------------------------------------------------- misc

  private log(level: ConsoleLogLevel, source: ConsoleLogSource, message: string): void {
    this.logs = [
      { id: `${Date.now()}-${this.logCounter++}`, timestamp: nowIso(), level, source, message },
      ...this.logs,
    ].slice(0, MAX_LOGS);
  }

  private emit(): void {
    this.onChange();
  }

  close(): void {
    if (["scanning", "paused", "finishing", "stopping"].includes(this.phase)) {
      void this.stop().catch(() => undefined).finally(() => this.link.close());
      return;
    }
    this.source.disable();
    this.link.close();
  }
}

export { degreesForPulses, PULSES_PER_REV };
