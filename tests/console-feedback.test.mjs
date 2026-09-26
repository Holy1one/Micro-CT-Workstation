/**
 * Adversarial tests for the console feedback derived from the shipped App.tsx.
 * The helper below extracts and compiles the actual exported functions; it does
 * not duplicate their implementation.
 */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { transformWithEsbuild } from "vite";

const ROOT = new URL("../", import.meta.url);
const START_MARKER = "/* Console feedback: the eight states";
const DERIVE_MARKER = "export function deriveConsoleFeedback(";
let excerptSequence = 0;
let trackerSequence = 0;

function balancedBlock(source, from) {
  const open = source.indexOf("{", from);
  assert.ok(open >= 0, "no opening brace found");
  let depth = 0;
  for (let i = open; i < source.length;) {
    const c = source[i];
    if (c === "/" && source[i + 1] === "/") {
      const nl = source.indexOf("\n", i);
      i = nl < 0 ? source.length : nl + 1;
      continue;
    }
    if (c === "/" && source[i + 1] === "*") {
      const close = source.indexOf("*/", i + 2);
      assert.ok(close >= 0, "unterminated block comment");
      i = close + 2;
      continue;
    }
    if (c === "\"" || c === "'") {
      const quote = c;
      i += 1;
      while (i < source.length && source[i] !== quote) {
        if (source[i] === "\\") i += 1;
        i += 1;
      }
      i += 1;
      continue;
    }
    if (c === "`") {
      i += 1;
      while (i < source.length && source[i] !== "`") {
        if (source[i] === "\\") i += 1;
        else if (source[i] === "$" && source[i + 1] === "{") {
          i = balancedBlock(source, i + 1).end;
          continue;
        }
        i += 1;
      }
      i += 1;
      continue;
    }
    if (c === "{") depth += 1;
    else if (c === "}") {
      depth -= 1;
      if (depth === 0) return { text: source.slice(open, i + 1), end: i + 1 };
    }
    i += 1;
  }
  throw new Error("unbalanced TypeScript block");
}

async function loadShippedFeedback() {
  const source = await readFile(new URL("src/App.tsx", ROOT), "utf8");
  const start = source.indexOf(START_MARKER);
  const derive = source.indexOf(DERIVE_MARKER);
  assert.ok(start >= 0, "console feedback declaration not found in src/App.tsx");
  assert.ok(derive > start, "exported deriveConsoleFeedback not found in src/App.tsx");
  const extracted = source.slice(start, balancedBlock(source, derive).end);
  for (const token of ["evaluateFeedbackEvidence", "deriveConsoleFeedback"]) {
    assert.ok(extracted.includes(token), `shipped feedback region is missing ${token}`);
  }
  assert.ok(!/^\s*import\s/m.test(extracted), "feedback region unexpectedly contains imports");

  const { code } = await transformWithEsbuild(
    extracted,
    "App.tsx.feedback-excerpt.ts",
    { loader: "ts", target: "es2022", format: "esm" },
  );
  const url = "data:text/javascript;base64," + Buffer.from(code).toString("base64") + "#instance-" + excerptSequence++;
  return import(url);
}

async function loadProductionFirstLaunchSnapshot() {
  const responsePath = new URL("tmp/tests/frontend/20260927-portable-feedback/first-launch-response.jsonl", ROOT);
  const contents = await readFile(responsePath, "utf8");
  const lines = contents.split(/\r?\n/).filter((line) => line.length > 0);
  assert.equal(lines.length, 1, "expected one real production ct-engine JSONL response");
  const response = JSON.parse(lines[0]);
  assert.equal(response.protocol_version, 1);
  assert.equal(response.command, "snapshot");
  assert.equal(response.error_code, null);
  assert.ok(response.payload && typeof response.payload === "object", "ct-engine response payload is missing");

  // EngineClient returns response.payload and Tauri's engine_snapshot command
  // forwards that Value unchanged. This is the actual shape consumed by React.
  const tauriSnapshot = response.payload;
  assert.ok(tauriSnapshot.workstation, "production first-start workstation projection is missing");
  return { response, tauriSnapshot };
}

class MemoryStorage {
  values = new Map();

  getItem(key) {
    return this.values.get(key) ?? null;
  }

  setItem(key, value) {
    this.values.set(key, value);
  }
}

