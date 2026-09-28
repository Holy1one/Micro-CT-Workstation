/**
 * Console FAULT-banner safety contract.
 *
 * A freshly launched desktop app reports `connectionState: "disconnected"`
 * because no device link was ever established. That benign startup state used to
 * be folded into the same fail-closed branch as a connection that was actually
 * lost, so a brand-new console drew a red FAULT banner and a red collapsed dock
 * before anything had been attempted. This test pins the corrected mapping and,
 * more importantly, pins that everything genuinely uncertain still fails closed:
 * `lost`, `degraded`, a stale/unknown snapshot and a post-connection
 * `disconnected` must all still be red.
 *
 * The derivation under test is NOT re-implemented here. `src/App.tsx` exports
 * `deriveConsoleFeedback`; this test lifts that exact source region out of the
 * shipped file (between two stable top-level declarations), compiles it with the
 * repository's own esbuild transform, and drives the real function with synthetic
 * snapshots. Tests pass session evidence through the exported function's input;
 * missing evidence must fail closed, and no duplicate derivation is maintained.
 */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { transformWithEsbuild } from "vite";

const ROOT = new URL("../", import.meta.url);

/* ------------------------------------------------------------------ *
 * Load the real derivation straight out of src/App.tsx                *
 * ------------------------------------------------------------------ */

const START_MARKER = "Console feedback: the eight states";
const END_MARKER = "export function deriveConsoleFeedback(";

/**
 * Match a brace-balanced TypeScript block starting at the first `{` at or after
 * `from`. Comments, quoted strings and template literals (including their
 * `${ ... }` substitutions) are skipped, so a brace inside a message string can
 * never end the block early.
 */
