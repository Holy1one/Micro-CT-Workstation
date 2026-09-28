/** Contract tests for the real frontend session-evidence implementation. */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { transformWithEsbuild } from "vite";

const ROOT = new URL("../", import.meta.url);
const STORAGE_KEY = "micro-ct.device-session-evidence.v1";
const STARTUP_MESSAGE = "ct-engine started; waiting for an explicit connection";

class MemoryStorage {
  values = new Map();

  getItem(key) {
    return this.values.get(key) ?? null;
  }

  setItem(key, value) {
    this.values.set(key, value);
  }
}

async function loadTracker() {
  const source = await readFile(new URL("src/engine/useEngine.ts", ROOT), "utf8");
  const start = source.indexOf("export type DeviceSessionEverEstablished");
  const end = source.indexOf("function errorMessage(reason: unknown): string", start);
  assert.ok(start >= 0 && end > start, "session evidence declarations moved or were removed");

  // Load the real implementation without importing the React hook or adapter.
  const implementation = source.slice(start, end);
  const { code } = await transformWithEsbuild(implementation, "engine-session-evidence.ts", { loader: "ts" });
  return import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
}

async function loadProductionFirstLaunchSnapshot() {
  const path = new URL("tmp/tests/frontend/20260927-portable-feedback/first-launch-response.jsonl", ROOT);
  const contents = await readFile(path, "utf8");
  const lines = contents.split(/\r?\n/).filter((line) => line.length > 0);
  assert.equal(lines.length, 1, "expected one real production ct-engine JSONL response");
  const response = JSON.parse(lines[0]);
  assert.equal(response.protocol_version, 1);
  assert.equal(response.command, "snapshot");
  assert.equal(response.error_code, null);
  assert.ok(response.payload.workstation, "Tauri's forwarded snapshot payload must include the workstation projection");
  return response.payload;
}

function log(sequence, message, source = "system", idSuffix = "same-engine") {
  return {
    id: `${sequence}-2026-09-26T10:00:${String(sequence % 60).padStart(2, "0")}.000Z-${idSuffix}`,
    timestamp: "2026-09-26T10:00:00.000Z",
    level: "info",
    source,
    message,
  };
}

function startupLog(idSuffix = "same-engine") {
  return log(0, STARTUP_MESSAGE, "system", idSuffix);
}

function snapshot(overrides = {}) {
  return {
    mode: "production_locked",
    modeLabel: "Production",
    connectionState: "disconnected",
    adapterLabel: "ct-engine",
    phase: "idle",
    phaseLabel: "Idle",
    preflightPassed: false,
    homed: false,
    requiresPreflight: true,
    requiresHome: true,
    safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "" },
    devices: [
      { id: "turntable", label: "Turntable", state: "offline", detail: "" },
      { id: "camera", label: "Camera", state: "offline", detail: "" },
      { id: "xray", label: "X-ray", state: "locked", detail: "" },
    ],
    parameters: { taskId: "", savePath: "", projectionCount: 1, angleStepDeg: 1, exposureMs: 1 },
    progress: { current: 0, total: 1, percent: 0, angleDeg: 0, etaSeconds: null },
    imageCount: 0,
    logs: [startupLog(), log(0, "NO REAL HARDWARE output remains fail-closed", "xray")],
    lastError: null,
    updatedAt: "2026-09-26T10:00:00.000Z",
    workstation: {
      xray: {
        connected: false,
        beamState: "unknown",
        beamOn: false,
        latched: false,
        setpointConfirmed: false,
      },
      cameraExposure: { minMs: 1, maxMs: 2000, known: false },
      scene: { angleDeg: 0, rotated: false, angleKnown: false, rotationDirection: 1 },
    },
    ...overrides,
  };
}

function stored(storage) {
  const value = storage.getItem(STORAGE_KEY);
  assert.ok(value, "session evidence was not persisted");
  return JSON.parse(value);
}

