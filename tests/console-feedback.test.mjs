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
  for (const token of ["evaluateFeedbackEvidence", "deriveConsoleFeedback", "resetLinkMemoryForTest"]) {
    assert.ok(extracted.includes(token), `shipped feedback region is missing ${token}`);
  }
  assert.ok(!/^\s*import\s/m.test(extracted), "feedback region unexpectedly contains imports");

  const { code } = await transformWithEsbuild(
    extracted,
    "App.tsx.feedback-excerpt.ts",
    { loader: "ts", target: "es2022", format: "esm" },
  );
  const url = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
  return import(url);
}

const feedbackModule = await loadShippedFeedback();
const { deriveConsoleFeedback, resetLinkMemoryForTest } = feedbackModule;
const ENGINE_PHASES = [
  "idle", "ready_for_home", "ready", "running", "paused", "finishing", "stopping", "stopped", "completed", "fault",
];
const CONNECTION_STATES = ["disconnected", "connected", "degraded", "lost"];
const BEAM_STATES = ["off", "on", "unknown"];
const isNeutral = (feedback) =>
  feedback.state === "Stopped" && feedback.tone === "muted" && feedback.active === false;

function snapshot(overrides = {}) {
  return {
    connectionState: "disconnected",
    phase: "idle",
    preflightPassed: false,
    homed: false,
    devices: [
      { id: "turntable", label: "Precision Turntable", state: "offline", detail: "disconnected" },
      { id: "camera", label: "Nikon D7100", state: "offline", detail: "disconnected" },
      { id: "xray", label: "Moxtek 12 W", state: "locked", detail: "fail-closed · connect confirms OFF" },
    ],
    progress: { current: 0, total: 0, percent: 0, angleDeg: 0, etaSeconds: null },
    imageCount: 0,
    logs: [],
    lastError: null,
    ...overrides,
  };
}

function workstation(overrides = {}) {
  const { xray = {}, progress = {}, scene = {}, ...rest } = overrides;
  return {
    dataState: "ready",
    phaseTone: "accent",
    xray: {
      connected: false,
      monKv: null,
      monUa: null,
      powerW: null,
      tempC: null,
      latched: false,
      beamState: "off",
      beamOn: false,
      setpointConfirmed: false,
      ...xray,
    },
    progress: { captured: 0, total: 0, percent: 0, ...progress },
    scene: { angleDeg: 0, angleKnown: false, ...scene },
    consoleLogs: [],
    frames: [],
    ...rest,
  };
}

function assertRedFailClosed(reading, label) {
  assert.equal(reading.state, "Fault", `${label} state was ${reading.state}`);
  assert.equal(reading.tone, "danger", `${label} tone was ${reading.tone}`);
}

function startConnectionSession() {
  resetLinkMemoryForTest();
  const reading = deriveConsoleFeedback(
    snapshot({ connectionState: "connected" }),
    workstation(),
    false,
  );
  assert.equal(reading.state, "Stopped");
}

test("a never-connected idle snapshot with a latched X-ray fault is red", () => {
  resetLinkMemoryForTest();
  const reading = deriveConsoleFeedback(
    snapshot(),
    workstation({ xray: { latched: true } }),
    false,
  );
  assertRedFailClosed(reading, "latched X-ray fault");
});

test("a never-connected idle snapshot with unknown beam under danger tone is red", () => {
  resetLinkMemoryForTest();
  const reading = deriveConsoleFeedback(
    snapshot(),
    workstation({ phaseTone: "danger", xray: { beamState: "unknown" } }),
    false,
  );
  assertRedFailClosed(reading, "unknown beam under danger tone");
});

test("a never-connected idle snapshot with unconfirmed beamOn is red", () => {
  resetLinkMemoryForTest();
  const reading = deriveConsoleFeedback(
    snapshot(),
    workstation({ xray: { beamOn: true, setpointConfirmed: false } }),
    false,
  );
  assertRedFailClosed(reading, "unconfirmed beamOn");
});

test("non-vacuity: a never-connected idle snapshot with no dangerous fields stays neutral", () => {
  resetLinkMemoryForTest();
  const reading = deriveConsoleFeedback(snapshot(), workstation(), false);
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
  resetLinkMemoryForTest();
  const reading = deriveConsoleFeedback(
    snapshot({ phase: "fault" }),
    workstation({ dataState: "fault", phaseTone: "danger" }),
    false,
  );
  assertRedFailClosed(reading, "real fault phase");
});

