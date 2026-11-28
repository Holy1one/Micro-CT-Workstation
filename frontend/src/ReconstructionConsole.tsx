import { localize } from "./i18n";
/** Compact reconstruction controls; events are supplied by the domain console log. */
import { englishDisplayText } from "./engine/native-labels";
import { useEffect, useRef, useState } from "react";
import type { DeviceOperation, ReconstructionMethodId, ReconstructionOptions, WorkstationView } from "./engine/types";
import { chooseMaterialCalibration } from "./platform/desktopPaths";
import { WorkstationDialog } from "./WorkstationDialog";

export function formatProjectRemainingTime(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return "Estimating remaining time…";
  if (seconds <= 10) return "Finishing soon…";
  return `About ${Math.ceil(seconds / 10) * 10}s remaining`;
}
export function ReconstructionConsole({ ws, projectOperation = null, openingProject = false, centerView, volumeReady, volume3DAvailable, busy, stale, volumeMode, onVolumeMode, onResetVolume, onOpenReconstruction, onShowEquipment, onStartReconstruction, cardGateReason = null, onOpenProject, openProjectReason }: { ws: WorkstationView; projectOperation?: DeviceOperation | null; openingProject?: boolean; centerView: string; volumeReady: boolean; volume3DAvailable: boolean; busy: boolean; stale: boolean; volumeMode: "mip" | "opacity"; onVolumeMode: (mode: "mip" | "opacity") => void; onResetVolume: () => void; onOpenReconstruction: () => void; onShowEquipment: () => void; onStartReconstruction: (method: ReconstructionMethodId, options: ReconstructionOptions) => void; cardGateReason?: string | null; onOpenProject: () => void; openProjectReason: string | null }) {
  const reconstruction = ws.reconstruction;
  const [method, setMethod] = useState<ReconstructionMethodId>("fdk");
  const [changing, setChanging] = useState(false);
  const [details, setDetails] = useState<"project" | "reconstruction" | null>(null);
  const [mode, setMode] = useState<ReconstructionOptions["mode"]>("monochrome");
  const [calibration, setCalibration] = useState<string | null>(null);
  const [calibrationError, setCalibrationError] = useState<string | null>(null);
  const [pickingCalibration, setPickingCalibration] = useState(false);
  const calibrationContext=useRef(0);
  useEffect(() => {calibrationContext.current+=1;return () => {calibrationContext.current+=1;};}, [ws.scanSetup.taskId,ws.scanSetup.savePath]);
  useEffect(() => {setCalibration(null);setCalibrationError(null);setPickingCalibration(false);setMode("monochrome");}, [ws.scanSetup.taskId, ws.scanSetup.savePath]);
  const selectCalibration=() => {
    if (calibration) {setCalibration(null);return;}
    const context=calibrationContext.current;setPickingCalibration(true);setCalibrationError(null);
    void chooseMaterialCalibration().then(value => {if(context===calibrationContext.current)setCalibration(value);})
      .catch(error => {if(context===calibrationContext.current)setCalibrationError(String(error));})
      .finally(() => {if(context===calibrationContext.current)setPickingCalibration(false);});
  };
  useEffect(() => { if (reconstruction.selectedMethod) setMethod(reconstruction.selectedMethod); if (reconstruction.status !== "completed") setChanging(false); }, [reconstruction.selectedMethod, reconstruction.status]);
  const loadingProject = openingProject || Boolean(projectOperation && ["queued", "running", "cancelling"].includes(projectOperation.status));
  const projectFailed = !loadingProject && Boolean(projectOperation && ["failed", "rejected", "cleanup_unconfirmed"].includes(projectOperation.status));
  const projectCancelled = !loadingProject && projectOperation?.status === "cancelled";
  const noDataset = ws.scanSetup.projectionCount === 0;
  const running = reconstruction.status === "running";
  const completed = reconstruction.status === "completed";
  const failed = reconstruction.status === "failed";
  // A camera-card task whose data is not imported and verified is refused here
  // with its explicit reason, independent of the engine's unlocked flag.
  const cardBlocked = Boolean(cardGateReason) && !loadingProject;
  const canChoose = reconstruction.unlocked && !loadingProject && !running && (!completed || changing) && !busy && !stale && !pickingCalibration && !cardGateReason;
  const canStart = canChoose && method === "fdk" && reconstruction.methods.some(entry => entry.id === method && entry.enabled);
  const volumeControls = !loadingProject && completed && centerView === "reconstruction" && volumeReady && volume3DAvailable;
  const percent = completed ? 100 : Math.max(0, Math.min(100, reconstruction.percent));
  const projectProgress = projectOperation?.progress;
  const loadDeterminate = Boolean(projectProgress && projectProgress.totalBytes > 0);
  const loadPercent = Math.max(0, Math.min(99, projectProgress?.percent || 0));
  const loadTime = stale ? "Waiting for the control service…"
    : projectOperation?.status === "cancelling" ? "Waiting for cancellation…"
    : formatProjectRemainingTime(projectProgress?.estimatedRemainingSec);
  const loadMessage = stale ? "Control service unavailable; project loading is unconfirmed."
    : projectOperation?.status === "cancelling" ? "Cancelling project validation…"
    : "Verifying project files…";
  const title = loadingProject ? "Opening scan project…" : projectFailed ? "Scan project could not be opened" : projectCancelled ? "Project opening cancelled"
    : cardBlocked ? "Verified card import required"
    : failed ? "Reconstruction failed" : running ? `Reconstructing · ${percent}%` : completed ? "Result cached" : reconstruction.unlocked ? "Ready to reconstruct" : noDataset ? "No scan project loaded" : "Awaiting sealed scan";
  const step = noDataset ? 0 : completed ? 3 : running || failed ? 2 : 1;
  const selectedReason = reconstruction.methods.find(entry => entry.id === method)?.reason;
  const startReason = cardGateReason || (loadingProject ? "Validating project…" : stale ? "Control service unavailable" : busy ? "Task or device operation occupied" : running ? `Reconstructing · ${percent}%` : completed && !changing ? "Change method to reconstruct again" : pickingCalibration ? "Selecting…" : !reconstruction.unlocked ? reconstruction.message : selectedReason || (!canStart ? reconstruction.message : null));
  return <div className="recon-workspace" aria-busy={loadingProject}>
    <nav className="recon-workspace__steps" aria-label={localize("Reconstruction steps")}>{["Dataset", "Parameters", "Reconstruct", "Result"].map((label, index) => <span key={label} className={index === step ? "is-current" : index < step ? "is-complete" : ""} aria-current={index === step ? "step" : undefined}>{index < step && "\u2713 "}{localize(label)}</span>)}</nav>
    <section className="recon-workspace__facts">
      <strong className={projectFailed || failed || cardBlocked ? "tone-danger" : ""} role="status">{localize(title)}</strong>
      {noDataset && <>{!loadingProject && <p className="recon-workspace__intro">{localize("Open a completed scan project first, then choose a method and view")}</p>}<button className="recon-workspace__primary" disabled={loadingProject || Boolean(openProjectReason)} onClick={onOpenProject}>{localize(loadingProject ? "Validating project…" : "Open Scan Project…")}</button>{openProjectReason && <p className="recon-workspace__reason">{localize(englishDisplayText(openProjectReason))}</p>}</>}
      {cardBlocked && <p className="recon-workspace__gate" role="status">{localize(cardGateReason)}</p>}
      <dl className="recon-workspace__fact-list">
        <dt>{localize("Dataset")}</dt><dd className={noDataset ? "is-muted" : ""} title={loadingProject || noDataset ? undefined : `${ws.scanSetup.savePath}/${ws.scanSetup.taskId}`}>{localize(loadingProject ? "Validating saved files…" : noDataset ? "Not loaded" : `${ws.progress.captured} / ${ws.scanSetup.projectionCount} projections`)}</dd>
        <dt>{localize("References")}</dt><dd className={noDataset ? "is-muted" : ""}>{localize(loadingProject ? "Validation pending" : noDataset ? "–" : `Dark ${ws.scanFlow.references.preDark} · Flat ${ws.scanFlow.references.preFlat}`)}</dd>
        <dt>{localize("Compute")}</dt><dd>{localize(method === "sirt" ? "SIRT · CPU" : "Auto CPU / GPU")}</dd>
      </dl>
      {loadingProject ? <div className="recon-workspace__progress recon-workspace__load-progress">
        <div className="recon-workspace__load-summary"><span>{localize(loadMessage)}</span><b>{localize(loadDeterminate ? `${loadPercent}%` : "–")}</b></div>
        <div className={`recon-workspace__progress-track${loadDeterminate ? "" : " is-indeterminate"}`} role="progressbar" aria-label={localize("Scan project loading progress")} aria-valuenow={loadDeterminate ? loadPercent : undefined} aria-valuemin={0} aria-valuemax={100} aria-valuetext={localize(loadDeterminate ? `${loadPercent}% of file data checked` : "Preparing project validation")}><i style={{ width: loadDeterminate ? `${loadPercent}%` : undefined }} /></div>
        <div className="recon-workspace__load-summary"><span>{localize(loadDeterminate ? `${projectProgress!.verifiedFiles} / ${projectProgress!.totalFiles} files verified` : "Reading project metadata…")}</span><b role="status">{localize(loadTime)}</b></div>
      </div>
        : projectFailed ? <div className="recon-workspace__progress"><span>{localize(noDataset ? "No dataset loaded" : "Previous dataset unchanged")}</span><button onClick={() => setDetails("project")}>{localize("Project error details…")}</button></div>
        : cardBlocked ? <div className="recon-workspace__progress"><span>{localize(cardGateReason)}</span></div>
        : noDataset ? null
        : <div className="recon-workspace__progress" role="progressbar" aria-label={localize("Reconstruction progress")} aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100}><span className="recon-workspace__progress-track"><i style={{ width: `${percent}%` }} /></span><b>{localize(percent)}%</b></div>}
    </section>
    <section className="recon-workspace__actions-group">
      {!noDataset && <><button className="recon-workspace__primary" disabled={!canStart} onClick={() => { setChanging(false); onStartReconstruction(method, {mode, ...(mode === "multicolor" && calibration ? {materialCalibrationPath: calibration} : {})}); }}>{localize(loadingProject ? "Validating project…" : running ? `Reconstructing · ${percent}%` : completed && !changing ? "Reconstruction complete" : failed ? "Retry reconstruction" : cardBlocked ? "Import and verify card data first" : "Start reconstruction")}</button>
      {!canStart && startReason && <p className="recon-workspace__reason">{localize(englishDisplayText(startReason))}</p>}</>}
      <div className="recon-workspace__card recon-workspace__parameters-card">
      <h3>{localize("Parameters")}</h3>
      <label className="recon-workspace__colour-label">{localize("Colour mode")}</label><div className="recon-workspace__colour-modes"><select aria-label={localize("Reconstruction colour mode")} value={mode} disabled={!canChoose} onChange={event => {setMode(event.target.value as ReconstructionOptions["mode"]);setMethod("fdk");}}><option value="monochrome">{localize("Monochrome")}</option><option value="multicolor">{localize("Multicolor")}</option></select></div>
      <p className="recon-workspace__band-note">{localize(mode === "monochrome" ? "Automatic best reference band" : "Automatic stack emission bands")}</p>
      {mode === "multicolor" && <div className="recon-workspace__calibration"><button disabled={!canChoose} title={localize(calibration || "Optional measured two-basis calibration; without it colours show camera responses only")} onClick={selectCalibration}>{localize(pickingCalibration ? "Selecting…" : calibration ? "Clear calibration" : "Material calibration…")}</button><span role="status">{localize(calibrationError ? englishDisplayText(calibrationError) : calibration ? "Engine will validate calibration" : "Colour responses · materials unverified")}</span></div>}
      </div>
      <div className="recon-workspace__card recon-workspace__method-card">
      <div className="recon-workspace__card-head"><h3>{localize("Method")}</h3><button className="recon-workspace__text-action" disabled={loadingProject || (!completed && !failed)} onClick={() => { if (failed) setDetails("reconstruction"); else setChanging(value => !value); }}>{localize(failed ? "Error details…" : changing ? "Cancel method change" : "Change method")}</button></div><div className="recon-workspace__methods" role="radiogroup" aria-label={localize("Reconstruction method")}>{reconstruction.methods.map(entry => <button key={entry.id} role="radio" aria-checked={method === entry.id} disabled={!canChoose || !entry.enabled || entry.id !== "fdk"} title={localize(entry.id !== "fdk" ? "Automatic colour processing currently requires FDK" : englishDisplayText(entry.reason || entry.label))} className={method === entry.id ? "is-selected" : ""} onClick={() => setMethod(entry.id)}>{localize(englishDisplayText(entry.label))}</button>)}</div>
      </div>
      <div className="recon-workspace__card recon-workspace__view-card"><h3>{localize("View")}</h3><div className="recon-workspace__view-controls"><button onClick={onShowEquipment} aria-pressed={centerView === "equipment"}>{localize("Equipment")}</button><button disabled={loadingProject || !completed} onClick={onOpenReconstruction} aria-pressed={centerView === "reconstruction"}>{localize("Result")}</button><button disabled={!volumeControls} onClick={() => onVolumeMode("mip")} aria-pressed={volumeControls && volumeMode === "mip"}>{localize("MIP")}</button><button disabled={!volumeControls} onClick={() => onVolumeMode("opacity")} aria-pressed={volumeControls && volumeMode === "opacity"}>{localize("Opacity")}</button><button disabled={!volumeControls} onClick={onResetVolume}>{localize("Reset view")}</button></div></div>
    </section>
    {details && <WorkstationDialog title={localize(details === "project" ? "Scan Project Error" : "Reconstruction Error")} onClose={() => setDetails(null)}><p className="reconstruction-full-error">{localize(englishDisplayText(details === "project" ? projectOperation?.error || "Scan project validation failed" : reconstruction.message || "Reconstruction failed"))}</p>{details === "project" && <p>{localize("The previous dataset is unchanged. Check the reported files, then use Task → Open Scan Project… to retry.")}</p>}</WorkstationDialog>}
  </div>;
}
