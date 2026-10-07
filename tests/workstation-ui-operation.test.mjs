/** Real UI waiter rejects queue acceptance, old sessions and unconfirmed cleanup as success. */
import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { transformWithEsbuild } from "vite";
import { loadSourceModule } from "./load-source-module.mjs";
const { awaitOperation } = await loadSourceModule("src/camera-draft.ts");
const { createPreparationRequestScope, runScanPreparationRequest } = await loadSourceModule("src/ScanPreparationDialog.tsx");
const { presetReadbackEvidence } = await loadSourceModule("src/CameraSettingsDialog.tsx");
const snapshot = status => ({ engineSessionId: "engine-A", operations: [{ operationId: "operation-1", kind: "camera_capture", status, error: status === "cleanup_unconfirmed" ? "C102 cleanup unconfirmed" : null }] });
test("queued acceptance waits for the same operation's completed snapshot", async () => {
  let current = snapshot("queued"); let complete = false;
  const result = awaitOperation(current, "operation-1", () => current, 500).then(value => { complete = true; return value; });
  await new Promise(resolve => setTimeout(resolve, 20)); assert.equal(complete, false);
  current = snapshot("completed"); assert.equal((await result).operations[0].status, "completed");
});
test("cleanup unknown, cancelled, failed and rejected receipts cannot claim completion", async () => {
  for (const status of ["cleanup_unconfirmed", "cancelled", "failed", "rejected"]) await assert.rejects(awaitOperation(snapshot(status), "operation-1", () => snapshot(status)), /unconfirmed|cancelled|failed|rejected/);
  await assert.rejects(awaitOperation(snapshot("completed"), "unrelated", () => snapshot("completed")), /receipt unavailable/);
});
test("a new engine session discards a late completed response", async () => {
  await assert.rejects(awaitOperation(snapshot("running"), "operation-1", () => ({ ...snapshot("completed"), engineSessionId: "engine-B" }), 500), /session changed/);
});

test("preset comparison rejects retained equal values when readback is unconfirmed or has an error", () => {
  const confirmed = { supported: true, readable: true, confirmed: true, actual: 800, error: null };
  for (const property of [undefined, { ...confirmed, confirmed: false }, { ...confirmed, error: "readback timeout" },
    { ...confirmed, supported: false }, { ...confirmed, readable: false }, { ...confirmed, actual: null }]) {
    assert.deepEqual(presetReadbackEvidence(property, 800), { actual: null, comparison: "UNKNOWN" });
  }
});

test("preset comparison uses confirmed actual values for Match and Different including native zero and strings", () => {
  const property = { supported: true, readable: true, confirmed: true, actual: 800, error: null, target: 100 };
  assert.deepEqual(presetReadbackEvidence(property, 800), { actual: 800, comparison: "Match" });
  assert.deepEqual(presetReadbackEvidence(property, 100), { actual: 800, comparison: "Different" });
  assert.deepEqual(presetReadbackEvidence({ ...property, actual: 0 }, 0), { actual: 0, comparison: "Match" });
  assert.deepEqual(presetReadbackEvidence({ ...property, actual: "6000x4000", error: "" }, "6000x4000"), { actual: "6000x4000", comparison: "Match" });
});