function balancedBlock(source, from) {
  const open = source.indexOf("{", from);
  assert.ok(open >= 0, "no opening brace found");
  let depth = 0;
  let i = open;
  while (i < source.length) {
    const c = source[i];
    if (c === "/" && source[i + 1] === "/") {
      const nl = source.indexOf("\n", i);
      i = nl < 0 ? source.length : nl + 1;
      continue;
    }
    if (c === "/" && source[i + 1] === "*") {
      const close = source.indexOf("*/", i + 2);
      assert.ok(close > 0, "unterminated block comment in the extracted region");
      i = close + 2;
      continue;
    }
    if (c === '"' || c === "'") {
      i += 1;
      while (i < source.length && source[i] !== c) {
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
          // A substitution may itself contain braces, strings or templates. The
          // scanner resumes on the character after its closing brace.
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
  throw new Error("unbalanced block");
}

async function loadRealDerivation() {
  const source = await readFile(new URL("src/App.tsx", ROOT), "utf8");
  const markerAt = source.indexOf(START_MARKER);
  const deriveAt = source.indexOf(END_MARKER);
  assert.ok(markerAt >= 0, `src/App.tsx no longer declares ${START_MARKER}`);
  assert.ok(deriveAt > markerAt, `src/App.tsx no longer declares ${END_MARKER}`);
  // Back up to the comment opener so the slice starts at a clean declaration
  // boundary instead of in the middle of a block comment.
  const start = source.lastIndexOf("/*", markerAt);
  assert.ok(start >= 0, "no comment opener before the console feedback block");

  const deriveEnd = balancedBlock(source, deriveAt).end;
  const derived = source.slice(start, deriveEnd);

  // Guard the extraction: everything the derivation needs must be inside the
  // slice, and the slice must not drag in React or any other import. Only tokens
  // that predate this contract are required here, so the same test can be run
  // against the pre-change file to show it fails there for a semantic reason.
  for (const token of [
    "CONSOLE_FEEDBACK_STATES",
    "SCAN_ENGAGED_PHASES",
    "evaluateFeedbackEvidence",
    "deriveConsoleFeedback",
  ]) {
    assert.ok(derived.includes(token), `extracted region is missing ${token}`);
  }
  assert.ok(!/^\s*import\s/m.test(derived), "extracted region contains an import statement");
  assert.ok(derived.includes("export function deriveConsoleFeedback("), "the real derivation is not exported");
  assert.ok(derived.includes("export function evaluateFeedbackEvidence("), "the evidence evaluation is not exported");

  // Stand-ins for the two type-only imports of src/App.tsx and for the evidence
  // interface, which is declared inside the extracted region itself.
  const preamble = `
    const EngineSnapshot = /** @type {any} */ (undefined);
    const WorkstationView = /** @type {any} */ (undefined);
  `;
  const { code } = await transformWithEsbuild(
    `${preamble}\n${derived}\nexport { CONSOLE_FEEDBACK_STATES };\n`,
    "App.tsx.excerpt.ts",
    { loader: "ts", target: "es2022", format: "esm" },
  );
  const url = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
  return { module: await import(url), derived, source };
}

const { module: derivation, derived, source } = await loadRealDerivation();
const { deriveConsoleFeedback, evaluateFeedbackEvidence } = derivation;
const CONSOLE_FEEDBACK_STATES = derivation.CONSOLE_FEEDBACK_STATES;

/* ------------------------------------------------------------------ *
 * Complete synthetic contract fixtures. Every EngineSnapshot and       *
 * WorkstationView layer is present so shape coverage is exercised.      *
 * ------------------------------------------------------------------ */

const STATE_NAMES = [...CONSOLE_FEEDBACK_STATES];
const isRed = (feedback) => feedback.tone === "danger" && feedback.state === "Fault";
const isNeutral = (feedback) => feedback.tone === "muted" && !isRed(feedback);
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
      beamState,
      beamOn: false,
      latched: false,
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
    dock: { home: false, play: false, restore: false, stop: false, playMode: "start", homeReason: "Connect the Nano first", restoreReason: "Connect the Nano first", playReason: "Connect the Nano first", ...dock },
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
    safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "", ...safety },
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

/** A never-connected fresh launch: engine up, no link, nothing attempted. */
const freshLaunch = () => snapshot();

/** A link that was established and then went away. */
const afterConnection = (connectionState) =>
  snapshot({
    connectionState,
    modeLabel: "PRODUCTION · NANO ONLINE · CAMERA/X-RAY REQUIRED",
    phase: "ready",
    phaseLabel: "Ready",
    preflightPassed: true,
    homed: true,
    requiresPreflight: false,
    requiresHome: false,
  });

/* ------------------------------------------------------------------ *
 * Row 1 — first `disconnected`, never connected yet: NEUTRAL           *
 * ------------------------------------------------------------------ */

test("a never-connected first `disconnected` renders neutral, not a red fault", () => {
  const feedback = deriveConsoleFeedback(freshLaunch(), workstation(), false, false);
  assert.equal(feedback.tone, "muted", `expected a neutral tone, got ${feedback.tone} (${feedback.state})`);
  assert.notEqual(feedback.state, "Fault");
  assert.match(feedback.detail, /not connected/i);
  assert.match(feedback.detail, /unavailable|unknown/i);
  assert.equal(feedback.active, false);
  // Stated as a plain reading, never as preparation for a scan.
  assert.doesNotMatch(feedback.detail, /ready for (the next )?(scan|operation)/i);
  assert.ok(isNeutral(feedback));
});

test("the neutral startup reading never claims a device or output state", () => {
  const feedback = deriveConsoleFeedback(freshLaunch(), workstation(), false, false);
  assert.doesNotMatch(feedback.detail, /\bON\b/);
  assert.doesNotMatch(feedback.detail, /\bOFF\b/);
  assert.doesNotMatch(feedback.detail, /safe|normal|ready\b/i);
  assert.doesNotMatch(feedback.detail, /\d+\s*(%|kV|µA|uA|°C)/);
});

test("contradictory safety enablement and output readback is red", () => {
  const reading = deriveConsoleFeedback(
    snapshot({ safety: { xrayAvailable: false, xrayEnabled: true, interlockOk: false, lockReason: "Stage 1 · X-ray locked" } }),
    workstation({ xray: { beamOn: false, beamState: "off" } }),
    false,
    false,
  );
  assert.ok(isRed(reading), `contradictory safety output must be red, got ${reading.state}/${reading.tone}`);
});

test("complete first-start evidence allows an unknown beam only while no link exists", () => {
  const startup = snapshot({
    safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "Stage 1 · X-ray locked" },
  });
  const unlinked = workstation({ xray: { beamState: "unknown" } });
  assert.ok(isNeutral(deriveConsoleFeedback(startup, unlinked, false, false)));
  assert.ok(isRed(deriveConsoleFeedback(startup, unlinked, false, true)), "post-session unknown output stays red");
});

/* ------------------------------------------------------------------ *
 * Row 2 — connecting / initialising: NEUTRAL, if such a state exists   *
 * ------------------------------------------------------------------ */

test("an initialising link with no connection evidence stays neutral and never claims a device", () => {
  // `EnginePhase` has no `connecting` member (src/engine/types.ts): the engine
  // only ever reports `idle` before it has touched a device, and every other
  // phase is by definition post-connection. So the reachable "initialising"
  // reading is a disconnected engine still in `idle`, and it is the same neutral
  // reading as row 1 — never a red fault, and never a claim about a device.
  const feedback = deriveConsoleFeedback(freshLaunch(), workstation(), false, false);
  assert.ok(isNeutral(feedback), `initialising must stay neutral, got ${feedback.state}/${feedback.tone}`);
  assert.doesNotMatch(feedback.detail, /fault/i);
});

test("a phase that can only be reached through a link is never reported as connecting", () => {
  // Fail-closed precedence: `ready_for_home` / `ready` prove the engine engaged a
  // link, so a `disconnected` snapshot carrying one of them is a connection that
  // was LOST, not an initialising one. Reporting these as neutral would be
  // fail-open, which is exactly what this change must not introduce.
  for (const phase of ["ready_for_home", "ready", "running", "paused", "finishing", "stopping", "stopped", "completed"]) {
    const feedback = deriveConsoleFeedback(
      snapshot({ phase, phaseLabel: phase, connectionState: "disconnected" }),
      workstation(),
      false,
      false,
    );
    assert.ok(isRed(feedback), `${phase} proves a link existed and must stay red, got ${feedback.state}/${feedback.tone}`);
    assert.match(feedback.detail, /unavailable|unknown/i);
  }
});

/* ------------------------------------------------------------------ *
 * Row 3 — connected with fresh feedback: the real device reading        *
 * ------------------------------------------------------------------ */

test("a fresh connected snapshot renders the real device feedback", () => {
  const connected = snapshot({
    connectionState: "connected",
    modeLabel: "PRODUCTION · NANO + D7100 + MOXTEK",
    phase: "running",
    phaseLabel: "Running",
    preflightPassed: true,
    homed: true,
    requiresPreflight: false,
    requiresHome: false,
  });
  const ws = workstation({
    dataState: "scanning",
    phaseTone: "warn",
    xray: { connected: true, beamState: "on", beamOn: true, setpointConfirmed: true },
    progress: { captured: 3, total: 10, percent: 30, angleDeg: -54 },
    scene: { angleDeg: -54, rotated: true, angleKnown: true, rotationDirection: -1 },
  });
  const feedback = deriveConsoleFeedback(connected, ws, false);
  assert.equal(feedback.state, "Running");
  assert.equal(feedback.tone, "accent");
  assert.equal(feedback.active, true);

  // Normal stays normal: idle-but-connected is the plain queue-idle reading.
  const idle = deriveConsoleFeedback(
    snapshot({ connectionState: "connected", phase: "idle", phaseLabel: "Idle" }),
    workstation({ xray: { connected: true, beamState: "off", beamOn: false, setpointConfirmed: true } }),
    false,
  );
  assert.equal(idle.state, "Stopped");
  assert.equal(idle.tone, "muted");

  // A genuine fault while connected is still a red fault.
  const faulted = deriveConsoleFeedback(
    snapshot({ connectionState: "connected", phase: "fault", phaseLabel: "Fault" }),
    workstation({ dataState: "fault", phaseTone: "danger" }),
    false,
  );
  assert.ok(isRed(faulted));
});

/* ------------------------------------------------------------------ *
 * Row 4 — fail-closed states. This block is the NON-VACUITY CONTROL:    *
 * making everything neutral would fail here.                           *
 * ------------------------------------------------------------------ */

test("non-vacuity: every genuinely uncertain state still fails closed to red", () => {
  const cases = [
    {
      label: "`lost`",
      build: () => [afterConnection("lost"), workstation()],
    },
    {
      label: "`degraded`",
      build: () => [afterConnection("degraded"), workstation()],
    },
    {
      label: "stale / unknown feedback on a disconnected link",
      build: () => [freshLaunch(), workstation(), true],
    },
    {
      label: "stale / unknown feedback on a degraded link",
      build: () => [afterConnection("degraded"), workstation(), true],
    },
    {
      label: "stale / unknown feedback while nominally connected",
      build: () => [afterConnection("connected"), workstation(), true],
    },
    {
      label: "`disconnected` after a connection existed",
      build: () => [afterConnection("disconnected"), workstation()],
    },
    {
      label: "real phase fault while nominally disconnected",
      build: () => [
        snapshot({ phase: "fault", phaseLabel: "Fault" }),
        workstation({ dataState: "fault", phaseTone: "danger" }),
      ],
    },
    {
      label: "latched X-ray fault",
      build: () => [
        snapshot({ phase: "fault", phaseLabel: "Fault" }),
        workstation({
          dataState: "fault",
          phaseTone: "danger",
          xray: { latched: true, beamState: "off", beamOn: false },
        }),
      ],
    },
    {
      label: "unconfirmed beam",
      build: () => [
        snapshot({ connectionState: "connected", phase: "ready", phaseLabel: "Ready" }),
        workstation({
          phaseTone: "danger",
          xray: { beamState: "unknown", beamOn: false, setpointConfirmed: false },
        }),
      ],
    },
  ];

  for (const { label, build } of cases) {
    const [snap, ws, stale = false] = build();
    const feedback = deriveConsoleFeedback(snap, ws, stale);
    assert.ok(isRed(feedback), `${label} must stay fail-closed red, got ${feedback.state}/${feedback.tone}`);
    assert.equal(feedback.detail.length > 0, true, `${label} must still explain itself`);
  }
});

test("a `disconnected` snapshot that proves a link existed is never neutral", () => {
  // Phase, preflight, and HOME evidence must reject an explicit false result.
  // A retained connection log is represented as unknown tracker evidence.
  const evidence = [
    [snapshot({ phase: "stopped", phaseLabel: "Stopped" }), false],
    [snapshot({ phase: "completed", phaseLabel: "Completed" }), false],
    [snapshot({ phase: "running", phaseLabel: "Running" }), false],
    [snapshot({ preflightPassed: true }), false],
    [snapshot({ homed: true }), false],
    [snapshot({ logs: [{ id: "i", timestamp: "now", level: "info", source: "系统", message: "Nano v1 connected · HOME not executed" }] }), "unknown"],
  ];
  for (const [index, [snap, sessionEvidence]] of evidence.entries()) {
    const feedback = deriveConsoleFeedback(snap, workstation(), false, sessionEvidence);
    assert.ok(isRed(feedback), `evidence case ${index} must stay red, got ${feedback.state}/${feedback.tone}`);
  }
});

test("explicit session evidence distinguishes a fresh launch from a post-session disconnect", () => {
  const connected = deriveConsoleFeedback(
    snapshot({ connectionState: "connected", phase: "idle" }),
    workstation(),
    false,
    true,
  );
  assert.equal(connected.state, "Stopped");

  const disconnected = deriveConsoleFeedback(
    snapshot({ connectionState: "disconnected", phase: "idle" }),
    workstation(),
    false,
    true,
  );
  assert.ok(isRed(disconnected));

  const freshAgain = deriveConsoleFeedback(
    snapshot({ connectionState: "disconnected", phase: "idle" }),
    workstation(),
    false,
    false,
  );
  assert.ok(isNeutral(freshAgain));
  assert.ok(isRed(deriveConsoleFeedback(snapshot(), workstation(), false)), "missing evidence must fail closed");
});

/* ------------------------------------------------------------------ *
 * Invariants that must survive the change                              *
 * ------------------------------------------------------------------ */

test("the derivation only ever reports one of the eight console states", () => {
  const snapshots = [
    freshLaunch(),
    snapshot({ phase: "ready_for_home" }),
    afterConnection("connected"),
    afterConnection("lost"),
    afterConnection("degraded"),
    afterConnection("disconnected"),
    snapshot({ phase: "fault" }),
  ];
  for (const snap of snapshots) {
    for (const stale of [false, true]) {
      const feedback = deriveConsoleFeedback(snap, workstation(), stale);
      assert.ok(
        STATE_NAMES.includes(feedback.state),
        `${feedback.state} is not one of the eight console feedback states`,
      );
      assert.ok(["accent", "warn", "danger", "muted", "ok"].includes(feedback.tone));
    }
  }
});

test("the neutral startup branch relaxes no gate and stays local to `disconnected`+`idle`", () => {
  // The evidence evaluator itself keeps `linkLost` true for every other link
  // state, and the neutral branch is reachable only for the two offline cases.
  const evidence = evaluateFeedbackEvidence(freshLaunch(), workstation(), false, false);
  assert.equal(evidence.linkLost, false);
  assert.equal(evidence.phaseFault, false);

  for (const connectionState of ["lost", "degraded"]) {
    const staleEvidence = evaluateFeedbackEvidence(afterConnection(connectionState), workstation(), false, false);
    assert.equal(staleEvidence.linkLost, true, `${connectionState} must remain linkLost`);
  }
  const staleEvidence = evaluateFeedbackEvidence(freshLaunch(), workstation(), true, false);
  assert.equal(staleEvidence.linkLost, true, "a stale snapshot must remain linkLost");
  assert.ok(isRed(deriveConsoleFeedback(freshLaunch(), workstation(), true, false)), "a stale snapshot must stay red");

  // The neutral reading carries no gate decision: `active` is false and the state
  // is one of the existing eight, so every panel rendering it keeps its own
  // preflight/HOME/parameter gating unchanged.
  const neutral = deriveConsoleFeedback(freshLaunch(), workstation(), false, false);
  assert.equal(neutral.active, false);

  // The change is confined to the console feedback derivation: no gate-relevant
  // helper (progress, tone mapping) was touched.
  assert.ok(derived.includes("requiresPreflight"), "the derivation must reconcile the preflight gate state");
  assert.ok(derived.includes("requiresHome"), "the derivation must reconcile the HOME gate state");
  assert.ok(derived.includes("snapshot.safety"), "the derivation must inspect safety evidence");
  assert.doesNotMatch(derived, /snapshot\.safety\.[A-Za-z_$][\w$]*\s*=(?!=)/, "the derivation must not rewrite safety state");
  assert.ok(!derived.includes("dispatch"), "the derivation must not send or enable a command");
  assert.ok(!source.includes("dock.home = true") && !source.includes("dock.play = true"), "dock gates untouched");
});