async function loadProductionSessionTracker(storage) {
  const source = await readFile(new URL("src/engine/useEngine.ts", ROOT), "utf8");
  const start = source.indexOf("export type DeviceSessionEverEstablished");
  const end = source.indexOf("function errorMessage(reason: unknown): string", start);
  assert.ok(start >= 0 && end > start, "session evidence declarations moved or were removed");

  // The appended export is a test seam for the same private observer called by
  // useEngine; the singleton and public hasDeviceSessionEverEstablished API
  // remain the production implementation.
  const implementation = source.slice(start, end) + "\nexport { observeDeviceSessionSnapshot };";
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, "sessionStorage");
  Object.defineProperty(globalThis, "sessionStorage", { configurable: true, writable: true, value: storage });
  try {
    const { code } = await transformWithEsbuild(implementation, "engine-session-evidence.ts", { loader: "ts" });
    const url = "data:text/javascript;base64," + Buffer.from(code).toString("base64") + "#portable-tracker-" + trackerSequence++;
    return await import(url);
  } finally {
    if (previousStorage) Object.defineProperty(globalThis, "sessionStorage", previousStorage);
    else delete globalThis.sessionStorage;
  }
}

const feedbackModule = await loadShippedFeedback();
const { deriveConsoleFeedback } = feedbackModule;
const ENGINE_PHASES = [
  "idle", "ready_for_home", "ready", "running", "paused", "finishing", "stopping", "stopped", "completed", "fault",
];
const CONNECTION_STATES = ["disconnected", "connected", "degraded", "lost"];
const BEAM_STATES = ["off", "on", "unknown"];
const SESSION_EVIDENCE = [
  { name: "true", value: true },
  { name: "false", value: false },
  { name: "unknown", value: "unknown" },
  { name: "missing", missing: true },
];
const isNeutral = (feedback) =>
  feedback.state === "Stopped" && feedback.tone === "muted" && feedback.active === false;

function snapshot(overrides = {}) {
  const { safety = {}, ...rest } = overrides;
  return {
    mode: "production_locked",
    modeLabel: "PRODUCTION LOCKED · NANO DISCONNECTED",
    connectionState: "disconnected",
    adapterLabel: "ct-engine · Nano CH340 · JSONL stdio",
    phase: "idle",
    phaseLabel: "Idle",
    preflightPassed: false,
    homed: false,
    requiresPreflight: true,
    requiresHome: true,
    safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "" , ...safety },
    devices: [
      { id: "turntable", label: "Precision Turntable", state: "offline", detail: "disconnected" },
      { id: "camera", label: "Nikon D7100", state: "offline", detail: "disconnected" },
      { id: "xray", label: "Moxtek 12 W", state: "locked", detail: "fail-closed · connect confirms OFF" },
    ],
    parameters: { taskId: "", savePath: "", projectionCount: 0, angleStepDeg: 0, exposureMs: 0 },
    progress: { current: 0, total: 0, percent: 0, angleDeg: 0, etaSeconds: null },
    imageCount: 0,
    logs: [],
    lastError: null,
    updatedAt: "2026-09-26T00:00:00.000Z",
    ...rest,
  };
}

