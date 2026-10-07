/** Executes the real frontend policy/order and browser-only control model.
 * These tests access no hardware and do not certify the production backend. */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { loadSourceModule as load } from "./load-source-module.mjs";
import { transformWithEsbuild } from "vite";
const root = new URL("../", import.meta.url);
const policy = await load("src/engine/deviceControls.ts");
const { PreviewDeviceControls } = await load("src/engine/rts9060/deviceControls.ts");
const { MemoryLiveViewFeed, liveViewDataUrl } = await load("src/platform/cameraLiveView.ts");
function workflow() {
  return { phase: "ready", checkpointAvailable: false, params: { taskId: "", savePath: "", exposureMs: 0 },
    confirmedNanoStatus: { state: "IDLE", pos: 0, target: 0, microsteps: 8, ppr: 96000, homed: true, rearmed: true, hall: false, captureId: 0 },
    setParams(patch) { Object.assign(this.params, patch); }, async readNanoStatus() {}, async independentHome() {},
    async independentMove() {}, async abandonTask() { this.checkpointAvailable = false; } };
}
async function connectedPreview() {
  const wf = workflow(); const devices = new PreviewDeviceControls(wf);
  await devices.dispatch({ type: "camera_connect" });
  assert.equal(devices.camera.connected, false, "enumeration must not implicitly select the only entry");
  await devices.dispatch({ type: "camera_connect", deviceId: "preview-camera-d7100" });
  wf.params.savePath="/Pictures/MicroCT";return { wf, devices };
}
test("native shutter fractions match exactly and never round scan exposure", () => {
  assert.equal(policy.shutterExposureMs(65536 + 8000, 0xd100), 0.125);
  assert.equal(policy.shutterExposureMs(30 * 65536 + 1, 0xd100), 30000);
  assert.equal(policy.shutterExposureMs(65536 + 60, 0xd100), 1000 / 60);
  assert.equal(policy.shutterExposureMs("1/60", 0xd100), null);
  assert.equal(policy.shutterExposureMs(0, 0xd100), null);
  assert.equal(policy.shutterExposureMs(0xffffffff, 0xd100), null);
  assert.equal(policy.shutterExposureMs(0xffffffff, 0x500d), null);
  const camera = { properties: { shutter: { code: 0xd100, supportedValues: [65536 + 60] } } };
  assert.equal(policy.exactShutterForExposure(camera, 16.667), null);
  assert.equal(policy.exactShutterForExposure(camera, 1000 / 60), 65536 + 60);
});
test("D7100 standard exposure time uses its actual property code", async () => {
  const { devices } = await connectedPreview();
  Object.assign(devices.camera.properties.shutter, { code: 0x500d, actual: 20000, supportedValues: [10000, 20000] });
  assert.equal(policy.shutterExposureMs(20000, 0x500d), 2000);
  assert.equal(policy.shutterExposureMs(20000, 0xd100), null);
  assert.equal(policy.shutterExposureMs(20000, 0), null);
  assert.equal(policy.exactShutterForExposure(devices.camera, 2000), 20000);
  assert.equal(policy.exactShutterForExposure(devices.camera, 2000.1), null);
  assert.equal(policy.validateCameraDraft(devices.camera, { shutter: 20000 }), null);
  assert.equal(policy.nativePropertyLabel("shutter", 20000, 0x500d), "2000 ms");
});
test("camera draft validation rejects unknown keys, partial evidence and unsupported native values", async () => {
  const { devices } = await connectedPreview();
  assert.equal(policy.validateCameraDraft(devices.camera, { shutter: 65536 + 5, iso: 200 }), null);
  assert.match(policy.validateCameraDraft(devices.camera, { shutter: "1/5" }), /not supported/);
  assert.match(policy.validateCameraDraft(devices.camera, { iso: 100.5 }), /integer/);
  assert.match(policy.validateCameraDraft(devices.camera, { xrayKv: 60 }), /unavailable/);
  devices.camera.properties.iso.confirmed = false;
  assert.equal(Object.hasOwn(policy.confirmedCameraDraft(devices.camera.properties), "iso"), false);
});
test("single capture requires confirmed RAM destination and one photo", async () => {
  const { devices } = await connectedPreview();
  assert.equal(policy.cameraCapturePreparationReason(devices.camera), null);
  devices.camera.properties.recordingMedia.actual = 0;
  assert.match(policy.cameraCapturePreparationReason(devices.camera), /RAM/);
  devices.camera.properties.recordingMedia.actual = 1;
  devices.camera.properties.burstNumber.actual = 2;
  assert.match(policy.cameraCapturePreparationReason(devices.camera), /single/);
  devices.camera.properties.burstNumber.actual = 1;
  devices.camera.properties.burstNumber.confirmed = false;
  assert.ok(policy.cameraCapturePreparationReason(devices.camera));
});
test("snapshot ordering keeps higher engine sequence despite UI request order and retires old sessions", () => {
  const order = new policy.SnapshotOrder();
  assert.equal(order.accept({ engineSessionId: "A", snapshotSequence: 20 }), true);
  assert.equal(order.accept({ engineSessionId: "A", snapshotSequence: 18 }), false);
  assert.equal(order.accept({ engineSessionId: "A", snapshotSequence: 20 }), false);
  assert.equal(order.accept({ engineSessionId: "B", snapshotSequence: 1 }), true);
  assert.equal(order.accept({ engineSessionId: "A", snapshotSequence: 99 }), false);
  assert.equal(order.accept({ engineSessionId: "B", snapshotSequence: NaN }), false);
});
test("Stop bypasses a stuck command and discards previously queued ordinary work", async () => {
  const scheduler = new policy.CommandScheduler();
  let finish; let unsafeSent = false; let stopped = false;
  const active = scheduler.submit(() => new Promise(resolve => { finish = resolve; }));
  await Promise.resolve();
  const oldQueued = scheduler.submit(async () => { unsafeSent = true; });
  await scheduler.submit(async () => { stopped = true; }, true);
  assert.equal(stopped, true); assert.equal(unsafeSent, false);
  const current = await scheduler.submit(async () => "newly admitted");
  assert.equal(current, "newly admitted");
  finish(); await active; await oldQueued; assert.equal(unsafeSent, false);
});
test("occupation and persistent unfinished locks block manual controls while read-only state stays available", () => {
  const snapshot = { phase: "idle", operations: [], deviceControls: { taskLock: { locked: false } }, workstation: { reconstruction: { status: "idle" } } };
  assert.equal(policy.manualControlReason(snapshot), null);
  for (const phase of ["running", "paused", "finishing", "stopping"]) assert.ok(policy.manualControlReason({ ...snapshot, phase }));
  assert.ok(policy.manualControlReason({ ...snapshot, deviceControls: { taskLock: { locked: true, reason: "unfinished" } } }));
  assert.ok(policy.manualControlReason({ ...snapshot, operations: [{ status: "cleanup_unconfirmed" }] }));
  assert.ok(policy.manualControlReason({ ...snapshot, workstation: { reconstruction: { status: "running" } } }));
  assert.ok(policy.manualControlReason(snapshot, true));
});

