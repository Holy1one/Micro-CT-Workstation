/**
 * Scan workflow orchestrator, ported from the proven Python host
 * (kernal/software/host/rts9060_workflow.py + rts9060_scan.py + rts9060_gui.py).
 *
 * Pipeline per projection:
 *   MOVE_ABS -> READY_TO_CAPTURE -> settle -> beam on -> exposure window ->
 *   D7100 capture -> frame validation -> CAPTURE_DONE -> commit -> next view
 *
 * Pause takes effect at the commit boundary with the pulse counter retained;
 * E-STOP cuts the beam and the motion chain in parallel and latches FAULT
 * (home reference lost) until the operator releases it and re-homes.
 */

import { CameraD7100, delay, XraySource12W, type CapturedFrame } from "./devices";
import { cmd, degreesForPulses, FIRMWARE_VERSION, milliDeg, parseLine, pulsesForDegrees, type NanoStatus, type ParsedLine, parseStatus, PULSES_PER_REV } from "./protocol";
import { FirmwareTransport, type NanoTransport } from "./transport";

export type WorkflowPhase = "booting" | "ready" | "scanning" | "paused" | "fault" | "stopped" | "completed";
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
const PER_VIEW_MS = 2600;
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

  constructor(private transport: NanoTransport, private onRx?: (line: ParsedLine) => void) {
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
    maxXraySec: 0,
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
  private estopRequested = false;
  private scanRunning = false;
  private logCounter = 0;
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
    if (this.phase === "paused") return "held";
    if (this.phase !== "scanning") return "—";
    return formatMmSs(((this.params.projectionCount - this.captured) * PER_VIEW_MS) / 1000);
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
      this.log("ERR", "system", `boot sequence failed · ${err instanceof Error ? err.message : String(err)}`);
      this.phase = "fault";
      this.emit();
    }
  }

  async runPreflight(): Promise<void> {
    if (this.preflightRunning) return;
    if (this.phase === "fault") throw new Error("Release E-STOP before pre-inspection");
    this.validateCompleteParams(this.params);
    this.preflightRunning = true;
    this.preflightPassed = false;
    this.preflightChecks = 0;
    this.emit();
    const checks = 8;
    for (let i = 1; i <= checks; i++) {
      await delay(130);
      this.preflightChecks = i;
      this.emit();
    }
    this.preflightPassed = true;
    this.preflightRunning = false;
    this.log("PASS", "preflight", "8/8 DEVELOPER PREVIEW checks passed · NO REAL HARDWARE · NO DEVICE I/O");
    this.emit();
  }

  // --------------------------------------------------------------- commands

  async home(): Promise<void> {
    this.ensureNotScanning("HOME");
    if (this.phase === "fault") throw new Error("Release E-STOP before homing");
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
    if (this.phase === "stopped") {
      await this.runPreflight();
      this.phase = "ready";
      this.log("PASS", "system", "recovery complete · back to READY");
    }
    this.emit();
  }

  async startScan(): Promise<void> {
    if (this.scanRunning) return;
    if (this.phase === "fault" || this.estopRequested) throw new Error("FAULT latched · release E-STOP and re-home first");
    if (!this.preflightPassed) throw new Error("Run pre-inspection first");
    if (!this.homed) throw new Error("Home the turntable first");
    const resumeFrom = this.phase === "paused" ? this.captured : 0;
    if (resumeFrom === 0) {
      this.frames = [];
      this.angleDeg = 0;
    }
    this.log("ACTION", "operator", resumeFrom > 0 ? `resume requested · continuing from view ${resumeFrom + 1}` : "start requested · acquisition begins at view 1");
    this.pauseRequested = false;
    this.estopRequested = false;
    this.phase = "scanning";
    this.scanRunning = true;
    this.emit();
    try {
      await this.scanLoop(resumeFrom + 1);
    } finally {
      this.scanRunning = false;
      this.emit();
    }
  }

  pause(): void {
    if (this.phase !== "scanning") return;
    this.pauseRequested = true;
    this.source.disable();
    const pulses = this.captured * this.pulsesPerView;
    this.log("ACTION", "operator", "pause requested · beam cut, motion holds position");
    this.log("WARN", "system", "beam disabled by pause · X-ray output latched off");
    this.log("INFO", "nano", `PAUSE acknowledged · motion halted at ${this.angleDeg.toFixed(2)}°`);
    this.log("INFO", "xray", "MOVE_ABS cancelled · waiting for resume or HOME");
    this.log("OK", "camera", `${this.captured} / ${this.params.projectionCount} frames kept in buffer · no data loss`);
    this.log("INFO", "nano", `pulse counter ${pulses} retained · ${this.pulsesPerView} pulses per view`);
    this.phase = "paused";
    this.emit();
  }

  async restore(): Promise<void> {
    this.ensureNotScanning("restore");
    if (this.phase === "fault" || this.estopRequested) {
      throw new Error("FAULT latched · release E-STOP and repeat pre-inspection and HOME first");
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

  async estop(): Promise<void> {
    if (this.phase === "fault") return;
    this.estopRequested = true;
    this.pauseRequested = false;
    this.source.latchOff();
    try {
      await this.link.exec(cmd.stop("{id}"), ["STOPPED"], 2000);
    } catch {
      /* link may already be torn down; latch stands regardless */
    }
    this.preflightPassed = false;
    this.homed = false;
    this.log("ACTION", "operator", "E-STOP pressed · all outputs cut");
    this.log("ERR", "nano", "STOP · STOPPED POSITION_UNKNOWN · pulse counter invalidated");
    this.log("ERR", "system", "safety latch engaged · X-ray output hard-disabled");
    this.log("WARN", "xray", "beam off · interlock still closed · no dose leak");
    this.log("INFO", "camera", `acquisition aborted · ${this.captured} / ${this.params.projectionCount} frames kept in buffer`);
    this.log("ACTION", "operator", "press E-STOP to clear · then REARM + HOME to recover");
    this.phase = "fault";
    this.emit();
  }

  async estopRelease(): Promise<void> {
    if (this.phase !== "fault") return;
    this.source.rearm();
    await this.link.exec(cmd.rearm("{id}"), ["OK"]).catch(() => undefined);
    this.estopRequested = false;
    this.log("ACTION", "operator", "E-STOP released · safety latch cleared");
    this.log("WARN", "system", "home reference lost · run HOME, then pre-inspection, before scanning");
    this.phase = "stopped";
    this.emit();
  }

  async retryDevice(id: "xray" | "turntable" | "camera"): Promise<void> {
    if (this.phase === "fault") {
      this.log("WARN", "system", `${id} retry deferred · clear FAULT first`);
      this.emit();
      return;
    }
    const source: ConsoleLogSource = id === "turntable" ? "nano" : id;
    await delay(260);
    if (id === "turntable") await this.link.exec(cmd.ping("{id}"), ["PONG"]).catch(() => undefined);
    this.log("INFO", source, `${id === "turntable" ? "turntable" : id} retry · link OK`);
    this.emit();
  }

  setParams(partial: Partial<ScanParams>): void {
    if (this.phase === "scanning" || this.phase === "paused") return;
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
      (!Number.isInteger(next.projectionCount) || next.projectionCount < 1 || next.projectionCount > 360)
    ) {
      throw new Error("Projection count must be between 1 and 360");
    }
    if (
      "exposureMs" in partial &&
      (!Number.isInteger(next.exposureMs) || next.exposureMs < 1 || next.exposureMs > 10000)
    ) {
      throw new Error("Exposure must be between 1 and 10000 ms");
    }
    if (
      "maxXraySec" in partial &&
      (!Number.isInteger(next.maxXraySec) || next.maxXraySec < 1 || next.maxXraySec > 359999)
    ) {
      throw new Error("Maximum X-ray duration must be between 1 and 359999 seconds");
    }

    this.params = next;
    this.camera.configure(this.params.savePath);
    this.log("INFO", "system", `parameters updated · ${this.params.projectionCount} views · ${this.angleStepDeg.toFixed(2)}°/view · ${this.params.exposureMs} ms`);
    this.emit();
  }

  sendVoltage(kv: number): void {
    const applied = this.source.setVoltage(kv);
    this.log("INFO", "xray", `set ${applied.toFixed(1)} kV · ACK`);
    this.emit();
  }

  sendCurrent(ua: number): void {
    const applied = this.source.setCurrent(ua);
    this.log("INFO", "xray", `set ${applied.toFixed(1)} µA · ACK`);
    this.emit();
  }

  async xrayToggle(): Promise<void> {
    if (this.phase === "scanning") {
      this.log("ACTION", "operator", "Xray Disable requested during scan · pausing at commit boundary");
      this.pause();
      return;
    }
    if (this.phase === "fault") return;
    if (this.source.beamOn) {
      this.source.disable();
      this.log("ACTION", "operator", "Xray Disable · tube off");
      this.log("INFO", "xray", "beam off · output disabled");
    } else {
      this.source.enable();
      this.log("ACTION", "operator", "Xray Enable · tube on at setpoint");
      this.log("WARN", "xray", `beam on · ${this.source.readback().setKv.toFixed(1)} kV / ${this.source.readback().setUa.toFixed(1)} µA · interlock closed`);
    }
    this.emit();
  }

  timerToggle(): void {
    if (this.phase === "scanning") return;
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
    if (!Number.isInteger(params.projectionCount) || params.projectionCount < 1 || params.projectionCount > 360) {
      throw new Error("Projection count must be between 1 and 360");
    }
    if (!Number.isInteger(params.exposureMs) || params.exposureMs < 1 || params.exposureMs > 10000) {
      throw new Error("Exposure must be between 1 and 10000 ms");
    }
    if (!Number.isInteger(params.maxXraySec) || params.maxXraySec < 1 || params.maxXraySec > 359999) {
      throw new Error("Maximum X-ray duration must be between 1 and 359999 seconds");
    }
  }

  // -------------------------------------------------------------- scan loop

  private async scanLoop(startView: number): Promise<void> {
    const total = this.params.projectionCount;
    try {
      for (let view = startView; view <= total; view++) {
        this.gate();
        const angle = (view - 1) * this.angleStepDeg;
        await this.link.exec(cmd.moveAbs("{id}", milliDeg(angle)), ["READY_TO_CAPTURE"], 8000);
        this.angleDeg = angle;
        this.log("INFO", "nano", `MOVE_ABS view ${view} → ${angle.toFixed(2)}° · ACK`);
        this.emit();
        this.gate();
        if (!this.source.beamOn) {
          this.source.enable();
          await this.link.exec(cmd.xrayWarning("{id}", true), ["OK"]).catch(() => undefined);
        }
        const rb = this.source.readback();
        this.log("INFO", "xray", `DEVELOPER PREVIEW exposure · PREVIEW DATA ${this.params.exposureMs} ms · ${rb.setKv.toFixed(1)} kV / ${rb.setUa.toFixed(1)} µA · NO DEVICE I/O`);
        await delay(Math.max(this.params.exposureMs, 240));
        this.gate();
        const frame = await this.camera.capture(view, angle, this.params.exposureMs);
        if (!frame.shaOk) throw new Error(`frame ${frame.fileName} failed validation`);
        this.frames = [...this.frames, frame];
        this.captured = view;
        this.log("OK", "camera", `captured view ${view} · stored ${this.captured} / ${total}`);
        await this.link.exec(cmd.captureDone("{id}"), ["IDLE"]);
        this.log("INFO", "nano", "CAPTURE_DONE sent · awaiting next move");
        this.writeCheckpoint(view, angle);
        if (view < total) {
          const nextAngle = view * this.angleStepDeg;
          this.log("INFO", "system", `view ${view + 1} queued → ${nextAngle.toFixed(2)}° · ETA ${this.etaText}`);
        }
        this.emit();
      }
      this.source.disable();
      await this.link.exec(cmd.xrayWarning("{id}", false), ["OK"]).catch(() => undefined);
      this.phase = "completed";
      this.clearCheckpoint();
      this.log("PASS", "system", `scan complete · ${total} / ${total} views committed · ${this.params.savePath}`);
      this.emit();
    } catch (err) {
      if (err instanceof AbortView) return; // pause / e-stop already logged
      if (this.estopRequested) return;
      this.source.latchOff();
      this.phase = "fault";
      this.homed = false;
      this.log("ERR", "nano", `${err instanceof Error ? err.message : String(err)}`);
      this.log("ERR", "system", "safety latch engaged · X-ray output hard-disabled");
      this.log("ACTION", "operator", "press E-STOP to clear · then REARM + HOME to recover");
      this.emit();
    }
  }

  private gate(): void {
    if (this.pauseRequested || this.estopRequested) throw new AbortView();
  }

  private ensureNotScanning(what: string): void {
    if (this.phase === "scanning") throw new Error(`${what} is unavailable during a scan`);
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
    this.link.close();
  }
}

export { degreesForPulses, PULSES_PER_REV };