/** Execute the actual component with deterministic hook/DOM lifetimes; browser QA verifies real focus/selection separately. */
async function dialogHarness() {
  const saved = Object.fromEntries(["document", "HTMLElement", "__microCtModalHooks"].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const listeners = new Set(); const slots = []; let cursor = 0; let pendingEffects = [];
  const document = { activeElement: null, adds: 0, removes: 0, addEventListener(_name, fn) { this.adds++; listeners.add(fn); }, removeEventListener(_name, fn) { this.removes++; listeners.delete(fn); } };
  class Element { isConnected = true; focuses = 0; disabled = false; focus() { this.focuses++; document.activeElement = this; } }
  const trigger = new Element(); const stop = new Element(); const close = new Element(); const input = new Element();
  const modal = new Element(); modal.querySelectorAll = () => [stop, close, input].filter(element => !element.disabled);
  document.activeElement = trigger;
  globalThis.document = document; globalThis.HTMLElement = Element;
  globalThis.__microCtModalHooks = {
    useRef(initial) { const index = cursor++; return slots[index] ||= { current: initial }; },
    useEffect(effect, dependencies) {
      const index = cursor++; const previous = slots[index];
      if (previous && dependencies.length === previous.dependencies.length && dependencies.every((value, position) => Object.is(value, previous.dependencies[position]))) return;
      pendingEffects.push(() => { previous?.cleanup?.(); slots[index] = { dependencies, cleanup: effect() }; });
    },
  };
  const source = await readFile(new URL("../src/WorkstationDialog.tsx", import.meta.url), "utf8");
  const { code } = await transformWithEsbuild(source, "WorkstationDialog.tsx", { loader: "tsx", jsx: "automatic", target: "es2022", format: "esm" });
  const hookUrl = `data:text/javascript;base64,${Buffer.from('export function useRef(value){return globalThis.__microCtModalHooks.useRef(value)} export function useEffect(effect,deps){return globalThis.__microCtModalHooks.useEffect(effect,deps)}').toString("base64")}`;
  const require = createRequire(import.meta.url);
  const output = code.replaceAll('"react"', `"${hookUrl}"`).replaceAll('"react/jsx-runtime"', `"${pathToFileURL(require.resolve("react/jsx-runtime")).href}"`);
  const { WorkstationDialog } = await import(`data:text/javascript;base64,${Buffer.from(output).toString("base64")}`);
  const render = props => {
    cursor = 0; pendingEffects = []; close.disabled = Boolean(props.closeDisabled);
    const tree = WorkstationDialog({ title: "Test dialog", children: "content", onStop: () => {}, ...props });
    tree.props.children.props.ref.current = modal;
    pendingEffects.forEach(effect => effect());
    return tree;
  };
  const unmount = () => { slots.forEach(slot => slot.cleanup?.()); };
  const restore = () => { for (const [key, descriptor] of Object.entries(saved)) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete globalThis[key]; } };
  return { document, listeners, trigger, stop, close, input, modal, render, unmount, restore };
}

test("dialog focus setup survives snapshot callback changes and restores its connected trigger only on unmount", async () => {
  const h = await dialogHarness();
  try {
    h.render({ onClose: () => {} }); assert.equal(h.modal.focuses, 1); assert.equal(h.document.adds, 1);
    h.input.focus();
    h.render({ onClose: () => {}, closeDisabled: true });
    h.render({ onClose: () => {}, closeDisabled: false });
    assert.equal(h.document.activeElement, h.input); assert.equal(h.modal.focuses, 1); assert.equal(h.document.adds, 1); assert.equal(h.document.removes, 0);
    h.unmount(); assert.equal(h.document.activeElement, h.trigger); assert.equal(h.trigger.focuses, 1); assert.equal(h.document.removes, 1);
  } finally { h.restore(); }
});

test("saved dialog callbacks honor the latest closeDisabled and only the latest onClose; Tab remains trapped", async () => {
  const h = await dialogHarness(); const calls = [];
  try {
    const tree = h.render({ onClose: () => calls.push("old") }); const key = [...h.listeners][0];
    const savedHeader = tree.props.children.props.children[0].props.children.at(-1).props.onClick;
    const savedBackdrop = tree.props.onMouseDown; const target = {};
    h.render({ onClose: () => calls.push("disabled"), closeDisabled: true });
    let backdropDefaultPrevented = false;
    key({ key: "Escape", preventDefault() {} }); savedHeader(); savedBackdrop({ target, currentTarget: target, preventDefault() { backdropDefaultPrevented = true; } });
    assert.deepEqual(calls, []);
    assert.equal(backdropDefaultPrevented, true, "a backdrop mousedown must not overwrite restored or retained focus");
    h.render({ onClose: () => calls.push("latest"), closeDisabled: false });
    key({ key: "Escape", preventDefault() {} }); assert.deepEqual(calls, ["latest"]);
    h.input.focus(); key({ key: "Tab", shiftKey: false, preventDefault() {} }); assert.equal(h.document.activeElement, h.stop);
    key({ key: "Tab", shiftKey: true, preventDefault() {} }); assert.equal(h.document.activeElement, h.input);
    h.trigger.isConnected = false; h.unmount(); assert.equal(h.trigger.focuses, 0); assert.equal(h.document.activeElement, h.input);
  } finally { h.restore(); }
});

