/**
 * Production frontend adapter for the two Tauri engine commands.
 * Every response must contain the complete workstation view. IPC failures are
 * surfaced to React and never trigger a fallback to the browser simulator.
 */

import { invoke } from "@tauri-apps/api/core";
import type { EngineAdapter, EngineCommand, EngineSnapshot } from "./types";

function assertSnapshot(snapshot: EngineSnapshot): EngineSnapshot {
  if (!snapshot.workstation) {
    throw new Error("Engine protocol error: workstation view is missing");
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
