import { localize } from "./i18n";
/** Auxiliary bench dialog; device controls remain in the permanent columns. */
import { englishDisplayText, diagnosticJson } from "./engine/native-labels";
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
  return <WorkstationDialog className="bench-dialog" title={localize("Isolated Turntable Bench")} onClose={onClose} onStop={onStop}>
    <section className="device-console-section"><h3>{localize("Bench mode ")}</h3>
      <div className="bench-warning" role="note" aria-label={localize("Bench mode safety warning")}><strong>{localize("X-ray stays UNKNOWN / unmonitored")}</strong><p>{localize("Bench mode prohibits beam commands. A previously unconfirmed OFF cannot be overridden.")}</p></div>
      <label className="recovery-confirm"><input type="checkbox" checked={confirmed} disabled={Boolean(reason) || Boolean(xrayConnected)} onChange={event => setConfirmed(event.target.checked)} />{localize(" I confirm the X-ray source is physically isolated for this turntable-only task.")}</label>
      <div className="bench-reasons"><p>{localize(xrayConnected ? "X-ray is connected; confirmation is currently unavailable." : reason || (!confirmed ? "Confirm physical isolation before enabling bench mode." : "Physical isolation confirmed for this request."))}</p><p>{localize("Leaving bench mode is available only while bench mode is enabled.")}</p></div>
      <div className="device-actions device-actions--wide"><button className="bench-enable" disabled={!confirmed || Boolean(reason) || Boolean(xrayConnected)} onClick={() => act({ type: "turntable_bench_mode", enabled: true, physicallyIsolated: true })}>{localize("Confirm bench mode")}</button><button disabled={Boolean(reason) || !snapshot.deviceControls.turntable.benchMode} onClick={() => act({ type: "turntable_bench_mode", enabled: false, physicallyIsolated: false })}>{localize("Leave bench mode")}</button></div>
      <div className="device-footer"><p role="status" className={`bench-mode-status ${stale || snapshot.deviceControls.turntable.benchMode ? "is-warning" : ""}`}>{localize(stale ? "Current mode: UNKNOWN / unconfirmed" : snapshot.deviceControls.turntable.benchMode ? "Current mode: BENCH (bench mode active)" : "Current mode: Normal (bench mode inactive)")}</p></div>
    </section>
    {error && <p role="alert" className="device-note">{localize(englishDisplayText(error))}</p>}
    <details><summary>{localize("Motion and pulse evidence")}</summary><pre className="diagnostic-json">{diagnosticJson(snapshot.deviceControls.turntable)}</pre></details>
  </WorkstationDialog>;
}
