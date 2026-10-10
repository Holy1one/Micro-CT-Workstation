/** Shared presentation drafts and correlated operation waiting. Device truth stays in snapshots. */
import { useEffect, useRef, useState } from "react";
import type { CameraProperties, CameraPreset, CameraValue, EngineCommand, EngineSnapshot } from "./engine/types";
import { cameraDraftChanges, confirmedCameraDraft, validateCameraDraft } from "./engine/deviceControls";

export type DeviceDispatch = (command: EngineCommand) => Promise<EngineSnapshot | undefined>;
const TERMINAL = new Set(["completed", "cancelled", "failed", "cleanup_unconfirmed", "rejected"]);
const SYNCHRONOUS = new Set(["prepare_scan", "set_capture_root", "camera_status", "camera_identity", "xray_status", "xray_identity", "turntable_identity", "preset_list", "preset_load", "preset_compare", "preset_save", "preset_delete", "preset_import", "preset_export"]);
const ASYNC_REQUIRED = new Set(["open_scan_project", "camera_connect", "camera_disconnect", "camera_read_properties", "camera_apply_properties", "camera_capture", "camera_test_capture", "camera_live_view_start", "camera_live_view_read", "camera_live_view_configure", "camera_autofocus", "camera_focus", "turntable_connect", "turntable_disconnect", "turntable_home", "turntable_relative", "turntable_to_angle", "preflight", "restore_previous"]);

export async function awaitOperation(ack: EngineSnapshot, operationId: string, current: () => EngineSnapshot | null, timeoutMs = 110_000): Promise<EngineSnapshot> {
  const deadline = performance.now() + timeoutMs;
  let snapshot = ack;
  while (true) {
    if (snapshot.engineSessionId !== ack.engineSessionId) throw new Error("Control session changed; old operation discarded");
    const operation = snapshot.operations.find(item => item.operationId === operationId);
    if (!operation) throw new Error("Operation receipt unavailable; completion is unconfirmed");
    if (TERMINAL.has(operation.status)) {
      if (operation.status !== "completed") throw new Error(operation.error || `${operation.kind}: ${operation.status}`);
      return snapshot;
    }
    if (performance.now() >= deadline) throw new Error("Operation completion remains unconfirmed; inspect status or use Stop");
    await new Promise<void>(resolve => setTimeout(resolve, 50));
    const next = current();
    if (!next) throw new Error("Control service unavailable; operation completion unknown");
    snapshot = next;
  }
}

export function useDeviceActions(snapshot: EngineSnapshot | null, dispatch: DeviceDispatch) {
  const current = useRef(snapshot); current.current = snapshot;
  const [pending, setPending] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // Monotonic attempt counter: the same failure text from a new attempt must re-arm the
  // failure banner, while the 400 ms snapshot poll must never re-open it.
  const [errorAttempt, setErrorAttempt] = useState(0);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; current.current = null; }; }, []);
  const run = async (command: EngineCommand): Promise<EngineSnapshot> => {
    const before = current.current;
    if (!before) throw new Error("Control service unavailable");
    const known = new Set(before.operations.map(operation => operation.operationId));
    setPending(value => value + 1); setError(null);
    try {
      const ack = await dispatch(command);
      if (!ack) throw new Error("Command rejected or discarded; inspect the engine error");
      if (ack.engineSessionId !== before.engineSessionId) throw new Error("Control session changed during the command");
      if (SYNCHRONOUS.has(command.type)) return ack;
      const operation = ack.operationId && !known.has(ack.operationId) ? ack.operations.find(item => item.operationId === ack.operationId) : null;
      if (operation) return await awaitOperation(ack, operation.operationId, () => current.current, command.type === "open_scan_project" ? 300_000 : 110_000);
      if (ASYNC_REQUIRED.has(command.type)) throw new Error("No new correlated operation receipt; completion is unconfirmed");
      return ack;
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      if (alive.current) { setError(message); setErrorAttempt(value => value + 1); }
      throw reason;
    } finally { if (alive.current) setPending(value => Math.max(0, value - 1)); }
  };
  return { run, pending: pending > 0, error, errorAttempt, clearError: () => setError(null) };
}

export function useCameraDraft(snapshot: EngineSnapshot | null) {
  const camera = snapshot?.deviceControls.camera;
  const scope = `${snapshot?.engineSessionId || "none"}/${camera?.sessionId || "none"}`;
  const [draft, setDraft] = useState<CameraProperties>({});
  const [presetId, setPresetId] = useState("");
  const initialized = useRef("");
  const mountedScope = useRef(scope); mountedScope.current = scope;
  useEffect(() => {
    if (initialized.current === scope) return;
    initialized.current = scope;
    setDraft(camera ? confirmedCameraDraft(camera.properties) : {}); setPresetId("");
  }, [scope]);
  const set = (key: string, value: CameraValue) => { setPresetId(""); setDraft(previous => ({ ...previous, [key]: value })); };
  const loadConfirmed = (result: EngineSnapshot) => {
    const selected = result.deviceControls.camera;
    if (`${result.engineSessionId}/${selected?.sessionId || "none"}` !== mountedScope.current) return;
    setDraft(selected ? confirmedCameraDraft(selected.properties) : {}); setPresetId("");
  };
  const loadPreset = (preset: CameraPreset) => {
    if (!camera?.identity || !preset.models.some(model => model.toLowerCase() === camera.identity!.model.toLowerCase())) throw new Error("Preset does not match this camera model");
    const invalid = validateCameraDraft(camera, preset.parameters, false);
    if (invalid) throw new Error(invalid);
    setDraft({ ...preset.parameters }); setPresetId(preset.id);
  };
  const changes = camera ? cameraDraftChanges(camera, draft) : {};
  const targets = presetId ? draft : changes;
  const issue = camera && Object.keys(draft).length ? validateCameraDraft(camera, draft, !presetId) : "Read confirmed camera parameters";
  return { draft, presetId, changes: targets, dirty: Object.keys(targets).length > 0, issue, set, loadConfirmed, loadPreset };
}
export type CameraDraftModel = ReturnType<typeof useCameraDraft>;
