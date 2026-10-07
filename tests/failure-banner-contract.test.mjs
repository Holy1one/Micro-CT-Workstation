/** One closable failure surface: stable keys under 400 ms polling, no re-surfacing of an
 *  old failure after a reconnect, and a cancelled operation that never becomes a fake failure. */
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { loadSourceModule } from "./load-source-module.mjs";
const { FAILURE_ADVICE, deriveFailureReport, failureNextAction, parseEngineCode } = await loadSourceModule("src/engine/deviceControls.ts");
const groups = new Set(["camera", "readiness", "turntable", "xray", "files"]);
const operation = (over = {}) => ({
  operationId: "op-1", device: "camera", kind: "camera_apply_properties", stage: "rejected",
  status: "failed", error: null, updatedAt: "2026-01-01T00:00:00.000Z", cancelRequested: false, ...over,
});
const snapshotOf = (operations, operationId = operations.at(-1)?.operationId ?? null) => ({ operations, operationId });

test("engine codes are read from the message and never invented", () => {
  assert.equal(parseEngineCode("ENGINE:CAMERA_NOT_READY"), "CAMERA_NOT_READY");
  assert.equal(parseEngineCode("prefix ENGINE:LV_NOT_ACTIVE suffix"), "LV_NOT_ACTIVE");
  assert.equal(parseEngineCode("camera unavailable: timeout"), null);
  assert.equal(parseEngineCode(null), null);
});

test("a terminal failed operation produces a stable key for the same receipt", () => {
  const snapshot = snapshotOf([operation({ status: "failed", error: "ENGINE:CAMERA_CONNECTION_FAILED" })]);
  const first = deriveFailureReport(snapshot, null, null, 0);
  const polled = deriveFailureReport(snapshot, null, null, 0);
  assert.equal(first.key, polled.key, "the 400 ms poll must not re-arm the banner");
  assert.equal(first.kind, "failed");
  assert.equal(first.code, "CAMERA_CONNECTION_FAILED");
  assert.equal(first.group, "camera");
  assert.match(first.stage, /camera_apply_properties · failed · rejected/);
  assert.equal(first.reason, "ENGINE:CAMERA_CONNECTION_FAILED", "the engine text is never rewritten");
});

test("the newest operation id decides, so an old failure cannot re-surface", () => {
  const old = operation({ status: "failed", error: "ENGINE:CAMERA_CONNECTION_FAILED" });
  const fresh = { ...operation({ operationId: "op-2", status: "completed", error: null }) };
  assert.equal(deriveFailureReport(snapshotOf([old, fresh], "op-2"), null, null, 0), null);
  assert.ok(deriveFailureReport(snapshotOf([old, fresh], "op-1"), null, null, 0));
});

test("a cancelled operation is not a failure, but a late failure inside one still is", () => {
  assert.equal(deriveFailureReport(snapshotOf([operation({ status: "cancelled", error: null })]), null, null, 0), null,
    "Stop and safety cleanup cancel work on purpose");
  const late = deriveFailureReport(snapshotOf([operation({ status: "cancelled", error: "camera cleanup unconfirmed; session isolated" })]), null, null, 0);
  assert.ok(late, "a cancelled receipt that carries a real error stays visible");
  assert.equal(late.kind, "failed");
});

test("a rejected value is presented as a correctable refusal", () => {
  const report = deriveFailureReport(snapshotOf([operation({ status: "rejected", error: "ENGINE:CAMERA_PROPERTY_UNSUPPORTED_VALUE" })]), null, null, 0);
  assert.equal(report.kind, "rejected");
  assert.equal(report.group, "camera");
  assert.match(report.title, /rejected the requested value/);
  assert.match(report.nextAction, /not a device fault/);
});

test("cleanup unknown keeps its own kind and never claims a safe state", () => {
  const report = deriveFailureReport(snapshotOf([operation({ status: "cleanup_unconfirmed", error: "ENGINE:DEVICE_CLEANUP_UNCONFIRMED cleanup unconfirmed" })]), null, null, 0);
  assert.equal(report.kind, "cleanup_unconfirmed");
  assert.equal(report.group, "files");
  assert.match(report.nextAction, /Cleanup is NOT confirmed/);
  // Without an engine code the group falls back to the device that failed, which is
  // the camera here; the kind still never claims a clean state.
  const codeless = deriveFailureReport(snapshotOf([operation({ status: "cleanup_unconfirmed", error: "camera cleanup unconfirmed" })]), null, null, 0);
  assert.equal(codeless.kind, "cleanup_unconfirmed");
  assert.equal(codeless.group, "camera");
});

test("message failures dedupe by attempt and keep an unknown code verbatim", () => {
  const first = deriveFailureReport(null, "ENGINE:SOME_FUTURE_CODE", null, 0);
  const again = deriveFailureReport(null, "ENGINE:SOME_FUTURE_CODE", null, 0);
  const retried = deriveFailureReport(null, "ENGINE:SOME_FUTURE_CODE", null, 1);
  assert.equal(first.key, again.key);
  assert.notEqual(first.key, retried.key, "a new attempt re-arms the banner");
  assert.equal(first.code, "SOME_FUTURE_CODE", "an undocumented code is never dropped");
  assert.match(first.nextAction, /not documented in this build/);
  const transport = deriveFailureReport(null, "Control service unavailable", null, 0);
  assert.equal(transport.kind, "transport");
  assert.equal(transport.code, null);
  assert.equal(deriveFailureReport(null, null, null, 3), null);
});

test("action errors outrank a stale engine error and are cleared by a new attempt", () => {
  const report = deriveFailureReport(null, "ENGINE:OLD", "ENGINE:PREFLIGHT_REQUIRED", 4);
  assert.equal(report.code, "PREFLIGHT_REQUIRED");
});

test("every advice entry is documented, actionable and free of localized or wishful text", () => {
  const doc = readFileSync(new URL("../docs/frontend-error-codes.md", import.meta.url), "utf8");
  const codes = Object.keys(FAILURE_ADVICE);
  assert.ok(codes.length >= 60, `the table covers the documented catalog (${codes.length})`);
  for (const code of codes) {
    assert.match(code, /^[A-Z][A-Z0-9_]+$/, `${code} is an engine code literal`);
    assert.ok(doc.includes(code), `${code} must exist in docs/frontend-error-codes.md`);
    const advice = FAILURE_ADVICE[code];
    assert.ok(groups.has(advice.group), `${code} uses a real Troubleshooting group`);
    assert.ok(advice.action.trim().length > 20, `${code} needs a concrete next action`);
    assert.doesNotMatch(advice.action, /[\u3400-\u9fff]/u, `${code} advice is English`);
    assert.doesNotMatch(advice.action, /restart (will|should|always) fix|just restart/i, `${code} must not promise a restart`);
  }
});

test("an undocumented code keeps a truthful, non-committal next action", () => {
  const action = failureNextAction("SOME_FUTURE_CODE", "engine");
  assert.match(action, /Record the code and the exact engine text/);
  assert.doesNotMatch(action, /[\u3400-\u9fff]/u);
  assert.match(failureNextAction(null, "cleanup_unconfirmed"), /do not treat the machine as cleaned up/);
});
