/** One local preparation draft; cancelling never submits partial scan metadata. */
import { englishDisplayText, englishPropertyLabel, unicodeDisplayText } from "./engine/native-labels";
import { useEffect, useMemo, useRef, useState } from "react";
import type { EngineCommand, EngineSnapshot, ReconstructionGeometry, ScanPreparation } from "./engine/types";
import { projectionError, xrayLimitSeconds, xrayLimitSummary, xrayLimitText } from "./scan-input";
import { displayFailureText, shutterExposureMs } from "./engine/deviceControls";
import { WorkstationDialog } from "./WorkstationDialog";
import { ScintillatorStackFields, blankLayer, parseLayerDrafts, type LayerDraft } from "./ScintillatorStackFields";

type PreparationContext = { engineSessionId: string; taskId: string };
type PreparationRequest = PreparationContext & { revision: number; previousTaskId: string; taskTransitionObserved: boolean };
export type PreparationStage = "submitting" | "checking" | null;

/** Presentation ownership only: the engine still owns submitted settings, operations and cleanup. */
export function createPreparationRequestScope() {
  let revision = 0;
  let active: PreparationRequest | null = null;
  let taskTransition: PreparationRequest | null = null;
  const accepts = (request: PreparationRequest, context: PreparationContext | null) => {
    if (!context || context.engineSessionId !== request.engineSessionId) return false;
    if (context.taskId === request.taskId) { request.taskTransitionObserved = true; return true; }
    return !request.taskTransitionObserved && context.taskId === request.previousTaskId;
  };
  return {
    get revision() { return revision; },
    get pending() { return active !== null; },
    replace(expectedRevision?: number) {
      if (active || (expectedRevision !== undefined && revision !== expectedRevision)) return false;
      revision += 1;
      return true;
    },
    begin(context: PreparationContext, taskId: string) {
      if (active) return null;
      active = { ...context, taskId, previousTaskId: context.taskId, revision, taskTransitionObserved: false };
      taskTransition = context.taskId === taskId ? null : active;
      return active;
    },
    current(request: PreparationRequest, context: PreparationContext | null) {
      return active === request && request.revision === revision && accepts(request, context);
    },
    acceptsContext(context: PreparationContext | null) { return active !== null && accepts(active, context); },
    acceptsTaskTransition(previous: PreparationContext | null, next: PreparationContext | null) {
      if (!taskTransition || !previous || !next || previous.engineSessionId !== taskTransition.engineSessionId
        || next.engineSessionId !== taskTransition.engineSessionId || previous.taskId !== taskTransition.previousTaskId || next.taskId !== taskTransition.taskId) return false;
      taskTransition.taskTransitionObserved = true;
      taskTransition = null;
      return true;
    },
    invalidate(preserveSubmittedTransition = false) { revision += 1; active = null; if (!preserveSubmittedTransition) taskTransition = null; },
    finish(request: PreparationRequest) {
      if (active !== request || request.revision !== revision) return false;
      active = null;
      return true;
    },
  };
}
export type PreparationRequestScope = ReturnType<typeof createPreparationRequestScope>;

/** Each await is a cancellation boundary; atomic prepare acknowledgement cannot be undone by closing UI. */
export async function runScanPreparationRequest(scope: PreparationRequestScope, draft: ScanPreparation, currentContext: () => PreparationContext | null,
  run: (command: EngineCommand) => Promise<EngineSnapshot>, onStage: (stage: PreparationStage) => void, onError: (message: string) => void): Promise<boolean> {
  const context = currentContext();
  if (!context) return false;
  const request = scope.begin(context, draft.taskId);
  if (!request) return false;
  const current = () => scope.current(request, currentContext());
  const matchesResult = (result: EngineSnapshot) => result.engineSessionId === request.engineSessionId && result.workstation?.scanSetup.taskId === request.taskId;
  onStage("submitting");
  try {
    const prepared = await run({ type: "prepare_scan", ...draft });
    if (!current()) return false;
    if (!matchesResult(prepared)) throw new Error("Preparation acknowledgement does not match the requested task and control session");
    onStage("checking");
    const result = await run({ type: "preflight" });
    if (!current()) return false;
    if (!matchesResult(result)) throw new Error("Preflight result does not match the prepared task and control session");
    if (!result.preflightPassed) onError(result.lastError || "Preflight did not pass. Review the requirements below before retrying.");
    return result.preflightPassed;
  } catch (reason) {
    if (current()) onError(String(reason));
    return false;
  } finally {
    if (scope.finish(request)) onStage(null);
  }
}

