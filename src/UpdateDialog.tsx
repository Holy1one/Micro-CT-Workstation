/** Presents native update state; confirmation is the only install trigger. */
import { useEffect, useState } from 'react';
import { cancelSoftwareUpdate, checkSoftwareUpdate, confirmSoftwareUpdate, getUpdateStatus, importSoftwareUpdate, type UpdateChannel, type UpdateState } from './platform/updater';

export interface UpdateDialogProps { open: boolean; mode: 'check' | 'import'; onClose: () => void }
const phaseText: Record<UpdateState['phase'], string> = {
  idle: 'Waiting to check', checking: 'Checking for updates', 'no-update': 'The queried release has nothing installable',
  downloading: 'Downloading in the background', verifying: 'Verifying signature and file integrity', ready: 'Package verified; waiting for confirmation',
  preparing: 'Preparing the maintenance stop', applying: 'Update helper started', cancelled: 'Cancelled', failed: 'Update failed',
};
const kinds = { portable: 'Portable EXE', nsis: 'NSIS installer', msi: 'MSI installer' };
const size = (bytes: number) => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
/** Receipt completion is informational; failures retain their alert semantics. */
export function UpdateFeedback({ notice, error }: { notice: string | null; error: string | null }) {
  return <>{notice && <p role="status" className="update-notice">{notice}</p>}{error && <p role="alert" className="update-error">{error}</p>}</>;
}
export function UpdateDialog({ open, mode, onClose }: UpdateDialogProps) {
  const [state, setState] = useState<UpdateState | null>(null);
  const [channel, setChannel] = useState<UpdateChannel>('candidate');
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [confirming, setConfirming] = useState(false);
  async function run(action: () => Promise<UpdateState | null>) {
    setError(null); setPending(true);
    setState(previous => previous ? { ...previous, notice: null, error: null } : previous);
    try { const next = await action(); if (next) setState(next); }
    catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
    finally { setPending(false); }
  }
  useEffect(() => {
    if (!open) return;
    let active = true;
    setConfirming(false);
    const poll = () => void getUpdateStatus().then(next => { if (active) setState(next); }).catch(failure => { if (active) setError(String(failure)); });
    void getUpdateStatus().then(next => {
      if (!active) return;
      setState(next); setChannel(next.channel);
      if (mode === 'check' && ['idle', 'no-update', 'cancelled', 'failed'].includes(next.phase)) void run(() => checkSoftwareUpdate(next.channel));
    }).catch(failure => { if (active) setError(String(failure)); });
    const timer = window.setInterval(poll, 500);
    if (mode === 'import') void run(importSoftwareUpdate);
    return () => { active = false; window.clearInterval(timer); };
  }, [open, mode]);
  if (!open) return null;
  const working = state && ['checking', 'downloading', 'verifying', 'preparing', 'applying'].includes(state.phase);
  const cancellable = state?.updateId && ['checking', 'downloading', 'verifying', 'ready'].includes(state.phase);
  const progress = state?.totalBytes ? Math.min(100, state.downloadedBytes / state.totalBytes * 100) : 0;
  return <div className="dialog-backdrop" role="presentation" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <section className="workstation-dialog update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-title">
      <header className="dialog-header"><h2 id="update-title">Software Update</h2><button type="button" onClick={onClose} aria-label="Close">×</button></header>
      <div className="dialog-content">
        <p>Current version {state?.currentVersion ?? 'reading'} · {state ? kinds[state.kind] : ''}</p>
        <label>Release channel <select value={channel} disabled={Boolean(working)} onChange={event => setChannel(event.target.value as UpdateChannel)}><option value="candidate">Candidate</option><option value="stable">Stable</option></select></label>
        <p role="status">{state ? phaseText[state.phase] : 'Reading update status'}</p>
        {state?.targetVersion && <p>Queried / target version: {state.targetVersion}</p>}
        {state?.source && <p className="update-source">Source: {state.source === 'offline' ? 'signed offline package' : state.source}</p>}
        {state?.totalBytes && <><progress max="100" value={progress} aria-label="Download progress" /><p>{size(state.downloadedBytes)} / {size(state.totalBytes)}</p></>}
        {state?.verified && <p>The release manifest and update file passed their signature, version, kind, size and SHA checks.</p>}
        <UpdateFeedback notice={state?.notice ?? null} error={error ?? state?.error ?? null} />
        {confirming && <div className="update-confirmation"><p>Install {state?.targetVersion}? The application will check device and task state, complete the maintenance stop and exit. You restart it yourself afterwards; tasks and original images are retained.</p><p>If a scan, preview, capture, motion or an unfinished task is active, finish the current operation first.</p></div>}
      </div>
      <footer className="dialog-footer">
        <button type="button" disabled={Boolean(working) || pending} onClick={() => { setConfirming(false); void run(() => checkSoftwareUpdate(channel)); }}>Check and download</button>
        <button type="button" disabled={Boolean(working) || pending} onClick={() => { setConfirming(false); void run(importSoftwareUpdate); }}>Import offline package</button>
        {cancellable && <button type="button" onClick={() => { setConfirming(false); void run(() => cancelSoftwareUpdate(state!.updateId!)); }}>Cancel update</button>}
        {state?.phase === 'ready' && state.verified && <button type="button" disabled={pending} onClick={() => {
          if (!confirming) { setConfirming(true); return; }
          void run(() => confirmSoftwareUpdate(state.updateId!, state.targetVersion!));
        }}>{confirming ? 'Confirm install and exit' : 'Install update…'}</button>}
        <button type="button" onClick={onClose}>Close</button>
      </footer>
    </section>
  </div>;
}