function workstation(overrides = {}) {
  const {
    cameraExposure = {}, xray = {}, progress = {}, scene = {}, preflight = {}, safetyBar,
    devices, floats, summary = {}, statusbar = {}, dock = {}, scanSetup = {}, consoleLogs = [],
    frames = [], ...rest
  } = overrides;
  const beamState = xray.beamState ?? "off";
  const outputText = beamState === "unknown" ? "OUTPUT UNKNOWN · READBACK REQUIRED" : "SAFETY · OUTPUT DISABLED";
  const base = {
    cameraExposure: { minMs: 0.125, maxMs: 30000, known: false },
    dataState: "ready",
    phaseWord: "IDLE",
    phaseTone: "accent",
    devices: [
      { id: "turntable", name: "Turntable-Nano", word: "OFFLINE", tone: "muted", spec: "DISCONNECTED" },
      { id: "camera", name: "Camera", word: "OFFLINE", tone: "muted", spec: "D7100 · DISCONNECTED" },
      { id: "xray", name: "X-Ray Source", word: beamState === "unknown" ? "OUTPUT UNKNOWN" : "OFF VERIFIED", tone: "muted", spec: "Moxtek · awaiting connection" },
    ],
    onlineSummary: "LOCKED · 0 REAL DEVICES",
    preflight: { word: "WAITING", percent: 0, tone: "warn", subline: "0/8 checks · no real hardware" },
    floats: [
      { key: "X-RAY", text: beamState === "unknown" ? "OUTPUT UNKNOWN · READBACK REQUIRED" : "OUTPUT OFF", tone: "muted" },
      { key: "CAMERA", text: "OFFLINE", tone: "muted" },
      { key: "SAMPLE", text: "UNKNOWN", tone: "muted" },
    ],
    safetyBar: { text: outputText, tone: "muted" },
    scene: { angleDeg: 0, rotated: false, angleKnown: false, rotationDirection: -1 },
    xray: {
      connected: false,
      setKv: 4,
      setUa: 10,
      monKv: null,
      monUa: null,
      powerW: null,
      tempC: null,
      latched: false,
      beamState,
      beamOn: false,
      onSec: 10,
      offSec: 20,
      timerOn: false,
      usbAutoShutDown: true,
      usbAutoShutDownKnown: false,
      usbShutdownDelay: null,
      manualControlsEnabled: false,
      timerControlsEnabled: false,
      setpointControlsEnabled: false,
      voltageConfirmed: false,
      currentConfirmed: false,
      setpointConfirmed: false,
      ...xray,
    },
    progress: {
      captured: 0, total: 0, percent: 0, angleDeg: 0, etaText: "—", barTone: "accent", ...progress,
      barLabel: progress.barLabel ?? `${progress.captured ?? 0} / ${progress.total ?? 0} · ${progress.percent ?? 0}%`,
    },
    summary: { savePath: "", acquisition: "Scan setup not configured", output: beamState === "unknown" ? "OUTPUT UNKNOWN · READBACK REQUIRED" : "OUTPUT OFF", ...summary },
    statusbar: { left: "PRODUCTION LOCKED · SETUP REQUIRED", right: "ct-engine · JSONL v1 · LOCKED", dotTone: "muted", ...statusbar },
    dock: { home: false, play: false, restore: false, stop: false, playMode: "start", homeReason: "Connect the Nano first", playReason: "Connect the Nano first", ...dock },
    scanSetup: { savePath: "", taskId: "", projectionCount: 0, angleStepDeg: 0, exposureMs: 0, maxXraySec: 600, ...scanSetup },
    consoleLogs,
    frames,
    checkpointAvailable: false,
  };
  return {
    ...base,
    ...rest,
    cameraExposure: { ...base.cameraExposure, ...cameraExposure },
    devices: devices ?? base.devices,
    preflight: { ...base.preflight, ...preflight },
    safetyBar: safetyBar ?? base.safetyBar,
    scene: { ...base.scene, ...scene },
  };
}

function assertRedFailClosed(reading, label) {
  assert.equal(reading.state, "Fault", `${label} state was ${reading.state}`);
  assert.equal(reading.tone, "danger", `${label} tone was ${reading.tone}`);
}

function startConnectionSession() {
  const reading = deriveConsoleFeedback(
    snapshot({ connectionState: "connected" }),
    workstation(),
    false,
    true,
  );
  assert.equal(reading.state, "Stopped");
}

