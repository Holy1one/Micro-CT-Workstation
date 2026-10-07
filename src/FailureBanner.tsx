/** One closable failure surface above every page and dialog.
 *
 *  It is portalled to <body> because the design canvas carries a CSS `zoom`, which
 *  would scale a `position: fixed` child. The layer itself never captures pointer
 *  events, so the safety Stop dock stays clickable underneath.
 *
 *  Closing this card hides a notice. It never confirms cleanup and never clears an
 *  engine lock, fault latch or unfinished-task lock. */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { LineIcon } from "./icons";
import type { FailureReport } from "./engine/deviceControls";

export function FailureBanner({ report, onDismiss, onTroubleshooting, onDiagnostics, topOffset }: {
  report: FailureReport | null;
  onDismiss: (key: string) => void;
  onTroubleshooting: (group: string) => void;
  onDiagnostics: () => void;
  /** Client-px clearance below the control-dock lane. The layer lives outside the design
   *  canvas, so its position cannot be expressed in canvas units; the caller supplies the
   *  lane height already scaled by the canvas zoom. */
  topOffset?: number;
}) {
  const [open, setOpen] = useState(false);
  const lastKey = useRef<string | null>(null);
  useEffect(() => {
    if (!report) { setOpen(false); lastKey.current = null; return; }
    // Key-only dependency: a 400 ms snapshot poll with the same key never reopens it.
    if (report.key !== lastKey.current) { lastKey.current = report.key; setOpen(true); }
  }, [report?.key]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!report || !open) return null;
  const persistent = report.kind === "cleanup_unconfirmed" || report.kind === "transport";
  return createPortal(
    <div className="failure-layer" style={topOffset === undefined ? undefined : { paddingTop: `${topOffset}px` }}>
      <section className={`failure-card failure-card--${report.kind}`} role="alert" aria-live="assertive">
        <header className="failure-card__head">
          <LineIcon name="warning" size={14} />
          <h2>{report.title}</h2>
          <span className="failure-card__stage">{report.stage}</span>
          <button type="button" className="failure-card__dismiss"
            title={persistent ? "Hides this notice. Cleanup is still unconfirmed; no lock is cleared." : "Hide this notice"}
            onClick={() => { setOpen(false); onDismiss(report.key); }}>
            {persistent ? "Hide · still unconfirmed" : "Dismiss"} ×
          </button>
        </header>
        <p className="failure-card__reason">{report.reason}</p>
        <dl className="failure-card__facts">
          <div><dt>Failed at</dt><dd>{report.stage}</dd></div>
          <div><dt>Engine code</dt><dd>{report.code ?? "no code in this message"}</dd></div>
          <div><dt>Next action</dt><dd>{report.nextAction}</dd></div>
        </dl>
        <div className="failure-card__actions">
          <button type="button" onClick={() => onTroubleshooting(report.group)}>Open Troubleshooting…</button>
          <button type="button" onClick={onDiagnostics}>View diagnostics</button>
          <button type="button" onClick={() => void navigator.clipboard?.writeText(
            [report.stage, report.code ?? "(no code)", report.reason].join("\n")).catch(() => undefined)}>Copy details</button>
        </div>
        {persistent && <p className="failure-card__guard">Hiding this notice does not confirm cleanup and does not clear the fault or unfinished-task lock. Stop and status reads stay available.</p>}
      </section>
    </div>,
    document.body,
  );
}
