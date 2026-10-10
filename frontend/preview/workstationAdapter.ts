/**
 * Browser-preview engine adapter: bridges the simulated RTS9060 workflow to
 * the EngineAdapter contract consumed by `useEngine`. The console
 * renders from `snapshot.workstation`; base EngineSnapshot fields stay
 * coherent with the Tauri sidecar path. This adapter never accesses hardware.
 */

import { FIRMWARE_VERSION, LINK_LABEL } from "./rts9060/protocol";
import { PreviewDeviceControls } from "./rts9060/deviceControls";
import { shutterExposureMs } from "../src/engine/deviceControls";
import { MAX_XRAY_LIMIT_SEC, MIN_XRAY_LIMIT_SEC, parseLayerDrafts } from "../src/scan-input";
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
  ReconstructionGeometry,
  ScanFlowView,
  WorkstationView,
  SoftwareUpdatePreparation,
} from "../src/engine/types";

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

// The preview uses the engine's own source literals so the console tabs filter the
// same way in both adapters and no localized source label can reach the log rail.
const logSourceToBase: Record<ConsoleLogEntry["source"], LogEntry["source"]> = {
  system: "system",
  preflight: "preflight",
  operator: "operator",
  xray: "xray",
  nano: "nano",
  camera: "camera",
  reconstruction:"reconstruction",
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
  private snapshotSequence = 0;
  private controls?: PreviewDeviceControls;
  private get nativeControls(): PreviewDeviceControls { return this.controls ??= new PreviewDeviceControls(this.workflow); }
  private scanFlow: ScanFlowView = {
    stage: "idle",
    prompt: null,
    references: { preDark: 0, preFlat: 0, postFlat: 0, postDark: 0, required: 10 },
    geometry: null,
    checkpointPath: null,
  };
  private referenceRun = 0;
  private prepared = false;
  private frozenCamera:string|null=null;
  private restoreAuthorized=false;
  private updateCancelRequested=false;
  private updatePreparation: SoftwareUpdatePreparation | null = null;

  constructor() {
    this.workflow = new ScanWorkflow(() => { this.revision++; });
    this.workflow.acquisitionGuard=()=>this.requireFixedCamera(false);
  }

  async getSnapshot(): Promise<EngineSnapshot> {
    return this.buildSnapshot();
  }

  close(): void {
    this.workflow.close();
  }

  async dispatch(command: EngineCommand): Promise<EngineSnapshot> {
    const wf = this.workflow;
    if (this.updatePreparation?.maintenanceLocked && !["stop", "xray_off", "camera_status", "camera_identity", "turntable_status", "turntable_identity", "xray_status", "xray_identity", "cancel_software_update_prepare"].includes(command.type)) throw new Error("SOFTWARE_UPDATE_MAINTENANCE_LOCKED");
    if (["set_parameters", "update_scan_setup"].includes(command.type) && this.nativeControls.snapshot().taskLock.locked) throw new Error("UNFINISHED_TASK_MANUAL_LOCK · retain the frozen task parameters");
    if(command.type==="home"&&this.nativeControls.snapshot().taskLock.locked&&!this.restoreAuthorized)throw new Error("UNFINISHED_TASK_LOCKED");
    if (/^(camera_|turntable_|preset_|abandon_scan)/.test(command.type) && command.type!=="camera_capture_descriptor" && await this.nativeControls.dispatch(command)) return this.buildSnapshot();
    switch (command.type) {
      case "open_scan_project": throw new Error("Opening a scan project requires the desktop application");
      case "set_capture_root": {
        this.requirePreparationAccess();
        if (!/^(?:[a-z]:[\\/]|[\\/])/i.test(command.savePath.trim()) || command.savePath.includes("\0")) throw new Error("INVALID_CAPTURE_ROOT");
        wf.setParams({ savePath: command.savePath.trim() });
        this.prepared = false;this.scanFlow.geometry=null;this.scanFlow.prompt=null;this.frozenCamera=null;
        break;
      }
      case "prepare_scan": {
        this.requirePreparationAccess();
        const task = command.taskId.trim();
        if (!task || task.length > 128 || /[<>:"/\\|?*\x00-\x1f]/.test(task)
          || /[. ]$/.test(task) || /^(?:cameracaptures|con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(task)
          || !wf.params.savePath.trim() || !Number.isInteger(command.projectionCount)
          || command.projectionCount < 1 || command.projectionCount > 3600
          || !Number.isInteger(command.maxXraySec) || command.maxXraySec < MIN_XRAY_LIMIT_SEC || command.maxXraySec > MAX_XRAY_LIMIT_SEC) throw new Error("INVALID_SCAN_PREPARATION");
        const geometry = this.validGeometry(command.geometry);
        const shutter = this.nativeControls.camera.properties.shutter;
        const exposure = this.nativeControls.camera.connected && shutter?.readable && shutter.dataType===6 && shutter.confirmed && !shutter.error && shutter.actual !== null && (shutter.target===null||shutter.target===shutter.actual) ?  shutterExposureMs(shutter.actual, shutter.code) : null;
        if (exposure === null || exposure < 0.125 || exposure > 30000) throw new Error("CONFIRMED_NATIVE_SHUTTER_REQUIRED");
        wf.setParams({ taskId: task, projectionCount: command.projectionCount, maxXraySec: command.maxXraySec, exposureMs: exposure });
        wf.camera.configure(`${wf.params.savePath}/${task}`);
        this.scanFlow.geometry = geometry;
        this.scanFlow.prompt = null;
        this.scanFlow.stage = "idle";
        this.prepared = true;this.frozenCamera=null;this.restoreAuthorized=false;
        this.scanFlow.references={preDark:0,preFlat:0,postFlat:0,postDark:0,required:10};
        break;
      }
      case "prepare_software_update": {
        this.requirePreparationAccess();
        if(this.nativeControls.camera.liveView.active)throw new Error("SOFTWARE_UPDATE_LIVE_VIEW_ACTIVE");
        if(!this.nativeControls.camera.cleanupConfirmed||this.nativeControls.camera.isolated)throw new Error("DEVICE_CLEANUP_UNCONFIRMED");
        this.updateCancelRequested=false;
        const op=this.nativeControls.beginOperation("prepare_software_update","software_update");const now=new Date().toISOString();
        this.updatePreparation={sessionId:this.nativeControls.session,operationId:op.operationId,status:"preparing",ready:false,maintenanceLocked:true,createdAt:now,completedAt:null,turntableStop:"unknown",cameraCleanup:"unknown",xrayOff:"unknown",noActiveTransactions:false,error:null};
        try{
          await wf.independentStop();this.nativeControls.stop();this.nativeControls.camera.connected=false;this.nativeControls.camera.sessionId=null;
          if(this.updatePreparation.status==="preparing")this.updatePreparation={...this.updatePreparation,status:this.updateCancelRequested?"cancelled":"ready",ready:!this.updateCancelRequested,maintenanceLocked:!this.updateCancelRequested,completedAt:new Date().toISOString(),turntableStop:"never-owned",cameraCleanup:"never-owned",xrayOff:"never-owned",noActiveTransactions:true,error:null};
          op.status="completed";op.stage="SIMULATED cleanup · NO HARDWARE";wf.recordEvent("INFO","system","NO HARDWARE · simulated update cleanup; not a production safety authority report");
        }catch(error){this.updatePreparation={...this.updatePreparation,status:"blocked",ready:false,error:String(error),completedAt:new Date().toISOString()};op.status="failed";op.error=String(error);throw error;}
        finally{op.updatedAt=new Date().toISOString();}
        break;
      }
      case "cancel_software_update_prepare": {
        if (!this.updatePreparation || this.updatePreparation.operationId !== command.operationId) throw new Error("SOFTWARE_UPDATE_OPERATION_MISMATCH");
        if(this.updatePreparation.status==="preparing"){this.updateCancelRequested=true;break;}
        if(this.updatePreparation.status==="blocked")throw new Error("DEVICE_CLEANUP_UNCONFIRMED");
        this.updatePreparation = { ...this.updatePreparation, status: "cancelled", ready: false, maintenanceLocked: false,completedAt:new Date().toISOString() };
        break;
      }
      case "camera_capture_descriptor":
        throw new Error("Browser preview has no registered NEF file");
      // The browser preview owns no camera card session and no USB transfer.
      // It must never report a started, cancelled or verified import.
      case "start_card_import":
        wf.recordEvent("ERR", "camera", "NO HARDWARE · browser preview has no camera card session; no USB import is performed");
        throw new Error("NO HARDWARE · the browser preview cannot import camera card data");
      case "cancel_card_import":
        wf.recordEvent("ERR", "camera", "NO HARDWARE · no camera card import is running in the browser preview");
        throw new Error("NO HARDWARE · no camera card import is running");
      case "connect":
      case "disconnect":
        // The workstation link is established at boot and stays up.
        break;
      case "preflight":
        if (this.nativeControls.snapshot().taskLock.locked && (!command.restore || !command.confirmUsbAutoShutdownRelease)) throw new Error("Use the confirmed Restore preflight entry in the device console");
        if (!this.prepared && !command.restore) throw new Error("SCAN_PREPARATION_REQUIRED");
        if(command.restore){const evidence=wf.captureEvidence;if(!evidence||evidence.schemaVersion!==6||evidence.profile!==this.nativeControls.profileEvidence())throw new Error("CAPTURE_POLICY_LEGACY_READ_ONLY_OR_MISMATCH");this.frozenCamera=evidence.profile;this.scanFlow.geometry=this.validGeometry(evidence.geometry as ReconstructionGeometry);this.restoreAuthorized=true;}
        this.requireFixedCamera(false);
        const profile=this.nativeControls.profileEvidence();
        await wf.runPreflight();
        if(profile!==this.nativeControls.profileEvidence()){wf.preflightPassed=false;throw new Error("CAPTURE_PROFILE_MISMATCH");}
        this.frozenCamera=profile;
        wf.captureEvidence={schemaVersion:6,profile,geometry:this.scanFlow.geometry};
        if (wf.preflightPassed && this.scanFlow.stage !== "projections") {
          this.scanFlow.stage = "idle";
          this.scanFlow.prompt = null;
        }
        break;
      case "home":
        await wf.home();
        break;
      case "start_scan":
        if(!wf.preflightPassed)throw new Error("PREFLIGHT_REQUIRED");if(!wf.homed)throw new Error("HOME_REQUIRED");
        if (!this.scanFlow.geometry) throw new Error("SCAN_PREPARATION_REQUIRED");
        this.requireFixedCamera(true);
        if (this.scanFlow.stage === "projections" && wf.phase === "paused") {
          void wf.startScan();
          break;
        }
        if (this.scanFlow.stage !== "idle" || this.scanFlow.prompt) {
          throw new Error("Confirm the current scan step first");
        }
        wf.preparationOccupied=true;
        this.scanFlow.prompt = this.prompt("preReferences");
        break;
      case "resume":
        this.requireFixedCamera(true);
        void wf.startScan();
        break;
      case "confirm_scan_stage":
        this.requireFixedCamera(true);
        if (this.scanFlow.prompt?.stage !== command.stage) throw new Error("Scan prompt is no longer active");
        if (command.stage === "geometry") {
          if (!command.geometry) throw new Error("Geometry is required");
          this.scanFlow.geometry = this.validGeometry(command.geometry);
          this.scanFlow.prompt = null;
        } else if (command.stage === "preReferences") {
          this.scanFlow.prompt = null;
          void this.simulateReferences();
        } else if (command.stage === "placeSample") {
          this.scanFlow.prompt = null;
          this.scanFlow.stage = "projections";
          wf.preparationOccupied=false;
          void wf.startScan();
        }
        break;
      case "cancel_scan_stage":
        if (this.scanFlow.prompt?.stage !== command.stage) throw new Error("Scan prompt is no longer active");
        this.referenceRun++;wf.preparationOccupied=false;
        this.scanFlow.prompt = null;
        this.scanFlow.stage = "stopped";
        if (["scanning", "paused", "finishing"].includes(wf.phase)) await wf.stop();
        break;
      case "start_reconstruction":
        wf.recordEvent("ERR","reconstruction","NO HARDWARE · browser preview has no acquired NEF dataset to reconstruct");
        throw new Error("Browser preview has no acquired NEF files to reconstruct");
      case "pause":
        wf.pause();
        break;
      case "restore_previous":
        this.requireFixedCamera(true);
        if(!this.restoreAuthorized)throw new Error("RESTORE_PREFLIGHT_CONFIRMATION_REQUIRED");
        await wf.restore();
        break;
      case "stop":
        {
        const endTask = wf.checkpointAvailable || wf.preparationOccupied
          || ["scanning", "paused", "finishing", "stopping"].includes(wf.phase)
          || ["preDark", "preFlat", "placeSample"].includes(this.scanFlow.stage);
        if(this.updatePreparation?.maintenanceLocked)this.updatePreparation={...this.updatePreparation,status:"blocked",ready:false,error:"New simulated safety operation invalidated update preparation"};
        this.nativeControls.stop();
        this.referenceRun++;wf.preparationOccupied=false;
        this.scanFlow.prompt = null;
        this.scanFlow.stage = "stopped";
        if (endTask && !this.updatePreparation?.maintenanceLocked) {
          await wf.endTask();this.prepared=false;this.restoreAuthorized=false;this.frozenCamera=null;
          this.scanFlow.geometry=null;this.scanFlow.references={preDark:0,preFlat:0,postDark:0,postFlat:0,required:10};
        } else if (["ready", "completed", "booting", "stopped", "fault"].includes(wf.phase)) await wf.independentStop(); else await wf.stop();
        break;
        }
      case "retry_device":
        void wf.retryDevice(command.device);
        break;
      case "xray_disconnect":
        wf.xrayDisconnect();
        break;
      case "xray_connect":
        await wf.retryDevice("xray");
        break;
      case "xray_status":
      case "xray_identity":
        break;
      case "xray_off":
        if(this.updatePreparation?.maintenanceLocked)this.updatePreparation={...this.updatePreparation,status:"blocked",ready:false,error:"New simulated OFF invalidated update preparation"};
        wf.source.disable();
        if (["scanning", "paused", "finishing", "stopping"].includes(wf.phase)) await wf.stop();
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
        if (command.parameters.exposureMs !== wf.params.exposureMs) throw new Error("EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY");
        wf.setParams({
          taskId: command.parameters.taskId,
          savePath: command.parameters.savePath,
          projectionCount: command.parameters.projectionCount,
          exposureMs: command.parameters.exposureMs,
        });
        break;
      case "update_scan_setup":
        if (command.setup.exposureMs !== undefined) throw new Error("EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY");
        wf.setParams(command.setup);
        this.prepared = false;
        break;
    }
    return this.buildSnapshot();
  }

  private requireFixedCamera(compare:boolean):void{
    if(!this.nativeControls.turntableConnected)throw new Error("NANO_NOT_CONNECTED");
    if(!this.nativeControls.scanReadiness().ready){this.workflow.preflightPassed=false;throw new Error("CAMERA_FIXED_SCAN_CONFIGURATION_REQUIRED");}
    const shutter=this.nativeControls.camera.properties.shutter;
    if(shutterExposureMs(shutter.actual!,shutter.code)!==this.workflow.params.exposureMs){this.workflow.preflightPassed=false;throw new Error("CAPTURE_PROFILE_MISMATCH");}
    if(compare&&(!this.frozenCamera||this.frozenCamera!==this.nativeControls.profileEvidence())){this.workflow.preflightPassed=false;throw new Error("CAPTURE_PROFILE_MISMATCH");}
  }
  private requirePreparationAccess(): void {
    if(this.nativeControls.busy||this.workflow.preflightRunning||this.workflow.preparationOccupied||["preDark","preFlat"].includes(this.scanFlow.stage))throw new Error("DEVICE_CONTROL_LOCKED");
    if (["scanning", "paused", "finishing", "stopping"].includes(this.workflow.phase)
        || this.nativeControls.snapshot().taskLock.locked) throw new Error("TASK_OCCUPIES_PREPARATION");
  }

  private prompt(stage: NonNullable<ScanFlowView["prompt"]>["stage"]): NonNullable<ScanFlowView["prompt"]> {
    const copy = {
      geometry: ["Geometry", "Enter measured scan geometry.", "Save geometry"],
      preReferences: ["Reference frames", "Remove sample. Capture dark, then X-ray flat frames?", "Continue"],
      placeSample: ["Place sample", "X-ray off. Place sample on turntable.", "Continue"],
    }[stage];
    return { stage, title: copy[0], message: copy[1], confirmLabel: copy[2], cancelLabel: "Cancel scan" };
  }

  private validGeometry(value: ReconstructionGeometry): ReconstructionGeometry {
    if (![value.sodMm, value.objectToDetectorMm, value.detectorWidthMm].every((n) => Number.isFinite(n) && n > 0)
      || ![value.centerOffsetXMm, value.centerOffsetYMm].every(Number.isFinite)
      || value.rotationDirection !== "counterclockwise"
      || value.measurement !== "measured"
      || typeof value.mirrorX !== "boolean") throw new Error("Invalid reconstruction geometry");
    if (value.scintillatorLayers?.length && !parseLayerDrafts(true,value.scintillatorLayers.map(layer=>({materialName:layer.materialName,emissionColor:layer.emissionColor,thicknessMm:String(layer.thicknessMm)})))) throw new Error("INVALID_SCINTILLATOR_STACK");
    return { ...value, ...(value.scintillatorLayers ? {scintillatorLayers:value.scintillatorLayers.map(layer=>({...layer}))} : {}) };
  }

  private async simulateReferences(): Promise<void> {
    const run = ++this.referenceRun;
    for (const [stage, field] of [["preDark", "preDark"], ["preFlat", "preFlat"]] as const) {
      this.scanFlow.stage = stage;
      for (let count = 1; count <= 10; count++) {
        await new Promise((resolve) => setTimeout(resolve, 80));
        if (this.referenceRun !== run) return;
        this.scanFlow.references[field] = count;
        this.revision++;
      }
    }
    this.scanFlow.stage = "placeSample";
    this.scanFlow.prompt = this.prompt("placeSample");
    this.revision++;
  }

  private refreshScanFlow(): void {
    if (this.scanFlow.stage === "projections" && this.workflow.phase === "completed") {
      this.scanFlow.stage = "completed";
    }
    if (this.scanFlow.stage === "projections" && this.workflow.phase === "fault") {
      this.scanFlow.stage = "fault";
      this.scanFlow.prompt = null;
    }
  }

  // ------------------------------------------------------------ view mapping

  private buildSnapshot(): EngineSnapshot {
    this.refreshScanFlow();
    const wf = this.workflow;
    const ws = this.buildWorkstationView();
    const rb = wf.source.readback();
    const dataStateToPhase = wf.preparationOccupied?"running":phaseToEngine[wf.phase];
    return {
      engineSessionId: this.nativeControls.session,
      snapshotSequence: this.snapshotSequence = (this.snapshotSequence ?? 0) + 1,
      operationId: this.nativeControls.lastOperationId,
      operations: structuredClone(this.nativeControls.operations),
      deviceControls: this.nativeControls.snapshot(),
      cameraScanReadiness: this.nativeControls.scanReadiness(),
      // The offline preview freezes no capture profile and has no camera card
      // session, so the card destination stays unknown and nothing is pending.
      // The import fields are the same frozen shape with neutral values: the
      // preview never reports an import it did not perform.
      cardAcquisition: {
        destination: "unknown",
        active: false,
        acquisitionStatus: null,
        dataStatus: null,
        recordCount: 0,
        pendingImportTasks: 0,
        currentTaskPendingImport: false,
        importRequired: false,
        importStatus: "idle",
        importOperationId: null,
        importError: null,
        importedRecords: 0,
        verifiedRecords: 0,
        acquiredProjectionCount: 0,
        importedProjectionCount: 0,
        verifiedProjectionCount: 0,
      },
      softwareUpdatePreparation: this.updatePreparation ? { ...this.updatePreparation } : null,
      cameraCaptureDescriptor: null,
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
        state: device.word === "OFFLINE"?"offline":device.tone === "danger" ? "fault" : device.tone === "warn" ? "busy" : "ready",
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

    const activePhase = wf.preparationOccupied || ["scanning", "finishing", "stopping"].includes(phase);
    const maintenance=this.updatePreparation?.maintenanceLocked===true;
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
    const tableWord = !this.nativeControls.turntableConnected ? "OFFLINE" : phase === "fault" ? "HOME LOST" : activePhase ? "MOVING" : phase === "paused" ? "HOLD" : "READY";
    const tableTone: ConsoleTone = !this.nativeControls.turntableConnected ? "muted" : phase === "fault" ? "danger" : activePhase || phase === "paused" ? "warn" : "ok";
    const cameraWord = !this.nativeControls.camera.connected ? "OFFLINE" : activePhase || phase === "paused" || phase === "fault" ? "ARMED" : "READY";
    const cameraTone: ConsoleTone = cameraWord === "OFFLINE"?"muted":cameraWord === "ARMED" ? "accent" : "ok";

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
      home: !maintenance && wf.preflightPassed && !activePhase && phase !== "fault" && phase !== "booting",
      play:
        !maintenance && (phase === "scanning" || phase === "paused")
          ? true
          : phase === "ready" || phase === "completed"
            ? !maintenance && wf.preflightPassed && wf.homed
            : false,
      restore:
        !maintenance && (phase === "ready" || phase === "completed") &&
        wf.preflightPassed &&
        wf.homed &&
        wf.checkpointAvailable,
      stop: phase === "scanning" || phase === "paused" || phase === "finishing",
      playMode: phase === "scanning" ? "pause" : phase === "paused" ? "resume" : phase === "fault" || phase === "finishing" || phase === "stopping" ? "disabled" : "start",
      homeReason: phase === "fault" ? "Run Preflight to recover from the device fault" : !wf.preflightPassed ? "Run Preflight first" : "",
      restoreReason: !wf.checkpointAvailable ? "No unfinished scan checkpoint was found" : !wf.preflightPassed ? "Run Preflight first" : !wf.homed ? "Run HOME first" : "",
      playReason: phase === "fault" ? "Run Preflight and HOME to recover from the device fault" : !wf.preflightPassed ? "Run Preflight first" : !wf.homed ? "Run HOME first" : "",
    };

    const statusLeft = !configured
      ? "SETUP REQUIRED  enter Task ID, Save Path, projections, exposure, and maximum X-ray time"
      : phase === "scanning"
        ? `SCANNING  view ${Math.min(captured + 1, total)} / ${total} · ${fixed2(step)}°/view · ${captured} / ${total} captured · ${fixed2(angle)}° · exposing · ETA ${wf.etaText}`
        : phase === "finishing"
          ? `FINISHING  ${captured} / ${total} committed · preserving final projection angle · Stop is available`
          : phase === "stopping"
            ? `STOPPING  ${captured} / ${total} committed · output disabled · waiting for the scan task to exit`
        : phase === "paused"
          ? `PAUSED  view ${Math.min(captured + 1, total)} / ${total} · held at ${fixed2(angle)}° · pulse counter kept · resume to continue`
          : phase === "fault"
            ? "FAULT  device checks required · output latched off · preflight then HOME to recover"
        : phase === "stopped"
              ? `STANDBY  scan ended · ${wf.preflightPassed ? "HOME required" : "preflight then HOME required"}`
              : phase === "completed"
                ? `COMPLETE  ${total} / ${total} captured · ${fixed2(angle)}° · simulated projections only · NO FILES WRITTEN`
                : `READY  ${total} views · ${fixed2(step)}°/view · ${captured} / ${total} captured · queue idle · awaiting operator`;

    return {
      scanFlow: { ...this.scanFlow, references: { ...this.scanFlow.references } },
      reconstruction: {
        unlocked: false,
        methods: (["fdk", "sirt", "cgls"] as const).map((id) => ({
          id, label: id.toUpperCase(), enabled: false,
          reason: "Browser preview has no acquired RAW dataset",
        })),
        selectedMethod: null,
        status: "idle",
        percent: 0,
        message: "No production reconstruction is available in browser preview",
        cachePath: null,
      },
      dataState,
      phaseWord,
      phaseTone,
      devices,
      onlineSummary: "PREVIEW · 0 REAL DEVICES",
      cameraExposure: { minMs: 0.125, maxMs: 30000, known: false },
      preflight,
      floats,
      safetyBar,
      scene: { angleDeg: angle, rotated: !(dataState === "ready" && angle === 0), angleKnown: this.nativeControls.turntableConnected && phase !== "fault" && phase !== "stopped" && phase !== "stopping", rotationDirection: -1 },
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
        manualControlsEnabled: !maintenance && wf.manualControlsEnabled,
        timerControlsEnabled: !maintenance && wf.timerControlsEnabled,
        setpointControlsEnabled: !maintenance && wf.setpointControlsEnabled,
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
        fileName: `preview-${frame.index}.frame`,
      })),
      checkpointAvailable: wf.checkpointAvailable,
    };
  }
}
