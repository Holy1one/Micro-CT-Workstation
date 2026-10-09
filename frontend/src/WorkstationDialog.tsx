/** Shared modal chrome. Closing UI never performs a device action implicitly. */
import { useEffect, useRef, type ReactNode } from "react";
export function WorkstationDialog({ title, onClose, children, onStop, closeDisabled = false, className = "", footer, footerNote }: { title: string; onClose: () => void; children: ReactNode; onStop?: () => void; closeDisabled?: boolean; className?: string; footer?: ReactNode; footerNote?: string }) {
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
      const controls = Array.from(scope.current?.querySelectorAll<HTMLElement>("button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),a[href]") || []);
      if (!controls.length) { event.preventDefault(); return; }
      const index = controls.indexOf(document.activeElement as HTMLElement);
      if (index < 0 || (!event.shiftKey && index === controls.length - 1) || (event.shiftKey && index === 0)) { event.preventDefault(); (event.shiftKey ? controls.at(-1) : controls[0])?.focus(); }
    };
    document.addEventListener("keydown", key);
    return () => { document.removeEventListener("keydown", key); if (previous?.isConnected) previous.focus(); };
  }, []);
  return <div className="modal-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) { event.preventDefault(); requestClose(); } }}><section ref={scope} tabIndex={-1} role="dialog" aria-modal="true" aria-label={title} className={`modal-card workstation-dialog ${className}`}><header className="modal-card__head"><h2>{title}</h2><span className="menu-spacer" />{onStop && <button className="device-safe-stop" title="Stop all devices · stops motion and the camera, confirms X-ray OFF and keeps every saved file" onClick={onStop}>Stop all devices</button>}<button className="modal-card__close" disabled={closeDisabled} onClick={requestClose} aria-label="Close dialog">×</button></header><div className="modal-card__body">{children}</div>{footer ? <footer className="modal-card__foot">{footerNote ? <span className="modal-card__foot-note">{footerNote}</span> : null}{footer}</footer> : null}</section></div>;
}
