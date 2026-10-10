/** Pure presentation rules for native device evidence, drafts and ordering.
 * No helper accesses a device or grants engine permission; unknown stays blocked. */
import type { CameraHealth, CameraProperties, CameraProperty, CameraScanReadiness, CameraValue, EngineSnapshot } from "./types";
import { NATIVE_ENUM_LABELS, englishDisplayText, unicodeDisplayText, englishOptionLabel, englishPropertyLabel, englishReason } from "./native-labels";

export function deviceConnectionReason(snapshot: EngineSnapshot, stale = false): string | null {
  if (stale) return "Control service unavailable · device evidence unknown";
  if (snapshot.softwareUpdatePreparation?.maintenanceLocked || snapshot.softwareUpdatePreparation?.ready) return "Software update preparation owns the devices";
  if (["running", "paused", "finishing", "stopping"].includes(snapshot.phase)) return "Scan or safety cleanup owns the devices";
  if (snapshot.workstation?.reconstruction.status === "running") return "Reconstruction owns the task";
  if (snapshot.operations.some(op => ["queued", "running", "cancelling", "cleanup_unconfirmed"].includes(op.status))) return "Operation pending or cleanup unconfirmed";
  return null;
}
export function manualControlReason(snapshot: EngineSnapshot, stale = false): string | null {
  const occupied = deviceConnectionReason(snapshot, stale);
  if (occupied) return occupied;
  if (snapshot.deviceControls.taskLock.locked) return englishDisplayText(snapshot.deviceControls.taskLock.reason) || "Unfinished task · abandon it before manual control";
  return null;
}
export function cameraControlReason(camera: CameraHealth | null): string | null {
  if (!camera?.connected) return englishDisplayText(camera?.lastError) || "Select and connect a camera";
  if (camera.isolated || !camera.cleanupConfirmed) return "Camera cleanup unconfirmed · session isolated";
  if (camera.busy) return "Camera operation is running";
  return null;
}
export function cameraCapturePreparationReason(camera: CameraHealth | null): string | null {
  if (!camera) return "Read camera parameters before capture";
  if (camera.capabilities.disabledReasons.captureReady) return englishReason(camera.capabilities.disabledReasons.captureReady);
  const media = camera.properties.recordingMedia;
  if (!media?.confirmed || media.error || media.actual !== 1) return "Apply and verify RAM recording media before capture";
  const burst = camera.properties.burstNumber;
  if (!burst?.confirmed || burst.error || burst.actual !== 1) return "Apply and verify a single photo before capture";
  return null;
}
export function shutterExposureMs(native: CameraValue, propertyCode: number | undefined): number | null {
  if (typeof native !== "number" || !Number.isSafeInteger(native) || native <= 0 || native > 0xffffffff) return null;
  if (propertyCode === 0x500d) return native === 0xffffffff ? null : native / 10;
  if (propertyCode !== 0xd100 || native >= 0xfffffffd) return null;
  const numerator = Math.floor(native / 65536); const denominator = native % 65536;
  return numerator > 0 && denominator > 0 ? numerator * 1000 / denominator : null;
}
export function nativePropertyLabel(key: string, value: CameraValue | null, propertyCode?: number): string {
  if (value === null) return "UNKNOWN";
  if (key === "shutter") {
    const ms = shutterExposureMs(value, propertyCode);
    if (ms !== null && typeof value === "number") return propertyCode === 0x500d
      ? `${ms} ms` : `${Math.floor(value / 65536)}/${value % 65536} s · ${ms} ms`;
  }
  if (key === "aperture" && typeof value === "number") return `f/${value / 100}`;
  return unicodeDisplayText(String(value));
}

/** D7100 manual dependencies for presentation/preview. Native permissions and
 * fresh reads are still enforced by the engine; drafts never unlock controls. */
function confirmedSetting(properties: Record<string, CameraProperty>, key: string): CameraValue | null {
  const property = properties[key];
  return property?.supported && property.readable && property.confirmed && !property.error ? property.actual : null;
}
const PASM_SETTINGS = new Set(["autoIso", "autoIsoHighLimit", "whiteBalance", "whiteBalanceFluorescent",
  "whiteBalanceTemperature", "pictureControl", "meteringMode", "centerWeightedArea", "activeDLighting",
  "hdrMode", "hdrExposureDifference", "hdrSmoothing", "bracketing", "flashCompensation"]);
