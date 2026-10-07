/** Help → Troubleshooting: symptom → engine code → exact next action.
 *
 *  The next-action text comes from the single FAILURE_ADVICE table in
 *  engine/deviceControls.ts, so the dialog and the failure banner can never drift
 *  apart. Raw engine codes and raw engine text stay verbatim; nothing here rewrites a
 *  device condition into success/off/normal/safe, and nothing promises that a restart
 *  resolves an unknown or unsupported condition. */
import { useState } from "react";
import type { EngineSnapshot } from "./engine/types";
import { failureAdvice } from "./engine/deviceControls";

type Row = { symptom: string; code: string };
interface Group { id: string; title: string; intro: string; rows: Row[]; notMeaning: string }

export const TROUBLESHOOTING_GROUPS: Group[] = [
  {
    id: "camera",
    title: "Camera",
    intro: "A value the body refused is not a device fault: the session stays usable and the fix is a different value, not a reconnect.",
    rows: [
      { symptom: "A parameter Apply was refused but the camera still works", code: "CAMERA_PROPERTY_UNSUPPORTED_VALUE" },
      { symptom: "A dependent value (for example colour temperature) is refused", code: "CAMERA_PROPERTY_DEPENDENCY_UNMET" },
      { symptom: "A parameter cannot be written at all", code: "CAMERA_PROPERTY_WRITE_UNSUPPORTED" },
      { symptom: "A parameter has no verifiable description from the body", code: "CAMERA_PROPERTY_UNDESCRIBED" },
      { symptom: "Focus cannot be changed from the software", code: "CAMERA_FOCUS_MODE_LOCKED" },
      { symptom: "No usable camera session exists", code: "CAMERA_NOT_READY" },
      { symptom: "No camera is connected", code: "CAMERA_NOT_CONNECTED" },
      { symptom: "Connecting an already-connected camera was refused", code: "CAMERA_ALREADY_CONNECTED" },
      { symptom: "Apply is refused because Live View is active", code: "LIVE_VIEW_APPLY_FORBIDDEN" },
      { symptom: "Capture is refused because Live View is active", code: "LIVE_VIEW_CAPTURE_FORBIDDEN" },
      { symptom: "Capture readiness is not satisfied", code: "CAMERA_CAPTURE_UNQUALIFIED" },
      { symptom: "Lens step drive is not eligible", code: "CAMERA_FOCUS_UNQUALIFIED" },
      { symptom: "Autofocus is not eligible", code: "CAMERA_AUTOFOCUS_UNQUALIFIED" },
      { symptom: "The native shutter has no confirmed readback", code: "CAMERA_SHUTTER_UNCONFIRMED" },
      { symptom: "A Live View read or control used an old session", code: "STALE_LIVE_VIEW_SESSION" },
      { symptom: "Live View controls were read while no preview was active", code: "LV_NOT_ACTIVE" },
      { symptom: "A lens focus step was rejected", code: "INVALID_CAMERA_FOCUS_STEP" },
      { symptom: "Exposure was edited through the compatibility entry", code: "EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY" },
      { symptom: "The native camera backend could not be established", code: "CAMERA_BACKEND_UNAVAILABLE" },
      { symptom: "The camera could not be connected", code: "CAMERA_CONNECTION_FAILED" },
    ],
    notMeaning: "None of these mean the camera is broken, that the scan must be re-planned, or that a reconnect is required. The body also keeps settings the software cannot write (mode dial, hardware focus switch); change those on the camera itself and read again.",
  },
  {
    id: "readiness",
    title: "Scan readiness",
    intro: "Preflight and Scan Preparation refuse rather than guess. The live readiness list below is the engine's own evidence for this session.",
    rows: [
      { symptom: "Preflight has not passed for the current settings", code: "PREFLIGHT_REQUIRED" },
      { symptom: "No prepared geometry exists yet", code: "SCAN_PREPARATION_REQUIRED" },
      { symptom: "Geometry was rejected", code: "INVALID_SCAN_GEOMETRY" },
      { symptom: "The prepare payload did not match the contract", code: "INVALID_SCAN_PREPARATION" },
      { symptom: "The engine still expects geometry or an answer to a prompt", code: "SCAN_GEOMETRY_REQUIRED" },
      { symptom: "No frozen capture profile is available", code: "CAPTURE_PROFILE_REQUIRED" },
      { symptom: "Exposure is outside the camera's reported range", code: "EXPOSURE_OUTSIDE_CAMERA_RANGE" },
      { symptom: "Exposure does not match a native shutter position", code: "EXPOSURE_NOT_NATIVE_CAMERA_VALUE" },
      { symptom: "Parameters changed after the start request was queued", code: "START_ADMISSION_INVALIDATED" },
    ],
    notMeaning: "Dismissing a readiness warning never allows a start. Multiple-exposure OFF is physically applicable on the D7100 but not observable through the verified native property catalog, and this build has no machine-evidence override: a real scan stays BLOCKED until a current session-bound operator attestation is recorded in Scan Preparation.",
  },
  {
    id: "turntable",
    title: "Turntable",
    intro: "Ordinary motion runs only in the production direction. HOME is the one bounded internal reverse move, and it needs either a confirmed X-ray OFF or isolated bench mode.",
    rows: [
      { symptom: "The engine holds no turntable adapter", code: "RECONNECT_REQUIRED" },
      { symptom: "No valid HOME/Hall reference is established", code: "HOME_REQUIRED" },
      { symptom: "No fresh Nano STATUS is available", code: "NANO_STATUS_UNKNOWN" },
      { symptom: "Serial communication with the Nano failed", code: "NANO_COMMUNICATION_FAILED" },
      { symptom: "HOME was refused because X-ray OFF was not confirmed", code: "HOME_REQUIRES_XRAY_OFF_OR_ISOLATED_BENCH" },
      { symptom: "Bench mode was enabled without the isolation confirmation", code: "PHYSICAL_ISOLATION_CONFIRMATION_REQUIRED" },
      { symptom: "The requested angle was rejected", code: "INVALID_MOTION_ANGLE" },
      { symptom: "The turntable is not connected", code: "NANO_NOT_CONNECTED" },
    ],
    notMeaning: "A refused angle is not a lost reference, and HOME never enables X-ray output. In isolated bench mode the engine still reports the X-ray state as UNKNOWN and unmonitored, and beam commands stay prohibited; once this engine process has owned an X-ray session, bench isolation can no longer override it.",
  },
  {
    id: "xray",
    title: "X-ray & interlocks",
    intro: "UNKNOWN is never OFF. Every emission is judged from the device readback, and the interlock and temperature verdicts belong to the engine, not to the interface.",
    rows: [
      { symptom: "A device fault is latched", code: "FAULT_RECOVERY_REQUIRED" },
      { symptom: "Beam-off was not confirmed by readback", code: "XRAY_OFF_UNCONFIRMED" },
      { symptom: "Interlock, temperature or setpoint health failed during Preflight", code: "XRAY_PREFLIGHT_FAILED" },
      { symptom: "OFF could not be re-confirmed while disconnecting", code: "XRAY_DISCONNECT_FAILED" },
      { symptom: "The setpoint write or readback failed", code: "XRAY_SETPOINT_FAILED" },
      { symptom: "A beam command was issued in isolated bench mode", code: "BENCH_MODE_XRAY_FORBIDDEN" },
      { symptom: "Bench isolation was requested after an X-ray session existed", code: "BENCH_CANNOT_OVERRIDE_XRAY_SESSION" },
      { symptom: "The X-ray source is not connected", code: "XRAY_NOT_CONNECTED" },
      { symptom: "The setpoints are not confirmed in this session", code: "XRAY_SETPOINT_CONFIRMATION_REQUIRED" },
      { symptom: "Output is active while an OFF-requiring action was requested", code: "XRAY_BEAM_ACTIVE" },
      { symptom: "The voltage/current combination exceeds the limit", code: "XRAY_POWER_LIMIT" },
      { symptom: "The source is already connected", code: "XRAY_ALREADY_CONNECTED" },
      { symptom: "Preflight needs the USB auto shut down to be released", code: "USB_AUTO_SHUTDOWN_RELEASE_REQUIRED" },
      { symptom: "The Moxtek link could not be established", code: "XRAY_CONNECTION_FAILED" },
      { symptom: "Beam-on failed closed", code: "XRAY_BEAM_ON_FAILED" },
      { symptom: "The preview-only timer control was used in production", code: "XRAY_STAGE_LOCKED" },
    ],
    notMeaning: "A readback that is missing, stale or unknown is not evidence that output is off, and no interface action clears an interlock. Below 2 seconds after a confirmed beam-off the device layer holds the next beam-on until the interval has elapsed rather than starting early; Stop, interlock, disconnect and fault paths are always allowed to turn output off immediately.",
  },
  {
    id: "files",
    title: "Files & recovery",
    intro: "Locks, retained tasks and capture roots protect the files that already exist. Nothing here deletes or overwrites scan output.",
    rows: [
      { symptom: "A scan, cleanup or reconstruction owns the devices", code: "DEVICE_CONTROL_LOCKED" },
      { symptom: "The engine is busy with the scan", code: "SCAN_ACTIVE" },
      { symptom: "Another operation for this device is in flight", code: "DEVICE_OPERATION_ACTIVE" },
      { symptom: "An update prepare was requested while work is active", code: "SOFTWARE_UPDATE_BUSY" },
      { symptom: "Software-update maintenance owns the devices", code: "SOFTWARE_UPDATE_MAINTENANCE_LOCKED" },
      { symptom: "A reconstruction is running", code: "RECONSTRUCTION_ACTIVE" },
      { symptom: "Reconstruction is locked by occupancy or an unfinished task", code: "RECONSTRUCTION_LOCKED" },
      { symptom: "An isolated session has unconfirmed cleanup", code: "DEVICE_CLEANUP_UNCONFIRMED" },
      { symptom: "A retained unfinished task locks manual work", code: "UNFINISHED_TASK_LOCKED" },
      { symptom: "The task is Stopped, so Restore no longer applies", code: "RECOVERY_REQUIRED" },
      { symptom: "Resume needs a Paused phase with valid Preflight and HOME", code: "NOT_RESUMABLE" },
      { symptom: "Retained task evidence does not match", code: "RESTORE_VALIDATION_FAILED" },
      { symptom: "The task identity did not verify", code: "TASK_REGISTRY_INVALID" },
      { symptom: "The registry write plan could not be built", code: "TASK_REGISTRY_SAVE_FAILED" },
      { symptom: "The scan did not finish within the shutdown wait", code: "SCAN_SHUTDOWN_TIMEOUT" },
      { symptom: "The phase is still Fault after STOP", code: "SCAN_STOP_FAILED" },
      { symptom: "The task directory already contains output", code: "SCAN_OUTPUT_EXISTS_USE_NEW_TASK" },
      { symptom: "The location matches a retained task record", code: "SCAN_OUTPUT_RETAINED" },
      { symptom: "The task directory contents could not be verified", code: "SCAN_OUTPUT_UNVERIFIED" },
      { symptom: "The task directory already has unrelated content", code: "SCAN_OUTPUT_EXISTS" },
      { symptom: "The task ID is reserved or contains illegal characters", code: "RESERVED_OR_INVALID_TASK_ID" },
      { symptom: "The task directory resolves under the capture namespace", code: "RESERVED_TASK_DIRECTORY" },
      { symptom: "No image root is set for independent captures", code: "CAMERA_CAPTURE_ROOT_REQUIRED" },
      { symptom: "Restore Preflight is missing a required confirmation", code: "RESTORE_PREFLIGHT_CONFIRMATION_REQUIRED" },
      { symptom: "The reconstruction method is disabled", code: "RECONSTRUCTION_METHOD_UNAVAILABLE" },
      { symptom: "The cached result failed its version/hash check", code: "RECONSTRUCTION_CACHE_INVALID" },
      { symptom: "The reconstruction method is not accepted", code: "INVALID_RECONSTRUCTION_METHOD" },
      { symptom: "Live View is active during an update prepare", code: "SOFTWARE_UPDATE_LIVE_VIEW_ACTIVE" },
      { symptom: "No maintenance preparation exists to cancel", code: "SOFTWARE_UPDATE_NOT_PREPARED" },
      { symptom: "The completion plan could not be built", code: "TASK_COMPLETION_PREPARATION_FAILED" },
      { symptom: "A capture ID is not registered in this engine lifetime", code: "CAPTURE_NOT_REGISTERED" },
      { symptom: "The operation receipt was cancelled or not completed", code: "OPERATION_CONTINUATION_CANCELLED" },
      { symptom: "The engine has no record of that operation ID", code: "OPERATION_RECEIPT_MISSING" },
    ],
    notMeaning: "Hiding a failure card or closing this dialog confirms nothing. A cleanup that is not confirmed keeps its lock, an unfinished task keeps its files and its lock, and abandonment is the only path that releases it — it never deletes output or marks a task completed.",
  },
];