test("bounded log eviction and UI remount cannot erase a proven device session", async () => {
  const retainedLogs = [];
  const appendLog = (id, message) => {
    retainedLogs.push({ id, timestamp: id, level: "info", source: "engine", message });
    if (retainedLogs.length > 100) retainedLogs.shift();
  };

  appendLog("connect", "Nano connected");
  assert.notEqual(
    deriveConsoleFeedback(snapshot({ connectionState: "connected", logs: [...retainedLogs] }), workstation(), false, true).state,
    "Fault",
  );
  appendLog("disconnect", "Nano disconnected");
  assertRedFailClosed(
    deriveConsoleFeedback(
      snapshot({ connectionState: "disconnected", phase: "idle", logs: [...retainedLogs] }),
      workstation(),
      false,
      true,
    ),
    "clean disconnect after a connection",
  );

  for (let update = 0; update < 101; update += 1) {
    appendLog("setup-" + update, "update_scan_setup " + update);
    deriveConsoleFeedback(
      snapshot({ connectionState: "disconnected", phase: "idle", logs: [...retainedLogs] }),
      workstation(),
      false,
      true,
    );
  }
  assert.equal(retainedLogs.length, 100);
  assert.equal(retainedLogs.some(({ message }) => /connected|disconnected/i.test(message)), false);

  const remountedSnapshot = snapshot({
    connectionState: "disconnected",
    phase: "idle",
    logs: [...retainedLogs],
  });
  const remountedWorkstation = workstation({
    xray: { beamState: "unknown", beamOn: false, setpointConfirmed: false },
  });
  assert.equal(remountedWorkstation.consoleLogs.length, 0);
  const remountedApp = await loadShippedFeedback();
  assertRedFailClosed(
    remountedApp.deriveConsoleFeedback(remountedSnapshot, remountedWorkstation, false, true),
    "post-connection unknown output after 101 setup updates and UI remount",
  );
});

test("only explicit false licenses neutral disconnected feedback", () => {
  const snap = snapshot();
  const ws = workstation({ xray: { beamState: "unknown" } });
  for (const [label, evidence] of [
    ["true", true],
    ["unknown", "unknown"],
    ["null", null],
    ["number", 0],
    ["string", "false"],
    ["object", {}],
  ]) {
    assertRedFailClosed(deriveConsoleFeedback(snap, ws, false, evidence), label + " session evidence");
  }
  assertRedFailClosed(deriveConsoleFeedback(snap, ws, false), "missing session evidence");
  assertRedFailClosed(
    deriveConsoleFeedback(snapshot({ phase: "ready" }), ws, false, false),
    "contradictory snapshot and session evidence",
  );
  assert.ok(isNeutral(deriveConsoleFeedback(snap, ws, false, false)), "only a strict clean startup stays neutral");
});

test("feedback source has no retained-log connection-history inference", async () => {
  const source = await readFile(new URL("src/App.tsx", ROOT), "utf8");
  assert.doesNotMatch(source, /linkNeverEstablished|connectionWasRecorded/);
});

test("a never-connected idle snapshot with a latched X-ray fault is red", () => {
  const reading = deriveConsoleFeedback(
    snapshot(),
    workstation({ xray: { latched: true } }),
    false,
  );
  assertRedFailClosed(reading, "latched X-ray fault");
});

test("a never-connected idle snapshot with unknown beam under danger tone is red", () => {
  const reading = deriveConsoleFeedback(
    snapshot(),
    workstation({ phaseTone: "danger", xray: { beamState: "unknown" } }),
    false,
  );
  assertRedFailClosed(reading, "unknown beam under danger tone");
});

test("a never-connected idle snapshot with unconfirmed beamOn is red", () => {
  const reading = deriveConsoleFeedback(
    snapshot(),
    workstation({ xray: { beamOn: true, setpointConfirmed: false } }),
    false,
  );
  assertRedFailClosed(reading, "unconfirmed beamOn");
});

test("non-vacuity: a never-connected idle snapshot with no dangerous fields stays neutral", () => {
  const reading = deriveConsoleFeedback(snapshot(), workstation(), false, false);
  assert.equal(reading.state, "Stopped");
  assert.equal(reading.tone, "muted");
});

test("lost, degraded, stale, and post-connection disconnected remain red", () => {
  for (const connectionState of ["lost", "degraded"]) {
    startConnectionSession();
    const reading = deriveConsoleFeedback(
      snapshot({ connectionState, phase: "ready" }),
      workstation(),
      false,
    );
    assertRedFailClosed(reading, connectionState);
  }

  startConnectionSession();
  assertRedFailClosed(
    deriveConsoleFeedback(snapshot({ connectionState: "connected" }), workstation(), true),
    "stale snapshot",
  );

  startConnectionSession();
  assertRedFailClosed(
    deriveConsoleFeedback(snapshot(), workstation(), false),
    "disconnected after connection existed",
  );
});

test("a real fault phase remains red", () => {
  const reading = deriveConsoleFeedback(
    snapshot({ phase: "fault" }),
    workstation({ dataState: "fault", phaseTone: "danger" }),
    false,
  );
  assertRedFailClosed(reading, "real fault phase");
});