function hdrOffReason(properties: Record<string, CameraProperty>): string | null {
  const hdr = confirmedSetting(properties, "hdrMode");
  if (hdr === 0) return null;
  if (hdr === 1 || hdr === 2) return "HDR is on; turn it Off, Apply and verify before using this setting";
  const property = properties.hdrMode;
  if (property && !property.supported && !property.readable && !property.error
    && [4, 5, 6, 7].includes(Number(confirmedSetting(properties, "rawFormat")))) return null;
  return "HDR readback is unconfirmed; refresh settings before using this control";
}
export function cameraSettingDependencyReason(key: string, properties: Record<string, CameraProperty>): string | null {
  const check = (parent: string, allowed: CameraValue[], reason: string) => {
    const value = confirmedSetting(properties, parent);
    return value === null ? `${englishPropertyLabel(parent)} readback is unconfirmed; refresh settings first`
      : allowed.includes(value) ? null : reason;
  };
  if (PASM_SETTINGS.has(key)) {
    const reason = check("exposureMode", [1, 2, 3, 4], "This control requires confirmed P/S/A/M mode; select the mode on the camera or Apply it, then refresh settings");
    if (reason) return reason;
  }
  switch (key) {
    case "shutter": return check("exposureMode", [1, 4], "The shutter is automatic in P/A mode; M/S enables a manual shutter");
    case "aperture": return check("exposureMode", [1, 3], "The aperture is automatic in P/S mode; M/A enables a manual aperture");
    case "rawBitDepth": case "rawCompression": return check("rawFormat", [4, 5, 6, 7], "The current image format contains no NEF, so RAW parameters have no effect");
    case "imageSize": return check("rawFormat", [0, 1, 2, 5, 6, 7], "NEF uses a fixed RAW size; this setting only affects the JPEG image size");
    case "whiteBalanceFluorescent": return check("whiteBalance", [5], "Select Fluorescent white balance and Apply first");
    case "whiteBalanceTemperature": return check("whiteBalance", [0x8012], "Select Color temperature white balance and Apply first");
    case "autoIsoHighLimit": return check("autoIso", [1], "Auto ISO is off, so the Auto ISO high limit has no effect");
    case "centerWeightedArea": return check("meteringMode", [2], "Select Center-weighted metering and Apply first");
    case "exposureCompensation": {
      const mode = confirmedSetting(properties, "exposureMode");
      if (mode === null) return "Exposure mode is unconfirmed; refresh settings before adjusting exposure compensation";
      return [0x8010, 0x8011, 0x8012, 0x8013, 0x8014, 0x8016, 0x8017, 0x8018].includes(Number(mode))
        ? "Exposure compensation is unavailable in this automatic/scene mode" : null;
    }
    case "hdrExposureDifference": case "hdrSmoothing": return check("rawFormat", [0, 1, 2], "HDR is unavailable with NEF") || check("hdrMode", [1, 2], "HDR is off, so its sub-parameters have no effect");
    case "focusMode": return confirmedSetting(properties, key) === 3 ? "Physical MF is locked by the camera/lens switch; change the switch, then read again" : null;
    case "flashCompensation": case "flashSyncSpeed": return hdrOffReason(properties);
    default: return null;
  }
}
export function cameraSettingOptionReason(key: string, value: CameraValue, properties: Record<string, CameraProperty>): string | null {
  if (key === "focusMode" && value === 3) return "Physical MF is selected by the camera/lens switch; use a supported software MF target instead";
  if (key === "hdrMode" && value !== 0) {
    const format = confirmedSetting(properties, "rawFormat");
    if (format === null) return "Image format readback is unconfirmed; refresh settings before enabling HDR";
    if (![0, 1, 2].includes(Number(format))) return "HDR cannot be enabled with NEF/NEF+JPEG; Off remains available";
    const bracket = confirmedSetting(properties, "bracketing");
    return bracket === 0 ? null : bracket === null ? "Bracketing readback is unconfirmed; refresh settings before enabling HDR"
      : "Bracketing is on; turn it Off, Apply and verify before enabling HDR";
  }
  if (key === "rawFormat" && [4, 5, 6, 7].includes(Number(value))) {
    const hdr = properties.hdrMode;
    return hdr && !hdr.supported && !hdr.readable && !hdr.error ? null : hdrOffReason(properties);
  }
  if (key === "bracketing" && value !== 0 || key === "flashMode" && value !== 2) {
    const reason = hdrOffReason(properties); if (reason) return reason;
  }
  if (key === "flashMode" && [0x8011, 0x8013].includes(Number(value))) {
    const mode = confirmedSetting(properties, "exposureMode");
    if (mode === null) return "Exposure mode is unconfirmed; refresh settings before choosing slow sync";
    if (mode === 1 || mode === 4) return "Slow sync and red-eye slow sync require P/A mode; they are unavailable in S/M";
  }
  return null;
}
export function validateCameraDraft(camera: CameraHealth, draft: CameraProperties, checkCurrentAvailability = true): string | null {
  if (!Object.keys(draft).length) return "No camera parameter draft";
  for (const [key, value] of Object.entries(draft)) {
    const property = camera.properties[key];
    if (!property) return `${key}: readable/writable capability unavailable`;
    if (property?.confirmed && !property.error && property.actual === value) continue;
    const reason = checkCurrentAvailability ? cameraPropertyReason(property)
      : !property.supported || !property.readable || !property.nativeWritable ? property.disabledReason || "Readable/writable capability unavailable" : null;
    if (reason) return `${key}: ${englishDisplayText(reason)}`;
    if (typeof value === "number" && !Number.isSafeInteger(value)) return `${key}: native value must be an integer`;
    if (property.supportedValues.length && !property.supportedValues.includes(value)) return `${key}: native value is not supported`;
    const option = property.options?.find(option => option.value === value);
    if (option && (!option.supported || (checkCurrentAvailability && !option.enabled))) return `${key}: ${englishDisplayText(option.reason) || "Option unavailable"}`;
    if (property.range) {
      const [min, max, step] = property.range;
      if (typeof value !== "number" || value < min || value > max || (step > 0 && (value - min) % step !== 0)) return `${key}: native value is outside described range`;
    }
    if (key === "shutter" && shutterExposureMs(value, property.code) === null) return "Shutter encoding is unknown or not a timed native value";
  }
  if (camera.backend === "windows_wpd_nikon_d7100" || camera.backend === "NO HARDWARE · in-memory camera preview") {
    const projected = Object.fromEntries(Object.entries(camera.properties).map(([key, property]) => [key,
      key in draft ? { ...property, actual: draft[key], confirmed: true, error: null } : property]));
    for (const [key, value] of Object.entries(draft)) {
      const property = camera.properties[key];
      if (property.confirmed && !property.error && property.actual === value) continue;
      const reason = cameraSettingDependencyReason(key, projected) || cameraSettingOptionReason(key, value, projected);
      if (reason) return `${englishPropertyLabel(key)}: ${reason}`;
    }
  }
  return null;
}
/** English title of a native parameter. The engine currently returns Chinese labels for
 *  15 of the 34 catalog keys, so the presentation layer owns the name. */
