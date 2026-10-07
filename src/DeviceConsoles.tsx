/** Auxiliary bench dialog; device controls remain in the permanent columns. */
import { useState } from "react";
import type { EngineCommand, EngineSnapshot } from "./engine/types";
import { manualControlReason } from "./engine/deviceControls";
import { WorkstationDialog } from "./WorkstationDialog";
export function TurntableBenchDialog({ snapshot, stale, pending, run, onClose, onStop }: { snapshot: EngineSnapshot; stale: boolean; pending: boolean; run: (command: EngineCommand) => Promise<EngineSnapshot>; onClose: () => void; onStop: () => void }) {
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const reason = manualControlReason(snapshot, stale) || (pending ? "Operation pending" : null);
  const xrayConnected = snapshot.workstation?.xray.connected;
  const act = (command: EngineCommand) => { setError(null); void run(command).catch(error => setError(String(error))); };
  return <WorkstationDialog title="Isolated Turntable Bench" onClose={onClose} onStop={onStop}>
    <section className="device-console-section"><h3>Bench mode <small>X-ray stays UNKNOWN / unmonitored</small></h3>
      <p className="device-note">Bench mode prohibits beam commands. A previously unconfirmed OFF cannot be overridden.</p>
      <label className="recovery-confirm"><input type="checkbox" checked={confirmed} disabled={Boolean(reason) || Boolean(xrayConnected)} onChange={event => setConfirmed(event.target.checked)} /> I confirm the X-ray source is physically isolated for this turntable-only task.</label>
      <div className="device-actions device-actions--wide"><button disabled={!confirmed || Boolean(reason) || Boolean(xrayConnected)} onClick={() => act({ type: "turntable_bench_mode", enabled: true, physicallyIsolated: true })}>Confirm bench mode</button><button disabled={Boolean(reason) || !snapshot.deviceControls.turntable.benchMode} onClick={() => act({ type: "turntable_bench_mode", enabled: false, physicallyIsolated: false })}>Leave bench mode</button></div>
      <div className="device-footer"><p role="status">{snapshot.deviceControls.turntable.benchMode ? "Bench mode active" : "Bench mode inactive"}</p><span className={`chip ${snapshot.deviceControls.turntable.benchMode ? "chip--warn" : "chip--muted"}`}>{snapshot.deviceControls.turntable.benchMode ? "BENCH" : "NORMAL"}</span></div>
    </section>
    {error && <p role="alert" className="device-note">{error}</p>}
    <details><summary>Motion and pulse evidence</summary><pre className="diagnostic-json">{JSON.stringify(snapshot.deviceControls.turntable, null, 2)}</pre></details>
  </WorkstationDialog>;
}