test("contradictory safety, projection, and newly introduced fields all fail closed", () => {
  const clean = snapshot();
  const cleanWorkstation = workstation();
  const cases = [
    ["xrayEnabled disagrees with beam readbacks", snapshot({ safety: { xrayEnabled: true } }), cleanWorkstation],
    ["xrayAvailable claims an unlinked source is available", snapshot({ safety: { xrayAvailable: true } }), cleanWorkstation],
    ["interlock claims a live confirmation without an X-ray link", snapshot({ safety: { interlockOk: true } }), cleanWorkstation],
    ["SafetyState field is missing", (() => { const value = snapshot(); delete value.safety.interlockOk; return value; })(), cleanWorkstation],
    ["SafetyState gains an unclassified field", snapshot({ safety: { serviceReady: false } }), cleanWorkstation],
    ["snapshot and passed workstation projections disagree", snapshot({ workstation: cleanWorkstation }), workstation({ xray: { beamState: "unknown" } })],
    ["snapshot and passed workstation projections agree", snapshot({ workstation: cleanWorkstation }), cleanWorkstation, true],
    ["workstation is missing a required projection layer", clean, (() => { const value = workstation(); delete value.xray; return value; })()],
    ["phase-tone contradicts an idle phase", clean, workstation({ phaseTone: "warn" })],
    ["preflight evidence contradicts its required gate", clean, workstation({ preflight: { word: "PASSED", percent: 100, tone: "pass" } })],
    ["requiresPreflight disagrees with preflightPassed", snapshot({ requiresPreflight: false }), cleanWorkstation],
    ["homed disagrees with requiresHome", snapshot({ homed: true, requiresHome: false }), cleanWorkstation],
    ["a first-start dock exposes a command", clean, workstation({ dock: { home: true } })],
    ["a first-start dock exposes play", clean, workstation({ dock: { play: true } })],
    ["a first-start dock exposes restore", clean, workstation({ dock: { restore: true } })],
    ["a first-start dock exposes stop", clean, workstation({ dock: { stop: true } })],
    ["snapshot and view device IDs are not a full matching set", clean, workstation({ devices: [{ id: "xray", name: "X-ray", word: "OFFLINE", tone: "muted", spec: "" }] })],
    ["workstation progress disagrees with engine progress", clean, workstation({ progress: { captured: 1 } })],
    ["safety bar contradicts a disabled output", clean, workstation({ safetyBar: { text: "BEAM ON · INTERLOCK OK", tone: "danger" } })],
    ["first-start snapshot gains a future field", { ...clean, futureSafetyLatch: false }, cleanWorkstation],
    ["workstation gains a future field", clean, { ...cleanWorkstation, futureSafetyLatch: false }],
    ["safety bar gains an unclassified field", clean, workstation({ safetyBar: { text: "OUTPUT DISABLED", tone: "muted", verified: false } })],
    ["checkpoint implies a prior scan", clean, workstation({ checkpointAvailable: true })],
    ["scene angle disagrees with progress", clean, workstation({ scene: { angleDeg: 1 } })],
    ["snapshot success log indicates a completed action", snapshot({ logs: [{ id: "ok", timestamp: "now", level: "success", source: "系统", message: "connected" }] }), cleanWorkstation],
    ["console action log indicates first-start activity", clean, workstation({ consoleLogs: [{ id: "action", timestamp: "now", level: "ACTION", source: "operator", message: "capture" }] })],
  ];
  for (const [label, snap, ws, shouldBeNeutral] of cases) {
    const feedback = deriveConsoleFeedback(snap, ws, false, false);
    if (shouldBeNeutral) assert.ok(isNeutral(feedback), label);
    else assertRedFailClosed(feedback, label);
  }
});