export function cameraPropertyLabel(property: CameraProperty | undefined): string {
  return property ? englishPropertyLabel(property.key, property.label) : "Parameter";
}
export function cameraPropertyReason(property: CameraProperty | undefined): string | null {
  if (!property?.supported || !property.readable) return englishReason(property?.disabledReason) || "Readable/writable capability unavailable";
  if (!property.effective || !property.writable) return englishReason(property.disabledReason) || "Parameter is read only or inactive in the current mode";
  return null;
}
export function cameraDraftChanges(camera: CameraHealth, draft: CameraProperties): CameraProperties {
  return Object.fromEntries(Object.entries(draft).filter(([key, value]) => {
    const property = camera.properties[key];
    return !property?.confirmed || Boolean(property.error) || property.actual !== value;
  }));
}
export function cameraPropertyValueLabel(property: CameraProperty, value: CameraValue | null): string {
  if (value === null) return "UNKNOWN";
  const option = property.options?.find(option => option.value === value);
  const english = englishOptionLabel(property.key, value, option?.label);
  if (english !== `Native value ${value}`) return english;
  // A value the presentation tables do not name yet: keep the key-specific native
  // formatting (shutter rational, f-number) but never fall back to a Chinese label.
  return NATIVE_ENUM_LABELS[property.key] ? english : nativePropertyLabel(property.key, value, property.code);
}
/** Use a labelled select for a fully described finite range as well as enums.
 * A partially labelled or open range keeps its numeric input and full range. */
export function cameraPropertyUsesSelect(property: CameraProperty): boolean {
  if (property.supportedValues.length || !property.range) return property.options.length > 0;
  const [min, max, step] = property.range;
  if (step <= 0) return false;
  const count = (max - min) / step + 1;
  if (!Number.isSafeInteger(count) || count < 1 || count > 4096) return false;
  const described = new Set(property.options.filter(option => option.supported && typeof option.value === "number"
    && option.value >= min && option.value <= max && (option.value - min) % step === 0).map(option => option.value));
  return described.size === count;
}
export function confirmedCameraDraft(properties: Record<string, CameraProperty>): CameraProperties {
  return Object.fromEntries(Object.values(properties).filter(p => !cameraPropertyReason(p) && p.confirmed && p.actual !== null).map(p => [p.key, p.actual!]));
}
export function exactShutterForExposure(camera: CameraHealth, exposureMs: number): CameraValue | null {
  const shutter = camera.properties.shutter;
  return shutter?.supportedValues.find(value => shutterExposureMs(value, shutter.code) === exposureMs) ?? null;
}
/** Engine-issued ordering replaces UI request order; retired sessions cannot return. */
export class SnapshotOrder {
  private session: string | null = null;
  private sequence = -1;
  private retired = new Set<string>();
  accept(snapshot: EngineSnapshot): boolean {
    if (!snapshot.engineSessionId || !Number.isSafeInteger(snapshot.snapshotSequence) || snapshot.snapshotSequence < 0) return false;
    if (snapshot.engineSessionId !== this.session) {
      if (this.retired.has(snapshot.engineSessionId)) return false;
      if (this.session !== null) this.retired.add(this.session);
      this.session = snapshot.engineSessionId; this.sequence = -1;
    }
    if (snapshot.snapshotSequence <= this.sequence) return false;
    this.sequence = snapshot.snapshotSequence;
    return true;
  }
}

