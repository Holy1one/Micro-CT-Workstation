/**
 * Browser-preview engine adapter: bridges the simulated RTS9060 workflow to
 * the EngineAdapter contract consumed by `useEngine`. The console
 * renders from `snapshot.workstation`; base EngineSnapshot fields stay
 * coherent with the Tauri sidecar path. This adapter never accesses hardware.
 */

import { FIRMWARE_VERSION, LINK_LABEL } from "./rts9060/protocol";
import { ScanWorkflow, type ConsoleLogEntry, type WorkflowPhase } from "./rts9060/workflow";
import type {
  ConsoleDataState,
  ConsoleDeviceView,
  ConsoleFloatView,
  ConsoleTone,
  EngineAdapter,
  EngineCommand,
  EnginePhase,
  EngineSnapshot,
  LogEntry,
  WorkstationView,
} from "./types";

const phaseToEngine: Record<WorkflowPhase, EnginePhase> = {
  booting: "idle",
  ready: "ready",
  scanning: "running",
  finishing: "finishing",
  stopping: "stopping",
  paused: "paused",
  fault: "fault",
  stopped: "stopped",
  completed: "completed",
};

const logLevelToBase: Record<ConsoleLogEntry["level"], LogEntry["level"]> = {
  PASS: "success",
  INFO: "info",
  OK: "info",
  WARN: "warning",
  ERR: "error",
  ACTION: "info",
};

const logSourceToBase: Record<ConsoleLogEntry["source"], LogEntry["source"]> = {
  system: "系统",
  preflight: "系统",
  operator: "系统",
  xray: "射线",
  nano: "转台",
  camera: "相机",
};

function fixed1(value: number): string {
  return value.toFixed(1);
}

function fixed2(value: number): string {
  return value.toFixed(2);
}

export class WorkstationAdapter implements EngineAdapter {
  readonly kind = "developer_preview" as const;
  private workflow: ScanWorkflow;
  private revision = 0;

  constructor() {
    this.workflow = new ScanWorkflow(() => {
      this.revision++;
    });
  }

  async getSnapshot(): Promise<EngineSnapshot> {
    return this.buildSnapshot();
  }

  close(): void {
    this.workflow.close();
  }

  async dispatch(command: EngineCommand): Promise<EngineSnapshot> {
    const wf = this.workflow;
    switch (command.type) {
      case "connect":
      case "disconnect":
        // The workstation link is established at boot and stays up.
        break;
      case "preflight":
        await wf.runPreflight();
        break;
      case "home":
        await wf.home();
        break;
      case "start_scan":
      case "resume":
        void wf.startScan();
        break;
      case "pause":
        wf.pause();
        break;
      case "restore_previous":
        void wf.restore();
        break;
      case "stop":
        await wf.stop();
        break;
      case "retry_device":
        void wf.retryDevice(command.device);
        break;
      case "xray_disconnect":
        wf.xrayDisconnect();
        break;
      case "camera_test_capture":
        throw new Error("Real camera capture is available only in the desktop runtime");
      case "xray_toggle":
        void wf.xrayToggle();
        break;
      case "timer_toggle":
        wf.timerToggle();
        break;
      case "usb_auto_shut_down_toggle":
        wf.usbAutoShutDownToggle();
        break;
      case "set_usb_shutdown_delay":
        wf.setUsbShutdownDelay(command.delay);
        break;
      case "send_voltage":
        wf.sendVoltage(command.kv);
        break;
      case "send_current":
        wf.sendCurrent(command.ua);
        break;
      case "set_parameters":
        wf.setParams({
          taskId: command.parameters.taskId,
          savePath: command.parameters.savePath,
          projectionCount: command.parameters.projectionCount,
          exposureMs: command.parameters.exposureMs,
        });
        break;
      case "update_scan_setup":
        wf.setParams(command.setup);
        break;
    }
    return this.buildSnapshot();
  }

  // ------------------------------------------------------------ view mapping