test("real production first-start JSONL flows through Tauri shape and tracker to neutral unknown feedback", async () => {
  const { response, tauriSnapshot } = await loadProductionFirstLaunchSnapshot();
  const storage = new MemoryStorage();
  const tracker = await loadProductionSessionTracker(storage);

  assert.equal(tauriSnapshot.connectionState, "disconnected");
  assert.equal(tauriSnapshot.phase, "idle");
  assert.deepEqual(tauriSnapshot.logs, tauriSnapshot.workstation.consoleLogs);
  assert.deepEqual(
    tauriSnapshot.logs.map(({ level, source }) => `${level} · ${source}`).sort(),
    ["INFO · system", "WARN · xray"],
    "fixture must retain the production engine's actual log format",
  );

  assert.equal(tracker.hasDeviceSessionEverEstablished(), "unknown", "the public API starts unknown before its first snapshot");
  tracker.observeDeviceSessionSnapshot(tauriSnapshot);
  const sessionEvidence = tracker.hasDeviceSessionEverEstablished();
  assert.equal(sessionEvidence, false, "an engine startup marker does not count as any device connection");
  console.log(`[portable feedback measurement] response=${response.command}; hasDeviceSessionEverEstablished()=${sessionEvidence}`);

  const feedback = deriveConsoleFeedback(tauriSnapshot, tauriSnapshot.workstation, false, sessionEvidence);
  assert.ok(isNeutral(feedback), `real first-start response was not neutral: ${JSON.stringify(feedback)}`);
  assert.match(feedback.detail, /not connected/i);
  assert.match(feedback.detail, /unavailable/i, "the neutral reading must keep device and output states explicitly unknown");
  assert.equal(tauriSnapshot.workstation.xray.beamState, "unknown");
  assert.equal(tauriSnapshot.workstation.devices.find(({ id }) => id === "camera").word, "OFFLINE");
  assert.match(tauriSnapshot.workstation.floats.find(({ key }) => key === "X-RAY").text, /OUTPUT UNKNOWN/i);
  assert.match(tauriSnapshot.workstation.floats.find(({ key }) => key === "SAMPLE").text, /UNKNOWN/i);
  assert.match(tauriSnapshot.workstation.onlineSummary, /0 REAL DEVICES/i);

  const connected = structuredClone(tauriSnapshot);
  connected.connectionState = "connected";
  connected.devices.find(({ id }) => id === "turntable").state = "connected";
  connected.logs.push({
    id: `1-${tauriSnapshot.updatedAt}-connected`,
    timestamp: tauriSnapshot.updatedAt,
    level: "INFO",
    source: "nano",
    message: "Nano v1 connected",
  });
  tracker.observeDeviceSessionSnapshot(connected);
  assert.equal(tracker.hasDeviceSessionEverEstablished(), true, "positive Nano connection evidence is retained");

  const disconnected = structuredClone(tauriSnapshot);
  disconnected.logs.push({
    id: `2-${tauriSnapshot.updatedAt}-disconnected`,
    timestamp: tauriSnapshot.updatedAt,
    level: "INFO",
    source: "system",
    message: "Nano disconnected",
  });
  tracker.observeDeviceSessionSnapshot(disconnected);
  const disconnectedEvidence = tracker.hasDeviceSessionEverEstablished();
  assert.equal(disconnectedEvidence, true);
  assertRedFailClosed(
    deriveConsoleFeedback(disconnected, disconnected.workstation, false, disconnectedEvidence),
    "disconnect after real tracker recorded positive device connection evidence",
  );
});