/** Read-only prerequisites, not a substitute for the engine's complete preflight. */
export function scanPreparationRequirements(snapshot: EngineSnapshot, dirtyCamera: boolean) {
  const camera = snapshot.deviceControls.camera;
  const xray = snapshot.workstation!.xray;
  const turntable = snapshot.deviceControls.turntable;
  return [
    { label: "Turntable", available: Boolean(turntable?.connected), detail: turntable?.connected ? "Connected; preflight validates motion state." : "Connect the turntable before preflight." },
    { label: "Camera", available: Boolean(camera?.connected && camera.cleanupConfirmed && !camera.isolated), detail: camera?.connected ? camera.isolated || !camera.cleanupConfirmed ? "Cleanup is unconfirmed; reconnect only after cleanup." : "Connected; preflight validates acquisition." : "Select and connect the camera." },
    { label: "Camera settings", available: !dirtyCamera && snapshot.cameraScanReadiness?.ready === true, detail: dirtyCamera ? "Apply the camera draft and confirm the readback." : snapshot.cameraScanReadiness?.ready === true ? "Fixed scan configuration is confirmed." : "Resolve the camera requirements below." },
    { label: "Image storage", available: Boolean(snapshot.parameters.savePath), detail: snapshot.parameters.savePath ? "Storage root is configured; preflight validates task output." : "Choose Image storage root in the Camera panel." },
    { label: "X-ray source", available: xray.connected, detail: xray.connected ? "Connected; preflight checks output, temperature and interlock." : "Connect the X-ray source." },
    { label: "X-ray output", available: xray.connected && xray.beamState === "off", detail: xray.connected && xray.beamState === "off" ? "Output OFF is confirmed; preflight revalidates it." : "Output OFF is not confirmed. Preflight must turn it OFF and verify the readback." },
    { label: "X-ray interlock", available: xray.connected && snapshot.safety.interlockOk, detail: xray.connected && snapshot.safety.interlockOk ? "Interlock reads clear; preflight revalidates it." : "Interlock is not confirmed clear. Check the X-ray panel." },
    { label: "X-ray temperature", available: xray.connected && xray.tempC !== null && Number.isFinite(xray.tempC) && xray.tempC <= 65, detail: xray.connected && xray.tempC !== null ? `${xray.tempC} °C · preflight limit 65 °C.` : "Temperature is unknown. Preflight requires a fresh reading at or below 65 °C." },
    { label: "X-ray setpoints", available: xray.connected && xray.setpointConfirmed, detail: xray.setpointConfirmed ? "Voltage and current readbacks are confirmed." : "Send voltage and current in the X-ray panel and confirm readback." },
    { label: "USB Auto Shut Down", available: xray.connected && xray.usbAutoShutDownKnown && !xray.usbAutoShutDown, detail: xray.usbAutoShutDownKnown && !xray.usbAutoShutDown ? "Disabled with confirmed readback." : "Disable it in the X-ray panel and confirm readback." },
  ];
}

