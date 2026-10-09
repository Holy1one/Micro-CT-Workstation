/**
 * React bridge to the selected EngineAdapter.
 * It owns only the latest immutable snapshot and request lifecycle; it never
 * predicts command success or advances production scan state locally.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { createEngineAdapter } from "./adapter";
import type { AdapterKind, EngineAdapter, EngineCommand, EngineSnapshot } from "./types";
import { CommandScheduler, SnapshotOrder } from "./deviceControls";

/**
 * Display-only evidence that a device session has been established in the
 * current ct-engine process. `false` is reserved for a positively identified
 * clean startup; callers must send `unknown` down their fail-closed path.
 */
export type DeviceSessionEverEstablished = boolean | "unknown";

type TrackedDevice = "nano" | "camera" | "xray";
type DeviceEvidence = Record<TrackedDevice, boolean>;

interface DeviceSessionStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

interface StoredDeviceSessionEvidence {
  version: 1;
  engineSessionId: string | null;
  devices: DeviceEvidence;
  lastLogSequence: number | null;
}

interface ParsedDeviceSessionSnapshot {
  pendingCommandAck: boolean;
  connectionState: "disconnected" | "connected" | "degraded" | "lost";
  phase: string;
  lastError: string | null;
  nanoState: string;
  cameraState: string;
  xrayState: string;
  xrayConnected: boolean;
  logs: Array<{ id: string; source: string; message: string }>;
  engineSessionId: string | null;
  maxLogSequence: number | null;
}

const DEVICE_SESSION_STORAGE_KEY = "micro-ct.device-session-evidence.v1";
const ENGINE_STARTUP_MESSAGE = "ct-engine started; waiting for an explicit connection";
const CONNECTED_DEVICE_STATES = new Set(["connected", "ready", "busy"]);
const CONNECTION_WORD = /\b(?:connect(?:ed|ion|ing)?|disconnect(?:ed|ion|ing)?|link verified|online)\b/i;
const SUCCESSFUL_CONNECTION_WORD = /\b(?:connected|auto-connected|link verified|connection established|connection verified)\b/i;
const FAILED_CONNECTION_WORD = /\b(?:not connected|disconnected|failed|failure|could not|unable to)\b/i;

