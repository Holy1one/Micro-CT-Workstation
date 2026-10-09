/** Only the central content layer is covered. The device state and Stop dock remain outside it. */
import { englishDisplayText, englishOptionLabel } from "./engine/native-labels";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CameraHealth, CameraProperties, EngineCommand, EngineSnapshot, IndependentCapturedImage } from "./engine/types";
import { cameraControlReason, cameraPropertyReason, cameraPropertyValueLabel } from "./engine/deviceControls";
import { cameraLiveViewFrame, validateLiveFrame } from "./platform/cameraControls";
import { cameraCapturePreview, capturePreviewDataUrl } from "./platform/cameraCaptures";
import { MemoryLiveViewFeed, liveViewDataUrl, type LiveViewImage } from "./platform/cameraLiveView";
export type WorkstationMediaMode = "equipment" | "reconstruction" | "photo" | "live";
/** Drafts for these two controls expire with their preview session; camera truth remains in the engine. */
function LiveCameraControls({ camera, reason, readReason, run }: { camera: CameraHealth | null; reason: string | null; readReason: string | null; run?: (command: EngineCommand) => Promise<EngineSnapshot> }) {
  const session = camera?.liveView.sessionId;
  const controls = camera?.liveViewControls;
  const current = useRef(session); current.current = session;
  const mounted = useRef(true);
  const [draft, setDraft] = useState<CameraProperties>({});
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setDraft({}); setPending(false); setError(null); }, [session]);
  const unavailable = reason || cameraControlReason(camera) || (pending ? "Live View operation pending" : null)
    || (!run ? "Camera controls unavailable" : null)
    || (!session || !camera?.liveView.active || controls?.sessionId !== session ? "Read the current Live View controls" : null);
  const changes = Object.fromEntries(Object.entries(draft).filter(([key, value]) => controls?.properties[key]?.actual !== value));
  const invalid = Object.entries(changes).some(([key, value]) => {
    const property = controls?.properties[key];
    return cameraPropertyReason(property) || !property?.options.some(option => option.value === value && option.supported && option.enabled);
  });
  const act = async (command: EngineCommand) => {
    if (!run || !session) return;
    const expected = session; setPending(true); setError(null);
    try {
      const response = await run(command);
      if (!mounted.current || current.current !== expected) return;
      if (!response.deviceControls.camera?.liveView.active || response.deviceControls.camera.liveViewControls?.sessionId !== expected) throw new Error("Live View session changed; old control result discarded");
      setDraft({});
    } catch (failure) { if (mounted.current && current.current === expected) setError(String(failure)); }
    finally { if (mounted.current && current.current === expected) setPending(false); }
  };
  return <div className="workstation-camera-controls" aria-label="Camera Live View controls">
    {[["cameraZoom", "Camera zoom"], ["focusMode", "Live focus"]].map(([key, label]) => {
      const property = controls?.sessionId === session ? controls?.properties[key] : undefined;
      const disabled = unavailable || cameraPropertyReason(property);
      return <label key={key} title={englishDisplayText(disabled || property?.label)}><span>{label}</span><select aria-label={label} disabled={Boolean(disabled)} value={JSON.stringify(draft[key] ?? property?.actual ?? "")} onChange={event => setDraft(previous => ({...previous,[key]:JSON.parse(event.target.value)}))}>
        <option value={JSON.stringify("")} disabled>UNKNOWN</option>{property?.options.map(option => <option key={JSON.stringify(option.value)} value={JSON.stringify(option.value)} disabled={!option.supported || !option.enabled} title={englishDisplayText(option.reason) || undefined}>{englishOptionLabel(key, option.value, option.label)}</option>)}
      </select><small>Actual: {property?.confirmed && !property.error ? cameraPropertyValueLabel(property,property.actual) : "UNKNOWN"}</small></label>;
    })}
    <button disabled={Boolean(readReason || pending || cameraControlReason(camera)) || !run || !session || !camera?.liveView.active} onClick={() => void act({type:"camera_live_view_read",sessionId:session!})}>Read controls</button>
    <button disabled={Boolean(unavailable) || invalid || !Object.keys(changes).length} onClick={() => void act({type:"camera_live_view_configure",sessionId:session!,controls:changes})}>Apply controls</button>
    {error && <p role="status" className="workstation-camera-control-error">{englishDisplayText(error)}</p>}
  </div>;
}
export function WorkstationMedia({ mode, camera, photo, requestRevision, onClose, run, controlReason = null, readReason = controlReason }: { mode: WorkstationMediaMode; camera: CameraHealth | null; photo: IndependentCapturedImage | null; requestRevision: number; onClose: () => void; run?: (command: EngineCommand) => Promise<EngineSnapshot>; controlReason?: string | null; readReason?: string | null }) {
  const [live, setLive] = useState<LiveViewImage | null>(null);
  const [still, setStill] = useState<{ id: string; url: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(0);
  const [zoom, setZoom] = useState(1);
  const [grid, setGrid] = useState(false);
  const [crosshair, setCrosshair] = useState(false);
  const feed = useRef<MemoryLiveViewFeed | null>(null);
  useLayoutEffect(() => { if (live) feed.current?.presented(live.sequence); }, [live]);
  useEffect(() => {
    if (mode !== "photo" || !photo) return;
    let active = true; setError(null); setStill(null);
    void cameraCapturePreview(photo.captureId).then(async preview => {
      if (preview.captureId !== photo.captureId || preview.sessionId !== photo.sessionId || preview.operationId !== photo.operationId) throw new Error("Unrelated capture preview rejected");
      const url = capturePreviewDataUrl(preview); const image = new Image(); image.src = url; await image.decode();
      if (active) setStill({ id: photo.captureId, url });
    }).catch(reason => { if (active) setError(`File saved · preview unavailable: ${String(reason)}`); });
    return () => { active = false; };
  }, [mode, photo?.captureId, requestRevision]);
  useEffect(() => {
    setLive(null); setError(null);
    const lv = camera?.liveView;
    if (mode !== "live" || !camera?.connected || camera.isolated || !camera.cleanupConfirmed || !lv?.active || !lv.sessionId) return;
    const stream = new MemoryLiveViewFeed(lv.sessionId, { fetch: cameraLiveViewFrame, validate: validateLiveFrame, decode: async frame => { const url = liveViewDataUrl(frame.jpeg); const image = new Image(); image.src = url; await image.decode(); return { url, width: image.naturalWidth, height: image.naturalHeight }; }, release: () => {}, update: (image, reason) => { setLive(image); setError(reason ?? image?.error ?? null); }, now: () => performance.now() });
    feed.current = stream;
    const refresh = () => { setNow(performance.now()); void stream.poll(); }; refresh(); const timer = window.setInterval(refresh, 100);
    return () => { window.clearInterval(timer); stream.stop(); if (feed.current === stream) feed.current = null; };
  }, [mode, camera?.connected, camera?.isolated, camera?.cleanupConfirmed, camera?.liveView.active, camera?.liveView.sessionId, requestRevision]);
  if (mode !== "photo" && mode !== "live") return null;
  const url = mode === "photo" ? still?.id === photo?.captureId ? still?.url : null : live?.url;
  const fresh = mode === "live" && live && camera?.liveView.fresh && !error && now - live.receivedAt <= 1000;
  return <div className={`workstation-media-layer ${grid ? "has-grid" : ""} ${crosshair ? "has-crosshair" : ""}`}><div className="workstation-media-toolbar"><strong>{mode === "photo" ? "Independent photo" : fresh ? "LIVE" : "STALE · waiting for current frame"}</strong><button onClick={() => setZoom(1)}>Fit</button><select aria-label="Image display zoom" value={zoom} onChange={event => setZoom(Number(event.target.value))}>{[1, 1.5, 2, 3, 4].map(value => <option key={value} value={value}>{value}×</option>)}</select><label><input type="checkbox" checked={grid} onChange={event => setGrid(event.target.checked)} /> Grid</label><label><input type="checkbox" checked={crosshair} onChange={event => setCrosshair(event.target.checked)} /> Crosshair</label><button onClick={onClose}>{mode === "live" ? "Stop preview / Equipment" : "Close / Equipment"}</button></div>{mode === "live" && <LiveCameraControls camera={camera} reason={controlReason} readReason={readReason} run={run} />}<div className="workstation-media-image">{url ? <img src={url} alt={mode === "photo" ? "Confirmed independent camera photo" : fresh ? "Current camera Live View" : "Last camera frame · stale"} style={{ transform: `scale(${zoom})` }} /> : <span role="status">{englishDisplayText(error || "Waiting for verified image")}</span>}</div>{error && url && <p role="status" className="workstation-media-error">{englishDisplayText(error)}</p>}</div>;
}
