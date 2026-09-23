import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { transformWithEsbuild } from "vite";

const rtsRoot = new URL("../src/engine/rts9060/", import.meta.url);

async function loadTypeScriptModule(fileName, imports = {}) {
  let source = await readFile(new URL(fileName, rtsRoot), "utf8");
  for (const [specifier, dataUrl] of Object.entries(imports)) {
    source = source.replaceAll(`from "${specifier}"`, `from "${dataUrl}"`);
  }
  const { code } = await transformWithEsbuild(source, fileName, { loader: "ts", target: "es2022", format: "esm" });
  return `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
}

const protocolUrl = await loadTypeScriptModule("protocol.ts");
const transportUrl = await loadTypeScriptModule("transport.ts", { "./protocol": protocolUrl });
const workflowUrl = await loadTypeScriptModule("workflow.ts", {
  "./devices": await loadTypeScriptModule("devices.ts"),
  "./protocol": protocolUrl,
  "./transport": transportUrl,
});
const adapterUrl = await loadTypeScriptModule("../workstationAdapter.ts", {
  "./rts9060/protocol": protocolUrl,
  "./rts9060/workflow": workflowUrl,
});
const { parseLine, PULSES_PER_REV } = await import(protocolUrl);
const { ScanWorkflow } = await import(workflowUrl);
const { WorkstationAdapter } = await import(adapterUrl);

const storage = new Map();
globalThis.window = {
  localStorage: {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, String(value)),
    removeItem: (key) => storage.delete(key),
  },
};

class ScriptedNanoTransport {
  label = "memory-only Nano transport";
  handler = null;
  state = "IDLE";
  position = 0;
  target = 0;
  homed = false;
  rearmed = false;
  holdFinalReturn = false;
  rejectStop = false;
  pendingFinalReturnId = null;
  commands = [];

  onLine(handler) {
    this.handler = handler;
    queueMicrotask(() => this.emit("READY VERSION=preview PROTOCOL=2 BUILD=test CAPS=HEARTBEAT_ACK,XRAY_WARNING"));
  }

  send(raw) {
    const line = parseLine(raw);
    const id = line.id ?? 0;
    this.commands.push({ token: line.token, rest: [...line.rest] });
    switch (line.token) {
      case "HEARTBEAT":
        this.emit(`HBACK ${id}`);
        break;
      case "STATUS":
        this.emit(`STATUS ${id} state=${this.state} pos=${this.position} target=${this.target} microsteps=8 ppr=${PULSES_PER_REV} reference=${this.homed ? 1 : 0} homed=${this.homed ? 1 : 0} rearmed=${this.rearmed ? 1 : 0} hall=0`);
        break;
      case "SET_MICROSTEPS":
        this.emit(`OK ${id} MICROSTEPS=8 PPR=${PULSES_PER_REV}`);
        break;
      case "REARM":
        this.rearmed = true;
        this.emit(`OK ${id} REARMED`);
        break;
      case "HOME":
        if (!this.rearmed) {
          this.emit(`ERR ${id} NOT_REARMED`);
          break;
        }
        this.position = 0;
        this.target = 0;
        this.homed = true;
        this.state = "IDLE";
        this.emit(`HOME_DONE ${id} POS=0`);
        break;
      case "MOVE_ABS":
        this.moveAbs(id, Number(line.rest[0]));
        break;
      case "CAPTURE_DONE":
        if (this.state !== "CAPTURE_HOLD") {
          this.emit(`ERR ${id} NOT_HOLDING`);
          break;
        }
        this.state = "IDLE";
        this.emit(`IDLE ${id} CAPTURE_RELEASED`);
        break;
      case "XRAY_WARNING":
        this.emit(`OK ${id} XRAY_WARNING ${line.rest[0] ?? "OFF"}`);
        break;
      case "STOP":
        this.state = "STOPPED";
        this.homed = false;
        this.rearmed = false;
        this.emit(`ACK ${id} STOP`);
        if (!this.rejectStop) this.emit(`STOPPED ${id} POSITION_UNKNOWN reason=STOP`);
        break;
      default:
        this.emit(`ERR ${id} UNKNOWN_COMMAND`);
    }
  }

  moveAbs(id, mdeg) {
    if (!Number.isFinite(mdeg) || mdeg < 0 || !this.homed || !this.rearmed || this.state === "STOPPED") {
      this.emit(`ERR ${id} INVALID_MOVE`);
      return;
    }
    const requested = Math.round((mdeg / 360_000) * PULSES_PER_REV);
    const targetWithinTurn = requested % PULSES_PER_REV;
    let turn = Math.max(Math.floor(this.position / PULSES_PER_REV), Math.floor(requested / PULSES_PER_REV));
    let destination = turn * PULSES_PER_REV + targetWithinTurn;
    if (destination < this.position) destination = ++turn * PULSES_PER_REV + targetWithinTurn;
    this.target = destination;
    this.state = "MOVING";
    if (mdeg === 360_000 && this.holdFinalReturn) {
      this.pendingFinalReturnId = id;
      return;
    }
    this.position = destination;
    this.state = "CAPTURE_HOLD";
    this.emit(`READY_TO_CAPTURE ${id} POS=${this.position}`);
  }

  releaseFinalReturn() {
    assert.notEqual(this.pendingFinalReturnId, null);
    this.position = this.target;
    this.state = "CAPTURE_HOLD";
    this.emit(`READY_TO_CAPTURE ${this.pendingFinalReturnId} POS=${this.position}`);
    this.pendingFinalReturnId = null;
  }

  close() {}

  emit(line) {
    this.handler?.(line);
  }
}

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

async function waitFor(predicate, message, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(message);
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

async function preparedWorkflow(projectionCount = 2) {
  storage.clear();
  const transport = new ScriptedNanoTransport();
  const workflow = new ScanWorkflow(() => undefined, transport);
  await waitFor(() => workflow.phase === "ready", "preview boot did not finish");
  workflow.setParams({
    savePath: "memory-only",
    taskId: "preview-lifecycle",
    projectionCount,
    exposureMs: 0.125,
    maxXraySec: 1,
  });
  await workflow.runPreflight();
  await workflow.home();
  return { workflow, transport };
}

async function snapshotFor(workflow) {
  const adapter = Object.create(WorkstationAdapter.prototype);
  adapter.workflow = workflow;
  return adapter.getSnapshot();
}

test("ETA uses completed projection time and includes a pending X-ray cooldown", async () => {
  const { workflow } = await preparedWorkflow(4);
  try {
    assert.equal((await snapshotFor(workflow)).progress.etaSeconds, null);
    workflow.startScan();
    await waitFor(() => workflow.captured >= 1, "first projection did not commit");
    const first = await snapshotFor(workflow);
    assert.ok(first.progress.etaSeconds > 0);
    assert.notEqual(first.workstation.progress.etaText, "—");
    await waitFor(() => workflow.cooldownUntil !== null, "preview did not enter the X-ray cooling wait", 6000);
    const cooling = await snapshotFor(workflow);
    assert.ok(cooling.progress.etaSeconds >= 300, "remaining time must include the five-minute cooling wait");
    assert.match(cooling.workstation.progress.etaText, /^\d+:\d{2}$/);
    await workflow.stop();
    assert.equal((await snapshotFor(workflow)).progress.etaSeconds, null);
  } finally {
    await workflow.stop();
    workflow.close();
  }
});

test("normal Stop cancels an uncommitted projection without fault-latching and requires preflight plus HOME", async () => {
  const { workflow, transport } = await preparedWorkflow();
  try {
    workflow.startScan();
    await waitFor(() => workflow.phase === "scanning" && workflow.source.beamOn, "scan did not reach its exposure window");

    await workflow.stop();

    assert.equal(workflow.phase, "stopped");
    assert.equal(workflow.source.beamOn, false);
    assert.equal(workflow.source.isLatched, false);
    assert.equal(workflow.captured, 0, "a projection without CAPTURE_DONE must not be committed");
    assert.equal(workflow.homed, false);
    assert.equal(workflow.preflightPassed, false);
    assert.equal(typeof workflow.estop, "undefined");
    assert.equal(typeof workflow.estopRelease, "undefined");

    await workflow.runPreflight();
    assert.equal(workflow.phase, "stopped");
    assert.equal(workflow.source.isLatched, false);
    await workflow.home();
    assert.equal(workflow.phase, "ready");
    assert.equal(workflow.homed, true);
    assert.ok(transport.commands.some(({ token }) => token === "STOP"));
  } finally {
    workflow.close();
  }
});

test("a device failure latches output until successful preflight and HOME", async () => {
  const { workflow } = await preparedWorkflow(1);
  try {
    workflow.camera.capture = async () => { throw new Error("simulated camera failure"); };
    workflow.startScan();
    await waitFor(() => workflow.phase === "fault" && !workflow.scanRunning, "camera failure did not settle as a fault");

    assert.equal(workflow.source.isLatched, true);
    assert.equal(workflow.homed, false);
    assert.equal(workflow.preflightPassed, false);
    const faultXray = (await snapshotFor(workflow)).workstation.xray;
    assert.equal(faultXray.manualControlsEnabled, false);
    assert.equal(faultXray.setpointControlsEnabled, false);
    assert.equal(faultXray.timerControlsEnabled, false);
    assert.throws(() => workflow.sendVoltage(60), /setpoint controls are unavailable/i);
    workflow.timerOn = true;
    workflow.timerToggle();
    assert.equal(workflow.timerOn, true, "timer state cannot be changed while the output fault is latched");
    assert.throws(() => workflow.startScan(), /fault latched/i);

    await workflow.runPreflight();
    assert.equal(workflow.source.isLatched, false);
    assert.equal(workflow.phase, "stopped", "preflight restores the gate but leaves HOME required");
    await workflow.home();
    assert.equal(workflow.phase, "ready");
    assert.equal(workflow.homed, true);
  } finally {
    workflow.close();
  }
});

test("pause waits for CAPTURE_DONE and resume retains each committed frame", async () => {
  const { workflow, transport } = await preparedWorkflow(2);
  const captureStarted = deferred();
  const releaseFirstCapture = deferred();
  try {
    workflow.camera.capture = async (index, angleDeg, exposureMs) => {
      if (index === 1) {
        captureStarted.resolve();
        await releaseFirstCapture.promise;
      }
      return {
        index,
        angleDeg,
        exposureMs,
        fileName: `frame-${String(index).padStart(4, "0")}.nef`,
        path: `memory-only/frame-${String(index).padStart(4, "0")}.nef`,
        shaOk: true,
      };
    };

    workflow.startScan();
    await captureStarted.promise;
    workflow.pause();
    assert.equal(workflow.phase, "scanning", "pause request must not advance phase mid-projection");
    assert.equal(workflow.source.beamOn, true, "the preview transaction remains intact until its commit boundary");
    releaseFirstCapture.resolve();
    await waitFor(() => workflow.phase === "paused", "scan did not pause after the completed projection");

    assert.equal(workflow.captured, 1);
    assert.deepEqual(workflow.frames.map(({ index }) => index), [1]);
    assert.equal(workflow.source.beamOn, false);
    assert.ok(transport.commands.some(({ token }) => token === "CAPTURE_DONE"));
    const pausedXray = (await snapshotFor(workflow)).workstation.xray;
    assert.equal(pausedXray.manualControlsEnabled, false);
    assert.equal(pausedXray.setpointControlsEnabled, false);
    assert.equal(pausedXray.timerControlsEnabled, false);

    workflow.startScan();
    await waitFor(() => workflow.phase === "completed", "resumed scan did not complete");
    assert.equal(workflow.captured, 2);
    assert.deepEqual(workflow.frames.map(({ index }) => index), [1, 2]);
    assert.equal(workflow.angleDeg, 0);
  } finally {
    workflow.close();
  }
});

test("completion stays stoppable while returning through the confirmed zero orientation", async () => {
  const { workflow, transport } = await preparedWorkflow(1);
  try {
    transport.holdFinalReturn = true;
    workflow.camera.capture = async (index, angleDeg, exposureMs) => ({
      index, angleDeg, exposureMs,
      fileName: `frame-${String(index).padStart(4, "0")}.nef`,
      path: `memory-only/frame-${String(index).padStart(4, "0")}.nef`,
      shaOk: true,
    });
    workflow.startScan();
    await waitFor(() => workflow.phase === "finishing", "scan never entered the visible finalization phase");
    assert.equal(workflow.captured, 1);
    assert.equal(workflow.source.beamOn, false);
    assert.equal(transport.pendingFinalReturnId !== null, true);

    transport.releaseFinalReturn();
    await waitFor(() => workflow.phase === "completed", "scan did not complete after zero-return confirmation");
    assert.equal(workflow.angleDeg, 0);
    assert.equal(transport.position, PULSES_PER_REV, "the simulator confirms a full forward turn back to the zero orientation");
    assert.equal(transport.commands.filter(({ token }) => token === "CAPTURE_DONE").length, 2);
    assert.equal(transport.commands.filter(({ token, rest }) => token === "MOVE_ABS" && rest[0] === "360000").length, 1);
    const completedXray = (await snapshotFor(workflow)).workstation.xray;
    assert.equal(completedXray.manualControlsEnabled, true);
    assert.equal(completedXray.setpointControlsEnabled, true);
    assert.equal(completedXray.timerControlsEnabled, true);

    await workflow.stop();
    assert.equal(workflow.phase, "completed", "Stop is a no-op after completion");
  } finally {
    workflow.close();
  }
});

test("normal Stop during finalization cancels the return move without becoming a device fault", async () => {
  const { workflow, transport } = await preparedWorkflow(1);
  try {
    transport.holdFinalReturn = true;
    workflow.camera.capture = async (index, angleDeg, exposureMs) => ({
      index, angleDeg, exposureMs,
      fileName: `frame-${String(index).padStart(4, "0")}.nef`,
      path: `memory-only/frame-${String(index).padStart(4, "0")}.nef`,
      shaOk: true,
    });
    workflow.startScan();
    await waitFor(() => workflow.phase === "finishing", "scan never entered finalization");

    await workflow.stop();

    assert.equal(workflow.phase, "stopped");
    assert.equal(workflow.source.isLatched, false);
    assert.equal(workflow.source.beamOn, false);
    assert.equal(workflow.captured, 1, "the committed frame remains available after stopping finalization");
    assert.equal(workflow.homed, false);
    assert.equal(workflow.preflightPassed, false);
  } finally {
    workflow.close();
  }
});

test("finishing and stopping phases lock manual X-ray, setpoint, and timer controls at the adapter and workflow", async () => {
  const { workflow } = await preparedWorkflow(1);
  try {
    workflow.timerOn = true;
    const before = workflow.source.readback();
    for (const phase of ["finishing", "stopping"]) {
      workflow.phase = phase;
      const xray = (await snapshotFor(workflow)).workstation.xray;
      assert.equal(xray.manualControlsEnabled, false, `${phase} must lock manual beam control`);
      assert.equal(xray.setpointControlsEnabled, false, `${phase} must lock voltage/current inputs`);
      assert.equal(xray.timerControlsEnabled, false, `${phase} must lock timer control`);

      await workflow.xrayToggle();
      assert.equal(workflow.source.beamOn, false, `${phase} must not enable the preview beam`);
      assert.throws(() => workflow.sendVoltage(65), /setpoint controls are unavailable/i);
      assert.throws(() => workflow.sendCurrent(150), /setpoint controls are unavailable/i);
      workflow.timerToggle();
      assert.equal(workflow.timerOn, true, `${phase} must preserve the timer state`);
      assert.equal(workflow.source.readback().setKv, before.setKv);
      assert.equal(workflow.source.readback().setUa, before.setUa);
    }
  } finally {
    workflow.close();
  }
});