test("the shipped derivation exhaustively partitions the full Cartesian console space", () => {
  const neutralTuples = [];
  let traversed = 0;
  let disconnectedRed = 0;
  let redAssertions = 0;

  for (const beamState of BEAM_STATES) {
    for (const beamOn of [false, true]) {
      for (const setpointConfirmed of [false, true]) {
        for (const latched of [false, true]) {
          for (const phase of ENGINE_PHASES) {
            for (const connectionState of CONNECTION_STATES) {
              for (const stale of [false, true]) {
                traversed += 1;
                const tuple = { beamState, beamOn, setpointConfirmed, latched, phase, connectionState, stale };
                const snap = snapshot({ connectionState, phase });
                const ws = workstation({
                  xray: {
                    connected: connectionState === "connected",
                    beamState,
                    beamOn,
                    setpointConfirmed,
                    latched,
                  },
                });
                // Reset before each tuple so connected rows cannot manufacture
                // history for the next never-connected disconnected row.
                resetLinkMemoryForTest();
                const feedback = deriveConsoleFeedback(snap, ws, stale);
                const isInitial =
                  beamOn === false &&
                  setpointConfirmed === false &&
                  latched === false &&
                  phase === "idle" &&
                  connectionState === "disconnected" &&
                  stale === false &&
                  (beamState === "off" || beamState === "unknown");

                if (connectionState === "disconnected") {
                  if (isInitial) {
                    neutralTuples.push(tuple);
                    assert.ok(isNeutral(feedback), `initial tuple must be neutral: ${JSON.stringify(tuple)}`);
                    assert.match(feedback.detail, /not connected/i);
                    continue;
                  }
                  disconnectedRed += 1;
                  redAssertions += 1;
                  assertRedFailClosed(feedback, `non-initial disconnected tuple ${JSON.stringify(tuple)}`);
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
                  assertRedFailClosed(feedback, `fail-closed tuple ${JSON.stringify(tuple)}`);
                }
              }
            }
          }
        }
      }
    }
  }

  const expectedNeutral = [
    { beamState: "off", beamOn: false, setpointConfirmed: false, latched: false, phase: "idle", connectionState: "disconnected", stale: false },
    { beamState: "unknown", beamOn: false, setpointConfirmed: false, latched: false, phase: "idle", connectionState: "disconnected", stale: false },
  ];
  // The brief labels this space 960, but 3×2×2×2×10×4×2 is 1,920.
  // Traverse the complete product so neither half of the stale axis count is missed.
  assert.equal(traversed, 1920, "the full 3×2×2×2×10×4×2 product must run");
  assert.deepEqual(neutralTuples, expectedNeutral, "only the off and startup-unknown tuples are neutral");
  assert.equal(disconnectedRed, 478, "all disconnected tuples except the two initial tuples must be red");
  assert.ok(redAssertions > disconnectedRed, "the sweep must also assert non-disconnected fail-closed cases");
  console.log(
    `[console-feedback sweep] traversed=${traversed}; neutral=${JSON.stringify(neutralTuples)}; disconnectedRed=${disconnectedRed}; redAssertions=${redAssertions}`,
  );
});

test("strict startup accepts drafts but rejects contradictory initial-state evidence", () => {
  resetLinkMemoryForTest();
  const drafts = snapshot({
    parameters: { taskId: "draft", savePath: "", projectionCount: 900, angleStepDeg: 0, exposureMs: 0 },
  });
  const draftWorkstation = workstation({ xray: { setKv: 80, setUa: 300 } });
  assert.ok(isNeutral(deriveConsoleFeedback(drafts, draftWorkstation, false)), "draft settings are not device readbacks");

  const contradictions = [
    [snapshot({ lastError: "startup failed" }), workstation()],
    [snapshot({ logs: [{ id: "e", timestamp: "now", level: "error", source: "系统", message: "error" }] }), workstation()],
    [snapshot({ logs: [{ id: "i", timestamp: "now", level: "info", source: "系统", message: "Nano v1 connected · HOME not executed" }] }), workstation()],
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
  for (const [snap, ws] of contradictions) {
    resetLinkMemoryForTest();
    assertRedFailClosed(deriveConsoleFeedback(snap, ws, false), "initial-state contradiction");
  }
});

test("non-vacuity: connected ordinary phases retain their real console states", () => {
  for (const [phase, expectedState] of [
    ["running", "Running"],
    ["paused", "Paused"],
    ["stopped", "Stopped"],
    ["completed", "Completed"],
  ]) {
    resetLinkMemoryForTest();
    const feedback = deriveConsoleFeedback(
      snapshot({ connectionState: "connected", phase }),
      workstation({ xray: { connected: true, beamState: "off", beamOn: false, setpointConfirmed: true } }),
      false,
    );
    assert.equal(feedback.state, expectedState, `${phase} should remain ${expectedState}`);
    assert.notEqual(feedback.tone, "danger");
  }
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
    resetLinkMemoryForTest();
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