export function TroubleshootingPane({ snapshot, initialGroup }: { snapshot: EngineSnapshot | null; initialGroup?: string }) {
  const [active, setActive] = useState(() => TROUBLESHOOTING_GROUPS.some(group => group.id === initialGroup) ? initialGroup! : TROUBLESHOOTING_GROUPS[0].id);
  const group = TROUBLESHOOTING_GROUPS.find(entry => entry.id === active) ?? TROUBLESHOOTING_GROUPS[0];
  const readiness = snapshot?.cameraScanReadiness;
  return (
    <div className="trouble-pane">
      <nav className="trouble-pane__nav" aria-label="Troubleshooting topics">
        {TROUBLESHOOTING_GROUPS.map(entry => (
          <button
            key={entry.id}
            type="button"
            className={`trouble-pane__tab${entry.id === group.id ? " is-active" : ""}`}
            aria-pressed={entry.id === group.id}
            onClick={() => setActive(entry.id)}
          >
            {entry.title}
          </button>
        ))}
      </nav>
      <div className="trouble-pane__body">
        <p className="modal-note">{group.intro}</p>
        <table className="trouble-table">
          <thead>
            <tr><th>Symptom</th><th>Engine code</th><th>Exact next action</th></tr>
          </thead>
          <tbody>
            {group.rows.map(row => (
              <tr key={row.code}>
                <td>{row.symptom}</td>
                <td className="trouble-table__code">{row.code}</td>
                <td>{failureAdvice(row.code)?.action ?? "This code has no documented action in this build. Record the exact code and text, keep the reported machine state, and do not assume a restart resolves it."}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {group.id === "readiness" && (
          <section className="trouble-readiness">
            <h3>Live scan readiness for this session</h3>
            {readiness && Array.isArray(readiness.issues) && readiness.issues.length > 0 ? (
              <table className="trouble-table">
                <thead>
                  <tr><th>Item</th><th>Actual</th><th>Requirement</th><th>How to remedy</th></tr>
                </thead>
                <tbody>
                  {readiness.issues.map(issue => (
                    <tr key={issue.key}>
                      <td className="trouble-table__code">{issue.key}</td>
                      <td className="trouble-table__code">{issue.actual === null || issue.actual === undefined ? "UNKNOWN" : String(issue.actual)}</td>
                      <td>{issue.requirement}</td>
                      <td>{issue.remedy}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p className="modal-note">
                {readiness ? "No readiness blockers are published for the current session." : "Readiness evidence is unavailable while the control service has no snapshot."}
              </p>
            )}
          </section>
        )}
        <p className="trouble-pane__guard"><strong>What this does NOT mean:</strong> {group.notMeaning}</p>
      </div>
    </div>
  );
}
