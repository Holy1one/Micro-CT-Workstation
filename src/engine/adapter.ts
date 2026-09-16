import { invoke } from "@tauri-apps/api/core";
import { DevPreviewAdapter } from "./devAdapter";
import type { EngineAdapter, EngineCommand, EngineSnapshot } from "./types";

class TauriEngineAdapter implements EngineAdapter {
  readonly kind = "tauri" as const;

  getSnapshot(): Promise<EngineSnapshot> {
    return invoke<EngineSnapshot>("engine_snapshot");
  }

  dispatch(command: EngineCommand): Promise<EngineSnapshot> {
    return invoke<EngineSnapshot>("engine_command", { command });
  }
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
}

export function createEngineAdapter(): EngineAdapter {
  return isTauriRuntime() ? new TauriEngineAdapter() : new DevPreviewAdapter();
}

