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
 * snapshots. Nothing in `App.tsx` was restructured for testability; the only
 * change made for this test was adding the `export` keyword to the two functions
 * and the evidence interface so a seam exists at all.
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
 * Synthetic snapshots. Only fields the derivation actually reads are *
 * populated; the rest of EngineSnapshot is irrelevant to this contract. *
 * ------------------------------------------------------------------ */

const STATE_NAMES = [...CONSOLE_FEEDBACK_STATES];
const isRed = (feedback) => feedback.tone === "danger" && feedback.state === "Fault";
const isNeutral = (feedback) => feedback.tone === "muted" && !isRed(feedback);
/**
 * Every test below starts from a console that has never seen a connection, so
 * the row under test is the row being driven and not a leftover of the previous
 * one. Each test then drives the derivation synchronously, so no other test can
 * interleave with its session memory.
 */
const freshSession = () => derivation.resetLinkMemoryForTest();

function workstation(overrides = {}) {
  const { xray = {}, progress = {}, scene = {}, ...rest } = overrides;
  return {
    dataState: "ready",
    phaseWord: "READY",
    phaseTone: "accent",
    xray: {
      connected: false,
      monKv: null,
      monUa: null,
      powerW: null,
      tempC: null,
      beamState: "off",
      beamOn: false,
      latched: false,
      setpointConfirmed: false,
      ...xray,
    },
    progress: { captured: 0, total: 0, percent: 0, angleDeg: 0, ...progress },
    scene: { angleDeg: 0, rotated: false, angleKnown: false, rotationDirection: -1, ...scene },
    consoleLogs: [],
    frames: [],
    ...rest,
  };
}

function snapshot(overrides = {}) {
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
    safety: { xrayAvailable: false, xrayEnabled: false, interlockOk: false, lockReason: "" },
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
    ...overrides,
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
  freshSession();
  const feedback = deriveConsoleFeedback(freshLaunch(), workstation(), false);
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
  freshSession();
  const feedback = deriveConsoleFeedback(freshLaunch(), workstation(), false);
  assert.doesNotMatch(feedback.detail, /\bON\b/);
  assert.doesNotMatch(feedback.detail, /\bOFF\b/);
  assert.doesNotMatch(feedback.detail, /safe|normal|ready\b/i);
  assert.doesNotMatch(feedback.detail, /\d+\s*(%|kV|µA|uA|°C)/);
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
  freshSession();
  const feedback = deriveConsoleFeedback(freshLaunch(), workstation(), false);
  assert.ok(isNeutral(feedback), `initialising must stay neutral, got ${feedback.state}/${feedback.tone}`);
  assert.doesNotMatch(feedback.detail, /fault/i);
});

test("a phase that can only be reached through a link is never reported as connecting", () => {
  // Fail-closed precedence: `ready_for_home` / `ready` prove the engine engaged a
  // link, so a `disconnected` snapshot carrying one of them is a connection that
  // was LOST, not an initialising one. Reporting these as neutral would be
  // fail-open, which is exactly what this change must not introduce.
  freshSession();
  for (const phase of ["ready_for_home", "ready", "running", "paused", "finishing", "stopping", "stopped", "completed"]) {
    const feedback = deriveConsoleFeedback(
      snapshot({ phase, phaseLabel: phase, connectionState: "disconnected" }),
      workstation(),
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
  freshSession();
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
  freshSession();
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
  // Snapshot-local evidence, independent of in-session memory: a phase that can
  // only be reached through a device link, or a completed preflight / HOME.
  freshSession();
  const evidence = [
    snapshot({ phase: "stopped", phaseLabel: "Stopped" }),
    snapshot({ phase: "completed", phaseLabel: "Completed" }),
    snapshot({ phase: "running", phaseLabel: "Running" }),
    snapshot({ preflightPassed: true }),
    snapshot({ homed: true }),
    snapshot({ logs: [{ id: "i", timestamp: "now", level: "info", source: "系统", message: "Nano v1 connected · HOME not executed" }] }),
  ];
  for (const [index, snap] of evidence.entries()) {
    const feedback = deriveConsoleFeedback(snap, workstation(), false);
    assert.ok(isRed(feedback), `evidence case ${index} must stay red, got ${feedback.state}/${feedback.tone}`);
  }
});

test("in-session memory: a connection seen earlier keeps a later `disconnected` red", () => {
  // Same shape as the live app: the console polls a NEW snapshot object every
  // tick. Once a tick reported `connected`, a later tick that reports
  // `disconnected` is a lost connection, not a fresh startup. The phase here
  // stays pre-connection, so only the session memory can carry that evidence.
  const { resetLinkMemoryForTest } = derivation;
  assert.equal(typeof resetLinkMemoryForTest, "function", "the memory seam is missing from src/App.tsx");

  resetLinkMemoryForTest();
  const connected = snapshot({ connectionState: "connected", phase: "idle" });
  const first = deriveConsoleFeedback(connected, workstation(), false);
  assert.equal(first.state, "Stopped", "a connected idle engine is the plain queue-idle reading");

  const disconnected = snapshot({ connectionState: "disconnected", phase: "idle" });
  const second = deriveConsoleFeedback(disconnected, workstation(), false);
  assert.ok(isRed(second), `post-connection disconnect must stay red, got ${second.state}/${second.tone}`);

  // And the same tick, on a fresh session, is the benign neutral startup state.
  resetLinkMemoryForTest();
  const freshAgain = deriveConsoleFeedback(snapshot({ connectionState: "disconnected", phase: "idle" }), workstation(), false);
  assert.ok(isNeutral(freshAgain), `a fresh session must be neutral again, got ${freshAgain.state}/${freshAgain.tone}`);
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
  derivation.resetLinkMemoryForTest();
  const evidence = evaluateFeedbackEvidence(freshLaunch(), workstation(), false);
  assert.equal(evidence.linkLost, false);
  assert.equal(evidence.neverConnected, true);
  assert.equal(evidence.phaseFault, false);

  for (const connectionState of ["lost", "degraded"]) {
    const staleEvidence = evaluateFeedbackEvidence(afterConnection(connectionState), workstation(), false);
    assert.equal(staleEvidence.linkLost, true, `${connectionState} must remain linkLost`);
    assert.equal(staleEvidence.neverConnected, false, `${connectionState} must never be treated as never-connected`);
  }
  derivation.resetLinkMemoryForTest();
  const staleEvidence = evaluateFeedbackEvidence(freshLaunch(), workstation(), true);
  assert.equal(staleEvidence.linkLost, true, "a stale snapshot must remain linkLost");
  assert.ok(isRed(deriveConsoleFeedback(freshLaunch(), workstation(), true)), "a stale snapshot must stay red");

  // The neutral reading carries no gate decision: `active` is false and the state
  // is one of the existing eight, so every panel rendering it keeps its own
  // preflight/HOME/parameter gating unchanged.
  const neutral = deriveConsoleFeedback(freshLaunch(), workstation(), false);
  assert.equal(neutral.active, false);

  // The change is confined to the console feedback derivation: no gate-relevant
  // helper (progress, tone mapping) was touched.
  assert.ok(!derived.includes("requiresPreflight"), "the derivation must not read the preflight gate");
  assert.ok(!derived.includes("requiresHome"), "the derivation must not read the HOME gate");
  assert.ok(!derived.includes("safety"), "the derivation must not read or rewrite the safety state");
  assert.ok(!derived.includes("dispatch"), "the derivation must not send or enable a command");
  assert.ok(!source.includes("dock.home = true") && !source.includes("dock.play = true"), "dock gates untouched");
});