test("unfinished tasks permit connection repair without unlocking parameters or motion", async () => {
  const wf = workflow(); wf.checkpointAvailable = true;
  const devices = new PreviewDeviceControls(wf);
  const snapshot = { phase: "stopped", operations: [], deviceControls: devices.snapshot(), workstation: { reconstruction: { status: "idle" } } };
  assert.equal(policy.deviceConnectionReason(snapshot), null);
  assert.ok(policy.manualControlReason(snapshot));
  await devices.dispatch({ type: "camera_connect" });
  await devices.dispatch({ type: "camera_connect", deviceId: "preview-camera-d7100" });
  await devices.dispatch({ type: "camera_read_properties" });
  await devices.dispatch({ type: "turntable_connect" });
  assert.equal(devices.snapshot().taskLock.locked, true);
  await assert.rejects(devices.dispatch({ type: "camera_apply_properties", properties: { iso: 200 } }), /occupies/);
  await assert.rejects(devices.dispatch({ type: "camera_live_view_start" }), /occupies/);
  await assert.rejects(devices.dispatch({ type: "turntable_relative", angleDeg: 90 }), /occupies/);
  for (const phase of ["running", "paused", "finishing", "stopping"]) assert.ok(policy.deviceConnectionReason({ ...snapshot, phase }));
});

