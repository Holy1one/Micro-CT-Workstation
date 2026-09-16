import type { EngineAdapter } from "./types";
import { WorkstationAdapter } from "./workstationAdapter";

export function createEngineAdapter(): EngineAdapter {
  // The RTS9060 link layer (scan workflow + device transports, ported from the
  // proven Python host in kernal/software) is the single device/task owner for
  // every runtime — browser dev server and the Tauri desktop shell alike.
  // The Rust ct-engine sidecar remains in place as the future bridge point for
  // physical serial / camera SDKs; its protocol and code are untouched.
  return new WorkstationAdapter();
}
