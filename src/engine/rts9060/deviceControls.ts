/** Browser-only native-device UI model. All capabilities, files and completion
 * evidence are explicitly simulated; no OS transport or camera I/O exists here. */
import type { CameraHealth, CameraProperties, DeviceControls, DeviceOperation, EngineCommand } from "../types";
import type { ScanWorkflow } from "./workflow";

export class PreviewDeviceControls {
  readonly session = `preview-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  operations: DeviceOperation[] = [];
  lastOperationId: string | null = null;
  private bench = false;
  private connectedTurntable = true;
  private presets: DeviceControls["presets"] = { schemaVersion: 1, presets: [], error: null };
  private exportJson: string | null = null;
  private epoch = 0;
  camera: CameraHealth = {
    connected: false, backend: "NO HARDWARE · in-memory camera preview", serial: null, transferPolicy: null,
    lastCapture: null, lastError: null, exposureMinMs: 0.125, exposureMaxMs: 30000, identity: null,
    discovered: [], sessionId: null, capabilities: { properties: false, hostCapture: false, liveView: false,
      autofocus: false, focusStep: false, scanNef: false, hardwareCertified: false, disabledReasons: { scanNef: "NO HARDWARE · not hardware certified" } },
    properties: {}, liveView: { active: false, fresh: false, sessionId: null, sequence: 0, lastFrameAt: null,
      width: null, height: null, fpsLimit: 10, error: null }, isolated: false, cleanupConfirmed: true, busy: false,
    operationId: null, phase: "disconnected", focusPosition: null,
  };
  constructor(private workflow: ScanWorkflow) {}
  snapshot(): DeviceControls {
    const status = this.workflow.confirmedNanoStatus;
    const unfinished = this.workflow.checkpointAvailable && this.workflow.phase !== "completed";
    return structuredClone({ camera: this.camera,
      turntable: { connected: this.connectedTurntable, identity: { device: "RTS9060_NANO · NO HARDWARE", version: "2.1.0 candidate preview", protocol: 2, build: "preview", buzzerInstalled: true, capabilities: ["HEARTBEAT_ACK", "XRAY_WARNING"] },
        status: status ? { state: status.state, positionPulses: status.pos, targetPulses: status.target, microsteps: status.microsteps,
          pulsesPerRev: status.ppr, referenceValid: status.homed, homed: status.homed, rearmed: status.rearmed,
          hallActive: status.hall, captureId: status.captureId } : null,
        confirmedAngleDeg: status?.homed ? status.pos * 360 / status.ppr : null,
        benchMode: this.bench, reason: null, hardwareCertified: false },
      taskLock: { locked: unfinished, reason: unfinished ? "Unfinished preview task · retain files and abandon to unlock" : null,
        taskId: unfinished ? this.workflow.params.taskId : null, savePath: unfinished ? this.workflow.params.savePath : null,
        status: unfinished ? "unfinished" : "unlocked", retained: unfinished }, presets: this.presets, exportJson: this.exportJson,
    });
  }
  private requireManual() {
    if (["scanning", "paused", "finishing", "stopping"].includes(this.workflow.phase) || this.snapshot().taskLock.locked) throw new Error("Task occupies manual controls");
  }
  private requireCamera() { if (!this.camera.connected) throw new Error("Camera not connected"); }
  async dispatch(command: EngineCommand): Promise<boolean> {
    if (!/^(camera_|turntable_|preset_|abandon_scan)/.test(command.type)) return false;
    const repair = ["camera_connect", "camera_disconnect", "camera_read_properties", "turntable_connect", "turntable_disconnect"].includes(command.type);
    if (repair && ["scanning", "paused", "finishing", "stopping"].includes(this.workflow.phase)) throw new Error("Task occupies device connections");
    if (!repair && !["camera_status", "camera_identity", "camera_live_view_stop", "turntable_status", "turntable_identity", "preset_list", "preset_export", "abandon_scan"].includes(command.type)) this.requireManual();
    const operationId = `${this.session}-${this.operations.length + 1}`;
    const epoch = this.epoch;
    this.lastOperationId = operationId;
    const operation: DeviceOperation = { operationId, deviceSessionId: this.session, device: command.type.split("_")[0], kind: command.type,
      stage: "simulated", cancelRequested: false, status: "running", error: null, startedAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
    this.operations.push(operation);
    try {
      switch (command.type) {
        case "camera_connect": {
          const identity = { deviceId: "preview-camera-d7100", manufacturer: "Nikon", model: "D7100", serial: "SIMULATED", backend: this.camera.backend };
          this.camera.discovered = [identity];
          if (!command.deviceId) break;
          if (command.deviceId !== identity.deviceId) throw new Error("Select discovered preview camera");
          this.camera.connected = true; this.camera.identity = identity; this.camera.sessionId = this.session;
          this.camera.phase = "ready"; this.camera.transferPolicy = "SIMULATED host RAM NEF · NO HARDWARE";
          Object.assign(this.camera.capabilities, { properties: true, hostCapture: true, liveView: true, autofocus: true, focusStep: true });
          const values: CameraProperties = { shutter: 65536 + 5, iso: 100, aperture: 560, exposureMode: 1, autoIso: 0,
            rawFormat: 4, rawBitDepth: 1, rawCompression: 1, crop: 0, focusMode: 1, driveMode: 0,
            exposureDelay: 0, longExposureNoiseReduction: 0, whiteBalance: 2, exposureCompensation: 0, colorSpace: 1 };
          for (const [key, value] of Object.entries(values)) this.camera.properties[key] = { key, label: key, code: 0, dataType: 4,
            readable: true, writable: true, supportedValues: key === "shutter" ? [65536 + 8000, 65536 + 1000, 65536 + 125, 65536 + 5, 65536 + 1, 1966081] : key === "iso" ? [100, 200, 400, 800] : [value], range: null, actual: value, target: value, confirmed: true, error: null };
          break;
        }
        case "camera_disconnect": this.camera.connected = false; this.camera.sessionId = null; this.camera.liveView.active = false; this.camera.liveView.fresh = false; break;
        case "camera_status": case "camera_identity": break;
        case "camera_read_properties": this.requireCamera(); break;
        case "camera_apply_properties": {
          this.requireCamera();
          for (const [key, value] of Object.entries(command.properties)) if (!this.camera.properties[key]?.supportedValues.includes(value)) throw new Error(`Unsupported native property ${key}`);
          for (const [key, value] of Object.entries(command.properties)) Object.assign(this.camera.properties[key], { actual: value, target: value, confirmed: true });
          const code = this.camera.properties.shutter.actual;
          if (typeof code === "number") this.workflow.setParams({ exposureMs: Math.floor(code / 65536) * 1000 / (code % 65536) });
          break;
        }
        case "camera_capture": case "camera_test_capture": this.requireCamera(); if (this.camera.liveView.active) throw new Error("LIVE_VIEW_CAPTURE_FORBIDDEN"); this.camera.lastCapture = `NO HARDWARE · ${command.type === "camera_capture" && command.directory ? command.directory : "Pictures/MicroCT/CameraCaptures"}/${operationId}.nef (not written)`; break;
        case "camera_live_view_start": this.requireCamera(); this.camera.liveView.active = true; this.camera.liveView.sessionId = this.session; this.camera.liveView.fresh = false; break;
        case "camera_live_view_stop": this.camera.liveView.active = false; this.camera.liveView.fresh = false; break;
        case "camera_autofocus": case "camera_focus": this.requireCamera(); break;
        case "turntable_connect": this.connectedTurntable = true; await this.workflow.readNanoStatus(); break;
        case "turntable_disconnect": this.connectedTurntable = false; break;
        case "turntable_status": case "turntable_identity": await this.workflow.readNanoStatus(); break;
        case "turntable_home": await this.workflow.independentHome(); break;
        case "turntable_relative": case "turntable_to_angle": if (!this.connectedTurntable) throw new Error("Turntable disconnected"); await this.workflow.independentMove(command.angleDeg, command.type === "turntable_relative"); break;
        case "turntable_bench_mode": if (command.enabled && !command.physicallyIsolated) throw new Error("Physical isolation confirmation required"); this.bench = command.enabled; break;
        case "preset_list": break;
        case "preset_save": this.requireCamera(); this.presets.presets.push({ id: operationId, name: command.name, description: command.description || "", models: ["D7100"], parameters: Object.fromEntries(Object.entries(this.camera.properties).map(([key, property]) => [key, property.actual!])), source: "NO HARDWARE · simulated readback", savedAt: new Date().toISOString() }); break;
        case "preset_delete": this.presets.presets = this.presets.presets.filter(preset => preset.id !== command.id); break;
        case "preset_export": this.exportJson = JSON.stringify({ schemaVersion: 1, presets: this.presets.presets }, null, 2); break;
        case "preset_load": case "preset_compare": break;
        case "preset_import": {
          this.requireCamera();
          const imported = JSON.parse(command.jsonText);
          if (command.jsonText.length > 1048576 || imported.schemaVersion !== 1 || !Array.isArray(imported.presets)
            || Object.keys(imported).sort().join(",") !== "presets,schemaVersion" || imported.presets.length > 128) throw new Error("Invalid preset file");
          const ids = new Set<string>();
          for (const preset of imported.presets) {
            if (!preset.id || ids.has(preset.id) || !preset.name || typeof preset.description !== "string"
              || !Array.isArray(preset.models) || !preset.models.includes("D7100") || typeof preset.source !== "string"
              || typeof preset.savedAt !== "string" || !preset.parameters || Object.keys(preset).sort().join(",") !== "description,id,models,name,parameters,savedAt,source") throw new Error("Invalid or incompatible preset entry");
            ids.add(preset.id);
            for (const [key, value] of Object.entries(preset.parameters)) if (!this.camera.properties[key]?.supportedValues.includes(value as number | string)) throw new Error(`Unsupported preset property ${key}`);
          }
          this.presets = { ...imported, error: null }; break;
        }
        case "abandon_scan": await this.workflow.abandonTask(); break;
      }
      if (epoch !== this.epoch) { operation.cancelRequested = true; operation.status = "cancelled"; operation.stage = "SIMULATED cancellation"; }
      else { operation.status = "completed"; operation.stage = "SIMULATED completion · NO HARDWARE"; }
    } catch (error) { operation.status = "failed"; operation.error = String(error); throw error; }
    finally { operation.updatedAt = new Date().toISOString(); }
    return true;
  }
  stop() { this.epoch++; this.camera.liveView.active = false; this.camera.liveView.fresh = false; }
}
