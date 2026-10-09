/** Check, download and installation are separate user actions. */
import { englishDisplayText } from './engine/native-labels';
import { useEffect, useState } from 'react';
import { cancelSoftwareUpdate, checkSoftwareUpdate, downloadSoftwareUpdate, confirmSoftwareUpdate, getUpdateStatus, type UpdateState } from './platform/updater';

export interface UpdateDialogProps { open: boolean; onClose: () => void }
const phaseText: Record<UpdateState['phase'], string> = {
  idle: 'Checking for updates…', checking: 'Checking for updates…', 'no-update': "You're up to date.",
  available: 'A new version is available.', downloading: 'Downloading update…', verifying: 'Finishing download…',
  ready: 'Your update is ready to install.', preparing: 'Preparing to install…', applying: 'Installing update…',
  cancelled: 'Update cancelled.', failed: "Couldn't complete the update.",
};
const size = (bytes: number) => (bytes / (1024 * 1024)).toFixed(1) + ' MB';
export function UpdateFeedback({ notice, error }: { notice: string | null; error: string | null }) {
  return <>{notice && <p role="status" className="update-notice">{englishDisplayText(notice)}</p>}{error && <p role="alert" className="update-error">{englishDisplayText(error)}</p>}</>;
}
export function UpdateDialog({ open, onClose }: UpdateDialogProps) {
  const [state, setState] = useState<UpdateState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  async function run(action: () => Promise<UpdateState>) {
    setError(null); setPending(true);
    try { setState(await action()); }
    catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
    finally { setPending(false); }
  }
  useEffect(() => {
    if (!open) return;
    let active = true;
    setError(null);
    void getUpdateStatus().then(async current => {
      if (!active) return;
      setState(current);
      if (['idle', 'no-update', 'cancelled', 'failed'].includes(current.phase)) {
        setPending(true);
        try { const next = await checkSoftwareUpdate('stable'); if (active) setState(next); }
        finally { if (active) setPending(false); }
      }
    }).catch(failure => { if (active) setError(String(failure)); });
    const timer = window.setInterval(() => void getUpdateStatus()
      .then(next => { if (active) setState(next); })
      .catch(failure => { if (active) setError(String(failure)); }), 500);
    return () => { active = false; window.clearInterval(timer); };
  }, [open]);
  if (!open) return null;
  const working = state && ['checking', 'downloading', 'verifying', 'preparing', 'applying'].includes(state.phase);
  const cancellable = state?.updateId && ['checking', 'downloading', 'verifying'].includes(state.phase);
  const progress = state?.totalBytes ? Math.min(100, state.downloadedBytes / state.totalBytes * 100) : 0;
  return <div className="dialog-backdrop" role="presentation" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <section className="workstation-dialog update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-title">
      <header className="dialog-header"><h2 id="update-title">Software Update</h2><button type="button" onClick={onClose} aria-label="Close">×</button></header>
      <div className="dialog-content">
        <p role="status">{state ? phaseText[state.phase] : 'Checking for updates…'}</p>
        <p>Installed version {state?.currentVersion ?? '…'}</p>
        {state?.targetVersion && ['available', 'downloading', 'verifying', 'ready', 'preparing', 'applying'].includes(state.phase) && <p>Version {state.targetVersion}</p>}
        {state && ['downloading', 'verifying'].includes(state.phase) && <><progress max="100" value={progress} aria-label="Download progress" /><p>{size(state.downloadedBytes)} / {size(state.totalBytes ?? 0)}</p></>}
        {state?.phase === 'ready' && <p>The app will close to install the update. Your tasks and images will be kept.</p>}
        <UpdateFeedback notice={state?.notice ?? null} error={error ?? state?.error ?? null} />
      </div>
      <footer className="dialog-footer">
        {state?.phase === 'available' && <button type="button" disabled={pending} onClick={() => void run(() => downloadSoftwareUpdate(state.updateId!))}>Download update</button>}
        {state?.phase === 'ready' && state.verified && <button type="button" disabled={pending} onClick={() => void run(() => confirmSoftwareUpdate(state.updateId!, state.targetVersion!))}>Install update</button>}
        {cancellable && <button type="button" onClick={() => void run(() => cancelSoftwareUpdate(state!.updateId!))}>Cancel</button>}
        {!working && !['available', 'ready'].includes(state?.phase ?? '') && <button type="button" disabled={pending} onClick={() => void run(() => checkSoftwareUpdate('stable'))}>{state?.phase === 'failed' || error ? 'Try again' : 'Check again'}</button>}
        <button type="button" onClick={onClose}>Close</button>
      </footer>
    </section>
  </div>;
}
