/** Retained-task recovery and explicit abandonment share the frozen task identity. */
import { englishDisplayText, unicodeDisplayText } from "./engine/native-labels";
import { useEffect, useRef, useState } from "react";
import type { EngineCommand, EngineSnapshot } from "./engine/types";
import { WorkstationDialog } from "./WorkstationDialog";
export function TaskRecoveryDialog({ snapshot, pending, stale, run, onClose, onStop }: { snapshot: EngineSnapshot; pending: boolean; stale: boolean; run: (command: EngineCommand) => Promise<EngineSnapshot>; onClose: () => void; onStop: () => void }) {
  const lock = snapshot.deviceControls.taskLock;
  const wasLocked = useRef(lock.locked);
  useEffect(() => {
    if (lock.locked) wasLocked.current = true;
    else if (wasLocked.current && snapshot.phase !== "stopping") { wasLocked.current = false; onClose(); }
  }, [lock.locked, snapshot.phase, onClose]);
  const [restoreConfirmed, setRestoreConfirmed] = useState(false);
  const [unchangedConfirmed, setUnchangedConfirmed] = useState(false);
  const [homeOriginConfirmed, setHomeOriginConfirmed] = useState(false);
  const [abandonConfirmed, setAbandonConfirmed] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  useEffect(() => { setRestoreConfirmed(false); setUnchangedConfirmed(false); setHomeOriginConfirmed(false); setAbandonConfirmed(false); }, [snapshot.engineSessionId, lock.taskId, lock.savePath]);
  const act = (command: EngineCommand) => { setMessage(null); void run(command).then(() => setMessage("Request completed; use the engine task state for the next step")).catch(reason => setMessage(String(reason))); };
  const status = snapshot.deviceControls.turntable.status;
  const atHomeOrigin = status?.positionPulses === 0 && status.hallActive;
  return <WorkstationDialog title="Retained Task Recovery" onClose={onClose} onStop={onStop}>
    <dl className="modal-facts"><div><dt>Task</dt><dd>{unicodeDisplayText(lock.taskId || snapshot.parameters.taskId || "UNKNOWN")}</dd></div><div><dt>Frozen root</dt><dd>{unicodeDisplayText(lock.savePath || snapshot.parameters.savePath || "UNKNOWN")}</dd></div><div><dt>Lock / status</dt><dd>{lock.locked ? englishDisplayText(lock.reason) || "Locked" : "Unlocked"} · {lock.status}</dd></div></dl>
    <p>Retained files and frozen acquisition settings remain unchanged. Ordinary manual controls stay locked until confirmed cleanup and explicit abandonment.</p>
    <label className="recovery-confirm"><input type="checkbox" checked={restoreConfirmed} onChange={event => setRestoreConfirmed(event.target.checked)} /> I confirm controlled recovery using the retained task’s frozen X-ray voltage/current, exposure, continuous-output limit and camera profile, including USB Auto Shut Down release.</label>
    <label className="recovery-confirm"><input type="checkbox" checked={unchangedConfirmed} onChange={event => { setUnchangedConfirmed(event.target.checked); setHomeOriginConfirmed(false); }} /> The turntable and sample have not moved. The controller has stayed powered without a reset, or remains at the exact most recent HOME position confirmed below. Keep the existing reference; do not run HOME.</label>
    {unchangedConfirmed && atHomeOrigin && !status?.referenceValid && <label className="recovery-confirm"><input type="checkbox" checked={homeOriginConfirmed} onChange={event => setHomeOriginConfirmed(event.target.checked)} /> The most recent controlled HOME completed at this exact position; the turntable and sample have stayed there since.</label>}
    <div className="permanent-actions">
      <button disabled={!restoreConfirmed || pending || stale || !lock.locked} onClick={() => act({ type: "preflight", restore: true, confirmUsbAutoShutdownRelease: true, confirmTurntableUnchanged: unchangedConfirmed, confirmReferenceAtHome: unchangedConfirmed && atHomeOrigin && homeOriginConfirmed })}>Restore preflight</button>
      <button disabled={pending || stale || !snapshot.workstation?.dock.restore} title={englishDisplayText(snapshot.workstation?.dock.restoreReason)} onClick={() => act({ type: "restore_previous" })}>Verify / restore</button>
      {!unchangedConfirmed && !snapshot.homed && <button disabled={pending || stale || !snapshot.preflightPassed} onClick={() => act({ type: "home" })}>Controlled HOME</button>}
    </div>
    <p>Unchanged-position recovery checks the current controller position and retained files without moving the turntable. A lost reference needs compatible controller firmware; unsupported or inconsistent positions remain blocked.</p>
    <hr />
    <label className="recovery-confirm"><input type="checkbox" checked={abandonConfirmed} onChange={event => setAbandonConfirmed(event.target.checked)} /> Explicitly abandon this task and retain every existing file.</label>
    <button disabled={!abandonConfirmed || pending || stale || !lock.locked || ["running", "finishing", "stopping"].includes(snapshot.phase) || snapshot.operations.some(operation => ["queued", "running", "cancelling"].includes(operation.status))} onClick={() => act({ type: "abandon_scan" })}>Abandon task and retain files</button>
    {message && <p role="status">{englishDisplayText(message)}</p>}
  </WorkstationDialog>;
}
