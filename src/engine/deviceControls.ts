/** Pure presentation rules for native device evidence, drafts and ordering.
 * No helper accesses a device or grants engine permission; unknown stays blocked. */
import type { CameraHealth, CameraProperties, CameraProperty, CameraValue, EngineSnapshot } from "./types";

export function deviceConnectionReason(snapshot: EngineSnapshot, stale = false): string | null {
  if (stale) return "Control service unavailable · device evidence unknown";
  if (["running", "paused", "finishing", "stopping"].includes(snapshot.phase)) return "Scan or safety cleanup owns the devices";
  if (snapshot.workstation?.reconstruction.status === "running") return "Reconstruction owns the task";
  if (snapshot.operations.some(op => ["queued", "running", "cancelling", "cleanup_unconfirmed"].includes(op.status))) return "Operation pending or cleanup unconfirmed";
  return null;
}
export function manualControlReason(snapshot: EngineSnapshot, stale = false): string | null {
  const occupied = deviceConnectionReason(snapshot, stale);
  if (occupied) return occupied;
  if (snapshot.deviceControls.taskLock.locked) return snapshot.deviceControls.taskLock.reason || "Unfinished task · abandon it before manual control";
  return null;
}
export function cameraControlReason(camera: CameraHealth | null): string | null {
  if (!camera?.connected) return camera?.lastError || "Select and connect a camera";
  if (camera.isolated || !camera.cleanupConfirmed) return "Camera cleanup unconfirmed · session isolated";
  if (camera.busy) return "Camera operation is running";
  return null;
}
export function shutterExposureMs(native: CameraValue): number | null {
  if (typeof native !== "number" || !Number.isSafeInteger(native) || native <= 0 || native > 0xffffffff) return null;
  const numerator = Math.floor(native / 65536); const denominator = native % 65536;
  return numerator > 0 && denominator > 0 ? numerator * 1000 / denominator : null;
}
export function nativePropertyLabel(key: string, value: CameraValue | null): string {
  if (value === null) return "UNKNOWN";
  if (key === "shutter") {
    const ms = shutterExposureMs(value);
    if (ms !== null && typeof value === "number") return `${Math.floor(value / 65536)}/${value % 65536} s · ${ms} ms`;
  }
  if (key === "aperture" && typeof value === "number") return `f/${value / 100}`;
  return String(value);
}
export function validateCameraDraft(camera: CameraHealth, draft: CameraProperties): string | null {
  if (!Object.keys(draft).length) return "No camera parameter draft";
  for (const [key, value] of Object.entries(draft)) {
    const property = camera.properties[key];
    if (!property?.writable || !property.readable) return `${key}: readable/writable capability unavailable`;
    if (typeof value === "number" && !Number.isSafeInteger(value)) return `${key}: native value must be an integer`;
    if (property.supportedValues.length && !property.supportedValues.includes(value)) return `${key}: native value is not supported`;
    if (property.range) {
      const [min, max, step] = property.range;
      if (typeof value !== "number" || value < min || value > max || (step > 0 && (value - min) % step !== 0)) return `${key}: native value is outside described range`;
    }
    if (key === "shutter" && shutterExposureMs(value) === null) return "Shutter is not a timed native rational value";
  }
  return null;
}
export function confirmedCameraDraft(properties: Record<string, CameraProperty>): CameraProperties {
  return Object.fromEntries(Object.values(properties).filter(p => p.writable && p.readable && p.confirmed && p.actual !== null).map(p => [p.key, p.actual!]));
}
export function exactShutterForExposure(camera: CameraHealth, exposureMs: number): CameraValue | null {
  return camera.properties.shutter?.supportedValues.find(value => shutterExposureMs(value) === exposureMs) ?? null;
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