test("the shipped derivation exhaustively partitions the full Cartesian console space", () => {
  const neutralTuples = [];
  let traversed = 0;
  let disconnectedRed = 0;
  let redAssertions = 0;
  const lockReasons = [
    { name: "empty", value: "" },
    { name: "nonempty", value: "Stage 1 · X-ray locked" },
    { name: "missing", missing: true },
  ];

  for (const beamState of BEAM_STATES) {
    for (const beamOn of [false, true]) {
      for (const setpointConfirmed of [false, true]) {
        for (const latched of [false, true]) {
          for (const phase of ENGINE_PHASES) {
            for (const connectionState of CONNECTION_STATES) {
              for (const stale of [false, true]) {
                for (const session of SESSION_EVIDENCE) {
                  for (const xrayAvailable of [false, true]) {
                    for (const xrayEnabled of [false, true]) {
                      for (const interlockOk of [false, true]) {
                        for (const lockReason of lockReasons) {
                  traversed += 1;
                  const tuple = {
                    sessionEvidence: session.name,
                    beamState,
                    beamOn,
                    setpointConfirmed,
                    latched,
                    phase,
                    connectionState,
                    stale,
                    safety: {
                      xrayAvailable,
                      xrayEnabled,
                      interlockOk,
                      lockReason: lockReason.missing ? "<missing>" : lockReason.value,
                    },
                  };
                  const snap = snapshot({
                    connectionState,
                    phase,
                    safety: {
                      xrayAvailable,
                      xrayEnabled,
                      interlockOk,
                      lockReason: lockReason.missing ? "" : lockReason.value,
                    },
                  });
                  if (lockReason.missing) delete snap.safety.lockReason;
                  const ws = workstation({
                    xray: {
                      connected: connectionState === "connected",
                      beamState,
                      beamOn,
                      setpointConfirmed,
                      latched,
                    },
                  });
                  const feedback = session.missing
                    ? deriveConsoleFeedback(snap, ws, stale)
                    : deriveConsoleFeedback(snap, ws, stale, session.value);
                  const strictlyInitial =
                    session.name === "false" &&
                    beamOn === false &&
                    setpointConfirmed === false &&
                    latched === false &&
                    phase === "idle" &&
                    connectionState === "disconnected" &&
                    stale === false &&
                    xrayAvailable === false &&
                    xrayEnabled === false &&
                    interlockOk === false &&
                    !lockReason.missing &&
                    (beamState === "off" || beamState === "unknown");

                  if (connectionState === "disconnected") {
                    if (strictlyInitial) {
                      neutralTuples.push(tuple);
                      assert.ok(isNeutral(feedback), "strictly initial tuple must be neutral: " + JSON.stringify(tuple));
                      assert.match(feedback.detail, /not connected/i);
                      continue;
                    }
                    disconnectedRed += 1;
                    redAssertions += 1;
                    assertRedFailClosed(feedback, "non-initial disconnected tuple " + JSON.stringify(tuple));
                    continue;
                  }

                  const contradictory =
                    (beamState === "on" && !beamOn) ||
                    (beamState === "off" && beamOn);
                  const expectedRed =
                    connectionState === "lost" ||
                    connectionState === "degraded" ||
                    stale ||
                    phase === "fault" ||
                    latched ||
                    beamState === "unknown" ||
                    contradictory ||
                    (beamOn && !setpointConfirmed) ||
                    (beamOn && phase !== "running");
                  if (expectedRed) {
                    redAssertions += 1;
                    assertRedFailClosed(feedback, "fail-closed tuple " + JSON.stringify(tuple));
                  }
                }
              }
                        }
                      }
                    }
                  }
            }
          }
        }
      }
    }
  }

  const expectedNeutral = [
    { sessionEvidence: "false", beamState: "off", beamOn: false, setpointConfirmed: false, latched: false, phase: "idle", connectionState: "disconnected", stale: false, safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "" } },
    { sessionEvidence: "false", beamState: "off", beamOn: false, setpointConfirmed: false, latched: false, phase: "idle", connectionState: "disconnected", stale: false, safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "Stage 1 · X-ray locked" } },
    { sessionEvidence: "false", beamState: "unknown", beamOn: false, setpointConfirmed: false, latched: false, phase: "idle", connectionState: "disconnected", stale: false, safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "" } },
    { sessionEvidence: "false", beamState: "unknown", beamOn: false, setpointConfirmed: false, latched: false, phase: "idle", connectionState: "disconnected", stale: false, safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "Stage 1 · X-ray locked" } },
  ];
  assert.equal(traversed, 184320, "the full Cartesian product including all four SafetyState dimensions must run");
  assert.deepEqual(neutralTuples, expectedNeutral, "neutral tuples must equal the strictly-initial set");
  assert.equal(disconnectedRed, 46076, "all disconnected tuples except the four strict initial tuples must be red");
  assert.ok(redAssertions > disconnectedRed, "the sweep must also assert non-disconnected fail-closed cases");
  console.log("[console-feedback sweep] combinations traversed=" + traversed);
  console.log("[console-feedback sweep] exact neutral tuple set=" + JSON.stringify(neutralTuples));
  console.log(
    "[console-feedback sweep] traversed=" + traversed +
    "; neutral=" + JSON.stringify(neutralTuples) +
    "; disconnectedRed=" + disconnectedRed +
    "; redAssertions=" + redAssertions,
  );
});

