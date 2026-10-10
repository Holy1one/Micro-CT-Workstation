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
export function ReconstructionConsole({ ws, projectOperation = null, openingProject = false, centerView, volumeReady, volume3DAvailable, busy, stale, volumeMode, onVolumeMode, onResetVolume, onOpenReconstruction, onShowEquipment, onStartReconstruction }: { ws: WorkstationView; projectOperation?: DeviceOperation | null; openingProject?: boolean; centerView: string; volumeReady: boolean; volume3DAvailable: boolean; busy: boolean; stale: boolean; volumeMode: "mip" | "opacity"; onVolumeMode: (mode: "mip" | "opacity") => void; onResetVolume: () => void; onOpenReconstruction: () => void; onShowEquipment: () => void; onStartReconstruction: (method: ReconstructionMethodId, options: ReconstructionOptions) => void }) {
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
  const canChoose = reconstruction.unlocked && !loadingProject && !running && (!completed || changing) && !busy && !stale && !pickingCalibration;
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
    : failed ? "Reconstruction failed" : running ? `Reconstructing · ${percent}%` : completed ? "Result cached" : reconstruction.unlocked ? "Ready to reconstruct" : noDataset ? "No scan project loaded" : "Awaiting sealed scan";
  return <div className="recon-workspace" aria-busy={loadingProject}>
    <section className="recon-workspace__facts">
      <strong className={projectFailed || failed ? "tone-danger" : ""} role="status">{title}</strong>
      <div><span>Dataset</span><b title={loadingProject || noDataset ? undefined : `${ws.scanSetup.savePath}/${ws.scanSetup.taskId}`}>{loadingProject ? "Validating saved files…" : noDataset ? "Not loaded" : `${ws.progress.captured} / ${ws.scanSetup.projectionCount} projections`}</b></div>
      <div><span>References</span><b>{loadingProject ? "Validation pending" : noDataset ? "—" : `Dark ${ws.scanFlow.references.preDark} · Flat ${ws.scanFlow.references.preFlat}`}</b></div>
      <div><span>Compute</span><b>{method === "sirt" ? "SIRT · CPU" : "Auto CPU / GPU"}</b></div>
      {loadingProject ? <div className="recon-workspace__progress recon-workspace__load-progress">
        <div className="recon-workspace__load-summary"><span>{loadMessage}</span><b>{loadDeterminate ? `${loadPercent}%` : "—"}</b></div>
        <div className={`recon-workspace__progress-track${loadDeterminate ? "" : " is-indeterminate"}`} role="progressbar" aria-label="Scan project loading progress" aria-valuenow={loadDeterminate ? loadPercent : undefined} aria-valuemin={0} aria-valuemax={100} aria-valuetext={loadDeterminate ? `${loadPercent}% of file data checked` : "Preparing project validation"}><i style={{ width: loadDeterminate ? `${loadPercent}%` : undefined }} /></div>
        <div className="recon-workspace__load-summary"><span>{loadDeterminate ? `${projectProgress!.verifiedFiles} / ${projectProgress!.totalFiles} files verified` : "Reading project metadata…"}</span><b role="status">{loadTime}</b></div>
      </div>
        : projectFailed ? <div className="recon-workspace__progress"><span>{noDataset ? "No dataset loaded" : "Previous dataset unchanged"}</span><button onClick={() => setDetails("project")}>Project error details…</button></div>
        : noDataset ? <div className="recon-workspace__progress"><span>Open a completed project from Task → Open Scan Project…</span></div>
        : <div className="recon-workspace__progress" role="progressbar" aria-label="Reconstruction progress" aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100}><span className="recon-workspace__progress-track"><i style={{ width: `${percent}%` }} /></span><b>{percent}%</b></div>}
    </section>
    <section className="recon-workspace__method-group">
      <h3>Colour mode</h3><div className="recon-workspace__colour-modes"><select aria-label="Reconstruction colour mode" value={mode} disabled={!canChoose} onChange={event => {setMode(event.target.value as ReconstructionOptions["mode"]);setMethod("fdk");}}><option value="monochrome">Monochrome</option><option value="multicolor">Multicolor</option></select></div>
      <p className="recon-workspace__band-note">{mode === "monochrome" ? "Automatic best reference band" : "Automatic stack emission bands"}</p>
      <h3>Method</h3><div className="recon-workspace__methods" role="radiogroup" aria-label="Reconstruction method">{reconstruction.methods.map(entry => <button key={entry.id} role="radio" aria-checked={method === entry.id} disabled={!canChoose || !entry.enabled || entry.id !== "fdk"} title={entry.id !== "fdk" ? "Automatic colour processing currently requires FDK" : englishDisplayText(entry.reason || entry.label)} className={method === entry.id ? "is-selected" : ""} onClick={() => setMethod(entry.id)}>{englishDisplayText(entry.label)}</button>)}</div>
    </section>
    <section className="recon-workspace__actions-group">
      <button className="recon-workspace__primary" disabled={!canStart} onClick={() => { setChanging(false); onStartReconstruction(method, {mode, ...(mode === "multicolor" && calibration ? {materialCalibrationPath: calibration} : {})}); }}>{loadingProject ? "Validating project…" : running ? "Reconstructing…" : completed && !changing ? "Reconstruction complete" : failed ? "Retry reconstruction" : "Start reconstruction"}</button>
      {mode === "multicolor" && <div className="recon-workspace__calibration"><button disabled={!canChoose} title={calibration || "Optional measured two-basis calibration; without it colours show camera responses only"} onClick={selectCalibration}>{pickingCalibration ? "Selecting…" : calibration ? "Clear calibration" : "Material calibration…"}</button><span role="status">{calibrationError ? englishDisplayText(calibrationError) : calibration ? "Engine will validate calibration" : "Colour responses · materials unverified"}</span></div>}
      <button disabled={loadingProject || (!completed && !failed)} onClick={() => { if (failed) setDetails("reconstruction"); else setChanging(value => !value); }}>{failed ? "Error details…" : changing ? "Cancel method change" : "Change method"}</button>
      <div className="recon-workspace__view-controls"><button onClick={onShowEquipment} aria-pressed={centerView === "equipment"}>Equipment</button><button disabled={loadingProject || !completed} onClick={onOpenReconstruction} aria-pressed={centerView === "reconstruction"}>Result</button><button disabled={!volumeControls} onClick={() => onVolumeMode("mip")} aria-pressed={volumeControls && volumeMode === "mip"}>MIP</button><button disabled={!volumeControls} onClick={() => onVolumeMode("opacity")} aria-pressed={volumeControls && volumeMode === "opacity"}>Opacity</button><button disabled={!volumeControls} onClick={onResetVolume}>Reset view</button></div>
    </section>
    {details && <WorkstationDialog title={details === "project" ? "Scan Project Error" : "Reconstruction Error"} onClose={() => setDetails(null)}><p className="reconstruction-full-error">{englishDisplayText(details === "project" ? projectOperation?.error || "Scan project validation failed" : reconstruction.message || "Reconstruction failed")}</p>{details === "project" && <p>The previous dataset is unchanged. Check the reported files, then use Task → Open Scan Project… to retry.</p>}</WorkstationDialog>}
  </div>;
}