const preparationDraft = { taskId: "new-task", projectionCount: 120, maxXraySec: 600, geometry: {} };
const preparationResult = (taskId = "new-task", engineSessionId = "engine-A", preflightPassed = true) => ({ engineSessionId, workstation: { scanSetup: { taskId } }, preflightPassed });
const deferred = () => { let resolve, reject; const promise = new Promise((res, rej) => { resolve = res; reject = rej; }); return { promise, resolve, reject }; };
function preparationHarness() {
  const scope = createPreparationRequestScope(); scope.replace();
  let context = { engineSessionId: "engine-A", taskId: "old-task" };
  const prepare = deferred(); const preflight = deferred();
  const commands = []; const stages = []; const errors = [];
  const run = command => { commands.push(command.type); return command.type === "prepare_scan" ? prepare.promise : preflight.promise; };
  return { scope, prepare, preflight, commands, stages, errors, setContext: value => { context = value; }, start: () => runScanPreparationRequest(scope, preparationDraft, () => context, run, value => stages.push(value), value => errors.push(value)) };
}

test("preparation owns its dialog while pending and a late close cannot close a replacement window", () => {
  const scope = createPreparationRequestScope();
  assert.equal(scope.replace(), true);
  const firstWindow = scope.revision;
  const request = scope.begin({ engineSessionId: "engine-A", taskId: "old-task" }, "new-task");
  assert.equal(scope.pending, true);
  assert.equal(scope.replace(firstWindow), false, "pending close and external replacement are blocked");
  assert.equal(scope.finish(request), true);
  assert.equal(scope.replace(), true, "a subsequent Camera dialog has its own revision");
  assert.equal(scope.replace(firstWindow), false, "the previous dialog callback is stale");
});

test("Stop before prepare acknowledgement prevents any later preflight command", async () => {
  const h = preparationHarness(); const result = h.start();
  h.scope.invalidate();
  h.prepare.resolve(preparationResult());
  assert.equal(await result, false);
  assert.deepEqual(h.commands, ["prepare_scan"]);
  assert.deepEqual(h.stages, ["submitting"]);
  assert.deepEqual(h.errors, []);
});

test("a submitted task transition after Stop cannot close a newer window and is accepted only once", async () => {
  const h = preparationHarness(); const result = h.start(); const oldWindow = h.scope.revision;
  h.scope.invalidate(true); assert.equal(h.scope.replace(), true);
  const newerWindow = h.scope.revision;
  const previous = { engineSessionId: "engine-A", taskId: "old-task" };
  const next = { engineSessionId: "engine-A", taskId: "new-task" };
  assert.equal(h.scope.acceptsTaskTransition(previous, next), true);
  assert.equal(h.scope.revision, newerWindow);
  assert.equal(h.scope.acceptsTaskTransition(previous, next), false);
  h.setContext(next); h.prepare.resolve(preparationResult());
  assert.equal(await result, false); assert.deepEqual(h.commands, ["prepare_scan"]);
  assert.equal(h.scope.replace(oldWindow), false);
});

test("Stop after preflight admission discards its late success and preserves a new dialog", async () => {
  const h = preparationHarness(); const revision = h.scope.revision; const result = h.start();
  h.prepare.resolve(preparationResult()); await Promise.resolve(); await Promise.resolve();
  assert.deepEqual(h.commands, ["prepare_scan", "preflight"]);
  h.scope.invalidate(); assert.equal(h.scope.replace(), true);
  h.preflight.resolve(preparationResult());
  assert.equal(await result, false);
  assert.equal(h.scope.replace(revision), false);
  assert.deepEqual(h.stages, ["submitting", "checking"]);
});

test("the expected atomic prepare taskId transition retains the request through correlated preflight", async () => {
  const h = preparationHarness(); const result = h.start();
  const context = { engineSessionId: "engine-A", taskId: "new-task" };
  assert.equal(h.scope.acceptsContext(context), true);
  h.setContext(context); h.prepare.resolve(preparationResult());
  await Promise.resolve(); await Promise.resolve();
  h.preflight.resolve(preparationResult());
  assert.equal(await result, true);
  assert.deepEqual(h.commands, ["prepare_scan", "preflight"]);
  assert.deepEqual(h.stages, ["submitting", "checking", null]);
  assert.equal(h.scope.pending, false);
});

