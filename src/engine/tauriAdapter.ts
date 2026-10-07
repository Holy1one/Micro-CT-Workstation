/**
 * Production frontend adapter for the two Tauri engine commands.
 * Every response must contain the complete workstation view. IPC failures are
 * surfaced to React and never trigger a fallback to the browser simulator.
 */

import { invoke } from "@tauri-apps/api/core";
import type { CameraProperty, EngineAdapter, EngineCommand, EngineSnapshot } from "./types";

function validLiveControl(property: CameraProperty, key: string): boolean {
  return property?.key === key && property.code === (key === "cameraZoom" ? 0xd1a3 : 0xd061)
    && typeof property.label === "string" && Number.isSafeInteger(property.dataType)
    && [property.readable, property.writable, property.confirmed, property.supported, property.nativeWritable, property.effective].every(value => typeof value === "boolean")
    && (property.disabledReason === null || typeof property.disabledReason === "string")
    && (property.error === null || typeof property.error === "string")
    && Array.isArray(property.supportedValues) && Array.isArray(property.options) && property.options.length <= 256
    && [property.actual, property.target, ...property.supportedValues].every(value => value === null || Number.isInteger(value) && Number(value) >= 0 && Number(value) <= 255)
    && (property.range === null || Array.isArray(property.range) && property.range.length === 3 && property.range.every(value => Number.isInteger(value) && value >= 0 && value <= 255))
    && property.options.every(option => typeof option.label === "string" && typeof option.supported === "boolean" && typeof option.enabled === "boolean"
      && (option.reason === null || typeof option.reason === "string") && Number.isInteger(option.value) && Number(option.value) >= 0 && Number(option.value) <= 255
      && (!option.enabled || option.supported && property.writable))
    && (!property.writable || property.supported && property.readable && property.nativeWritable && property.confirmed && property.effective && property.actual !== null && !property.error)
    && (!property.supported || property.dataType === 2);
}