/** Engine error codes are raw strings, never a whitelist (docs/architecture/frontend-error-codes.md §0.2). */
export function parseEngineCode(message: string | null | undefined): string | null {
  if (!message) return null;
  const match = /ENGINE:([A-Z0-9_]+)/.exec(message);
  if (match) return match[1];
  return /^([A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)(?::|\s|$)/.exec(message.trim())?.[1] ?? null;
}

export type FailureKind = "cleanup_unconfirmed" | "failed" | "transport" | "rejected" | "engine";
export interface FailureReport {
  /** Stable dedup key. A new attempt must produce a new key, a polling re-render must not. */
  key: string;
  kind: FailureKind;
  title: string;
  /** Failure stage, e.g. "camera_apply_properties · rejected". */
  stage: string;
  /** Verbatim engine text. Never truncated, never rewritten. */
  reason: string;
  /** Readable presentation; raw engine details stay in reason and Copy details. */
  displayReason?: string;
  code: string | null;
  nextAction: string;
  /** Troubleshooting group id, see TroubleshootingPane. */
  group: string;
}

export type FailureAdvice = { action: string; group: string };
/** Keyed by engine code. An unknown code must stay raw and must never be
 *  reclassified as success/off/normal/safe (docs/architecture/frontend-error-codes.md §0.2). */
export const FAILURE_ADVICE: Record<string, FailureAdvice> = {
  // G2 occupancy and locks — "recoverable, retry" (§G2)
  DEVICE_CONTROL_LOCKED: { action: "A scan, cleanup or reconstruction owns the devices. Wait for it to finish, or use Stop if you intend to end it. Do not reconnect.", group: "files" },
  SCAN_ACTIVE: { action: "The engine is busy with the scan. Use Stop to end it, or wait for the current phase to finish.", group: "files" },
  DEVICE_OPERATION_ACTIVE: { action: "Another operation for this device is already in flight. Wait for it to complete; do not resend.", group: "files" },
  SOFTWARE_UPDATE_BUSY: { action: "A scan, operation or maintenance prepare is active. Finish or stop it before preparing an update.", group: "files" },
  SOFTWARE_UPDATE_MAINTENANCE_LOCKED: { action: "Software-update maintenance owns the devices. Cancel the maintenance prepare before any device command.", group: "files" },
  RECONSTRUCTION_ACTIVE: { action: "A reconstruction is running. Wait for it to finish, then retry.", group: "files" },
  RECONSTRUCTION_LOCKED: { action: "Reconstruction is locked by occupancy or an unfinished task. Release the task lock first.", group: "files" },
  // G2 — "recovery path required"
  DEVICE_CLEANUP_UNCONFIRMED: { action: "An isolated session has unconfirmed cleanup, so ordinary commands are refused. Retry the camera connection with the explicit 'acknowledge unconfirmed cleanup' path, or restart the engine. Cleanup is NOT confirmed and no lock is cleared by dismissing this.", group: "files" },
  FAULT_RECOVERY_REQUIRED: { action: "A device fault is latched, and the only clearing path is a successful Preflight. Fix the device condition, connect all devices, then run Preflight and HOME.", group: "xray" },
  UNFINISHED_TASK_LOCKED: { action: "A retained unfinished task locks parameters, focus and motion. Finish, restore, or explicitly abandon it (files are kept) in Task → Restore/Abandon.", group: "files" },
  RECONNECT_REQUIRED: { action: "The engine holds no turntable adapter any more. Reconnect the turntable, then re-run Preflight and HOME.", group: "turntable" },
  RECOVERY_REQUIRED: { action: "The task is Stopped, so Restore no longer applies. Use the recovery flow (Preflight, then HOME, then restore) or explicitly abandon the task.", group: "files" },
  NOT_RESUMABLE: { action: "Resume needs the Paused phase plus a valid Preflight and HOME reference. Re-establish them, then resume.", group: "files" },
  RESTORE_VALIDATION_FAILED: { action: "The retained task's hash/geometry/parameter evidence does not match. Do not resume; inspect the retained directory and choose the correct task.", group: "files" },
  TASK_REGISTRY_INVALID: { action: "The task identity did not verify, so abandon/complete are refused. Inspect the task directory and the retained record before retrying.", group: "files" },
  TASK_REGISTRY_SAVE_FAILED: { action: "The registry write plan could not be built. Check write permission and free space on the image root, then retry.", group: "files" },
  HOME_REQUIRED: { action: "No valid HOME/Hall reference is established. With X-ray off (or in isolated bench mode), run HOME, then Preflight again.", group: "turntable" },
  NANO_STATUS_UNKNOWN: { action: "No fresh Nano STATUS is available. Read Status, reconnect the turntable if it stays unknown, then re-run HOME.", group: "turntable" },
  NANO_COMMUNICATION_FAILED: { action: "Serial communication with the Nano failed. Check the cable and port, reconnect the turntable, then re-run Preflight and HOME. X-ray output must be treated as off only after a confirmed readback.", group: "turntable" },
  XRAY_OFF_UNCONFIRMED: { action: "Beam-off was not confirmed by readback, so the engine treats output as UNKNOWN. Do not treat the machine as safe. Re-read X-ray status and repeat OFF; if it stays unconfirmed, stop and inspect the source.", group: "xray" },
  XRAY_PREFLIGHT_FAILED: { action: "Interlock, temperature or setpoint health failed during Preflight. Inspect the panel readings, clear the condition, then re-run Preflight.", group: "xray" },
  XRAY_DISCONNECT_FAILED: { action: "OFF could not be re-confirmed while disconnecting. Treat output as UNKNOWN and re-verify at the source before any further action.", group: "xray" },
  XRAY_SETPOINT_FAILED: { action: "The setpoint write or readback failed. Confirm voltage and current read back on the panel, or re-send the setpoints.", group: "xray" },
  SCAN_SHUTDOWN_TIMEOUT: { action: "The scan did not finish within the shutdown wait. Check the engine process and the log; do not assume the devices were cleaned up.", group: "files" },
  SCAN_STOP_FAILED: { action: "The phase is still Fault after STOP. Inspect the device panels and complete the recovery flow; do not report a safe stop.", group: "files" },
  // G3 camera — "configuration fix first"
  CAMERA_PROPERTY_UNSUPPORTED_VALUE: { action: "The camera rejected this value even though the native descriptor lists it. Choose a different confirmed value, or restore the original actual value, then Apply again. This is not a device fault and the session stays usable.", group: "camera" },
  CAMERA_PROPERTY_DEPENDENCY_UNMET: { action: "A parent property is not yet confirmed. Apply and read back the parent (for white balance: set the mode, Apply, read back) before changing this dependent value.", group: "camera" },
  CAMERA_PROPERTY_WRITE_UNSUPPORTED: { action: "The camera does not advertise a write operation for this property. Use the camera body or lens control, then Read again.", group: "camera" },
  CAMERA_PROPERTY_UNDESCRIBED: { action: "The camera did not describe this property in a verifiable way, so it cannot be written. Read the device info and try again; do not guess native values.", group: "camera" },
  CAMERA_FOCUS_MODE_LOCKED: { action: "The focus mode is locked by the current body/lens switch state. Change it on the lens or body, then Read again.", group: "camera" },
  CAMERA_NOT_READY: { action: "The camera session is not usable (not connected, or isolated with unconfirmed cleanup). Select and connect the camera again, then Read.", group: "camera" },
  CAMERA_NOT_CONNECTED: { action: "No camera session exists. Run Discover, select the camera explicitly, then Connect.", group: "camera" },
  CAMERA_ALREADY_CONNECTED: { action: "A camera session is already connected. Disconnect first if you need to switch devices; the session was not disturbed.", group: "camera" },
  LIVE_VIEW_APPLY_FORBIDDEN: { action: "Photographic parameter Apply is refused while Live View is active. Stop Live View explicitly, apply the parameters, then restart Live View if needed.", group: "camera" },
  LIVE_VIEW_CAPTURE_FORBIDDEN: { action: "Capture is refused while Live View is active, and the engine will not stop the preview for you. Stop Live View, then capture.", group: "camera" },
  CAMERA_CAPTURE_UNQUALIFIED: { action: "The published capture readiness is not satisfied (recording media, single release, Live View off, read-only run diagnostics). Resolve each item in Camera → Advanced, then Apply and verify.", group: "camera" },
  CAMERA_FOCUS_UNQUALIFIED: { action: "Lens step drive eligibility is not established. Start Live View, read the Live View controls, and confirm the lens/focus evidence before stepping.", group: "camera" },
  CAMERA_AUTOFOCUS_UNQUALIFIED: { action: "Autofocus eligibility is not established. Live View must be active with a focus mode that permits AF, and the lens evidence must be fresh.", group: "camera" },
  CAMERA_SHUTTER_UNCONFIRMED: { action: "A confirmed native shutter readback is required before preparing a scan. Read the camera properties, Apply and verify the shutter, then repeat Preflight.", group: "camera" },
  CAMERA_CAPTURE_ROOT_REQUIRED: { action: "No image root is set, so independent captures have nowhere to go. Choose the image storage root in the Camera panel.", group: "files" },
  STALE_LIVE_VIEW_SESSION: { action: "The Live View session changed (Stop, disconnect or a new Start). Re-read the Live View controls for the current session; the old result was discarded.", group: "camera" },
  LV_NOT_ACTIVE: { action: "No Live View endpoint is active. Start Live View, then read its controls.", group: "camera" },
  INVALID_CAMERA_FOCUS_STEP: { action: "The focus step was not an accepted integer step. Read the Live View controls and use the step values the lens reports as available.", group: "camera" },
  // G4 turntable bench/motion
  HOME_REQUIRES_XRAY_OFF_OR_ISOLATED_BENCH: { action: "HOME is refused because X-ray off was not confirmed. Confirm OFF by readback, or use the isolated turntable bench mode with the physical-isolation confirmation.", group: "turntable" },
  BENCH_MODE_XRAY_FORBIDDEN: { action: "Beam commands are prohibited in isolated bench mode. Leave bench mode and re-verify the X-ray session before enabling output.", group: "xray" },
  BENCH_CANNOT_OVERRIDE_XRAY_SESSION: { action: "This engine process already owned an X-ray session, so bench isolation cannot override it. Restart the engine with the source physically isolated before bench work.", group: "xray" },
  PHYSICAL_ISOLATION_CONFIRMATION_REQUIRED: { action: "Tick the physical-isolation confirmation before enabling bench mode. The X-ray state stays UNKNOWN and unmonitored.", group: "turntable" },
  INVALID_MOTION_ANGLE: { action: "Use a finite angle with at most 3 decimals: relative motion must be greater than 0 and at most 360°, phase motion at least 0 and less than 360°. Reverse motion is not offered.", group: "turntable" },
  NANO_NOT_CONNECTED: { action: "The turntable is not connected. Connect it in the Turntable panel, then repeat the operation.", group: "turntable" },
  // G5 x-ray
  XRAY_NOT_CONNECTED: { action: "The X-ray source is not connected. Connect it, confirm OFF by readback, then continue.", group: "xray" },
  XRAY_SETPOINT_CONFIRMATION_REQUIRED: { action: "The setpoints have not been confirmed in this session. Send voltage and current, confirm the readback, then continue.", group: "xray" },
  XRAY_BEAM_ACTIVE: { action: "Output is active. Turn X-ray off and wait for the confirmed readback before changing setpoints or running an OFF-requiring operation.", group: "xray" },
  XRAY_POWER_LIMIT: { action: "The voltage/current combination exceeds the 12 W limit. Lower kV or µA until the combination is within 12 W, then re-send.", group: "xray" },
  XRAY_ALREADY_CONNECTED: { action: "The source is already connected with a confirmed OFF. Continue with setpoints or Preflight.", group: "xray" },
  USB_AUTO_SHUTDOWN_RELEASE_REQUIRED: { action: "Preflight needs USB auto shut down to be released for this session. Release it in the X-ray panel, then re-run Preflight.", group: "xray" },
  XRAY_CONNECTION_FAILED: { action: "The Moxtek link could not be established. Check power, the USB link and the port, then retry the connection.", group: "xray" },
  XRAY_BEAM_ON_FAILED: { action: "Beam-on failed closed and the source is not emitting. Read X-ray status, confirm the interlock/temperature readings and the confirmed setpoints, then retry.", group: "xray" },
  XRAY_STAGE_LOCKED: { action: "The timer control is unavailable in production. It exists only in the offline preview; use the X-ray panel controls instead.", group: "xray" },
  // G6 scan preparation / recovery
  PREFLIGHT_REQUIRED: { action: "Preflight has not passed for the current settings. Run Task → Scan Preparation / Preflight, resolve every readiness item, then retry.", group: "readiness" },
  SCAN_PREPARATION_REQUIRED: { action: "No prepared geometry exists yet. Complete Scan Preparation, then run Preflight.", group: "readiness" },
  INVALID_SCAN_GEOMETRY: { action: "Geometry must be finite and positive, measured, counterclockwise, with a valid mirror flag. Correct the fields in Scan Preparation.", group: "readiness" },
  INVALID_SCINTILLATOR_STACK: { action: "Complete up to five scintillator layers in incident order, including material, emission colour and thickness in mm.", group: "readiness" },
  INVALID_RECONSTRUCTION_OPTIONS: { action: "Select monochrome or multicolor reconstruction. Material calibration requires multicolor mode; automatic colour processing currently uses FDK.", group: "readiness" },
  INVALID_SCAN_PREPARATION: { action: "The prepare payload did not match the required structure. Close and reopen Scan Preparation so the draft is rebuilt from the current contract.", group: "readiness" },
  SCAN_GEOMETRY_REQUIRED: { action: "The engine still needs confirmed geometry or a pending scan prompt is unanswered. Answer the prompt or complete Scan Preparation.", group: "readiness" },
  SCAN_OUTPUT_EXISTS_USE_NEW_TASK: { action: "The task directory already contains output. Choose a new task ID; existing files are never overwritten.", group: "files" },
  SCAN_OUTPUT_RETAINED: { action: "This task directory still contains retained data. Use a new task ID, or delete the entire old task directory and retry. A missing or unreadable record does not authorize overwriting existing files.", group: "files" },
  SCAN_OUTPUT_UNVERIFIED: { action: "The task directory contents could not be verified. Check the path and permissions on the image root, then retry.", group: "files" },
  SCAN_OUTPUT_EXISTS: { action: "The task directory already has content that is not a matching re-registration. Use a new task ID.", group: "files" },
  RESERVED_OR_INVALID_TASK_ID: { action: "This task ID is reserved or contains illegal characters (and may not end in a dot or space). Choose another ID.", group: "files" },
  RESERVED_TASK_DIRECTORY: { action: "The task directory resolves under the CameraCaptures namespace. Choose a different task ID.", group: "files" },
  CAPTURE_PROFILE_REQUIRED: { action: "No frozen capture profile is present. Connect the camera, apply and read back the required parameters, then repeat Preflight.", group: "readiness" },
  EXPOSURE_OUTSIDE_CAMERA_RANGE: { action: "The exposure is outside the camera's reported range. Choose 0.125–30000 ms for a D7100, or the range actually read back.", group: "readiness" },
  EXPOSURE_NOT_NATIVE_CAMERA_VALUE: { action: "The value does not match a native shutter position. Pick a shutter value offered by the descriptor.", group: "readiness" },
  EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY: { action: "Exposure cannot be edited through the compatibility entry. Change the native shutter in the Camera panel, Apply and verify, then repeat Preflight.", group: "camera" },
  RESTORE_PREFLIGHT_CONFIRMATION_REQUIRED: { action: "Restore Preflight needs both the restore flag and the explicit USB-auto-shutdown-release confirmation.", group: "files" },
  // G7/G8/G9
  RECONSTRUCTION_METHOD_UNAVAILABLE: { action: "The method is disabled (fewer than 3 projections, or SIRT exceeds the CPU budget). Use FDK or add projections.", group: "files" },
  RECONSTRUCTION_CACHE_INVALID: { action: "The cached result failed its version/hash check. Recompute from the archived projections; raw data is unchanged.", group: "files" },
  INVALID_RECONSTRUCTION_METHOD: { action: "Only fdk/sirt/cgls are accepted. Reload the reconstruction tab so the method list comes from the engine.", group: "files" },
  SOFTWARE_UPDATE_LIVE_VIEW_ACTIVE: { action: "Stop Live View before preparing a software update.", group: "files" },
  SOFTWARE_UPDATE_NOT_PREPARED: { action: "No maintenance preparation exists to cancel. Prepare first, then cancel.", group: "files" },
  TASK_COMPLETION_PREPARATION_FAILED: { action: "The completion plan could not be built because the task identity does not match. Verify the task directory and retained record.", group: "files" },
  CAPTURE_NOT_REGISTERED: { action: "This capture ID is not registered in the current engine lifetime. Re-open the photo from the current session; paths alone never re-authorise a preview.", group: "files" },
  OPERATION_CONTINUATION_CANCELLED: { action: "The operation receipt was cancelled or is not completed. Re-run the operation instead of continuing it.", group: "files" },
  OPERATION_RECEIPT_MISSING: { action: "The engine has no record of that operation ID (it may have been evicted). Repeat the operation.", group: "files" },
  START_ADMISSION_INVALIDATED: { action: "Parameters, Preflight or HOME changed after the request was queued. Re-run Scan Preparation and Preflight, then start again.", group: "readiness" },
  CAMERA_BACKEND_UNAVAILABLE: { action: "The native WPD camera backend could not be established. Check the USB link and the Windows camera service, then retry.", group: "camera" },
  CAMERA_CONNECTION_FAILED: { action: "The camera could not be connected. Check the USB link, then Discover and Connect again.", group: "camera" },
};

/** Every documented code gets an action; nothing here may claim a restart fixes an
 *  unknown or unsupported condition (docs/architecture/frontend-error-codes.md §0.2). */
export function failureAdvice(code: string | null): FailureAdvice | null {
  return code ? FAILURE_ADVICE[code] ?? null : null;
}
/** Decode only the engine's explicit camera-preflight rejection. Unknown or
 * malformed messages retain their original text; no permission is inferred. */
export function cameraPreflightFailure(message: string | null): { summary: string; action: string } | null {
  if (!message) return null;
  const marker = "camera safety check failed: ";
  const start = message.indexOf(marker);
  if (start < 0) return null;
  const jsonStart = message.indexOf("[", start + marker.length);
  if (jsonStart < 0) return null;
  try {
    // Preparation may append advice after the JSON; search for its exact close.
    let depth = 0; let quoted = false; let escaped = false; let end = -1;
    for (let index = jsonStart; index < message.length; index++) {
      const char = message[index];
      if (quoted) { if (escaped) escaped = false; else if (char === "\\") escaped = true; else if (char === '"') quoted = false; }
      else if (char === '"') quoted = true;
      else if (char === "[") depth++;
      else if (char === "]" && --depth === 0) { end = index + 1; break; }
    }
    if (end < 0) return null;
    const issues: CameraScanReadiness["issues"] = JSON.parse(message.slice(jsonStart, end));
    if (!Array.isArray(issues) || !issues.length || issues.length > 128 || issues.some(issue => !issue
      || typeof issue.key !== "string" || !issue.key || typeof issue.requirement !== "string"
      || typeof issue.remedy !== "string" || (issue.actual !== null && typeof issue.actual !== "string" && !Number.isSafeInteger(issue.actual)))) return null;
    return {
      summary: issues.map(issue => `${englishPropertyLabel(issue.key)}: ${issue.actual === null ? "UNKNOWN" : englishOptionLabel(issue.key, issue.actual, String(issue.actual))}. Required: ${englishDisplayText(issue.requirement)}.`).join("\n"),
      action: [...new Set(issues.map(issue => englishDisplayText(issue.remedy)))].join("\n"),
    };
  } catch { return null; }
}
export function displayFailureText(message: string): string {
  const camera = cameraPreflightFailure(message);
  return camera ? `${camera.summary}\n${camera.action}` : englishDisplayText(message);
}
export function failureNextAction(code: string | null, kind: FailureKind): string {
  const advice = failureAdvice(code);
  if (advice) return advice.action;
  if (kind === "cleanup_unconfirmed") return "Cleanup is not confirmed and the session stays isolated. This condition is not documented for this build: record the exact code and text, and do not treat the machine as cleaned up.";
  if (kind === "rejected") return "The camera refused the requested value while the session stayed healthy. Restore or change the value and Apply again in the same session.";
  if (code) return "This code is not documented in this build. Record the code and the exact engine text, keep the machine state as reported, and do not assume a restart resolves it.";
  if (kind !== "transport") return "Review the reported device requirements and status, correct the cause, then retry. Stop and status reads remain available.";
  return "The control service did not answer. Stop and status reads remain available; verify the engine process before retrying.";
}
export function failureGroup(code: string | null, kind: string | null): string {
  const advice = failureAdvice(code);
  if (advice) return advice.group;
  if (kind && /camera/.test(kind)) return "camera";
  return "readiness";
}

/** The newest terminal operation that failed. It is matched against
 *  `snapshot.operationId` (the engine projects its newest operation id there), so
 *  the bounded operation history can never re-surface an old failure after a
 *  reconnect; `latestAttempt` is the local attempt counter that re-arms the banner
 *  when the same message-producing command fails twice.
 *  A cancelled operation is NOT a failure by itself: Stop and safety cleanup cancel
 *  in-flight work deliberately. It is surfaced only when it carries an error, which
 *  is how a real late failure inside a cancelled operation stays visible. */
export function deriveFailureReport(
  snapshot: EngineSnapshot | null,
  engineError: string | null,
  actionError: string | null,
  latestAttempt: number,
): FailureReport | null {
  const operations = snapshot?.operations ?? [];
  const current = snapshot?.operationId
    ? operations.find(op => op.operationId === snapshot.operationId)
    : operations.reduce<(typeof operations)[number] | null>(
        (newest, op) => (!newest || op.updatedAt > newest.updatedAt ? op : newest), null);
  if (current) {
    const status = current.status;
    const failed = status === "cleanup_unconfirmed" || status === "failed" || status === "rejected"
      || (status === "cancelled" && Boolean(current.error));
    if (failed) {
      const kind: FailureKind = status === "cleanup_unconfirmed" || status === "failed" || status === "rejected"
        ? status : "failed";
      const code = parseEngineCode(current.error);
      const camera = kind === "cleanup_unconfirmed" ? null : cameraPreflightFailure(current.error);
      const folder = kind !== "cleanup_unconfirmed" && code?.startsWith("SCAN_OUTPUT_");
      return {
        key: `${kind}|${current.operationId}|${code ?? ""}`,
        kind,
        title: folder ? "Scan folder needs attention" : camera ? "Preflight needs attention" : kind === "cleanup_unconfirmed" ? "Cleanup is not confirmed" : kind === "rejected" ? "The camera rejected the requested value" : "The device operation failed",
        stage: englishDisplayText(`${current.kind} · ${status} · ${current.stage}`),
        reason: englishDisplayText(current.error) || `${current.kind} ended as ${status} without an error detail`,
        ...(camera ? { displayReason: camera.summary } : {}),
        code,
        nextAction: camera?.action || failureNextAction(code, kind),
        group: camera ? "camera" : failureGroup(code, current.kind),
      };
    }
  }
  const message = actionError || engineError;
  if (!message) return null;
  const code = parseEngineCode(message);
  const camera = cameraPreflightFailure(message);
  const kind: FailureKind = code || camera ? "engine" : "transport";
  return {
    key: `${kind}|${latestAttempt}|${code ?? ""}|${message.slice(0, 160)}`,
    kind,
    title: camera ? "Preflight needs attention" : code ? "The command was refused" : "The control service did not answer",
    stage: camera ? "preflight · camera settings" : code ? `engine_command · ${code}` : "engine_command · transport",
    reason: englishDisplayText(message),
    ...(camera ? { displayReason: camera.summary } : {}),
    code,
    nextAction: camera?.action || failureNextAction(code, kind),
    group: camera ? "camera" : failureGroup(code, null),
  };
}

/** Safety requests run immediately and invalidate unstarted ordinary requests. */
export class CommandScheduler {
  private queue: Promise<unknown> = Promise.resolve();
  private epoch = 0;
  submit<T>(run: () => Promise<T>, priority = false): Promise<T | undefined> {
    if (priority) { this.epoch++; this.queue = Promise.resolve(); return run(); }
    const epoch = this.epoch;
    const queued = this.queue.then(() => epoch === this.epoch ? run() : undefined);
    this.queue = queued.catch(() => undefined);
    return queued;
  }
}