export function ScanPreparationDialog({ snapshot, dirtyCamera, submissionError = null, onSubmit, onClose, onAdvanced, onStop, resetRevision = 0, stage = null }: { snapshot: EngineSnapshot; dirtyCamera: boolean; submissionError?: string | null; onSubmit: (draft: ScanPreparation) => Promise<boolean>; onClose: () => void; onAdvanced: () => void; onStop: () => void; resetRevision?: number; stage?: PreparationStage }) {
  const existing = snapshot.workstation?.scanFlow.geometry;
  const setup = snapshot.workstation!.scanSetup;
  const [values, setValues] = useState<Record<string, string>>(() => ({ taskId: resetRevision ? "" : setup.taskId, count: resetRevision ? "120" : String(setup.projectionCount || ""), maxXray: resetRevision ? "10:00" : xrayLimitText(setup.maxXraySec || 600), sodMm: existing?.sodMm.toString() || "", objectToDetectorMm: existing?.objectToDetectorMm.toString() || "", detectorWidthMm: existing?.detectorWidthMm.toString() || "", centerOffsetXMm: existing?.centerOffsetXMm.toString() || "", centerOffsetYMm: existing?.centerOffsetYMm.toString() || "", mirrorX: existing ? String(existing.mirrorX) : "", measurement: existing?.measurement || "" }));
  const [pending, setPending] = useState(false);
  const [earlyRelease, setEarlyRelease] = useState(resetRevision ? true : setup.earlyRelease ?? true);
  const [stacked, setStacked] = useState(Boolean(existing?.scintillatorLayers?.length));
  const [layers, setLayers] = useState<LayerDraft[]>(() => Array.from({ length: 5 }, (_, index) => {
    const layer = existing?.scintillatorLayers?.[index];
    return layer ? { materialName: layer.materialName, emissionColor: layer.emissionColor, thicknessMm: String(layer.thicknessMm) } : blankLayer();
  }));
  const [error, setError] = useState<string | null>(null);
  const [attemptLogs, setAttemptLogs] = useState<ReadonlySet<string> | null>(null);
  const submitting = useRef(false);
  const lifetime = useRef(0);
  useEffect(() => { lifetime.current += 1; return () => { lifetime.current += 1; }; }, []);
  const close = () => { if (!submitting.current) onClose(); };
  const advanced = () => { if (!submitting.current) onAdvanced(); };
  const prepared = useMemo<ScanPreparation | null>(() => {
    const numeric = ["sodMm", "objectToDetectorMm", "detectorWidthMm", "centerOffsetXMm", "centerOffsetYMm"];
    if (!values.taskId.trim() || projectionError(values.count) || numeric.some(key => !values[key].trim() || !Number.isFinite(Number(values[key])))) return null;
    const maxXraySec = xrayLimitSeconds(values.maxXray);
    if (!maxXraySec || !["true", "false"].includes(values.mirrorX) || !["measured", "estimated"].includes(values.measurement)) return null;
    const geometry: ReconstructionGeometry = { sodMm: Number(values.sodMm), objectToDetectorMm: Number(values.objectToDetectorMm), detectorWidthMm: Number(values.detectorWidthMm), centerOffsetXMm: Number(values.centerOffsetXMm), centerOffsetYMm: Number(values.centerOffsetYMm), mirrorX: values.mirrorX === "true", rotationDirection: existing?.rotationDirection || "counterclockwise", measurement: values.measurement as ReconstructionGeometry["measurement"] };
    if (geometry.sodMm <= 0 || geometry.objectToDetectorMm <= 0 || geometry.detectorWidthMm <= 0) return null;
    const scintillatorLayers = parseLayerDrafts(stacked, layers);
    if (!scintillatorLayers) return null;
    if (scintillatorLayers.length) geometry.scintillatorLayers = scintillatorLayers;
    return { taskId: values.taskId.trim(), projectionCount: Number(values.count), maxXraySec, earlyRelease, geometry };
  }, [values, existing?.rotationDirection, stacked, layers, earlyRelease]);
  const shutter = snapshot.deviceControls.camera?.properties.shutter;
  const exposure = shutter?.confirmed && shutter.actual !== null ? shutterExposureMs(shutter.actual, shutter.code) : null;
  const field = (key: string, label: string, numeric = true) => <label className="scan-geometry-field"><span>{label}</span><input value={values[key]} inputMode={numeric ? "decimal" : "text"} disabled={pending} onChange={event => setValues(current => ({ ...current, [key]: event.target.value }))} /></label>;
  const select = (key: string, label: string, choices: [string, string][]) => <label className="scan-geometry-field"><span>{label}</span><select value={values[key]} disabled={pending} onChange={event => setValues(current => ({ ...current, [key]: event.target.value }))}><option value="">Select</option>{choices.map(([value, text]) => <option key={value} value={value}>{text}</option>)}</select></label>;
  const readiness = snapshot.cameraScanReadiness;
  const readinessIssues = readiness && !readiness.ready ? readiness.issues : [];
  const requirements = scanPreparationRequirements(snapshot, dirtyCamera);
  const operation = snapshot.operations.find(item => item.operationId === snapshot.operationId && item.kind === "preflight");
  const recentLogs = attemptLogs ? snapshot.logs.filter(entry => !attemptLogs.has(entry.id) && ["preflight", "camera", "xray", "nano", "devices"].includes(entry.source)).slice(-6) : [];
  const displayedError = submissionError || error;
  const stageText = pending
    ? stage === "submitting" ? "Submitting the complete task."
      : stage === "checking" ? `Preflight is running${operation ? ` · ${operation.status} · ${englishDisplayText(operation.stage)}` : "; waiting for the operation receipt"}.`
      : "Waiting for the current request to settle."
    : "Cancel discards this draft before submission. Stop stays available; submitted settings remain.";
  return <WorkstationDialog title="Scan Preparation" onClose={close} closeDisabled={pending} onStop={onStop} className="scan-preparation-dialog" footerNote={stageText} footer={<div className="scan-stage-actions"><button disabled={pending} onClick={close}>Cancel</button><button disabled={pending || !prepared || dirtyCamera} onClick={() => { if (!prepared || submitting.current) return; submitting.current = true; const revision = lifetime.current; setPending(true); setError(null); setAttemptLogs(new Set(snapshot.logs.map(entry => entry.id))); void onSubmit(prepared).then(success => { if (lifetime.current === revision) { if (success) onClose(); else setError("Preflight is not complete. Review the requirements and the reported reason before retrying."); } }).catch(reason => { if (lifetime.current === revision) setError(String(reason)); }).finally(() => { if (lifetime.current === revision) { submitting.current = false; setPending(false); } }); }}>{pending ? "Checking…" : "Prepare and run preflight"}</button></div>}>
    <p className="scan-prep__lead">Prepare the complete task once. Exposure and storage come from the confirmed camera configuration.</p>
    <label className="device-note"><input type="checkbox" checked={earlyRelease} disabled={pending} onChange={event => setEarlyRelease(event.target.checked)} /> Move as soon as exposure ends</label>
    <p className="device-note">Enabled by default for SD scans. Each frame falls back to confirmed card completion if the early signal is absent. After the scan, X-ray OFF is confirmed before images are imported into the task folder. Once every local file is verified, only this scan’s SD images are removed; existing card files are kept.</p>
    {(pending || displayedError) && <section className="scan-prep__result" aria-live="polite"><h3>{pending ? "Preflight in progress" : "Preflight needs attention"}</h3><p className={displayedError ? "scan-stage-error" : "scan-prep__stage"}>{displayedError ? displayFailureText(displayedError) : stageText}</p>{recentLogs.length > 0 && <ul className="scan-prep__log">{recentLogs.map(entry => <li key={entry.id}>{displayFailureText(entry.message)}</li>)}</ul>}{pending && <p className="scan-prep__stage">Waiting for engine confirmation. Stop remains available.</p>}</section>}
    <div className="scan-prep">
      <div className="scan-prep__main">
        <ScintillatorStackFields enabled={stacked} rows={layers} disabled={pending} onEnabled={setStacked} onRows={setLayers} />
        <section className="scan-prep-card"><h3>Task</h3><div className="scan-geometry-grid">{field("taskId", "Task ID", false)}{field("count", "Total projections · 1–3600")}{field("maxXray", "Maximum continuous X-ray · mm:ss (1 s–16:40)", false)}</div></section>
        <section className="scan-prep-card"><h3>Geometry</h3><div className="scan-geometry-grid">{field("sodMm", "Source to axis · mm")}{field("objectToDetectorMm", "Axis to detector · mm")}{field("detectorWidthMm", "Active detector width · mm")}{field("centerOffsetXMm", "Horizontal offset · mm")}{field("centerOffsetYMm", "Vertical offset · mm")}{select("mirrorX", "Mirror image horizontally", [["false", "No"], ["true", "Yes"]])}{select("measurement", "Geometry source", [["measured", "Measured"], ["estimated", "Estimated"]])}</div></section>
        <section className="scan-prep-card"><h3>Storage and exposure</h3><dl className="modal-facts"><div><dt>Image root</dt><dd>{unicodeDisplayText(snapshot.parameters.savePath || "UNKNOWN")}</dd></div><div><dt>Task directory</dt><dd>{snapshot.parameters.savePath && values.taskId.trim() ? `${snapshot.parameters.savePath}/${values.taskId.trim()}` : "Not configured"}</dd></div><div><dt>Confirmed exposure</dt><dd>{exposure === null ? "UNKNOWN" : `${exposure} ms`}</dd></div><div><dt>Prescribed geometry direction</dt><dd>{existing?.rotationDirection || "counterclockwise"}</dd></div></dl></section>
        {cameraBodyCheckRequired(snapshot) && <section className="scan-prep-card" aria-label="Camera body checks"><h3>Camera body checks</h3><p className="device-note">Before preflight, use the D7100 shooting menu to set Multiple exposure mode to Off. If its indicator is lit or flashing, stop and complete the check on the camera before starting the scan.</p><p className="device-note">A USB connection or a grayed-out menu does not confirm Off. This setting cannot be read by the software; it is an operator check.</p></section>}
        {dirtyCamera && <p role="alert" className="scan-prep__alert">Camera draft is not applied. Apply and read back the requested parameters first.</p>}
      </div>
      <aside className="scan-prep__side">
        <h3>Preflight requirements</h3>
        <p className={prepared ? "scan-prep__ready" : "scan-prep__alert"}>{prepared ? `Task draft is complete · X-ray limit ${xrayLimitSummary(prepared.maxXraySec)}; preflight validates the submitted settings.` : "Complete the task ID, projection count, X-ray time and all geometry fields and selections to enable preflight."}</p>
        <ul className="scan-prep__requirements">{requirements.map(item => <li key={item.label}><div><strong>{item.label}</strong><span className={item.available ? "scan-prep__available" : "scan-prep__missing"}>{item.available ? "Available" : "Needs attention"}</span></div><p>{item.detail}</p></li>)}</ul>
        <h3>Camera requirements</h3>
        {readinessIssues.length === 0
          ? <p className="scan-prep__ready" role="status">{readiness?.ready ? "Fixed scan configuration is confirmed complete." : "Camera readiness is unknown; connect and read its parameters."}</p>
          : <ul className="scan-prep__requirements" aria-label="Fixed scan configuration issues">{readinessIssues.map(issue => <li key={issue.key}><div><strong>{englishPropertyLabel(issue.key)}</strong><span>{issue.actual === null ? "UNKNOWN" : unicodeDisplayText(String(issue.actual))}</span></div><p>Required: {englishDisplayText(issue.requirement)}</p><p>{englishDisplayText(issue.remedy)}</p></li>)}</ul>}
        <button disabled={pending} onClick={advanced}>Open Camera Settings…</button>
      </aside>
    </div>
  </WorkstationDialog>;
}

/** Informational only: no attestation, readiness override or automatic command. */
export function cameraBodyCheckRequired(snapshot: EngineSnapshot): boolean {
  const camera = snapshot.deviceControls.camera;
  return camera?.backend === "windows_wpd_nikon_d7100" || /^(?:NIKON\s+)?D7100$/i.test(camera?.identity?.model.trim() || "");
}