test("production first-launch JSONL snapshot proves the engine startup marker is not a device connection", async () => {
  const { DeviceSessionEvidenceTracker } = await loadTracker();
  const storage = new MemoryStorage();
  const tracker = new DeviceSessionEvidenceTracker(storage);
  const tauriSnapshot = await loadProductionFirstLaunchSnapshot();

  assert.equal(tracker.observeSnapshot(tauriSnapshot), false);
  assert.equal(tracker.hasEverEstablished(), false);
  assert.deepEqual(stored(storage).devices, { nano: false, camera: false, xray: false });
  assert.equal(
    stored(storage).engineSessionId,
    tauriSnapshot.logs.find(({ message }) => message === STARTUP_MESSAGE).id,
    "the startup log identifies this engine process without setting device evidence",
  );
});

test("successful Nano, camera, and X-ray responses are tracked independently", async () => {
  const { DeviceSessionEvidenceTracker } = await loadTracker();

  const nanoStorage = new MemoryStorage();
  const nano = new DeviceSessionEvidenceTracker(nanoStorage);
  const nanoResponse = snapshot({
    connectionState: "connected",
    devices: [
      { id: "turntable", label: "Turntable", state: "connected", detail: "" },
      { id: "camera", label: "Camera", state: "offline", detail: "" },
      { id: "xray", label: "X-ray", state: "locked", detail: "" },
    ],
    logs: [startupLog(), log(1, "Nano v1 build connected", "nano")],
  });
  assert.equal(nano.observeSnapshot(nanoResponse, { type: "connect", adapter: "real_hardware" }), true);
  assert.deepEqual(stored(nanoStorage).devices, { nano: true, camera: false, xray: false });

  const cameraStorage = new MemoryStorage();
  const camera = new DeviceSessionEvidenceTracker(cameraStorage);
  const cameraResponse = snapshot({
    devices: [
      { id: "turntable", label: "Turntable", state: "offline", detail: "" },
      { id: "camera", label: "Camera", state: "connected", detail: "" },
      { id: "xray", label: "X-ray", state: "locked", detail: "" },
    ],
    logs: [startupLog(), log(1, "D7100 connected", "camera")],
  });
  assert.equal(camera.observeSnapshot(cameraResponse, { type: "retry_device", device: "camera" }), true);
  assert.deepEqual(stored(cameraStorage).devices, { nano: false, camera: true, xray: false });

  const xrayStorage = new MemoryStorage();
  const xray = new DeviceSessionEvidenceTracker(xrayStorage);
  const xrayResponse = snapshot({
    devices: [
      { id: "turntable", label: "Turntable", state: "offline", detail: "" },
      { id: "camera", label: "Camera", state: "offline", detail: "" },
      { id: "xray", label: "X-ray", state: "connected", detail: "" },
    ],
    logs: [startupLog(), log(1, "Moxtek 12 W connected", "xray")],
    workstation: {
      ...snapshot().workstation,
      xray: { ...snapshot().workstation.xray, connected: true },
    },
  });
  assert.equal(xray.observeSnapshot(xrayResponse, { type: "retry_device", device: "xray" }), true);
  assert.deepEqual(stored(xrayStorage).devices, { nano: false, camera: false, xray: true });
});

test("disconnect remains positive evidence after the live fields go offline", async () => {
  const { DeviceSessionEvidenceTracker } = await loadTracker();
  const storage = new MemoryStorage();
  const tracker = new DeviceSessionEvidenceTracker(storage);

  assert.equal(tracker.observeSnapshot(snapshot({
    connectionState: "connected",
    devices: [
      { id: "turntable", label: "Turntable", state: "connected", detail: "" },
      { id: "camera", label: "Camera", state: "offline", detail: "" },
      { id: "xray", label: "X-ray", state: "locked", detail: "" },
    ],
  })), true);

  const disconnected = snapshot({
    logs: [startupLog(), log(2, "Engine disconnected; safety conditions invalidated", "system")],
  });
  assert.equal(tracker.observeSnapshot(disconnected, { type: "disconnect" }), true);
  assert.equal(stored(storage).devices.nano, true);
});

