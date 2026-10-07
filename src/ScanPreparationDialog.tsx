/** One local preparation draft; cancelling never submits partial scan metadata. */
import { useEffect, useMemo, useRef, useState } from "react";
import type { EngineCommand, EngineSnapshot, ReconstructionGeometry, ScanPreparation } from "./engine/types";
import { minutesToSeconds, projectionError, secondsToMinutes } from "./scan-input";
import { shutterExposureMs } from "./engine/deviceControls";
import { WorkstationDialog } from "./WorkstationDialog";

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
    return result.preflightPassed;
  } catch (reason) {
    if (current()) onError(String(reason));
    return false;
  } finally {
    if (scope.finish(request)) onStage(null);
  }
}

export function ScanPreparationDialog({ snapshot, dirtyCamera, onSubmit, onClose, onAdvanced, onStop, resetRevision = 0, stage = null }: { snapshot: EngineSnapshot; dirtyCamera: boolean; onSubmit: (draft: ScanPreparation) => Promise<boolean>; onClose: () => void; onAdvanced: () => void; onStop: () => void; resetRevision?: number; stage?: PreparationStage }) {
  const existing = snapshot.workstation?.scanFlow.geometry;
  const setup = snapshot.workstation!.scanSetup;
  const [values, setValues] = useState<Record<string, string>>(() => ({ taskId: resetRevision ? "" : setup.taskId, count: resetRevision ? "120" : String(setup.projectionCount || ""), maxXray: resetRevision ? "10" : secondsToMinutes(setup.maxXraySec || 600), sodMm: existing?.sodMm.toString() || "", objectToDetectorMm: existing?.objectToDetectorMm.toString() || "", detectorWidthMm: existing?.detectorWidthMm.toString() || "", centerOffsetXMm: existing?.centerOffsetXMm.toString() || "", centerOffsetYMm: existing?.centerOffsetYMm.toString() || "", mirrorX: existing ? String(existing.mirrorX) : "", measurement: existing?.measurement || "" }));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submitting = useRef(false);
  const lifetime = useRef(0);
  useEffect(() => { lifetime.current += 1; return () => { lifetime.current += 1; }; }, []);
  const close = () => { if (!submitting.current) onClose(); };
  const advanced = () => { if (!submitting.current) onAdvanced(); };
  const prepared = useMemo<ScanPreparation | null>(() => {
    const numeric = ["sodMm", "objectToDetectorMm", "detectorWidthMm", "centerOffsetXMm", "centerOffsetYMm"];
    if (!values.taskId.trim() || projectionError(values.count) || numeric.some(key => !values[key].trim() || !Number.isFinite(Number(values[key])))) return null;
    const maxXraySec = minutesToSeconds(values.maxXray);
    if (!maxXraySec || maxXraySec > 600 || !["true", "false"].includes(values.mirrorX) || !["measured", "estimated"].includes(values.measurement)) return null;
    const geometry: ReconstructionGeometry = { sodMm: Number(values.sodMm), objectToDetectorMm: Number(values.objectToDetectorMm), detectorWidthMm: Number(values.detectorWidthMm), centerOffsetXMm: Number(values.centerOffsetXMm), centerOffsetYMm: Number(values.centerOffsetYMm), mirrorX: values.mirrorX === "true", rotationDirection: existing?.rotationDirection || "counterclockwise", measurement: values.measurement as ReconstructionGeometry["measurement"] };
    if (geometry.sodMm <= 0 || geometry.objectToDetectorMm <= 0 || geometry.detectorWidthMm <= 0) return null;
    return { taskId: values.taskId.trim(), projectionCount: Number(values.count), maxXraySec, geometry };
  }, [values, existing?.rotationDirection]);
  const shutter = snapshot.deviceControls.camera?.properties.shutter;
  const exposure = shutter?.confirmed && shutter.actual !== null ? shutterExposureMs(shutter.actual, shutter.code) : null;
  const field = (key: string, label: string, numeric = true) => <label className="scan-geometry-field"><span>{label}</span><input value={values[key]} inputMode={numeric ? "decimal" : "text"} disabled={pending} onChange={event => setValues(current => ({ ...current, [key]: event.target.value }))} /></label>;
  const select = (key: string, label: string, choices: [string, string][]) => <label className="scan-geometry-field"><span>{label}</span><select value={values[key]} disabled={pending} onChange={event => setValues(current => ({ ...current, [key]: event.target.value }))}><option value="">Select</option>{choices.map(([value, text]) => <option key={value} value={value}>{text}</option>)}</select></label>;
  const readiness = snapshot.cameraScanReadiness;
  const readinessIssues = readiness && !readiness.ready ? readiness.issues : [];
  const stageText = pending
    ? stage === "submitting" ? "Submitting the complete task."
      : stage === "checking" ? "Task settings are submitted; preflight is running."
      : "Waiting for the current request to settle."
    : "Cancel discards this draft before submission. Stop stays available; submitted settings remain.";
  return <WorkstationDialog title="Scan Preparation" onClose={close} closeDisabled={pending} onStop={onStop} className="scan-preparation-dialog" footerNote={stageText} footer={<div className="scan-stage-actions"><button disabled={pending} onClick={close}>Cancel</button><button disabled={pending || !prepared || dirtyCamera} onClick={() => { if (!prepared || submitting.current) return; submitting.current = true; const revision = lifetime.current; setPending(true); setError(null); void onSubmit(prepared).then(success => { if (success && lifetime.current === revision) onClose(); }).catch(reason => { if (lifetime.current === revision) setError(String(reason)); }).finally(() => { if (lifetime.current === revision) { submitting.current = false; setPending(false); } }); }}>{pending ? "Checking…" : "Prepare and run preflight"}</button></div>}>
    <p className="scan-prep__lead">Prepare the complete task once. Exposure and storage come from the confirmed camera configuration.</p>
    {pending && <p role="status" className="scan-prep__stage">{stageText} Closing and camera settings are locked. Stop remains available; submitted settings remain.</p>}
    <div className="scan-prep">
      <div className="scan-prep__main">
        <section className="scan-prep-card"><h3>Task</h3><div className="scan-geometry-grid">{field("taskId", "Task ID", false)}{field("count", "Total projections · 1–3600")}{field("maxXray", "Maximum continuous X-ray · min")}</div></section>
        <section className="scan-prep-card"><h3>Geometry</h3><div className="scan-geometry-grid">{field("sodMm", "Source to axis · mm")}{field("objectToDetectorMm", "Axis to detector · mm")}{field("detectorWidthMm", "Active detector width · mm")}{field("centerOffsetXMm", "Horizontal offset · mm")}{field("centerOffsetYMm", "Vertical offset · mm")}{select("mirrorX", "Mirror image horizontally", [["false", "No"], ["true", "Yes"]])}{select("measurement", "Geometry source", [["measured", "Measured"], ["estimated", "Estimated"]])}</div></section>
        <section className="scan-prep-card"><h3>Storage and exposure</h3><dl className="modal-facts"><div><dt>Image root</dt><dd>{snapshot.parameters.savePath || "UNKNOWN"}</dd></div><div><dt>Task directory</dt><dd>{snapshot.parameters.savePath && values.taskId.trim() ? `${snapshot.parameters.savePath}/${values.taskId.trim()}` : "Not configured"}</dd></div><div><dt>Confirmed exposure</dt><dd>{exposure === null ? "UNKNOWN" : `${exposure} ms`}</dd></div><div><dt>Prescribed geometry direction</dt><dd>{existing?.rotationDirection || "counterclockwise"}</dd></div></dl></section>
        {dirtyCamera && <p role="alert" className="scan-prep__alert">Camera draft is not applied. Apply and read back the requested parameters first.</p>}
        {error && <p role="alert" className="scan-stage-error">{error}</p>}
      </div>
      <aside className="scan-prep__side">
        <h3>Fixed scan configuration needs attention</h3>
        {readinessIssues.length === 0
          ? <p className="scan-prep__ready" role="status">Fixed scan configuration is confirmed complete.</p>
          : <div className="scan-readiness-report__table" tabIndex={0} role="region" aria-label="Fixed scan configuration issues"><table className="camera-properties"><thead><tr><th>Item</th><th>Actual</th><th>Required</th><th>Remedy</th></tr></thead><tbody>{readinessIssues.map(issue => <tr key={issue.key}><td>{issue.key}</td><td>{issue.actual === null ? "UNKNOWN" : String(issue.actual)}</td><td>{issue.requirement}</td><td>{issue.remedy}</td></tr>)}</tbody></table></div>}
        <button disabled={pending} onClick={advanced}>Open Camera Settings…</button>
      </aside>
    </div>
  </WorkstationDialog>;
}
