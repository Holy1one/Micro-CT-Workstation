/**
 * Production frontend adapter for the two Tauri engine commands.
 * Every response must contain the complete workstation view. IPC failures are
 * surfaced to React and never trigger a fallback to the browser simulator.
 */

import { invoke } from "@tauri-apps/api/core";
import type { EngineAdapter, EngineCommand, EngineSnapshot } from "./types";

function assertSnapshot(snapshot: EngineSnapshot): EngineSnapshot {
  if (!snapshot.engineSessionId || !Number.isSafeInteger(snapshot.snapshotSequence) || snapshot.snapshotSequence < 0
    || !(snapshot.operationId === null || typeof snapshot.operationId === "string") || !Array.isArray(snapshot.operations)
    || snapshot.operations.some(op => !op.operationId || !op.deviceSessionId || typeof op.cancelRequested !== "boolean"
      || typeof op.device !== "string" || typeof op.kind !== "string" || typeof op.stage !== "string"
      || typeof op.startedAt !== "string" || typeof op.updatedAt !== "string" || !(op.error === null || typeof op.error === "string")
      || !["queued", "running", "cancelling", "completed", "cancelled", "failed", "cleanup_unconfirmed"].includes(op.status))
    || !snapshot.deviceControls || typeof snapshot.deviceControls.taskLock?.locked !== "boolean"
    || !Array.isArray(snapshot.deviceControls.presets?.presets) || snapshot.deviceControls.presets.schemaVersion !== 1
    || typeof snapshot.deviceControls.turntable?.connected !== "boolean") {
    throw new Error("Engine protocol error: native control/session/operation evidence is missing");
  }
  const camera = snapshot.deviceControls.camera;
  if (camera && (typeof camera.connected !== "boolean" || !Array.isArray(camera.discovered)
    || !camera.capabilities || !camera.liveView || typeof camera.liveView.active !== "boolean"
    || typeof camera.liveView.fresh !== "boolean" || !camera.properties
    || [camera.isolated, camera.cleanupConfirmed, camera.busy, camera.capabilities.properties,
      camera.capabilities.hostCapture, camera.capabilities.liveView, camera.capabilities.autofocus,
      camera.capabilities.focusStep, camera.capabilities.scanNef, camera.capabilities.hardwareCertified].some(value => typeof value !== "boolean")
    || !(camera.sessionId === null || typeof camera.sessionId === "string")
    || !(camera.liveView.sessionId === null || typeof camera.liveView.sessionId === "string")
    || !Number.isSafeInteger(camera.liveView.sequence) || camera.liveView.sequence < 0
    || !camera.capabilities.disabledReasons
    || Object.values(camera.capabilities.disabledReasons).some(value => typeof value !== "string")
    || camera.discovered.some(identity => typeof identity.deviceId !== "string" || !identity.deviceId || typeof identity.model !== "string" || typeof identity.manufacturer !== "string")
    || Object.values(camera.properties).some(property => !Array.isArray(property.supportedValues)
      || typeof property.key !== "string" || typeof property.label !== "string"
      || [property.readable, property.writable, property.confirmed].some(value => typeof value !== "boolean")
      || !(property.error === null || typeof property.error === "string")
      || (property.range !== null && (!Array.isArray(property.range) || property.range.length !== 3 || !property.range.every(Number.isSafeInteger)))
      || [property.actual, property.target, ...property.supportedValues].some(value => value !== null
        && typeof value !== "string" && !Number.isSafeInteger(value))))) {
    throw new Error("Engine protocol error: native camera readback is malformed");
  }
  if (!snapshot.workstation) {
    throw new Error("Engine protocol error: workstation view is missing");
  }
  const flow = snapshot.workstation.scanFlow;
  const reconstruction = snapshot.workstation.reconstruction;
  if (!flow || !flow.references || typeof flow.stage !== "string"
      || !Number.isFinite(flow.references.preDark)
      || !Number.isFinite(flow.references.preFlat)
      || !Number.isFinite(flow.references.postFlat)
      || !Number.isFinite(flow.references.postDark)
      || !reconstruction || typeof reconstruction.unlocked !== "boolean"
      || !Array.isArray(reconstruction.methods)
      || !Number.isFinite(reconstruction.percent)
      || !["idle", "running", "completed", "failed"].includes(reconstruction.status)) {
    throw new Error("Engine protocol error: scan flow or reconstruction state is missing");
  }
  const xray = snapshot.workstation.xray;
  const scene = snapshot.workstation.scene;
  if (!scene || !Number.isFinite(scene.angleDeg) || typeof scene.angleKnown !== "boolean"
      || ![-1, 1].includes(scene.rotationDirection)) {
    throw new Error("Engine protocol error: confirmed turntable feedback is missing");
  }
  const exposure = snapshot.workstation.cameraExposure;
  if (!exposure || !Number.isFinite(exposure.minMs) || !Number.isFinite(exposure.maxMs)
      || exposure.minMs <= 0 || exposure.maxMs < exposure.minMs) {
    throw new Error("Engine protocol error: camera exposure limits are missing");
  }
  if (!xray || !["on", "off", "unknown"].includes(xray.beamState)
      || [xray.monKv, xray.monUa, xray.powerW, xray.tempC].some(
        (value) => value !== null && (typeof value !== "number" || !Number.isFinite(value)),
      )) {
    throw new Error("Engine protocol error: confirmed X-ray telemetry is missing");
  }
  return snapshot;
}

export class TauriEngineAdapter implements EngineAdapter {
  readonly kind = "tauri" as const;

  async getSnapshot(): Promise<EngineSnapshot> {
    return assertSnapshot(await invoke<EngineSnapshot>("engine_snapshot"));
  }

  async dispatch(command: EngineCommand): Promise<EngineSnapshot> {
    return assertSnapshot(await invoke<EngineSnapshot>("engine_command", { command }));
  }
}