function assertSnapshot(snapshot: EngineSnapshot): EngineSnapshot {
  if (!snapshot.engineSessionId || !Number.isSafeInteger(snapshot.snapshotSequence) || snapshot.snapshotSequence < 0
    || !(snapshot.operationId === null || typeof snapshot.operationId === "string") || !Array.isArray(snapshot.operations)
    || snapshot.operations.some(op => !op.operationId || !op.deviceSessionId || typeof op.cancelRequested !== "boolean"
      || typeof op.device !== "string" || typeof op.kind !== "string" || typeof op.stage !== "string"
      || typeof op.startedAt !== "string" || typeof op.updatedAt !== "string" || !(op.error === null || typeof op.error === "string")
      || !["queued", "running", "cancelling", "completed", "cancelled", "failed", "cleanup_unconfirmed", "rejected"].includes(op.status))
    || !snapshot.deviceControls || typeof snapshot.deviceControls.taskLock?.locked !== "boolean"
    || !Array.isArray(snapshot.deviceControls.presets?.presets) || snapshot.deviceControls.presets.schemaVersion !== 1
    || typeof snapshot.deviceControls.turntable?.connected !== "boolean") {
    throw new Error("Engine protocol error: native control/session/operation evidence is missing");
  }
  const camera = snapshot.deviceControls.camera;
  const liveControls = camera?.liveViewControls;
  if (camera && (!liveControls || !liveControls.properties || Array.isArray(liveControls.properties)
      || !(liveControls.sessionId === null || typeof liveControls.sessionId === "string" && /^[a-f0-9]{32}$/.test(liveControls.sessionId))
      || (liveControls.sessionId !== null && (!camera.connected || camera.isolated || !camera.liveView?.active || liveControls.sessionId !== camera.liveView.sessionId))
      || Object.entries(liveControls.properties).some(([key, property]) => !["cameraZoom", "focusMode"].includes(key) || !validLiveControl(property, key))
      || Object.values(liveControls.properties).some(property => property.writable && liveControls.sessionId === null))) {
    throw new Error("Engine protocol error: Live View controls lack current native session/descriptor evidence");
  }
  const multipleExposure = camera?.multipleExposureEvidence;
  if (camera && (!multipleExposure
      || typeof multipleExposure.physicalApplicable !== "boolean" || typeof multipleExposure.observable !== "boolean"
      || !["unknown", "off", "on", "not_applicable"].includes(multipleExposure.status)
      || !["none", "native_property", "documented_transport_exclusion"].includes(multipleExposure.source)
      || !(multipleExposure.reason === null || typeof multipleExposure.reason === "string")
      || (multipleExposure.status === "not_applicable" && multipleExposure.physicalApplicable)
      || (multipleExposure.status === "unknown" && multipleExposure.source !== "none")
      || (["off", "on"].includes(multipleExposure.status)
        && (!multipleExposure.physicalApplicable || !multipleExposure.observable || multipleExposure.source === "none")))) {
    throw new Error("Engine protocol error: physical multiple-exposure evidence is malformed or simulated");
  }
  // The operator attestation is a separate contract from the UNKNOWN machine
  // state: a "confirmed" record must be current, fingerprinted and session-bound.
  const operator = camera?.multipleExposureOperator;
  if (camera && operator && (
    typeof operator.attested !== "boolean" || typeof operator.valid !== "boolean"
    || !["none", "confirmed", "stale"].includes(operator.state)
    || typeof operator.machineObservable !== "boolean"
    || !["unknown", "off", "on", "not_applicable"].includes(operator.machineStatus)
    || (operator.state === "confirmed" && (!operator.valid || !operator.attested || operator.policyVersion !== 1
      || typeof operator.evidenceFingerprint !== "string" || !/^[a-f0-9]{64}$/.test(operator.evidenceFingerprint)
      || typeof operator.sessionId !== "string" || operator.sessionId.length !== 32
      || typeof operator.confirmedAt !== "string" || typeof operator.operator !== "string" || !operator.operator))
    || (operator.state === "none" && (operator.attested || operator.valid))
    || (operator.state === "stale" && (!operator.attested || operator.valid)))) {
    throw new Error("Engine protocol error: operator multiple-exposure attestation is malformed or not session-bound");
  }
  const validCapture = (value: NonNullable<EngineSnapshot["cameraCaptureDescriptor"]>) =>
    typeof value.captureId === "string" && value.captureId.length > 0
    && typeof value.sessionId === "string" && value.sessionId.length > 0
    && typeof value.operationId === "string" && value.operationId.length > 0
    && typeof value.path === "string" && value.path.length > 0
    && Number.isSafeInteger(value.bytes) && value.bytes > 0 && value.bytes <= 128 * 1024 * 1024
    && typeof value.sha256 === "string" && /^[a-f0-9]{64}$/.test(value.sha256)
    && value.format === "NEF" && value.exposureEnded === true;
  if ((camera?.lastCapturedImage && !validCapture(camera.lastCapturedImage))
      || (snapshot.cameraCaptureDescriptor && !validCapture(snapshot.cameraCaptureDescriptor))) {
    throw new Error("Engine protocol error: independent capture registration is malformed");
  }
  const readiness = snapshot.cameraScanReadiness;
  if (readiness?.ready && (!multipleExposure || multipleExposure.status !== "off"
      || !multipleExposure.physicalApplicable || !multipleExposure.observable || multipleExposure.source === "none")) {
    throw new Error("Engine protocol error: fixed readiness lacks physical multiple-exposure evidence");
  }
  if (readiness && (typeof readiness.ready !== "boolean" || !Array.isArray(readiness.issues)
      || readiness.issues.length > 128 || readiness.issues.some(issue => !issue.key
        || typeof issue.requirement !== "string" || typeof issue.remedy !== "string"
        || (issue.actual !== null && typeof issue.actual !== "string" && !Number.isSafeInteger(issue.actual)))
      || readiness.ready !== (readiness.issues.length === 0))) {
    throw new Error("Engine protocol error: fixed scan readiness is malformed");
  }
  const maintenance = snapshot.softwareUpdatePreparation;
  const evidence = ["confirmed", "never-owned", "unknown"];
  if (maintenance && (maintenance.sessionId !== snapshot.engineSessionId || !maintenance.operationId
      || !["preparing", "ready", "blocked", "cancelled"].includes(maintenance.status)
      || [maintenance.ready, maintenance.maintenanceLocked, maintenance.noActiveTransactions].some(value => typeof value !== "boolean")
      || typeof maintenance.createdAt !== "string" || !(maintenance.completedAt === null || typeof maintenance.completedAt === "string")
      || [maintenance.cameraCleanup, maintenance.turntableStop, maintenance.xrayOff].some(value => !evidence.includes(value))
      || !(maintenance.error === null || typeof maintenance.error === "string")
      || maintenance.ready !== (maintenance.status === "ready")
      || (maintenance.ready && (!maintenance.maintenanceLocked || !maintenance.noActiveTransactions || maintenance.error !== null
        || maintenance.completedAt === null || [maintenance.cameraCleanup, maintenance.turntableStop, maintenance.xrayOff].includes("unknown"))))) {
    throw new Error("Engine protocol error: software update authority is malformed");
  }
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
      || [property.readable, property.writable, property.confirmed, property.supported, property.nativeWritable, property.effective].some(value => typeof value !== "boolean")
      || !(property.disabledReason === null || typeof property.disabledReason === "string")
      || !Array.isArray(property.options) || property.options.length > 4096
      || property.options.some(option => typeof option.label !== "string"
        || [option.supported, option.enabled].some(value => typeof value !== "boolean")
        || !(option.reason === null || typeof option.reason === "string")
        || (typeof option.value !== "string" && !Number.isSafeInteger(option.value))
        || (option.enabled && !option.supported))
      || !(property.error === null || typeof property.error === "string")
      || (property.range !== null && (!Array.isArray(property.range) || property.range.length !== 3 || !property.range.every(Number.isSafeInteger)))
      || [property.actual, property.target, ...property.supportedValues].some(value => value !== null
        && typeof value !== "string" && !Number.isSafeInteger(value))))) {
    throw new Error("Engine protocol error: native camera readback is malformed");
  }
  const info = camera?.deviceInfo;
  if (info && ([info.standardVersion, info.vendorExtensionId, info.vendorExtensionVersion,
    info.functionalMode, info.datasetSize].some(value => !Number.isSafeInteger(value) || value < 0)
    || [info.vendorExtensionDescription, info.deviceVersion, info.adapterSelection].some(value => typeof value !== "string")
    || typeof info.datasetSha256 !== "string" || !/^[a-f0-9]{64}$/.test(info.datasetSha256)
    || [info.operations, info.events, info.properties, info.captureFormats, info.imageFormats].some(values =>
      !Array.isArray(values) || values.some(value => !Number.isSafeInteger(value) || value < 0 || value > 0xffff)))) {
    throw new Error("Engine protocol error: camera device-info evidence is malformed");
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
