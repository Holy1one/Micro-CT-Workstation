/** Permanent prescribed-direction controls and the single scan-preparation entry.
 *  Same instrument grammar as the camera panel: open device group, titled
 *  move section, mono position readout and a footer with the preflight status. */
import { deviceConnectionLabel, englishDisplayText } from "./engine/native-labels";
import { useState } from "react";
import type { EngineCommand, EngineSnapshot } from "./engine/types";
import { deviceConnectionReason, manualControlReason } from "./engine/deviceControls";
import { LineIcon } from "./icons";
export function TurntablePanel({ snapshot, pending, stale, run, onPreflight, onRecovery, onBench }: { snapshot: EngineSnapshot; pending: boolean; stale: boolean; run: (command: EngineCommand) => Promise<EngineSnapshot>; onPreflight: () => void; onRecovery: () => void; onBench: () => void }) {
  const turntable = snapshot.deviceControls.turntable;
  const ws = snapshot.workstation!;
  const [kind, setKind] = useState<"relative" | "phase">("relative");
  const [angle, setAngle] = useState("3");
  const [error, setError] = useState<string | null>(null);
  const manualReason = manualControlReason(snapshot, stale) || (pending ? "Operation pending" : null);
  const connectionReason = deviceConnectionReason(snapshot, stale) || (pending ? "Operation pending" : null);
  const homeReason = manualReason || (!turntable.connected ? "Connect the turntable" : !ws.xray.connected && !turntable.benchMode ? "Confirmed OFF or physically isolated bench required" : null);
  const motionReason = manualReason || (!turntable.connected || turntable.status?.state !== "IDLE" || !turntable.status.referenceValid || !turntable.status.homed || !turntable.status.rearmed ? "Confirmed HOME reference and IDLE required" : null);
  const number = Number(angle);
  const invalid = !angle.trim() || !Number.isFinite(number) || number < 0 || (kind === "relative" ? number <= 0 || number > 360 : number >= 360) || Math.abs(number * 1000 - Math.round(number * 1000)) > 1e-6;
  const act = (command: EngineCommand) => { setError(null); void run(command).catch(reason => setError(String(reason))); };
  const referenceKnown = Boolean(turntable.status?.referenceValid && turntable.status.homed);
  const position = stale || turntable.confirmedAngleDeg === null ? "— °" : `${turntable.confirmedAngleDeg.toFixed(3)}°`;
  return <section className="panel turntable-panel" id="turntable-panel"><header className="panel__header"><LineIcon name="rotate" size={15} className="panel__icon" /><h2>Turntable</h2><span className={`chip ${!stale && turntable.connected ? "chip--accent" : "chip--muted"}`} title={`Motion state: ${stale ? "UNKNOWN" : turntable.status?.state || "UNKNOWN"}`}>{deviceConnectionLabel(turntable.connected, stale)}</span></header><div className="device-body">
    <div className="device-group">
      <div className="device-group__head"><h3 className="device-group__title">Position</h3><span className={`chip ${referenceKnown ? "chip--accent" : "chip--muted"}`} title={referenceKnown ? "HOME reference confirmed" : "Reference unknown / invalid"}>{referenceKnown ? "REFERENCE OK" : "REFERENCE UNKNOWN"}</span></div>
      <div className="device-well device-well--readout"><span className="device-value" title={position === "— °" ? "Position unknown; confirmed readback required" : position} aria-label={position === "— °" ? "Position unknown" : undefined}>{position}</span></div>
      <div className="device-grid device-grid--4"><button disabled={Boolean(connectionReason)} onClick={() => act({ type: turntable.connected ? "turntable_disconnect" : "turntable_connect" })}>{turntable.connected ? "Disconnect" : "Connect"}</button><button onClick={() => act({ type: "turntable_status" })}>Status</button><button disabled={Boolean(homeReason)} title={englishDisplayText(homeReason) || undefined} onClick={() => act({ type: "turntable_home" })}>HOME</button><button onClick={onBench} title="Bench mode, warning state and manual turntable checks">Bench…</button></div>
    </div>
    <div className="device-section">
      <div className="device-section__head"><h3 className="device-section__title">Move · prescribed direction only</h3></div>
      <div className="device-grid device-grid--pick" style={{ gridTemplateColumns: "minmax(0, 1.4fr) minmax(0, 1fr) 68px" }}><select className="device-control" aria-label="Turntable move kind" title="Targets require confirmed arrival; reverse motion is not offered" value={kind} onChange={event => setKind(event.target.value as typeof kind)}><option value="relative">Forward relative °</option><option value="phase">Forward to phase °</option></select><input className="device-control" aria-label="Prescribed direction angle" value={angle} inputMode="decimal" onChange={event => setAngle(event.target.value)} /><button disabled={Boolean(motionReason) || invalid} title={englishDisplayText(motionReason) || undefined} onClick={() => act({ type: kind === "relative" ? "turntable_relative" : "turntable_to_angle", angleDeg: number })}>Rotate</button></div>
    </div>
    <div className="device-footer"><p role="status" title={englishDisplayText(error || turntable.reason || homeReason || motionReason) || undefined}>{englishDisplayText(error || turntable.reason || homeReason || motionReason || "Motion ready")}</p><span className={`chip ${ws.preflight.tone === "pass" ? "chip--accent" : "chip--muted"}`}>{ws.preflight.word}</span></div>
    <div className="permanent-preflight"><button disabled={stale || (!snapshot.deviceControls.taskLock.locked && (pending || ["running", "paused", "finishing", "stopping"].includes(snapshot.phase)))} onClick={snapshot.deviceControls.taskLock.locked ? onRecovery : onPreflight}>{snapshot.deviceControls.taskLock.locked ? "Resume / end retained task…" : "Prepare scan / Preflight…"}</button><div className="preflight" role="progressbar" aria-valuenow={ws.preflight.percent} aria-valuemin={0} aria-valuemax={100}><span className="preflight__fill" style={{ width: `${ws.preflight.percent}%` }} /></div></div>
  </div></section>;
}
