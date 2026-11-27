import { localize } from "./i18n";
/** Shared modal chrome. Closing UI never performs a device action implicitly. */
import { useEffect, useRef, type ReactNode } from "react";
export function WorkstationDialog({ title, titleIcon, onClose, children, onStop, closeDisabled = false, className = "", footer, footerNote }: { title: string; titleIcon?: ReactNode; onClose: () => void; children: ReactNode; onStop?: () => void; closeDisabled?: boolean; className?: string; footer?: ReactNode; footerNote?: string }) {
  const scope = useRef<HTMLElement>(null);
  const currentClose = useRef(onClose); currentClose.current = onClose;
  const currentCloseDisabled = useRef(closeDisabled); currentCloseDisabled.current = closeDisabled;
  const requestClose = () => { if (!currentCloseDisabled.current) currentClose.current(); };
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    scope.current?.focus();
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); requestClose(); }
      if (event.key !== "Tab") return;
      // Include disclosure summaries, but never trap focus in hidden content.
      const controls = Array.from(scope.current?.querySelectorAll<HTMLElement>("button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),a[href],summary,[tabindex]") || [])
        .filter(control => control.tabIndex >= 0 && control.getClientRects().length > 0 && getComputedStyle(control).visibility !== "hidden");
      if (!controls.length) { event.preventDefault(); return; }
      const index = controls.indexOf(document.activeElement as HTMLElement);
      if (index < 0 || (!event.shiftKey && index === controls.length - 1) || (event.shiftKey && index === 0)) { event.preventDefault(); (event.shiftKey ? controls.at(-1) : controls[0])?.focus(); }
    };
    document.addEventListener("keydown", key);
    return () => { document.removeEventListener("keydown", key); if (previous?.isConnected) previous.focus(); };
  }, []);
  return <div className="modal-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) { event.preventDefault(); requestClose(); } }}><section ref={scope} tabIndex={-1} role="dialog" aria-modal="true" aria-label={localize(title)} className={`modal-card workstation-dialog ${className}`}><header className="modal-card__head">{localize(titleIcon)}<h2>{localize(title)}</h2><span className="menu-spacer" />{onStop && <button className="device-safe-stop" title={localize("Stop all devices · stops motion and the camera, confirms X-ray OFF and keeps every saved file")} onClick={onStop}>{localize("Stop all devices")}</button>}<button className="modal-card__close" disabled={closeDisabled} onClick={requestClose} aria-label={localize("Close dialog")}>×</button></header><div className="modal-card__body">{localize(children)}</div>{footer ? <footer className="modal-card__foot">{footerNote ? <span className="modal-card__foot-note">{localize(footerNote)}</span> : null}{localize(footer)}</footer> : null}</section></div>;
}