  private buildSnapshot(): EngineSnapshot {
    const wf = this.workflow;
    const ws = this.buildWorkstationView();
    const rb = wf.source.readback();
    const dataStateToPhase = phaseToEngine[wf.phase];
    return {
      mode: "developer_preview",
      modeLabel: "DEVELOPER PREVIEW · NO REAL HARDWARE",
      connectionState: "connected",
      adapterLabel: `Browser Preview · ${LINK_LABEL}`,
      phase: dataStateToPhase,
      phaseLabel: ws.phaseWord,
      preflightPassed: wf.preflightPassed,
      homed: wf.homed,
      requiresPreflight: !wf.preflightPassed,
      requiresHome: !wf.homed,
      safety: {
        xrayAvailable: !rb.latched,
        xrayEnabled: rb.beamOn,
        interlockOk: rb.interlockOk,
        lockReason: rb.latched ? "Output latched off" : "",
      },
      devices: ws.devices.map((device) => ({
        id: device.id,
        label: device.name,
        state: device.tone === "danger" ? "fault" : device.tone === "warn" ? "busy" : "ready",
        detail: device.spec,
      })),
      parameters: {
        taskId: wf.params.taskId,
        savePath: wf.params.savePath,
        projectionCount: wf.params.projectionCount,
        angleStepDeg: wf.angleStepDeg,
        exposureMs: wf.params.exposureMs,
      },
      progress: {
        current: ws.progress.captured,
        total: ws.progress.total,
        percent: ws.progress.percent,
        angleDeg: ws.progress.angleDeg,
        etaSeconds: wf.etaSeconds,
      },
      imageCount: wf.frames.length,
      logs: wf.logs.map((entry) => ({
        id: entry.id,
        timestamp: entry.timestamp,
        level: logLevelToBase[entry.level],
        source: logSourceToBase[entry.source],
        message: entry.message,
      })),
      lastError: null,
      updatedAt: new Date().toISOString(),
      workstation: ws,
    };
  }

