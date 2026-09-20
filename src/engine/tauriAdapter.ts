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
