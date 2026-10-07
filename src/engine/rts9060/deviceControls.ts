/** Browser-only native-device UI model. All capabilities, files and completion
 * evidence are explicitly simulated; no OS transport or camera I/O exists here. */
import type { CameraHealth, CameraProperties, CameraScanReadiness, DeviceControls, DeviceOperation, EngineCommand } from "../types";
import { shutterExposureMs } from "../deviceControls";
import type { ScanWorkflow } from "./workflow";

export class PreviewDeviceControls {
  readonly session = `preview-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  operations: DeviceOperation[] = [];
  lastOperationId: string | null = null;
  private bench = false;
  private connectedTurntable = false;
  private counter=0;
  get turntableConnected():boolean{return this.connectedTurntable;}
  get busy():boolean{return this.operations.some(op=>op.status==="running"&&!/_(?:status|identity)$/.test(op.kind));}
  private presets: DeviceControls["presets"] = { schemaVersion: 1, presets: [], error: null };
  private exportJson: string | null = null;
  private epoch = 0;
  camera: CameraHealth = {
    connected: false, backend: "NO HARDWARE · in-memory camera preview", serial: null, transferPolicy: null,
    lastCapture: null, lastError: null, exposureMinMs: 0.125, exposureMaxMs: 30000, identity: null, deviceInfo: null,
    discovered: [], sessionId: null, lastEventDiagnostic: null, lastCaptureEventDiagnostic: null, capabilities: { properties: false, hostCapture: false, liveView: false,
      autofocus: false, focusStep: false, scanNef: false, hardwareCertified: false, disabledReasons: { scanNef: "NO HARDWARE · not hardware certified" } },
    properties: {}, runtimeProperties: {},multipleExposureEvidence:{physicalApplicable:false,observable:false,status:"unknown",source:"none",reason:"NO HARDWARE · no physical acquisition evidence"}, liveView: { active: false, fresh: false, sessionId: null, sequence: 0, lastFrameAt: null,
      width: null, height: null, fpsLimit: 10, error: null }, isolated: false, cleanupConfirmed: true, busy: false,
    operationId: null, phase: "disconnected", focusPosition: null,
    liveViewControls: { sessionId: null, properties: {} },
    multipleExposureOperator: { state: "none", attested: false, valid: false, confirmedAt: null, operator: null, deviceId: null, sessionId: null, policyVersion: null, evidenceFingerprint: null, machineStatus: "unknown", machineObservable: false },
  };
  constructor(private workflow: ScanWorkflow) {}
  scanReadiness(): CameraScanReadiness {
    const issues: CameraScanReadiness["issues"] = [];
    const issue = (key: string, requirement: string, remedy: string) => issues.push({
      key, actual: this.camera.properties[key]?.actual ?? null, requirement, remedy,
    });
    if (!this.camera.connected || this.camera.isolated || !this.camera.cleanupConfirmed || !this.camera.capabilities.scanNef || !this.camera.sessionId || this.camera.identity?.model!=="D7100") {
      issue("connection", "Confirmed camera session", "Connect and read the simulated camera");
      return { ready: false, issues };
    }
    const multi=this.camera.multipleExposureEvidence;
    // Preview parity: the machine state stays UNKNOWN; a session-bound operator
    // attestation is the only additional path, never a rewritten machine value.
    const operatorValid=this.camera.multipleExposureOperator?.valid===true;
    if(!operatorValid&&(!multi||!multi.physicalApplicable||!multi.observable||multi.status!=="off"||multi.source!=="SIMULATED TEST ONLY"))issue("multipleExposure","Verified single physical acquisition; multiple exposure OFF/inactive (machine evidence or a current session-bound operator attestation)","D7100 multiple exposure is physically applicable but currently unobservable; no inferred OFF state. Set the body control OFF and record the operator confirmation for this session");
    if (this.camera.liveView.active) issue("liveView", "Live View OFF", "Stop Live View explicitly");
    const required: Record<string, number | number[]> = { exposureMode: 1, autoIso: 0,
      rawFormat: 4, driveMode: 1, recordingMedia: 1, burstNumber: 1, focusMode: [3, 4],
      exposureDelay: 3, longExposureNoiseReduction: 0, activeDLighting: 4, hdrMode: 0,
      whiteBalance: [4, 5, 6, 7, 0x8010, 0x8011, 0x8012, 0x8013],bracketing:0 };
    for (const key of ["iso", "aperture", "rawBitDepth", "rawCompression", "crop"]) {
      const property = this.camera.properties[key];
      if (!property?.readable || !property.confirmed || property.actual === null || property.error || property.target!==null&&property.target!==property.actual) issue(key, "Confirmed fixed scan parameter", "Read and Apply the camera parameter before preflight");
    }
    for (const [key, expected] of Object.entries(required)) {
      const property = this.camera.properties[key];
      if(key==="hdrMode"&&property?.supported===false&&!property.readable&&!property.error&&this.camera.properties.rawFormat?.confirmed&&!this.camera.properties.rawFormat.error&&this.camera.properties.rawFormat.actual===4)continue;
      const values = Array.isArray(expected) ? expected : [expected];
      if (!property?.readable || !property.confirmed || property.actual === null || property.error || property.target!==null&&property.target!==property.actual || key==="bracketing"&&(property.code!==0xd0c0||property.dataType!==2) || typeof property.actual!=="number" || !Number.isInteger(property.actual) || !values.includes(property.actual)) {
        issue(key, `Fixed scan requires ${key}=${values.join(" or ")}`, key === "focusMode"
          ? "Select manual focus on the lens/body and read again"
          : "Change the camera draft, Apply and read the confirmed value");
      }
    }
    const shutter = this.camera.properties.shutter;
    const exposure = shutter?.readable && shutter.dataType===6 && shutter.confirmed && shutter.actual !== null && !shutter.error && (shutter.target===null||shutter.target===shutter.actual)
      ? shutterExposureMs(shutter.actual, shutter.code) : null;
    if (exposure === null || exposure < 0.125 || exposure > 30000) issue("shutter", "Confirmed native timed shutter", "Select a supported timed shutter and Apply");
    for(const [key,p] of Object.entries(this.camera.properties))if(p.supported&&p.readable&&(!p.confirmed||p.actual===null||p.error||p.target!==null&&p.target!==p.actual)&&!issues.some(issue=>issue.key===key))issue(key,"Confirmed actual value matching Apply","Read or explicitly Apply intended changes");
    return { ready: issues.length === 0, issues };
  }
  snapshot(): DeviceControls {
    const status = this.connectedTurntable?this.workflow.confirmedNanoStatus:null;
    const unfinished = this.workflow.checkpointAvailable && this.workflow.phase !== "completed";
    return structuredClone({ camera: this.camera,
      turntable: { connected: this.connectedTurntable, identity: { device: "RTS9060_NANO · NO HARDWARE", version: "2.1.0 candidate preview", protocol: 2, build: "preview", buzzerInstalled: true, capabilities: ["HEARTBEAT_ACK", "XRAY_WARNING"] },
        status: status ? { state: status.state, positionPulses: status.pos, targetPulses: status.target, microsteps: status.microsteps,
          pulsesPerRev: status.ppr, referenceValid: status.homed, homed: status.homed, rearmed: status.rearmed,
          hallActive: status.hall, captureId: status.captureId } : null,
        confirmedAngleDeg: status?.homed ? status.pos * 360 / status.ppr : null,
        benchMode: this.bench, reason: null, hardwareCertified: false },
      taskLock: { locked: unfinished, reason: this.workflow.checkpointError || (unfinished ? "Unfinished preview task · retain simulated metadata and abandon to unlock" : null),
        taskId: unfinished ? this.workflow.params.taskId : null, savePath: unfinished ? this.workflow.params.savePath : null,
        status: unfinished ? "unfinished" : "unlocked",
        // Same shape as production (`Parameters`): an array of retained task records, never a boolean.
        retained: unfinished ? [{ parameters: { taskId: this.workflow.params.taskId, savePath: this.workflow.params.savePath, projectionCount: this.workflow.params.projectionCount, angleStepDeg: this.workflow.params.projectionCount > 0 ? 360 / this.workflow.params.projectionCount : 0, exposureMs: this.workflow.params.exposureMs }, status: "unfinished", registeredAt: new Date().toISOString(), updatedAt: new Date().toISOString() }] : [] }, presets: this.presets, exportJson: this.exportJson,
    });
  }
  private requireManual() {
    if (this.busy || this.workflow.preflightRunning || this.workflow.preparationOccupied || ["scanning", "paused", "finishing", "stopping"].includes(this.workflow.phase) || this.snapshot().taskLock.locked) throw new Error("Task occupies manual controls");
  }
  private requireCamera() { if (!this.camera.connected) throw new Error("Camera not connected"); }
  private liveControls(sessionId: string) {
    this.requireCamera();
    if (this.camera.isolated || !this.camera.cleanupConfirmed || !this.camera.liveView.active || this.camera.liveView.sessionId !== sessionId || this.camera.liveViewControls?.sessionId !== sessionId) throw new Error("Live View session changed, stopped or isolated");
    return this.camera.liveViewControls;
  }
  private beginLiveControls() {
    const sessionId = crypto.randomUUID().replaceAll("-", "");
    this.camera.liveView.sessionId = sessionId;
    this.camera.liveView.sequence = 0;
    const property = (key: string, code: number, values: number[], actual: number) => ({
      key, code, label: `SIMULATED · ${key === "cameraZoom" ? "Camera zoom" : "Live focus mode"}`, dataType: 2,
      readable: true, writable: true, supported: true, nativeWritable: true, effective: true, disabledReason: null,
      supportedValues: values, range: null, actual, target: null, confirmed: true, error: null,
      options: values.map(value => ({ value, label: key === "cameraZoom" ? `Zoom level ${value}` : ({ 0: "AF-S", 2: "AF-F", 3: "MF (physical)", 4: "MF (software)" } as Record<number, string>)[value], supported: true, enabled: value !== 3, reason: value === 3 ? "Physical MF cannot be selected in software" : null })),
    });
    this.camera.liveViewControls = { sessionId, properties: {
      cameraZoom: property("cameraZoom", 0xd1a3, [0, 1, 2], 0),
      focusMode: property("focusMode", 0xd061, [0, 2, 3, 4], 4),
    } };
  }
  profileEvidence():string{return JSON.stringify({identity:this.camera.identity,multipleExposure:this.camera.multipleExposureEvidence,properties:Object.fromEntries(Object.entries(this.camera.properties).sort(([a],[b])=>a.localeCompare(b)).filter(([,p])=>p.supported&&p.readable).map(([key,p])=>[key,{code:p.code,dataType:p.dataType,actual:p.actual}]))});}
  private captureReason():string|null{
    if(!this.camera.connected||this.camera.isolated||!this.camera.cleanupConfirmed||!this.camera.capabilities.hostCapture||!this.camera.sessionId||this.camera.identity?.model!=="D7100")return "CAMERA_NOT_READY";
    if(this.camera.liveView.active)return "LIVE_VIEW_CAPTURE_FORBIDDEN";
    const value=(key:string)=>this.camera.properties[key]?.actual;
    const mode=value("exposureMode"),auto=value("autoIso");
    if(![1,2,3,4].includes(Number(mode))||![0,1].includes(Number(auto)))return "Unknown photographic exposure mode";
    const dynamic=(key:string)=>key==="iso"&&auto===1||key==="shutter"&&[2,3].includes(Number(mode))||key==="aperture"&&[2,4].includes(Number(mode));
    for(const [key,p] of Object.entries(this.camera.properties))if(p.supported&&p.readable&&!dynamic(key)&&(!p.confirmed||p.error||p.actual===null||p.target!==null&&p.target!==p.actual))return `Unknown or mismatched host capture parameter ${key}`;
    for(const [key,expected] of Object.entries({rawFormat:4,driveMode:1,recordingMedia:1,burstNumber:1,longExposureNoiseReduction:0,hdrMode:0}))if(value(key)!==expected)return `Single host NEF requires ${key}=${expected}`;
    if(!dynamic("shutter")){const p=this.camera.properties.shutter;const ms=p&&shutterExposureMs(p.actual!,p.code);if(ms===null||ms===undefined||ms<0.125||ms>30000)return "Confirmed native timed shutter required";}
    return null;
  }
  beginOperation(kind:string,device=kind.split("_")[0]):DeviceOperation{
    const operationId=`${this.session}-op-${++this.counter}`;this.lastOperationId=operationId;
    const op:DeviceOperation={operationId,deviceSessionId:this.session,device,kind,stage:"SIMULATED · NO HARDWARE",cancelRequested:false,status:"running",error:null,startedAt:new Date().toISOString(),updatedAt:new Date().toISOString()};this.operations.push(op);return op;
  }
  private refreshPropertyAvailability() {
    const actual = (key: string) => this.camera.properties[key]?.actual;
    const inactive = (key: string, reason: string) => {
      const property = this.camera.properties[key]; if (!property) return;
      property.effective = false; property.writable = false; property.disabledReason = reason;
      property.options.forEach(option => { option.enabled = false; option.reason = reason; });
    };
    for (const property of Object.values(this.camera.properties)) {
      property.effective = property.supported; property.writable = property.nativeWritable;
      property.disabledReason = null;
      property.options.forEach(option => { option.enabled = option.supported && property.writable; option.reason = option.supported ? null : "SIMULATED · device does not support this option"; });
    }
    if (![1, 4].includes(Number(actual("exposureMode")))) inactive("shutter", "SIMULATED · shutter is automatic in P/A mode");
    if (![1, 3].includes(Number(actual("exposureMode")))) inactive("aperture", "SIMULATED · aperture is automatic in P/S mode");
    if (actual("whiteBalance") !== 5) inactive("whiteBalanceFluorescent", "Select Fluorescent white balance first");
    if (actual("whiteBalance") !== 0x8012) inactive("whiteBalanceTemperature", "Select Color temperature white balance first");
    if (actual("autoIso") !== 1) inactive("autoIsoHighLimit", "Enable Auto ISO first");
    if (actual("hdrMode") === 0) for (const key of ["hdrExposureDifference", "hdrSmoothing"]) inactive(key, "Enable HDR first");
    if (actual("rawFormat") === 4) inactive("imageSize", "NEF image dimensions use the selected image area");
    if ([0, 1, 2, 3].includes(Number(actual("rawFormat")))) for (const key of ["rawBitDepth", "rawCompression"]) inactive(key, "Select a format containing NEF first");
    if (actual("meteringMode") !== 2) inactive("centerWeightedArea", "Select Center weighted metering first");
    for (const property of Object.values(this.camera.properties)) if (this.camera.liveView.active) {
      property.writable = false; property.disabledReason = "Stop Live View explicitly before applying parameters";
      property.options.forEach(option => { option.enabled = false; option.reason = property.disabledReason; });
    }
    if ([4, 5, 6, 7].includes(Number(actual("rawFormat")))) this.camera.properties.hdrMode.options.forEach(option => {
      if (option.value !== 0) { option.enabled = false; option.reason = "HDR is unavailable with NEF/NEF + JPEG"; }
    });
    delete this.camera.capabilities.disabledReasons.captureReady;
    const reason=this.captureReason();if(reason)this.camera.capabilities.disabledReasons.captureReady=`SIMULATED · ${reason}`;
    this.refreshFocusCapabilities();
  }
  private refreshFocusCapabilities() {
    const runtime = this.camera.runtimeProperties;
    const verified = (key: string, value: number) => runtime?.[key]?.supported && runtime[key].readable && runtime[key].confirmed && !runtime[key].error && runtime[key].actual === value;
    const live = this.camera.liveView.active;
    if (live && this.camera.liveViewControls?.sessionId === this.camera.liveView.sessionId) {
      for (const [key, property] of Object.entries(this.camera.liveViewControls.properties)) {
        const physicalMf = key === "focusMode" && property.actual === 3;
        property.effective = this.camera.connected && !this.camera.isolated && this.camera.cleanupConfirmed && property.supported && property.readable && property.confirmed && !property.error && !physicalMf;
        property.writable = property.effective && property.nativeWritable;
        property.disabledReason = physicalMf ? "Physical MF switch locks this control" : property.writable ? null : "Current Live View readback or write capability is unavailable";
        for (const option of property.options) {
          option.enabled = option.supported && property.writable && !(key === "focusMode" && option.value === 3);
          option.reason = option.enabled ? null : property.disabledReason || "Physical MF cannot be selected in software";
        }
      }
    }
    const control = live ? this.camera.liveViewControls?.properties.focusMode : this.camera.properties.focusMode;
    const mode = control?.supported && control.readable && control.confirmed && !control.error && typeof control.actual === "number" && Number.isSafeInteger(control.actual) && (!live || control.dataType === 2) ? control.actual : null;
    const eligible = this.camera.connected && !this.camera.isolated && this.camera.cleanupConfirmed && verified("lensSort",1)
      && (!live || verified("liveViewSelector",0) && this.camera.liveViewControls?.sessionId === this.camera.liveView.sessionId);
    this.camera.capabilities.autofocus = Boolean(eligible && mode !== null && (live ? mode === 0 : [0,1,2].includes(mode)));
    this.camera.capabilities.focusStep = Boolean(eligible && live && mode !== null && [0,4].includes(mode));
    for (const [key, permitted] of [["autofocus", this.camera.capabilities.autofocus],["focusStep", this.camera.capabilities.focusStep]] as const) {
      if (permitted) delete this.camera.capabilities.disabledReasons[key];
      else this.camera.capabilities.disabledReasons[key] = "SIMULATED · current lens, photo Live View or bounded focus-mode eligibility is unconfirmed";
    }
  }
  async dispatch(command: EngineCommand): Promise<boolean> {
    if (command.type === "camera_capture_descriptor") return false;
    if (!/^(camera_|turntable_|preset_|abandon_scan)/.test(command.type)) return false;
    const repair = ["camera_connect", "camera_disconnect", "camera_read_properties", "camera_live_view_read", "turntable_connect", "turntable_disconnect"].includes(command.type);
    if (repair && (this.busy || this.workflow.preflightRunning || this.workflow.preparationOccupied || ["scanning", "paused", "finishing", "stopping"].includes(this.workflow.phase))) throw new Error("Task occupies device connections");
    if (!repair && !["camera_status", "camera_identity", "camera_live_view_stop", "turntable_status", "turntable_identity", "preset_list", "preset_export", "abandon_scan"].includes(command.type)) this.requireManual();
    const epoch=this.epoch;const operation=this.beginOperation(command.type);const operationId=operation.operationId;
    try {
      switch (command.type) {
        case "camera_connect": {
          const identity = { deviceId: "preview-camera-d7100", manufacturer: "Nikon", model: "D7100", serial: "SIMULATED", backend: this.camera.backend };
          this.camera.discovered = [identity];
          if (!command.deviceId) break;
          if (command.deviceId !== identity.deviceId) throw new Error("Select discovered preview camera");
          this.camera.connected = true; this.camera.identity = identity; this.camera.sessionId = this.session;
          this.camera.phase = "ready";this.camera.multipleExposureEvidence={physicalApplicable:true,observable:false,status:"unknown",source:"none",reason:"NO HARDWARE · model multiple exposure is applicable but native OFF evidence is not available"};
          // A new session invalidates any previous operator attestation.
          this.camera.multipleExposureOperator={state:"none",attested:false,valid:false,confirmedAt:null,operator:null,deviceId:null,sessionId:null,policyVersion:null,evidenceFingerprint:null,machineStatus:"unknown",machineObservable:false};
          this.camera.transferPolicy = "SIMULATED host RAM NEF · NO HARDWARE";
          Object.assign(this.camera.capabilities, { properties: true, hostCapture: true, liveView: true, autofocus: true, focusStep: true,scanNef:true });
          const values: CameraProperties = { shutter: 65536 + 5, iso: 100, aperture: 560, exposureMode: 1, autoIso: 0,
            bracketing:0,rawFormat: 4, rawBitDepth: 1, rawCompression: 1, crop: 0, focusMode: 4, driveMode: 1,
            exposureDelay: 3, longExposureNoiseReduction: 0, whiteBalance: 2, exposureCompensation: 0, colorSpace: 1, recordingMedia: 1, burstNumber: 1,
            meteringMode: 2, imageSize: "6000x4000", highIsoNoiseReduction: 0, whiteBalanceFluorescent: 0,
            whiteBalanceTemperature: 20, autoIsoHighLimit: 20, centerWeightedArea: 0, flashMode: 0x8010,
            flashCompensation: 0, flashSyncSpeed: 0, activeDLighting: 4, hdrMode: 0, hdrExposureDifference: 0, hdrSmoothing: 0 };
          const choices: Record<string, (number | string)[]> = {
            exposureMode: [1, 2, 3, 4], autoIso: [0, 1], whiteBalance: [2, 4, 5, 6, 7, 0x8010, 0x8011, 0x8012, 0x8013],
            bracketing:[0,1],focusMode:[0,1,2,3,4],activeDLighting:[0,1,2,3,4,5],
            rawFormat: [0, 1, 2, 4, 5, 6, 7], rawBitDepth: [0, 1], rawCompression: [0, 1], hdrMode: [0, 1, 2], meteringMode: [2, 3, 4],
          };
          const labels: Record<string, Record<string, string>> = {
            exposureMode: { 1: "M · Manual", 2: "P · Program", 3: "A · Aperture priority", 4: "S · Shutter priority" },
            whiteBalance: { 2: "Auto", 4: "Daylight", 5: "Fluorescent", 6: "Incandescent", 7: "Flash", 32784: "Cloudy", 32785: "Shade", 32786: "Color temperature", 32787: "Preset manual" },
            rawFormat: { 0: "JPEG basic", 1: "JPEG normal", 2: "JPEG fine", 4: "NEF (RAW)", 5: "NEF + JPEG basic", 6: "NEF + JPEG normal", 7: "NEF + JPEG fine" },
            whiteBalanceTemperature: { 20: "5000 K" }, autoIsoHighLimit: { 20: "ISO 6400" },
          };
          for (const [key, value] of Object.entries(values)) this.camera.properties[key] = { key, label: key,
            code: key === "shutter" ? 0xd100 : key === "recordingMedia" ? 0xd10b : key === "burstNumber" ? 0x5018 : key==="bracketing"?0xd0c0:0,
            dataType: key === "shutter" ? 6 : key === "recordingMedia" ||key==="bracketing" ? 2 : 4,
            readable: true, writable: true, supported: true, nativeWritable: true, effective: true, disabledReason: null, options: [],
            supportedValues: key === "shutter" ? [65536 + 8000, 65536 + 1000, 65536 + 125, 65536 + 5, 65536 + 1, 1966081] : key === "iso" ? [100, 200, 400, 800] : [value], range: null, actual: value, target: value, confirmed: true, error: null };
          for (const [key, property] of Object.entries(this.camera.properties)) {
            property.supported = true; property.nativeWritable = true; property.effective = true; property.disabledReason = null;
            property.supportedValues = choices[key] || property.supportedValues;
            property.options = property.supportedValues.map(value => ({ value, label: labels[key]?.[String(value)] || String(value), supported: true, enabled: true, reason: null }));
            if (key === "rawFormat") property.options.push({ value: 3, label: "TIFF RGB", supported: false, enabled: false, reason: "SIMULATED · this model does not advertise TIFF" });
          }
          this.camera.runtimeProperties = Object.fromEntries([["lensSort",0xd0e1,1],["liveViewSelector",0xd1a6,0]].map(([key,code,value]) => [String(key),{
            ...structuredClone(this.camera.properties.focusMode),key:String(key),code:Number(code),label:`SIMULATED · ${key}`,dataType:1,
            nativeWritable:false,writable:false,effective:true,disabledReason:"Read-only simulated diagnostic",actual:Number(value),target:null,
            options:[],supportedValues:[Number(value)],semanticStatus:"unknown" as const,
          }]));
          this.refreshPropertyAvailability();
          if(!this.snapshot().taskLock.locked)this.workflow.setParams({exposureMs:200});
          break;
        }
        case "camera_disconnect": this.camera.connected = false; this.camera.sessionId = null; this.camera.liveView.active = false; this.camera.liveView.fresh = false; this.camera.liveViewControls = {sessionId:null,properties:{}}; this.camera.multipleExposureOperator={state:"none",attested:false,valid:false,confirmedAt:null,operator:null,deviceId:null,sessionId:null,policyVersion:null,evidenceFingerprint:null,machineStatus:"unknown",machineObservable:false}; this.workflow.preflightPassed=false; break;
        case "camera_status": case "camera_identity": break;
        case "camera_read_properties": this.requireCamera(); break;
        case "camera_confirm_multiple_exposure_off": {
          this.requireCamera();
          if (!command.confirmed) { this.camera.multipleExposureOperator={state:"none",attested:false,valid:false,confirmedAt:null,operator:null,deviceId:null,sessionId:null,policyVersion:null,evidenceFingerprint:null,machineStatus:this.camera.multipleExposureEvidence?.status ?? "unknown",machineObservable:this.camera.multipleExposureEvidence?.observable ?? false}; this.workflow.preflightPassed=false; break; }
          const operator = command.operator ?? "operator";
          this.camera.multipleExposureOperator={state:"confirmed",attested:true,valid:true,confirmedAt:new Date().toISOString(),operator,deviceId:this.camera.identity?.deviceId ?? null,sessionId:this.camera.sessionId,policyVersion:1,evidenceFingerprint:"0".repeat(64),machineStatus:this.camera.multipleExposureEvidence?.status ?? "unknown",machineObservable:this.camera.multipleExposureEvidence?.observable ?? false};
          this.workflow.preflightPassed=false;
          break;
        }
        case "camera_apply_properties": {
          this.requireCamera();
          for (const [key, value] of Object.entries(command.properties)) {
            const property = this.camera.properties[key];
            if (!property?.supportedValues.includes(value)) throw new Error(`Unsupported native property ${key}`);
            if (property.actual !== value && !property.writable) throw new Error(property.disabledReason || `Inactive property ${key}`);
            const option = property.options.find(option => option.value === value);
            if (property.actual !== value && option && !option.enabled) throw new Error(option.reason || "Option disabled");
          }
          const changing = Object.entries(command.properties).some(([key, value]) => this.camera.properties[key]?.actual !== value);
          for (const [key, value] of Object.entries(command.properties)) Object.assign(this.camera.properties[key], { actual: value, target: value, confirmed: true });
          this.refreshPropertyAvailability();
          this.workflow.preflightPassed=false;
          // Any changed confirmed parameter invalidates a session-bound attestation.
          if (changing && this.camera.multipleExposureOperator?.attested) this.camera.multipleExposureOperator = { ...this.camera.multipleExposureOperator, state: "stale", valid: false };
          if("shutter" in command.properties){const shutter=this.camera.properties.shutter;const exposure=shutterExposureMs(shutter.actual!,shutter.code);if(exposure===null)throw new Error("Confirmed native shutter required");this.workflow.setParams({exposureMs:exposure});}
          break;
        }
        case "camera_capture": case "camera_test_capture": {
          this.requireCamera();
          if (this.camera.liveView.active) throw new Error("LIVE_VIEW_CAPTURE_FORBIDDEN");
          const reason=this.captureReason();if(reason)throw new Error(reason);
          const root = this.workflow.params.savePath?.trim();
          if (!root && !command.directory) throw new Error("Choose the image storage root first");
          if(command.type==="camera_capture"&&!root)throw new Error("CAMERA_CAPTURE_ROOT_REQUIRED");
          const directory = `${root?.replace(/[\\/]$/, "")}/CameraCaptures`;
          if (command.type === "camera_capture" && command.directory && command.directory.replaceAll("\\", "/") !== directory.replaceAll("\\", "/")) throw new Error("Independent photo directory must use the camera storage root");
          this.camera.lastCapture = `NO HARDWARE · ${command.directory || directory}/${operationId}.nef (not written)`;
          this.camera.lastCapturedImage = null;
          this.workflow.recordEvent?.("INFO","camera","NO HARDWARE · simulated independent photograph; no image file or capture registration exists");
          break;
        }
        case "camera_live_view_start": this.requireCamera(); if (!this.camera.liveView.active) { this.camera.liveView.active = true; this.beginLiveControls(); this.camera.liveView.fresh = false; this.refreshPropertyAvailability(); } break;
        case "camera_live_view_stop": this.camera.liveView.active = false; this.camera.liveView.fresh = false; this.camera.liveViewControls = {sessionId:null,properties:{}}; this.refreshPropertyAvailability(); break;
        case "camera_live_view_read": this.liveControls(command.sessionId); this.refreshFocusCapabilities(); break;
        case "camera_live_view_configure": {
          const current = this.liveControls(command.sessionId);
          const entries = Object.entries(command.controls);
          if (!entries.length || entries.some(([key, value]) => !["cameraZoom", "focusMode"].includes(key) || !current.properties[key]?.options.some(option => option.value === value && option.supported && option.enabled) || !current.properties[key].writable)) throw new Error("Unsupported or inactive Live View control");
          for (const [key, value] of entries) Object.assign(current.properties[key], {actual:value,target:value,confirmed:true,error:null});
          this.refreshFocusCapabilities();
          break;
        }
        case "camera_autofocus": this.requireCamera(); this.refreshFocusCapabilities(); if (!this.camera.capabilities.autofocus) throw new Error("Autofocus eligibility unconfirmed"); break;
        case "camera_focus": this.requireCamera(); this.refreshFocusCapabilities(); if (!this.camera.capabilities.focusStep || !Number.isSafeInteger(command.step) || command.step === 0 || Math.abs(command.step)>500) throw new Error("Bounded focus step eligibility unconfirmed"); break;
        case "turntable_connect": this.connectedTurntable = true; await this.workflow.readNanoStatus(); break;
        case "turntable_disconnect": this.connectedTurntable = false; break;
        case "turntable_status": case "turntable_identity": await this.workflow.readNanoStatus(); break;
        case "turntable_home": if(!this.connectedTurntable)throw new Error("Turntable disconnected");await this.workflow.independentHome(); break;
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
        case "abandon_scan": if(this.workflow.checkpointError)throw new Error("TASK_REGISTRY_INVALID");await this.workflow.abandonTask(); break;
      }
      if (epoch !== this.epoch) { operation.cancelRequested = true; operation.status = "cancelled"; operation.stage = "SIMULATED cancellation"; }
      else { operation.status = "completed"; operation.stage = "SIMULATED completion · NO HARDWARE"; }
    } catch (error) { operation.status = "failed"; operation.error = String(error); throw error; }
    finally { operation.updatedAt = new Date().toISOString(); }
    return true;
  }
  stop() { this.epoch++; this.camera.liveView.active = false; this.camera.liveView.fresh = false;this.camera.liveViewControls={sessionId:null,properties:{}};this.workflow.preflightPassed=false;this.refreshPropertyAvailability(); }
}