  private buildWorkstationView(): WorkstationView {
    const wf = this.workflow;
    const rb = wf.source.readback();
    const phase = wf.phase;
    const total = wf.params.projectionCount;
    const captured = wf.captured;
    const percent = total > 0 ? Math.round((captured / total) * 100) : 0;
    const angle = wf.angleDeg;
    const step = wf.angleStepDeg;
    const configured =
      wf.params.taskId.trim().length > 0 &&
      wf.params.savePath.trim().length > 0 &&
      wf.params.projectionCount > 0 &&
      wf.params.exposureMs > 0 &&
      wf.params.maxXraySec > 0;

    const activePhase = ["scanning", "finishing", "stopping"].includes(phase);
    const dataState: ConsoleDataState =
      activePhase ? "scanning" : phase === "paused" ? "paused" : phase === "fault" ? "fault" : "ready";

    const phaseWord =
      phase === "booting"
        ? "BOOT"
        : phase === "stopped"
          ? "STANDBY"
          : phase === "completed"
            ? "COMPLETE"
            : phase === "finishing"
              ? "FINISHING"
              : phase === "stopping"
                ? "STOPPING"
                : dataState.toUpperCase();
    const phaseTone: WorkstationView["phaseTone"] = activePhase ? "warn" : phase === "fault" ? "danger" : "accent";

    const xrayConnected = wf.source.isConnected;
    const xrayWord = !xrayConnected ? "OFFLINE" : phase === "fault" ? "FAILED" : rb.beamOn ? "EMITTING" : phase === "paused" ? "STANDBY" : "READY";
    const xrayTone: ConsoleTone = phase === "fault" ? "danger" : rb.beamOn || phase === "paused" ? "warn" : "ok";
    const tableWord = phase === "fault" ? "HOME LOST" : activePhase ? "MOVING" : phase === "paused" ? "HOLD" : "READY";
    const tableTone: ConsoleTone = phase === "fault" ? "danger" : activePhase || phase === "paused" ? "warn" : "ok";
    const cameraWord = activePhase || phase === "paused" || phase === "fault" ? "ARMED" : "READY";
    const cameraTone: ConsoleTone = cameraWord === "ARMED" ? "accent" : "ok";

    const xraySpec =
      phase === "fault"
        ? "12 W · PREVIEW · OUTPUT LATCHED"
        : rb.beamOn
          ? `12 W · PREVIEW BEAM · ${fixed1(rb.setUa)} µA SET`
          : phase === "paused"
            ? `12 W · PREVIEW PAUSED · ${fixed1(rb.setUa)} µA SET`
            : "12 W · PREVIEW ONLY · NO REAL HARDWARE";

    const devices: ConsoleDeviceView[] = [
      { id: "xray", name: "X-Ray Source", word: xrayWord, tone: xrayTone, spec: xraySpec },
      { id: "turntable", name: "Turntable-Nano", word: tableWord, tone: tableTone, spec: `POS ${fixed2(angle)}° · 60:1 · 8 µSTEP` },
      { id: "camera", name: "Camera", word: cameraWord, tone: cameraTone, spec: "D7100 · DEVELOPER PREVIEW · PREVIEW DATA · NO DEVICE I/O" },
    ];

    const floats: ConsoleFloatView[] = [
      {
        key: "X-RAY",
        text: phase === "fault" ? "LATCHED OFF" : rb.beamOn ? "TUBE ON" : "TUBE OFF",
        tone: phase === "fault" || rb.beamOn ? "danger" : "muted",
      },
      {
        key: "CAMERA",
        text: phase === "scanning" ? "EXPOSING" : activePhase ? "ENDING" : phase === "ready" || phase === "completed" ? "READY" : "STANDBY",
        tone: activePhase ? "warn" : phase === "ready" || phase === "completed" ? "ok" : "muted",
      },
      {
        key: "SAMPLE",
        text: phase === "fault" ? "HOME LOST" : phase === "paused" ? `HOLD ${fixed2(angle)}°` : `${fixed2(angle)}°`,
        tone: phase === "fault" ? "danger" : phase === "paused" ? "warn" : "accent",
      },
    ];

    const safetyBar: WorkstationView["safetyBar"] =
      phase === "fault"
        ? { text: "SAFETY · DEVICE FAULT · OUTPUT LATCHED OFF", tone: "dangerBold" }
        : rb.beamOn
          ? { text: "SAFETY · BEAM ON · INTERLOCK OK", tone: "danger" }
          : phase === "paused"
            ? { text: "SAFETY · OUTPUT DISABLED · PAUSED", tone: "muted" }
            : { text: "SAFETY · OUTPUT DISABLED", tone: "muted" };

    const barTone: WorkstationView["progress"]["barTone"] =
      phase === "paused" || phase === "stopping" || phase === "finishing" ? "warn" : phase === "fault" ? "danger" : "accent";
    const barLabel =
      phase === "paused"
        ? `${captured} / ${total} · ${percent}% · held`
      : phase === "fault"
          ? `${captured} / ${total} · ${percent}% · aborted`
          : `${captured} / ${total} · ${percent}%`;

    const preflight: WorkstationView["preflight"] = wf.preflightRunning
      ? {
          word: "RUNNING",
          percent: Math.round((wf.preflightChecks / 8) * 100),
          tone: "running",
          subline: `${wf.preflightChecks}/8 checks · USB link · fw ${FIRMWARE_VERSION}`,
        }
      : wf.preflightPassed
        ? {
            word: "PASSED",
            percent: 100,
            tone: "pass",
            subline: `8/8 checks passed · USB link · fw ${FIRMWARE_VERSION}`,
          }
        : {
            word: "WAITING",
            percent: 0,
            tone: "warn",
            subline: `0/8 checks · USB link · fw ${FIRMWARE_VERSION}`,
          };

    const dock: WorkstationView["dock"] = {
      home: wf.preflightPassed && !activePhase && phase !== "fault" && phase !== "booting",
      play:
        phase === "scanning" || phase === "paused"
          ? true
          : phase === "ready" || phase === "completed"
            ? wf.preflightPassed && wf.homed
            : false,
      restore:
        (phase === "ready" || phase === "completed") &&
        wf.preflightPassed &&
        wf.homed &&
        wf.checkpointAvailable,
      stop: phase === "scanning" || phase === "paused" || phase === "finishing",
      playMode: phase === "scanning" ? "pause" : phase === "paused" ? "resume" : phase === "fault" || phase === "finishing" || phase === "stopping" ? "disabled" : "start",
      homeReason: phase === "fault" ? "Run Preflight to recover from the device fault" : !wf.preflightPassed ? "Run Preflight first" : "",
      playReason: phase === "fault" ? "Run Preflight and HOME to recover from the device fault" : !wf.preflightPassed ? "Run Preflight first" : !wf.homed ? "Run HOME first" : "",
    };

    const statusLeft = !configured
      ? "SETUP REQUIRED  enter Task ID, Save Path, projections, exposure, and maximum X-ray time"
      : phase === "scanning"
        ? `SCANNING  view ${Math.min(captured + 1, total)} / ${total} · ${fixed2(step)}°/view · ${captured} / ${total} captured · ${fixed2(angle)}° · exposing · ETA ${wf.etaText}`
        : phase === "finishing"
          ? `FINISHING  ${captured} / ${total} committed · returning the turntable to zero · Stop is available`
          : phase === "stopping"
            ? `STOPPING  ${captured} / ${total} committed · output disabled · waiting for the scan task to exit`
        : phase === "paused"
          ? `PAUSED  view ${Math.min(captured + 1, total)} / ${total} · held at ${fixed2(angle)}° · pulse counter kept · resume to continue`
          : phase === "fault"
            ? "FAULT  device checks required · output latched off · preflight then HOME to recover"
        : phase === "stopped"
              ? `STANDBY  scan ended · ${wf.preflightPassed ? "HOME required" : "preflight then HOME required"}`
              : phase === "completed"
                ? `COMPLETE  ${total} / ${total} captured · ${fixed2(angle)}° · projections on disk`
                : `READY  ${total} views · ${fixed2(step)}°/view · ${captured} / ${total} captured · queue idle · awaiting operator`;

    return {
      dataState,
      phaseWord,
      phaseTone,
      devices,
      onlineSummary: "PREVIEW · 0 REAL DEVICES",
      cameraExposure: { minMs: 0.125, maxMs: 30000, known: false },
      preflight,
      floats,
      safetyBar,
      scene: { angleDeg: angle, rotated: !(dataState === "ready" && angle === 0), angleKnown: phase !== "fault" && phase !== "stopped" && phase !== "stopping", rotationDirection: 1 },
      xray: {
        connected: xrayConnected,
        setKv: rb.setKv,
        setUa: rb.setUa,
        monKv: xrayConnected ? rb.monKv : null,
        monUa: xrayConnected ? rb.monUa : null,
        powerW: xrayConnected ? rb.powerW : null,
        tempC: xrayConnected ? rb.tempC : null,
        beamState: !xrayConnected ? "unknown" : rb.beamOn ? "on" : "off",
        beamOn: rb.beamOn,
        latched: rb.latched,
        onSec: phase === "scanning" ? 14 : 10,
        offSec: phase === "scanning" ? 6 : 20,
        timerOn: phase === "scanning" ? true : wf.timerOn,
        usbAutoShutDown: wf.usbAutoShutDown,
        usbAutoShutDownKnown: true,
        usbShutdownDelay: wf.usbShutdownDelay,
        manualControlsEnabled: wf.manualControlsEnabled,
        timerControlsEnabled: wf.timerControlsEnabled,
        setpointControlsEnabled: wf.setpointControlsEnabled,
        voltageConfirmed: true,
        currentConfirmed: true,
        setpointConfirmed: true,
      },
      progress: { captured, total, percent, angleDeg: angle, etaText: wf.etaText, barTone, barLabel },
      summary: {
        savePath: wf.params.savePath,
        acquisition: configured
          ? `${total} views · ${fixed2(step)}° · ${wf.params.exposureMs} ms`
          : "Scan setup not configured",
        output: `${fixed1(rb.setKv)} kV · ${fixed1(rb.setUa)} µA · ${fixed1(rb.powerW)} W`,
      },
      statusbar: {
        left: `DEVELOPER PREVIEW · ${statusLeft}`,
        right: `NO REAL HARDWARE · ${LINK_LABEL} · fw ${FIRMWARE_VERSION} · ${fixed1(rb.powerW)} W SET`,
        dotTone: activePhase ? "warn" : phase === "paused" ? "accent" : phase === "fault" ? "danger" : "ok",
      },
      dock,
      scanSetup: {
        savePath: wf.params.savePath,
        taskId: wf.params.taskId,
        projectionCount: wf.params.projectionCount,
        angleStepDeg: step,
        exposureMs: wf.params.exposureMs,
        maxXraySec: wf.params.maxXraySec,
      },
      consoleLogs: wf.logs.map((entry) => ({ ...entry })),
      frames: wf.frames.map((frame) => ({
        index: frame.index,
        angleDeg: frame.angleDeg,
        exposureMs: frame.exposureMs,
        fileName: frame.fileName,
      })),
      checkpointAvailable: wf.checkpointAvailable,
    };
  }
}
