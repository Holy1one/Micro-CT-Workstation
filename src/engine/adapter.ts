/**
 * Runtime adapter selection boundary.
 * Tauri always selects the production IPC adapter; a normal browser selects
 * the isolated preview. Selection occurs once and does not change on errors.
 */

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