test("controlled restore preflight requires its dedicated operator confirmation and retains manual locks", async () => {
  const ui = await readFile(new URL("src/TaskRecoveryDialog.tsx", root), "utf8");
  assert.match(ui, /checked=\{restoreConfirmed\}/);
  assert.match(ui, /disabled=\{!restoreConfirmed \|\| pending \|\| stale \|\| !lock.locked\}/);
  assert.match(ui, /type: "preflight", restore: true, confirmUsbAutoShutdownRelease: true/);
  assert.match(ui, /frozen X-ray voltage\/current, exposure, continuous-output limit and camera profile/);
  assert.match(ui, /Ordinary manual controls stay locked/);
});
test("independent capture and presets preserve scan directories, counts and source-native values", async () => {
  const { wf, devices } = await connectedPreview();
  await devices.dispatch({ type: "camera_capture" });
  assert.equal(wf.params.savePath, "/Pictures/MicroCT"); assert.equal(wf.params.taskId, "");
  assert.match(devices.camera.lastCapture, /Pictures\/MicroCT\/CameraCaptures/);
  await devices.dispatch({ type: "camera_apply_properties", properties: { shutter: 65536 + 8000, iso: 200 } });
  assert.equal(wf.params.exposureMs, 0.125);
  await devices.dispatch({ type: "preset_save", name: "D7100 low dose", description: "test" });
  const preset = devices.snapshot().presets.presets[0];
  assert.equal(preset.parameters.shutter, 65536 + 8000);
  assert.equal(Object.hasOwn(preset.parameters, "focusPosition"), false);
  assert.equal(Object.hasOwn(preset.parameters, "savePath"), false);
  assert.equal(devices.camera.capabilities.hardwareCertified, false);
});
test("Live View rejects capture until explicit stop and does not restore itself", async () => {
  const { devices } = await connectedPreview();
  await devices.dispatch({ type: "camera_live_view_start" });
  await assert.rejects(devices.dispatch({ type: "camera_capture" }), /LIVE_VIEW_CAPTURE_FORBIDDEN/);
  assert.equal(devices.camera.liveView.active, true);
  await devices.dispatch({ type: "camera_live_view_stop" });
  await devices.dispatch({ type: "camera_capture" });
  assert.equal(devices.camera.liveView.active, false);
});
test("preset import refuses incompatible schema/model/unknown fields as one file", async () => {
  const { devices } = await connectedPreview();
  await assert.rejects(devices.dispatch({ type: "preset_import", jsonText: JSON.stringify({ schemaVersion: 2, presets: [] }) }), /Invalid/);
  await devices.dispatch({ type: "preset_save", name: "valid" });
  const preset = devices.snapshot().presets.presets[0];
  await assert.rejects(devices.dispatch({ type: "preset_import", jsonText: JSON.stringify({ schemaVersion: 1, presets: [{ ...preset, parameters: { xray: 70 } }] }) }), /Unsupported/);
  assert.equal(devices.snapshot().presets.presets[0].name, "valid");
});
test("device UI preserves the native media boundary and keeps Safety Stop unconditional", async () => {
  const [ui, native, hook, css, dialog] = await Promise.all(["src/CameraPanel.tsx", "src/platform/cameraControls.ts", "src/engine/useEngine.ts", "src/styles.css", "src/WorkstationDialog.tsx"].map(file => readFile(new URL(file, root), "utf8")));
  assert.match(dialog, /role="dialog"/);assert.match(dialog,/onStop && <button.*onClick=\{onStop\}>Stop/);
  assert.match(ui, /camera_live_view_stop/); assert.match(ui, /snapshot\.mode === "developer_preview"/);
  assert.match(native, /"camera_live_view_frame", \{ afterSeq \}/);
  assert.doesNotMatch(native, /pipeName|pipePath|readFile/);
  assert.match(hook, /commandScheduler\.current\.submit\(run, true\)/);
  assert.doesNotMatch(hook, /requestRevisionRef|commandActiveRef/);
  assert.match(css, /\.device-console-body \{ overflow: auto/);
});

test("native Live View frame validation refuses old sessions, replay, truncation and oversized decode", async () => {
  const source = (await readFile(new URL("src/platform/cameraControls.ts", root), "utf8")).replace(/^import .*;\r?\n/gm, "");
  const { code } = await transformWithEsbuild(source, "camera-media-contract.ts", { loader: "ts", target: "es2022", format: "esm" });
  const { validateLiveFrame } = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
  const frame = { sessionId: "camera-A", sequence: 7, width: 640, height: 424, capturedAtMs: 123, jpeg: [255, 216, 255, 217] };
  assert.equal(validateLiveFrame(frame, "camera-A", 6), frame);
  for (const invalid of [{ ...frame, sessionId: "camera-B" }, { ...frame, sequence: 6 }, { ...frame, jpeg: [255, 216] },
    { ...frame, jpeg: [255, 216, 1.5, 255, 217] }, { ...frame, width: 8192, height: 8192 }]) {
    assert.throws(() => validateLiveFrame(invalid, "camera-A", 6), /invalid/);
  }
});

test("the Tauri adapter validates actual v2 native evidence and fails closed on malformed booleans/values", async () => {
  const source = await readFile(new URL("src/engine/tauriAdapter.ts", root), "utf8");
  const start = source.indexOf("function validLiveControl("); const end = source.indexOf("export class TauriEngineAdapter", start);
  const { code } = await transformWithEsbuild(source.slice(start, end) + "\nexport { assertSnapshot };", "native-snapshot.ts", { loader: "ts", target: "es2022", format: "esm" });
  const { assertSnapshot } = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
  const actual = JSON.parse((await readFile(new URL("tests/first-launch-response.jsonl", root), "utf8")).trim()).payload;
  assert.equal(assertSnapshot(actual), actual);
  // Engine-sent fields the TS interfaces must declare (they were missing and are easy
  // to drop again): camera.lastSeenAt and the frame's capturedAtMs.
  assert.ok(Object.prototype.hasOwnProperty.call(actual.deviceControls.camera, "lastSeenAt"), "the engine snapshot carries camera.lastSeenAt");
  assert.ok(actual.deviceControls.camera.lastSeenAt === null || typeof actual.deviceControls.camera.lastSeenAt === "string");
  const forwarder = await readFile(new URL("src-tauri/src/camera_forwarder.rs", root), "utf8");
  assert.match(forwarder, /"capturedAtMs":frame\.captured_at_ms/, "the frame payload carries capturedAtMs");
  const bad = structuredClone(actual); bad.deviceControls.camera.capabilities.hostCapture = "false";
  assert.throws(() => assertSnapshot(bad), /protocol error/);
  const broken = structuredClone(actual); broken.snapshotSequence = 1.5;
  assert.throws(() => assertSnapshot(broken), /protocol error/);
  const missingMode = structuredClone(actual); delete missingMode.deviceControls.camera.multipleExposureEvidence;
  assert.throws(() => assertSnapshot(missingMode), /multiple-exposure evidence/);
  const fakeOff = structuredClone(actual); fakeOff.deviceControls.camera.multipleExposureEvidence = { physicalApplicable: true, observable: false, status: "off", source: "none", reason: null };
  assert.throws(() => assertSnapshot(fakeOff), /multiple-exposure evidence/);
  const simulatedOff = structuredClone(actual); simulatedOff.deviceControls.camera.multipleExposureEvidence = { physicalApplicable: true, observable: true, status: "off", source: "SCRIPTED TEST ONLY", reason: null };
  assert.throws(() => assertSnapshot(simulatedOff), /multiple-exposure evidence/);
  const unknownReady = structuredClone(actual); unknownReady.cameraScanReadiness = { ready: true, issues: [] };
  assert.throws(() => assertSnapshot(unknownReady), /physical multiple-exposure evidence/);
  const absentControls = structuredClone(actual); delete absentControls.deviceControls.camera.liveViewControls;
  assert.throws(() => assertSnapshot(absentControls), /Live View controls/);
  // A device value rejection is a first-class terminal status: the snapshot
  // validator must accept it (operations history keeps it, so rejecting it would
  // make every later snapshot throw) and the operation waiter must stop polling.
  const rejected = structuredClone(actual);
  rejected.operations = [{ operationId: "op-rejected", deviceSessionId: rejected.engineSessionId, cancelRequested: false, device: "camera", kind: "camera_apply_properties", stage: "rejected", status: "rejected", error: "Camera parameter application partial: {\"whiteBalance\": \"camera command failed: PTP response 0x201c\"}", startedAt: "2026-10-06T18:00:00.000Z", updatedAt: "2026-10-06T18:00:00.100Z" }];
  assert.equal(assertSnapshot(rejected), rejected);
  const draft = await readFile(new URL("src/camera-draft.ts", root), "utf8");
  const term = draft.slice(draft.indexOf("const TERMINAL"), draft.indexOf("\n", draft.indexOf("const TERMINAL")));
  assert.match(term, /"rejected"/, "the operation waiter must treat rejected as terminal");
  const { devices } = await connectedPreview();
  await devices.dispatch({ type: "camera_live_view_start" });
  const active = structuredClone(actual); const camera = active.deviceControls.camera;
  camera.connected = true; camera.isolated = false;
  camera.liveView = structuredClone(devices.camera.liveView);
  camera.liveViewControls = structuredClone(devices.camera.liveViewControls);
  assert.equal(assertSnapshot(active), active, "typed transient native descriptors are accepted separately from photographic properties");
  const staleControls = structuredClone(active); staleControls.deviceControls.camera.liveViewControls.sessionId = "a".repeat(32);
  assert.throws(() => assertSnapshot(staleControls), /Live View controls/);
  const wrongWidth = structuredClone(active); wrongWidth.deviceControls.camera.liveViewControls.properties.cameraZoom.dataType = 6;
  assert.throws(() => assertSnapshot(wrongWidth), /Live View controls/);
  const forgedWrite = structuredClone(active); forgedWrite.deviceControls.camera.liveViewControls.properties.focusMode.confirmed = false;
  assert.throws(() => assertSnapshot(forgedWrite), /Live View controls/);
  const arbitraryCode = structuredClone(active); arbitraryCode.deviceControls.camera.liveViewControls.properties.cameraZoom.code = 0xd1a6;
  assert.throws(() => assertSnapshot(arbitraryCode), /Live View controls/);
});

test("Live View preserves the decoded image through missing frames/errors and releases only after DOM replacement", async () => {
  let state = null; let next = { sessionId: "A", sequence: 1, width: 640, height: 424, jpeg: [] };
  let fetches = 0; const released = [];
  const media = { fetch: async () => { fetches++; if (next instanceof Error) throw next; return next; },
    validate: (frame, session, after) => { assert.equal(frame.sessionId, session); assert.ok(frame.sequence > after); },
    decode: async frame => ({ url: `blob:${frame.sequence}`, width: frame.width, height: frame.height }),
    release: url => released.push(url), update: image => { state = image; }, now: () => 1000 };
  const feed = new MemoryLiveViewFeed("A", media);
  await feed.poll(); assert.equal(state.url, "blob:1");
  next = null; await feed.poll(); assert.equal(state.url, "blob:1"); assert.deepEqual(released, []);
  next = new Error("transient path not found"); await feed.poll();
  assert.equal(state.url, "blob:1"); assert.match(state.error, /path not found/);
  next = { sessionId: "A", sequence: 2, width: 640, height: 424, jpeg: [] };
  await feed.poll(); assert.equal(state.url, "blob:2"); assert.equal(state.error, null);
  assert.deepEqual(released, [], "previous image stays alive until React commits the new src");
  const before = fetches; await feed.poll(); assert.equal(fetches, before, "two-image bound until presentation");
  feed.presented(2); assert.deepEqual(released, ["blob:1"]);
  feed.stop(); assert.equal(state, null); assert.deepEqual(released, ["blob:1", "blob:2"]);
});

test("Live View retires pending fetch/decode on stop without showing or leaking a late image", async () => {
  let finish; let state = null; const released = []; let calls = 0;
  const feed = new MemoryLiveViewFeed("A", {
    fetch: async () => { calls++; return { sequence: 1, width: 10, height: 10 }; }, validate: () => {},
    decode: () => new Promise(resolve => { finish = resolve; }),
    release: url => released.push(url), update: image => { state = image; }, now: () => 0,
  });
  const pending = feed.poll(); await Promise.resolve(); await feed.poll(); assert.equal(calls, 1);
  feed.stop(); finish({ url: "blob:late", width: 10, height: 10 }); await pending;
  assert.equal(state, null); assert.deepEqual(released, ["blob:late"]);
});

test("Live View refresh does not restart on freshness changes and has no persistence API", async () => {
  const [ui, media] = await Promise.all(["src/WorkstationMedia.tsx", "src/platform/cameraLiveView.ts"].map(file => readFile(new URL(file, root), "utf8")));
  assert.doesNotMatch(ui, /\[.*lv\?\.fresh.*\]/);
  assert.match(ui, /STALE · waiting for current frame/);
  assert.doesNotMatch(media, /writeFile|localStorage|indexedDB|sessionStorage|saveAs|download=/);
  assert.doesNotMatch(ui, /new Blob|createObjectURL/);
  assert.equal(liveViewDataUrl([255, 216, 255, 217]), "data:image/jpeg;base64,/9j/2Q==");
});

test("camera mode readback disables ineffective controls and unsupported options without rewriting the actual value", async () => {
  const { devices } = await connectedPreview();
  await devices.dispatch({ type: "camera_apply_properties", properties: { exposureMode: 2 } });
  const shutter = devices.camera.properties.shutter;
  assert.match(policy.cameraPropertyReason(shutter), /automatic/);
  assert.equal(shutter.actual, 65536 + 5);
  assert.ok(shutter.options.every(option => !option.enabled));
  assert.match(policy.validateCameraDraft(devices.camera, { shutter: 65536 + 1000 }), /automatic/);
  assert.equal(policy.validateCameraDraft(devices.camera, { shutter: shutter.actual, iso: 200 }), null);
  assert.deepEqual(policy.cameraDraftChanges(devices.camera, { shutter: shutter.actual, iso: 200 }), { iso: 200 });
  assert.equal(Object.hasOwn(policy.confirmedCameraDraft(devices.camera.properties), "shutter"), false);
  await devices.dispatch({ type: "camera_apply_properties", properties: { exposureMode: 1 } });
  assert.equal(policy.cameraPropertyReason(shutter), null);
  const iso = devices.camera.properties.iso;
  iso.options.find(option => option.value === 200).enabled = false;
  iso.options.find(option => option.value === 200).reason = "Unsupported in this camera mode";
  assert.match(policy.validateCameraDraft(devices.camera, { iso: 200 }), /Unsupported in this camera mode/);
});

test("a fully labelled finite range uses a dropdown without dropping an unlabelled valid option", async () => {
  const { devices } = await connectedPreview();
  const property = devices.camera.properties.whiteBalanceTemperature;
  Object.assign(property, { supportedValues: [], range: [0, 2, 1], options: [0, 1, 2].map(value => ({ value, label: `${2500 + value * 100} K`, supported: true, enabled: true, reason: null })) });
  assert.equal(policy.cameraPropertyUsesSelect(property), true);
  property.options.pop(); assert.equal(policy.cameraPropertyUsesSelect(property), false);
  property.range = [0, 65535, 1]; assert.equal(policy.cameraPropertyUsesSelect(property), false);
});

test("production adapter sends exact live-control and capture-ID commands and propagates IPC failure without preview fallback", async () => {
  const source = await readFile(new URL("src/engine/tauriAdapter.ts", root), "utf8");
  const actual = JSON.parse((await readFile(new URL("tests/first-launch-response.jsonl", root), "utf8")).trim()).payload;
  const calls = []; globalThis.__microCtIpcContract = { calls, actual, fail: false };
  const stub = `export async function invoke(name,args){const state=globalThis.__microCtIpcContract;state.calls.push({name,args});if(state.fail)throw new Error("NO HARDWARE test IPC unavailable");return structuredClone(state.actual)}`;
  const stubUrl = `data:text/javascript;base64,${Buffer.from(stub).toString("base64")}`;
  const { code } = await transformWithEsbuild(source, "tauriAdapter.ts", { loader: "ts", target: "es2022", format: "esm" });
  const moduleCode = code.replaceAll('"@tauri-apps/api/core"', JSON.stringify(stubUrl));
  try {
    const { TauriEngineAdapter } = await import(`data:text/javascript;base64,${Buffer.from(moduleCode).toString("base64")}`);
    const adapter = new TauriEngineAdapter(); const sessionId = "a".repeat(32);
    for (const command of [{ type: "camera_live_view_read", sessionId },
      { type: "camera_live_view_configure", sessionId, controls: { cameraZoom: 2, focusMode: 4 } },
      { type: "camera_capture_descriptor", captureId: "NO-HARDWARE-registered-id" }]) {
      assert.deepEqual(await adapter.dispatch(command), actual);
      assert.deepEqual(calls.at(-1), { name: "engine_command", args: { command } });
    }
    assert.equal(adapter.kind, "tauri"); globalThis.__microCtIpcContract.fail = true;
    await assert.rejects(adapter.getSnapshot(), /IPC unavailable/); assert.equal(adapter.kind, "tauri");
  } finally { delete globalThis.__microCtIpcContract; }
});

test("Live View configuration is session-bound, outside presets, and rejects stale or unsupported requests atomically", async () => {
  const { devices, wf } = await connectedPreview();
  const photographic = structuredClone(devices.camera.properties);
  await devices.dispatch({ type: "camera_live_view_start" });
  const first = devices.camera.liveView.sessionId;
  assert.match(first, /^[a-f0-9]{32}$/);
  const controls = devices.camera.liveViewControls;
  await devices.dispatch({ type: "camera_live_view_configure", sessionId:first, controls:{cameraZoom:1,focusMode:0} });
  assert.equal(controls.properties.cameraZoom.actual, 1);
  assert.equal(controls.properties.focusMode.actual, 0);
  assert.equal(devices.camera.properties.focusMode.actual, photographic.focusMode.actual);
  assert.equal(Object.hasOwn(devices.camera.properties, "cameraZoom"), false);
  const before = structuredClone(controls);
  for (const invalid of [{},{cameraZoom:2,focusMode:3},{cameraZoom:2,arbitrary:1},{cameraZoom:1.5}]) {
    await assert.rejects(devices.dispatch({type:"camera_live_view_configure",sessionId:first,controls:invalid}), /Unsupported/);
    assert.deepEqual(devices.camera.liveViewControls, before);
  }
  wf.phase="scanning";
  await assert.rejects(devices.dispatch({type:"camera_live_view_read",sessionId:first}), /Task occupies/);
  wf.phase="ready";
  await devices.dispatch({type:"camera_live_view_stop"});
  assert.deepEqual(devices.camera.liveViewControls,{sessionId:null,properties:{}});
  await devices.dispatch({type:"camera_live_view_start"});
  assert.notEqual(devices.camera.liveView.sessionId,first);
  await assert.rejects(devices.dispatch({type:"camera_live_view_configure",sessionId:first,controls:{cameraZoom:1}}), /session changed/);
  devices.stop();
  assert.equal(devices.camera.liveView.active,false);
  assert.deepEqual(devices.camera.liveViewControls,{sessionId:null,properties:{}});
});

test("focus eligibility follows the current photo-LV mode and lens evidence rather than normal focus mode", async () => {
  const { devices } = await connectedPreview();
  assert.equal(devices.camera.properties.focusMode.actual,4);
  await assert.rejects(devices.dispatch({type:"camera_autofocus"}), /eligibility/);
  await assert.rejects(devices.dispatch({type:"camera_focus",step:1}), /eligibility/);
  await devices.dispatch({type:"camera_live_view_start"});
  const sessionId=devices.camera.liveView.sessionId;
  await devices.dispatch({type:"camera_focus",step:1});
  await devices.dispatch({type:"camera_live_view_configure",sessionId,controls:{focusMode:0}});
  await devices.dispatch({type:"camera_autofocus"});
  await devices.dispatch({type:"camera_live_view_configure",sessionId,controls:{focusMode:2}});
  await assert.rejects(devices.dispatch({type:"camera_focus",step:1}), /eligibility/);
  await assert.rejects(devices.dispatch({type:"camera_autofocus"}), /eligibility/);
  await devices.dispatch({type:"camera_live_view_configure",sessionId,controls:{focusMode:4}});
  devices.camera.runtimeProperties.liveViewSelector.actual=1;
  await assert.rejects(devices.dispatch({type:"camera_focus",step:1}), /eligibility/);
  devices.camera.runtimeProperties.liveViewSelector.actual=0;
  devices.camera.runtimeProperties.lensSort.confirmed=false;
  await assert.rejects(devices.dispatch({type:"camera_focus",step:1}), /eligibility/);
  devices.camera.runtimeProperties.lensSort.confirmed=true;
  devices.camera.liveViewControls.properties.focusMode.confirmed=false;
  await assert.rejects(devices.dispatch({type:"camera_focus",step:1}), /eligibility/);
  await assert.rejects(devices.dispatch({type:"camera_autofocus"}), /eligibility/);
});