test("returning to the old task after observing the prepared task is an unexpected context", () => {
  const h = preparationHarness(); const request = h.scope.begin({ engineSessionId: "engine-A", taskId: "old-task" }, "new-task");
  assert.equal(h.scope.current(request, { engineSessionId: "engine-A", taskId: "new-task" }), true);
  assert.equal(h.scope.current(request, { engineSessionId: "engine-A", taskId: "old-task" }), false);
});

test("unexpected task, engine session and unavailable context each retire the submission before preflight", async () => {
  for (const context of [null, { engineSessionId: "engine-B", taskId: "new-task" }, { engineSessionId: "engine-A", taskId: "unrelated-task" }]) {
    const h = preparationHarness(); const result = h.start();
    h.setContext(context); assert.equal(h.scope.acceptsContext(context), false);
    h.prepare.resolve(preparationResult());
    assert.equal(await result, false);
    assert.deepEqual(h.commands, ["prepare_scan"]);
    assert.deepEqual(h.errors, []);
  }
});

test("prepare and preflight acknowledgements must match the submitted task and control session", async () => {
  for (const bad of [preparationResult("wrong-task"), preparationResult("new-task", "engine-B")]) {
    const h = preparationHarness(); const result = h.start(); h.prepare.resolve(bad);
    assert.equal(await result, false); assert.deepEqual(h.commands, ["prepare_scan"]);
    assert.match(h.errors[0], /acknowledgement does not match/);
  }
  const h = preparationHarness(); const result = h.start(); h.prepare.resolve(preparationResult());
  await Promise.resolve(); await Promise.resolve(); h.preflight.resolve(preparationResult("wrong-task"));
  assert.equal(await result, false); assert.match(h.errors[0], /Preflight result does not match/);
});

test("an invalidated old failure cannot set errors or finish a new preparation request", async () => {
  const h = preparationHarness(); const old = h.start(); h.scope.invalidate();
  const newPrepare = deferred();
  const newer = runScanPreparationRequest(h.scope, preparationDraft, () => ({ engineSessionId: "engine-A", taskId: "old-task" }),
    () => newPrepare.promise, value => h.stages.push(value), value => h.errors.push(value));
  h.prepare.reject(new Error("late old command failure"));
  assert.equal(await old, false);
  assert.equal(h.scope.pending, true);
  assert.deepEqual(h.errors, []);
  assert.deepEqual(h.stages, ["submitting", "submitting"], "old finally cannot clear the new stage");
  h.scope.invalidate(); newPrepare.resolve(preparationResult());
  assert.equal(await newer, false);
  assert.deepEqual(h.errors, []);
});

test("duplicate preparation while pending submits only once and unavailable initial context sends no command", async () => {
  const h = preparationHarness(); const first = h.start();
  assert.equal(await h.start(), false); assert.deepEqual(h.commands, ["prepare_scan"]);
  h.scope.invalidate(); h.prepare.resolve(preparationResult()); assert.equal(await first, false);
  h.setContext(null); assert.equal(await h.start(), false); assert.deepEqual(h.commands, ["prepare_scan"]);
});

test("App routes menu, Stop, context changes and dialog callbacks through preparation ownership", async () => {
  const app = await readFile(new URL("../src/App.tsx", import.meta.url), "utf8");
  assert.match(app, /if \(preparationScope\.current\.pending && id !== "task\.stop"\) return/);
  assert.match(app, /const stop = \(\) => \{ preparationScope\.current\.invalidate\(true\)/);
  assert.match(app, /if \(preparationScope\.current\.acceptsContext\(preparationContext\.current\)\) return/);
  assert.match(app, /runScanPreparationRequest\(preparationScope\.current, draft/);
  assert.match(app, /onClose=\{\(\) => setAuxiliary\(null, auxiliaryRevision\)\}/);
  assert.match(app, /onAdvanced=\{\(\) => setAuxiliary\("camera", auxiliaryRevision\)\}/);
});
