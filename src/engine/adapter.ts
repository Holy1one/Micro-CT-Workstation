import { TauriEngineAdapter } from "./tauriAdapter";
import type { EngineAdapter } from "./types";
import { WorkstationAdapter } from "./workstationAdapter";

function isTauriRuntime(): boolean {
  const runtimeWindow = window as Window & {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
  };
  return Boolean(runtimeWindow.__TAURI_INTERNALS__ ?? runtimeWindow.__TAURI__);
}

export function createEngineAdapter(): EngineAdapter {
  return isTauriRuntime() ? new TauriEngineAdapter() : new WorkstationAdapter();
}
