/** Executes the real frontend policy/order and browser-only control model.
 * These tests access no hardware and do not certify the production backend. */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { transformWithEsbuild } from "vite";
const root = new URL("../", import.meta.url);
async function load(file) {
  const source = await readFile(new URL(file, root), "utf8");
  const { code } = await transformWithEsbuild(source, file, { loader: "ts", target: "es2022", format: "esm" });
  return import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
}
const policy = await load("src/engine/deviceControls.ts");
const { PreviewDeviceControls } = await load("src/engine/rts9060/deviceControls.ts");
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
  return { wf, devices };
}
test("native shutter fractions match exactly and never round scan exposure", () => {
  assert.equal(policy.shutterExposureMs(65536 + 8000), 0.125);
  assert.equal(policy.shutterExposureMs(30 * 65536 + 1), 30000);
  assert.equal(policy.shutterExposureMs(65536 + 60), 1000 / 60);
  assert.equal(policy.shutterExposureMs("1/60"), null);
  assert.equal(policy.shutterExposureMs(0), null);
  const camera = { properties: { shutter: { supportedValues: [65536 + 60] } } };
  assert.equal(policy.exactShutterForExposure(camera, 16.667), null);
  assert.equal(policy.exactShutterForExposure(camera, 1000 / 60), 65536 + 60);
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
  const ui = await readFile(new URL("src/DeviceConsoles.tsx", root), "utf8");
  assert.match(ui, /checked=\{restoreConfirmed\}/);
  assert.match(ui, /disabled=\{!restoreConfirmed \|\| Boolean\(connectionReason\)\}/);
  assert.match(ui, /type: "preflight", restore: true, confirmUsbAutoShutdownRelease: true/);
  assert.match(ui, /frozen X-ray voltage\/current, Exposure, continuous-output limit and camera profile/);
  assert.match(ui, /Ordinary parameters, focus and motion remain locked/);
});
test("independent capture and presets preserve scan directories, counts and source-native values", async () => {
  const { wf, devices } = await connectedPreview();
  await devices.dispatch({ type: "camera_capture" });
  assert.equal(wf.params.savePath, ""); assert.equal(wf.params.taskId, "");
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
  const [ui, native, hook, css] = await Promise.all(["src/DeviceConsoles.tsx", "src/platform/cameraControls.ts", "src/engine/useEngine.ts", "src/styles.css"].map(file => readFile(new URL(file, root), "utf8")));
  assert.match(ui, /role="dialog"/); assert.match(ui, /Safety Stop/);
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
  const frame = { sessionId: "camera-A", sequence: 7, width: 640, height: 424, jpeg: [255, 216, 255, 217] };
  assert.equal(validateLiveFrame(frame, "camera-A", 6), frame);
  for (const invalid of [{ ...frame, sessionId: "camera-B" }, { ...frame, sequence: 6 }, { ...frame, jpeg: [255, 216] },
    { ...frame, jpeg: [255, 216, 1.5, 255, 217] }, { ...frame, width: 8192, height: 8192 }]) {
    assert.throws(() => validateLiveFrame(invalid, "camera-A", 6), /invalid/);
  }
});

test("the Tauri adapter validates actual v2 native evidence and fails closed on malformed booleans/values", async () => {
  const source = await readFile(new URL("src/engine/tauriAdapter.ts", root), "utf8");
  const start = source.indexOf("function assertSnapshot("); const end = source.indexOf("export class TauriEngineAdapter", start);
  const { code } = await transformWithEsbuild(source.slice(start, end) + "\nexport { assertSnapshot };", "native-snapshot.ts", { loader: "ts", target: "es2022", format: "esm" });
  const { assertSnapshot } = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
  const actual = JSON.parse((await readFile(new URL("tests/first-launch-response.jsonl", root), "utf8")).trim()).payload;
  assert.equal(assertSnapshot(actual), actual);
  const bad = structuredClone(actual); bad.deviceControls.camera.capabilities.hostCapture = "false";
  assert.throws(() => assertSnapshot(bad), /protocol error/);
  const broken = structuredClone(actual); broken.snapshotSequence = 1.5;
  assert.throws(() => assertSnapshot(broken), /protocol error/);
});