test("evidence survives UI reload and more than 100 setup logs evicting connection history", async () => {
  const { DeviceSessionEvidenceTracker } = await loadTracker();
  const storage = new MemoryStorage();
  const firstWebview = new DeviceSessionEvidenceTracker(storage);

  assert.equal(firstWebview.observeSnapshot(snapshot({
    connectionState: "connected",
    devices: [
      { id: "turntable", label: "Turntable", state: "connected", detail: "" },
      { id: "camera", label: "Camera", state: "offline", detail: "" },
      { id: "xray", label: "X-ray", state: "locked", detail: "" },
    ],
  }), { type: "connect", adapter: "real_hardware" }), true);

  const cappedLogs = Array.from({ length: 101 }, (_, index) =>
    log(index + 200, "Scan setup updated; repeat preflight and HOME", "system"));
  assert.ok(cappedLogs.length > 100, "the setup update count must exceed the engine's 100-log cap");
  assert.equal(cappedLogs.some((entry) => entry.message.includes("connected")), false);
  assert.equal(cappedLogs.some((entry) => entry.message === STARTUP_MESSAGE), false);

  const afterEviction = snapshot({
    logs: cappedLogs,
    updatedAt: "2026-09-26T10:05:00.000Z",
  });
  assert.equal(firstWebview.observeSnapshot(afterEviction), true);

  const reloadedWebview = new DeviceSessionEvidenceTracker(storage);
  assert.equal(reloadedWebview.observeSnapshot(afterEviction), true);
  assert.equal(stored(storage).devices.nano, true);
});

test("a different engine startup identity clears the previous process history", async () => {
  const { DeviceSessionEvidenceTracker } = await loadTracker();
  const storage = new MemoryStorage();
  const tracker = new DeviceSessionEvidenceTracker(storage);

  assert.equal(tracker.observeSnapshot(snapshot({
    connectionState: "connected",
    devices: [
      { id: "turntable", label: "Turntable", state: "connected", detail: "" },
      { id: "camera", label: "Camera", state: "offline", detail: "" },
      { id: "xray", label: "X-ray", state: "locked", detail: "" },
    ],
  })), true);

  assert.equal(tracker.observeSnapshot(snapshot({
    logs: [startupLog("new-engine"), log(1, "Engine disconnected; safety conditions invalidated", "system", "new-engine")],
  })), "unknown", "a retained disconnect line blocks a never-connected reset");
  assert.equal(tracker.observeSnapshot(snapshot({ logs: [startupLog("new-engine-2")] })), false);
  assert.deepEqual(stored(storage).devices, { nano: false, camera: false, xray: false });
});

test("storage failures, missing fields, and contradictory fields return unknown", async () => {
  const { DeviceSessionEvidenceTracker } = await loadTracker();
  const connectedNano = snapshot({
    connectionState: "connected",
    devices: [
      { id: "turntable", label: "Turntable", state: "connected", detail: "" },
      { id: "camera", label: "Camera", state: "offline", detail: "" },
      { id: "xray", label: "X-ray", state: "locked", detail: "" },
    ],
  });

  const unavailable = new DeviceSessionEvidenceTracker(null);
  assert.equal(unavailable.observeSnapshot(connectedNano), "unknown");

  const writeFailure = new DeviceSessionEvidenceTracker({
    getItem: () => null,
    setItem: () => { throw new Error("storage quota/security failure"); },
  });
  assert.equal(writeFailure.observeSnapshot(connectedNano), "unknown");

  const tracker = new DeviceSessionEvidenceTracker(new MemoryStorage());
  assert.equal(tracker.observeSnapshot(snapshot({ workstation: undefined })), "unknown");
  assert.equal(tracker.observeSnapshot(snapshot({
    connectionState: "connected",
    devices: [
      { id: "turntable", label: "Turntable", state: "offline", detail: "" },
      { id: "camera", label: "Camera", state: "offline", detail: "" },
      { id: "xray", label: "X-ray", state: "locked", detail: "" },
    ],
  })), "unknown");
});

test("the public hook API starts unknown until the hook observes a complete snapshot", async () => {
  const { hasDeviceSessionEverEstablished } = await loadTracker();
  assert.equal(hasDeviceSessionEverEstablished(), "unknown");
});