test("strict startup accepts drafts but rejects contradictory initial-state evidence", () => {
  const draftParameters = { taskId: "draft", savePath: "", projectionCount: 900, angleStepDeg: 0, exposureMs: 0 };
  const drafts = snapshot({
    parameters: draftParameters,
    progress: { current: 0, total: 900, percent: 0, angleDeg: 0, etaSeconds: null },
  });
  const draftWorkstation = workstation({
    xray: { setKv: 80, setUa: 300 },
    progress: { total: 900 },
    scanSetup: { ...draftParameters, maxXraySec: 600 },
  });
  assert.ok(isNeutral(deriveConsoleFeedback(drafts, draftWorkstation, false, false)), "draft settings are not device readbacks");

  const contradictions = [
    [snapshot({ lastError: "startup failed" }), workstation()],
    [snapshot({ logs: [{ id: "e", timestamp: "now", level: "error", source: "系统", message: "error" }] }), workstation()],
    [snapshot({ logs: [{ id: "i", timestamp: "now", level: "info", source: "系统", message: "Nano v1 connected · HOME not executed" }] }), workstation(), "unknown"],
    [snapshot({ devices: [{ id: "xray", label: "X-ray", state: "connected", detail: "connected" }] }), workstation()],
    [snapshot({ imageCount: 1 }), workstation()],
    [snapshot({ progress: { current: 1, total: 10, percent: 10, angleDeg: 0, etaSeconds: null } }), workstation()],
    [snapshot(), workstation({ progress: { captured: 1, total: 10, percent: 10 } })],
    [snapshot(), workstation({ dataState: "scanning" })],
    [snapshot(), workstation({ phaseTone: "warn" })],
    [snapshot(), workstation({ scene: { angleKnown: true } })],
    [snapshot(), workstation({ xray: { connected: true } })],
    [snapshot(), workstation({ xray: { monKv: 0 } })],
    [snapshot(), workstation({ xray: { tempC: 22 } })],
    [snapshot(), workstation({ consoleLogs: [{ id: "e", timestamp: "now", level: "ERR", source: "system", message: "error" }] })],
  ];
  for (const [snap, ws, sessionEvidence = false] of contradictions) {
    assertRedFailClosed(deriveConsoleFeedback(snap, ws, false, sessionEvidence), "initial-state contradiction");
  }
});

test("non-vacuity: connected ordinary phases retain their real console states", () => {
  for (const [phase, expectedState] of [
    ["running", "Running"],
    ["paused", "Paused"],
    ["finishing", "Finishing"],
    ["stopping", "Stopping"],
    ["stopped", "Stopped"],
    ["completed", "Completed"],
  ]) {
    const feedback = deriveConsoleFeedback(
      snapshot({ connectionState: "connected", phase }),
      workstation({ xray: { connected: true, beamState: "off", beamOn: false, setpointConfirmed: true } }),
      false,
    );
    assert.equal(feedback.state, expectedState, `${phase} should remain ${expectedState}`);
    assert.notEqual(feedback.tone, "danger");
  }
});


test("connected scan evidence retains Cooling while its deadline is not projected", () => {
  const cooling = deriveConsoleFeedback(
    snapshot({ connectionState: "connected", phase: "ready", progress: { current: 1, percent: 10 } }),
    workstation({
      xray: { connected: true, beamState: "off", beamOn: false, setpointConfirmed: true },
      progress: { captured: 1, total: 10, percent: 10 },
    }),
    false,
    true,
  );
  assert.equal(cooling.state, "Cooling");
  assert.notEqual(cooling.tone, "danger");
});

test("active, contradictory, and post-connection unknown output stays red", () => {
  const cases = [
    ["disconnected confirmed ON", "disconnected", "on", true, true, "idle"],
    ["ON state with beamOn false", "connected", "on", false, true, "running"],
    ["OFF state with beamOn true", "connected", "off", true, true, "running"],
    ["unknown after connection under accent", "connected", "unknown", false, false, "running"],
    ["active output outside acquisition", "connected", "on", true, true, "ready"],
  ];
  for (const [label, connectionState, beamState, beamOn, setpointConfirmed, phase] of cases) {
    assertRedFailClosed(
      deriveConsoleFeedback(
        snapshot({ connectionState, phase }),
        workstation({ xray: { connected: connectionState === "connected", beamState, beamOn, setpointConfirmed } }),
        false,
      ),
      label,
    );
  }
});