function browserSessionStorage(): DeviceSessionStorage | null {
  try {
    const storage = globalThis.sessionStorage;
    return storage && typeof storage.getItem === "function" && typeof storage.setItem === "function"
      ? storage
      : null;
  } catch {
    return null;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isDeviceState(value: unknown): value is string {
  return typeof value === "string" && ["offline", "connected", "ready", "busy", "locked", "fault"].includes(value);
}

function parseSnapshot(snapshot: unknown): ParsedDeviceSessionSnapshot | null {
  if (!isRecord(snapshot)) return null;
  const connectionState = snapshot.connectionState;
  if (!["disconnected", "connected", "degraded", "lost"].includes(String(connectionState))) return null;
  if (typeof snapshot.phase !== "string" || !(snapshot.lastError === null || typeof snapshot.lastError === "string")) return null;
  if (!Array.isArray(snapshot.devices) || !Array.isArray(snapshot.logs)) return null;

  const deviceStates = new Map<string, string>();
  for (const device of snapshot.devices) {
    if (!isRecord(device) || typeof device.id !== "string" || !isDeviceState(device.state)) return null;
    if (["turntable", "camera", "xray"].includes(device.id)) {
      if (deviceStates.has(device.id)) return null;
      deviceStates.set(device.id, device.state);
    }
  }
  const nanoState = deviceStates.get("turntable");
  const cameraState = deviceStates.get("camera");
  const xrayState = deviceStates.get("xray");
  if (!nanoState || !cameraState || !xrayState) return null;

  if (!isRecord(snapshot.workstation) || !isRecord(snapshot.workstation.xray)
      || typeof snapshot.workstation.xray.connected !== "boolean") return null;

  const logs: ParsedDeviceSessionSnapshot["logs"] = [];
  for (const entry of snapshot.logs) {
    if (!isRecord(entry) || typeof entry.id !== "string" || typeof entry.source !== "string"
        || typeof entry.message !== "string") return null;
    logs.push({ id: entry.id, source: entry.source, message: entry.message });
  }

  const startupLogs = logs.filter((entry) => entry.message.trim() === ENGINE_STARTUP_MESSAGE);
  if (startupLogs.length > 1 || startupLogs.some((entry) => entry.id.length === 0)) return null;
  const startupLog = startupLogs[0];
  const sequences = logs
    .map((entry) => /^(\d+)-/.exec(entry.id))
    .filter((match): match is RegExpExecArray => match !== null)
    .map((match) => Number(match[1]))
    .filter((sequence) => Number.isSafeInteger(sequence));

  return {
    pendingCommandAck: typeof snapshot.operationId === "string" && Array.isArray(snapshot.operations)
      && snapshot.operations.some(op => isRecord(op) && op.operationId === snapshot.operationId && op.status !== "completed"),
    connectionState: connectionState as ParsedDeviceSessionSnapshot["connectionState"],
    phase: snapshot.phase,
    lastError: snapshot.lastError,
    nanoState,
    cameraState,
    xrayState,
    xrayConnected: snapshot.workstation.xray.connected,
    logs,
    engineSessionId: typeof snapshot.engineSessionId === "string" && snapshot.engineSessionId.length > 0 ? snapshot.engineSessionId : startupLog?.id ?? null,
    maxLogSequence: sequences.length ? Math.max(...sequences) : null,
  };
}

function isConnectedState(state: string): boolean {
  return CONNECTED_DEVICE_STATES.has(state);
}

function deviceLogEvidence(logs: ParsedDeviceSessionSnapshot["logs"]): {
  devices: DeviceEvidence;
  anyConnectionEvidence: boolean;
} {
  const devices: DeviceEvidence = { nano: false, camera: false, xray: false };
  let anyConnectionEvidence = false;

  for (const entry of logs) {
    if (entry.message.trim() === ENGINE_STARTUP_MESSAGE) continue;
    if (!CONNECTION_WORD.test(entry.message)) continue;
    anyConnectionEvidence = true;
    if (!SUCCESSFUL_CONNECTION_WORD.test(entry.message) || FAILED_CONNECTION_WORD.test(entry.message)) continue;

    const source = `${entry.source} ${entry.message}`.toLowerCase();
    if (/\b(?:nano|turntable|ch340)\b/.test(source)) devices.nano = true;
    else if (/\b(?:camera|nikon|d7100)\b/.test(source)) devices.camera = true;
    else if (/\b(?:xray|x-ray|moxtek)\b/.test(source)) devices.xray = true;
    else if (/developer preview connected/i.test(entry.message)) devices.nano = true;
  }

  return { devices, anyConnectionEvidence };
}

function snapshotDeviceEvidence(snapshot: ParsedDeviceSessionSnapshot): DeviceEvidence | null {
  const nanoIsConnected = isConnectedState(snapshot.nanoState);
  const xrayDeviceIsConnected = isConnectedState(snapshot.xrayState);
  const nanoLinkIsConnected = snapshot.connectionState === "connected";

  // These fields describe the same live links. A disagreement is not safe to
  // reinterpret as a clean disconnect or as proof that a link is still up.
  if (nanoIsConnected !== nanoLinkIsConnected || xrayDeviceIsConnected !== snapshot.xrayConnected) return null;

  return {
    nano: snapshot.connectionState !== "disconnected",
    camera: isConnectedState(snapshot.cameraState),
    xray: snapshot.xrayConnected,
  };
}

function commandDeviceEvidence(
  command: EngineCommand | undefined,
  snapshot: ParsedDeviceSessionSnapshot,
  current: DeviceEvidence,
): DeviceEvidence | null {
  const result: DeviceEvidence = { nano: false, camera: false, xray: false };
  if (!command) return result;
  // The command response acknowledges enqueue/cancellation. Device state and
  // a completed operation may provide evidence later; an ACK alone cannot.
  if (snapshot.pendingCommandAck) return result;

  switch (command.type) {
    case "connect":
      if (!current.nano) return null;
      result.nano = true;
      break;
    case "retry_device":
      if (!current[command.device === "turntable" ? "nano" : command.device]) return null;
      result[command.device === "turntable" ? "nano" : command.device] = true;
      break;
    case "preflight":
      if (!current.nano || !current.camera || !current.xray) return null;
      result.camera = true;
      result.xray = true;
      break;
    case "camera_test_capture":
      if (!current.camera) return null;
      result.camera = true;
      break;
    case "camera_connect":
    case "camera_capture":
    case "camera_read_properties":
    case "camera_apply_properties":
      if (current.camera) result.camera = true;
      break;
    case "turntable_connect":
    case "turntable_home":
      if (current.nano) result.nano = true;
      break;
    case "xray_connect":
      if (current.xray) result.xray = true;
      break;
    case "xray_disconnect":
      // In production this command can succeed only after an X-ray link was
      // established and the OFF state was confirmed.
      result.xray = true;
      break;
    default:
      break;
  }

  return result;
}

function emptyDeviceEvidence(): DeviceEvidence {
  return { nano: false, camera: false, xray: false };
}

function anyDeviceEvidence(evidence: DeviceEvidence): boolean {
  return evidence.nano || evidence.camera || evidence.xray;
}

function parseStoredEvidence(value: string | null): StoredDeviceSessionEvidence | null | "invalid" {
  if (value === null) return null;
  try {
    const parsed: unknown = JSON.parse(value);
    if (!isRecord(parsed) || parsed.version !== 1
        || !(parsed.engineSessionId === null || (typeof parsed.engineSessionId === "string" && parsed.engineSessionId.length > 0))
        || !isRecord(parsed.devices)
        || typeof parsed.devices.nano !== "boolean"
        || typeof parsed.devices.camera !== "boolean"
        || typeof parsed.devices.xray !== "boolean"
        || !(parsed.lastLogSequence === null || (typeof parsed.lastLogSequence === "number"
          && Number.isSafeInteger(parsed.lastLogSequence) && parsed.lastLogSequence >= 0))) return "invalid";
    return parsed as unknown as StoredDeviceSessionEvidence;
  } catch {
    return "invalid";
  }
}

/**
 * Tracks per-device connection observations without relying on retained log
 * history. A changed startup-log ID identifies a new engine process. If the
 * startup line has been evicted, a positive stored record remains monotonic;
 * a stored negative is never reused without seeing that startup line again.
 */
export class DeviceSessionEvidenceTracker {
  private value: DeviceSessionEverEstablished = "unknown";
  private storageUnavailable: boolean;

  constructor(private readonly storage: DeviceSessionStorage | null = browserSessionStorage()) {
    this.storageUnavailable = storage === null;
  }

  hasEverEstablished(): DeviceSessionEverEstablished {
    if (this.storageUnavailable || !this.storage || this.value === "unknown") return "unknown";
    try {
      const stored = parseStoredEvidence(this.storage.getItem(DEVICE_SESSION_STORAGE_KEY));
      return stored === null || stored === "invalid" ? "unknown" : this.value;
    } catch {
      this.storageUnavailable = true;
      return "unknown";
    }
  }

  observeSnapshot(snapshotValue: unknown, successfulCommand?: EngineCommand): DeviceSessionEverEstablished {
    if (this.storageUnavailable || !this.storage) return (this.value = "unknown");
    const snapshot = parseSnapshot(snapshotValue);
    if (!snapshot) return (this.value = "unknown");

    const currentEvidence = snapshotDeviceEvidence(snapshot);
    if (!currentEvidence) return (this.value = "unknown");
    const commandEvidence = commandDeviceEvidence(successfulCommand, snapshot, currentEvidence);
    if (!commandEvidence) return (this.value = "unknown");
    const logEvidence = deviceLogEvidence(snapshot.logs);

    let stored: StoredDeviceSessionEvidence | null | "invalid";
    try {
      stored = parseStoredEvidence(this.storage.getItem(DEVICE_SESSION_STORAGE_KEY));
    } catch {
      this.storageUnavailable = true;
      return (this.value = "unknown");
    }
    if (stored === "invalid") return (this.value = "unknown");

    const newEngineSession = snapshot.engineSessionId !== null
      && stored !== null
      && stored.engineSessionId !== snapshot.engineSessionId;
    const retainedStartupContradictsStoredPositive = snapshot.engineSessionId !== null
      && stored !== null
      && stored.engineSessionId === snapshot.engineSessionId
      && anyDeviceEvidence(stored.devices)
      && !logEvidence.anyConnectionEvidence
      && !anyDeviceEvidence(currentEvidence)
      && !anyDeviceEvidence(commandEvidence);
    if (retainedStartupContradictsStoredPositive) return (this.value = "unknown");

    const base = newEngineSession || stored === null ? emptyDeviceEvidence() : { ...stored.devices };

    if (stored !== null && snapshot.engineSessionId === null
        && snapshot.maxLogSequence !== null && stored.lastLogSequence !== null
        && snapshot.maxLogSequence < stored.lastLogSequence) {
      // A sequence rollback without the startup marker suggests an engine
      // restart, but cannot license `false`: the startup evidence is absent.
      return (this.value = "unknown");
    }

    const devices: DeviceEvidence = {
      nano: base.nano || currentEvidence.nano || logEvidence.devices.nano || commandEvidence.nano,
      camera: base.camera || currentEvidence.camera || logEvidence.devices.camera || commandEvidence.camera,
      xray: base.xray || currentEvidence.xray || logEvidence.devices.xray || commandEvidence.xray,
    };

    const cleanStartup = snapshot.engineSessionId !== null
      && !logEvidence.anyConnectionEvidence
      && !anyDeviceEvidence(currentEvidence)
      && !anyDeviceEvidence(commandEvidence)
      && snapshot.connectionState === "disconnected"
      && snapshot.nanoState === "offline"
      && snapshot.cameraState === "offline"
      && ["offline", "locked"].includes(snapshot.xrayState)
      && !snapshot.xrayConnected
      && snapshot.phase === "idle"
      && snapshot.lastError === null;

    if (!anyDeviceEvidence(devices) && !cleanStartup) return (this.value = "unknown");
    if (!anyDeviceEvidence(devices) && logEvidence.anyConnectionEvidence) return (this.value = "unknown");

    const previousLogSequence = newEngineSession ? null : stored?.lastLogSequence ?? null;
    const maxLogSequence = snapshot.maxLogSequence === null
      ? previousLogSequence
      : Math.max(snapshot.maxLogSequence, previousLogSequence ?? snapshot.maxLogSequence);
    const next: StoredDeviceSessionEvidence = {
      version: 1,
      engineSessionId: snapshot.engineSessionId ?? stored?.engineSessionId ?? null,
      devices,
      lastLogSequence: maxLogSequence,
    };
    try {
      this.storage.setItem(DEVICE_SESSION_STORAGE_KEY, JSON.stringify(next));
    } catch {
      this.storageUnavailable = true;
      return (this.value = "unknown");
    }

    return (this.value = anyDeviceEvidence(devices) ? true : false);
  }
}

const deviceSessionEvidence = new DeviceSessionEvidenceTracker();

/** Returns `false` only after an identified clean engine startup; unknown must fail closed. */
export function hasDeviceSessionEverEstablished(): DeviceSessionEverEstablished {
  return deviceSessionEvidence.hasEverEstablished();
}

function observeDeviceSessionSnapshot(snapshot: EngineSnapshot, command?: EngineCommand): void {
  deviceSessionEvidence.observeSnapshot(snapshot, command);
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

export function useEngine() {
  const adapterRef = useRef<EngineAdapter | null>(null);
  const orderRef = useRef(new SnapshotOrder());
  const commandScheduler = useRef(new CommandScheduler());
  const commandCountRef = useRef(0);
  const generationRef = useRef(0);
  const refreshActiveRef = useRef(false);
  const [adapterKind, setAdapterKind] = useState<AdapterKind>("developer_preview");
  const [snapshot, setSnapshot] = useState<EngineSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [transportError, setTransportError] = useState<string | null>(null);

  const commitSnapshot = useCallback((generation: number, next: EngineSnapshot, successfulCommand?: EngineCommand): void => {
    if (generation === generationRef.current && orderRef.current.accept(next)) {
      observeDeviceSessionSnapshot(next, successfulCommand);
      setSnapshot(next);
      setTransportError(null);
    }
  }, []);

  const refresh = useCallback(async (): Promise<void> => {
    const adapter = adapterRef.current;
    if (!adapter || refreshActiveRef.current) return;
    refreshActiveRef.current = true;
    const generation = generationRef.current;
    try { commitSnapshot(generation, await adapter.getSnapshot()); }
    catch (reason) {
      if (generation === generationRef.current) {
        setError(errorMessage(reason)); setTransportError(errorMessage(reason));
      }
    } finally { refreshActiveRef.current = false; }
  }, [commitSnapshot]);

  useEffect(() => {
    const adapter = createEngineAdapter();
    adapterRef.current = adapter;
    orderRef.current = new SnapshotOrder();
    generationRef.current++;
    setAdapterKind(adapter.kind);
    void refresh();
    // Poll independently of command dispatch and hardware operation lifetime.
    const interval = window.setInterval(() => void refresh(), 400);
    return () => {
      generationRef.current++;
      window.clearInterval(interval); adapter.close?.(); adapterRef.current = null;
    };
  }, [refresh]);

  const dispatch = useCallback(async (command: EngineCommand): Promise<EngineSnapshot | undefined> => {
    const generation = generationRef.current;
    const run = async (): Promise<EngineSnapshot | undefined> => {
      const adapter = adapterRef.current;
      if (!adapter || generation !== generationRef.current) return;
      commandCountRef.current++; setBusy(true);
      try {
        const response = await adapter.dispatch(command);
        commitSnapshot(generation, response, command);
        if (generation === generationRef.current) setError(null);
        return response;
      } catch (reason) {
        if (generation === generationRef.current) {
          const message = errorMessage(reason); setError(message);
          if (!message.startsWith("ENGINE:")) setTransportError(message);
        }
        return undefined;
      } finally {
        commandCountRef.current--;
        if (generation === generationRef.current) setBusy(commandCountRef.current > 0);
      }
    };
    // Safety commands bypass pending ordinary calls. A queued ordinary request
    // admitted before Stop is discarded; the engine separately invalidates workers.
    if (command.type === "stop" || command.type === "xray_off" || command.type === "camera_live_view_stop"
      || (command.type === "xray_toggle" && snapshot?.workstation?.xray.beamState !== "off")) {
      return commandScheduler.current.submit(run, true);
    }
    return commandScheduler.current.submit(run);
  }, [commitSnapshot, snapshot?.workstation?.xray.beamState]);
  return { adapterKind, snapshot, busy, error, transportError, refresh, dispatch };
}
