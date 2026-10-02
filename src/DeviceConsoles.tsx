/** Device consoles render native engine evidence and send explicit domain commands.
 * Drafts and view overlays are presentation-only. No control starts an X-ray beam.
 * The engine enforces occupation, cancellation and capability checks independently. */
import { useEffect, useRef, useState } from "react";
import type { CameraHealth, CameraProperties, CameraPreset, EngineCommand, EngineSnapshot } from "./engine/types";
import { cameraControlReason, confirmedCameraDraft, deviceConnectionReason, exactShutterForExposure, manualControlReason, nativePropertyLabel, validateCameraDraft } from "./engine/deviceControls";
import { cameraLiveViewFrame, chooseCameraCaptureDirectory, exportCameraPresets, importCameraPresets, resolveCameraCaptureDirectory, validateLiveFrame } from "./platform/cameraControls";

type Dispatch = (command: EngineCommand) => Promise<EngineSnapshot | undefined>;
type ConsoleProps = { kind: "camera" | "turntable"; snapshot: EngineSnapshot; busy: boolean; stale: boolean; dispatch: Dispatch; onClose: () => void };

function LiveView({ camera, preview }: { camera: CameraHealth | null; preview: boolean }) {
  const lv = camera?.liveView;
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const [grid, setGrid] = useState(false);
  const [crosshair, setCrosshair] = useState(false);
  const last = useRef(0);
  const currentUrl = useRef<string | null>(null);
  useEffect(() => {
    let active = true; let pending = false;
    last.current = 0; setUrl(null); setError(null);
    if (currentUrl.current) { URL.revokeObjectURL(currentUrl.current); currentUrl.current = null; }
    if (!lv?.active || !lv.fresh || !lv.sessionId || preview) return;
    const session = lv.sessionId;
    const refresh = async () => {
      if (pending || !active) return;
      pending = true;
      try {
        const frame = await cameraLiveViewFrame(last.current);
        if (!frame || !active) return;
        validateLiveFrame(frame, session, last.current);
        const next = URL.createObjectURL(new Blob([new Uint8Array(frame.jpeg)], { type: "image/jpeg" }));
        const decoded = new Image(); decoded.src = next;
        try { await decoded.decode(); } catch { URL.revokeObjectURL(next); throw new Error("Live View JPEG decoding failed"); }
        if (!active || decoded.naturalWidth !== frame.width || decoded.naturalHeight !== frame.height) {
          URL.revokeObjectURL(next);
          if (active) throw new Error("Live View JPEG dimensions disagree with frame metadata");
          return;
        }
        if (currentUrl.current) URL.revokeObjectURL(currentUrl.current);
        currentUrl.current = next; last.current = frame.sequence; setUrl(next); setError(null);
      } catch (reason) {
        if (active) { setUrl(null); setError(String(reason)); }
      } finally { pending = false; }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 150);
    return () => { active = false; window.clearInterval(timer); if (currentUrl.current) URL.revokeObjectURL(currentUrl.current); currentUrl.current = null; };
  }, [lv?.active, lv?.fresh, lv?.sessionId, preview]);
  return <section className="device-console-section">
    <h3>Live View <small>Latest frame · up to {lv?.fpsLimit ?? 10} fps</small></h3>
    <div className="live-view-tools">
      <button onClick={() => setZoom(1)}>Fit window</button>
      <label>Zoom <select value={zoom} onChange={event => setZoom(Number(event.target.value))}>{[1, 1.5, 2, 3, 4].map(n => <option key={n} value={n}>{n === 1 ? "Fit" : `${n}×`}</option>)}</select></label>
      <label><input type="checkbox" checked={grid} onChange={event => setGrid(event.target.checked)} /> Grid</label>
      <label><input type="checkbox" checked={crosshair} onChange={event => setCrosshair(event.target.checked)} /> Crosshair</label>
    </div>
    <div className={`camera-live-viewport ${grid ? "camera-live-viewport--grid" : ""} ${crosshair ? "camera-live-viewport--crosshair" : ""}`}>
      {url && lv?.active && lv.fresh ? <img src={url} alt="Current camera Live View" style={{ transform: `scale(${zoom})` }} />
        : <span>{error || (preview && lv?.active ? "NO HARDWARE · preview state only; no live camera image" : lv?.error || (lv?.active ? "Live View unavailable or stale · no current frame" : "Live View stopped"))}</span>}
      {lv?.active && lv.fresh && url && <b className="live-frame-label">LIVE · {last.current}</b>}
    </div>
  </section>;
}

function CameraConsole({ snapshot, disabledReason, connectionReason, dispatch, act, pending }: { snapshot: EngineSnapshot; disabledReason: string | null; connectionReason: string | null; dispatch: Dispatch; act: (command: EngineCommand) => Promise<EngineSnapshot | undefined>; pending: boolean }) {
  const camera = snapshot.deviceControls.camera;
  const [selected, setSelected] = useState("");
  const [draft, setDraft] = useState<CameraProperties>({});
  const [presetId, setPresetId] = useState("");
  const [draftPresetId, setDraftPresetId] = useState("");
  const [name, setName] = useState(""); const [description, setDescription] = useState("");
  const [directory, setDirectory] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [comparison, setComparison] = useState<CameraPreset | null>(null);
  useEffect(() => { setDraft(camera ? confirmedCameraDraft(camera.properties) : {}); setPresetId(""); setDraftPresetId(""); setComparison(null); }, [camera?.sessionId]);
  useEffect(() => {
    if (!camera?.connected || !camera.properties.shutter) return;
    const code = exactShutterForExposure(camera, snapshot.parameters.exposureMs);
    if (code !== null) { setDraft(value => ({ ...value, shutter: code })); setDraftPresetId(""); }
    else setMessage("Scan Exposure has no exact described native shutter value · adjust the target before preflight");
  }, [snapshot.parameters.exposureMs, camera?.sessionId]);
  useEffect(() => {
    let active = true;
    void resolveCameraCaptureDirectory().then(path => { if (active && path) setDirectory(path); }).catch(reason => { if (active) setMessage(String(reason)); });
    return () => { active = false; };
  }, []);
  const reason = disabledReason || cameraControlReason(camera);
  const propertyReason = reason || (camera?.capabilities.properties ? null : camera?.capabilities.disabledReasons.properties || "Property capability unavailable");
  const readReason = connectionReason || cameraControlReason(camera) || (camera?.capabilities.properties ? null : "Property capability unavailable");
  const draftError = camera && Object.keys(draft).length ? validateCameraDraft(camera, draft) : "Read confirmed camera parameters or load a preset";
  const preset = snapshot.deviceControls.presets.presets.find(p => p.id === presetId);
  const capabilityReason = (key: "hostCapture" | "liveView" | "autofocus" | "focusStep") => reason || (camera?.capabilities[key] ? null : camera?.capabilities.disabledReasons[key] || `${key} capability unavailable`);
  const captureReason = capabilityReason("hostCapture") || (camera?.liveView.active ? "Live View is active · stop preview explicitly before capture" : null);
  const read = async () => { const result = await act({ type: "camera_read_properties" }); if (result?.deviceControls.camera) setDraft(confirmedCameraDraft(result.deviceControls.camera.properties)); };
  const loadPreset = () => {
    if (!preset || !camera) return;
    if (!camera.identity || !preset.models.some(model => model.toLowerCase() === camera.identity!.model.toLowerCase())) { setMessage("Preset does not apply to the selected camera model"); return; }
    const invalid = validateCameraDraft(camera, preset.parameters);
    if (invalid) { setMessage(invalid); return; }
    setDraft({ ...preset.parameters }); setDraftPresetId(preset.id); setMessage("Preset loaded into draft · press Apply to write and verify");
  };
  return <>
    <section className="device-console-section"><h3>Identity and connection</h3>
      <div className="device-console-actions">
        <button disabled={Boolean(connectionReason)} title={connectionReason || "Enumerate native WPD camera devices"} onClick={() => void act({ type: "camera_connect" })}>Discover cameras</button>
        <select aria-label="Active camera device" value={selected} onChange={e => setSelected(e.target.value)} disabled={Boolean(connectionReason)}><option value="">Select one camera explicitly</option>{camera?.discovered.map(identity => <option key={identity.deviceId} value={identity.deviceId}>{identity.manufacturer} {identity.model} · {identity.serial || identity.deviceId}</option>)}</select>
        <button disabled={!selected || Boolean(connectionReason) || Boolean(camera?.connected)} onClick={() => void act({ type: "camera_connect", deviceId: selected })}>Connect selected</button>
        <button disabled={!camera?.connected || Boolean(connectionReason)} onClick={() => void act({ type: "camera_disconnect" })}>Disconnect</button>
        <button onClick={() => void act({ type: "camera_status" })}>Read status</button>
      </div>
      <dl className="device-evidence"><dt>Camera</dt><dd>{camera?.identity ? `${camera.identity.manufacturer} ${camera.identity.model}` : "UNKNOWN / not connected"}</dd><dt>Device / serial</dt><dd>{camera?.identity?.deviceId || "UNKNOWN"} / {camera?.identity?.serial || "UNKNOWN"}</dd><dt>Backend / session</dt><dd>{camera?.backend || "Windows WPD/PTP"} / {camera?.sessionId || "UNKNOWN"}</dd><dt>State</dt><dd>{camera?.isolated ? "ISOLATED · cleanup unconfirmed" : camera?.connected ? camera.phase : "DISCONNECTED"}</dd><dt>Focus position</dt><dd>{camera?.focusPosition ?? "UNKNOWN · not a saved preset coordinate"}</dd><dt>Capture policy</dt><dd>{camera?.transferPolicy || "UNKNOWN · host capture capability required"}</dd></dl>
      {(camera?.lastError || reason) && <p className="device-control-reason" role="status">{camera?.lastError || reason}</p>}
      {camera && Object.entries(camera.capabilities.disabledReasons).map(([key, value]) => <p className="device-control-reason" key={key}>{key}: {value}</p>)}
    </section>
    <section className="device-console-section"><h3>Camera parameters <small>Native values · draft → Apply → readback</small></h3>
      <div className="device-console-actions"><button disabled={Boolean(readReason)} title={readReason || "Read actual properties"} onClick={() => void read()}>Read parameters</button><button disabled={Boolean(propertyReason || draftError)} title={propertyReason || draftError || "Write serially and verify every readback"} onClick={() => void act({ type: "camera_apply_properties", properties: draft, ...(draftPresetId ? { presetId: draftPresetId } : {}) })}>Apply and verify</button></div>
      <p>Shutter and scan Exposure share one target. Applying verified shutter parameters updates scan Exposure; native fractions are matched exactly.</p>
      <table className="camera-properties"><thead><tr><th>Parameter / capability</th><th>Draft target</th><th>Actual readback</th><th>Evidence</th></tr></thead><tbody>
        {Object.values(camera?.properties || {}).map(property => <tr key={property.key}><td>{property.label}<small>{property.key} · {property.readable ? "read" : "unreadable"} / {property.writable ? "write" : "read only"}</small></td><td>
          {property.supportedValues.length ? <select aria-label={`${property.label} draft`} disabled={Boolean(propertyReason) || !property.writable} value={JSON.stringify(draft[property.key] ?? property.target ?? property.actual ?? "")} onChange={event => { setDraftPresetId(""); setDraft(value => ({ ...value, [property.key]: JSON.parse(event.target.value) as number | string })); }}><option value={JSON.stringify("")} disabled>Select native value</option>{property.supportedValues.map(value => <option key={JSON.stringify(value)} value={JSON.stringify(value)}>{nativePropertyLabel(property.key, value)}</option>)}</select>
          : property.range ? <input type="number" aria-label={`${property.label} draft`} min={property.range[0]} max={property.range[1]} step={property.range[2] || 1} disabled={Boolean(propertyReason) || !property.writable} value={draft[property.key] ?? ""} onChange={e => { setDraftPresetId(""); setDraft(value => ({ ...value, [property.key]: Number(e.target.value) })); }} /> : <span>{nativePropertyLabel(property.key, property.target)}</span>}
        </td><td>{nativePropertyLabel(property.key, property.actual)}</td><td>{property.error || (property.confirmed ? "Readback confirmed" : "UNKNOWN / not confirmed")}</td></tr>)}
        {!Object.keys(camera?.properties || {}).length && <tr><td colSpan={4}>No described camera properties. Connect a selected device and read its capabilities.</td></tr>}
      </tbody></table>
      {(propertyReason || draftError) && <p className="device-control-reason">{propertyReason || draftError}</p>}
    </section>
    <section className="device-console-section"><h3>Camera presets <small>Camera parameters only</small></h3>
      <div className="device-console-actions"><select aria-label="Camera preset" value={presetId} onChange={e => setPresetId(e.target.value)}><option value="">Select named preset</option>{snapshot.deviceControls.presets.presets.map(p => <option key={p.id} value={p.id}>{p.name} · {p.models.join(", ")}</option>)}</select><button disabled={!preset || Boolean(reason)} onClick={loadPreset}>Load draft</button><button disabled={!preset || !camera} onClick={() => setComparison(preset || null)}>Compare readback</button><button disabled={!preset || pending} onClick={() => void act({ type: "preset_delete", id: presetId })}>Delete preset</button></div>
      <div className="device-console-actions"><input aria-label="Preset name" placeholder="Preset name" value={name} onChange={e => setName(e.target.value)} /><input aria-label="Preset description" placeholder="Description" value={description} onChange={e => setDescription(e.target.value)} /><button disabled={!name.trim() || Boolean(propertyReason) || !Object.keys(confirmedCameraDraft(camera?.properties || {})).length} onClick={() => void act({ type: "preset_save", name: name.trim(), description, source: "native-camera-confirmed-readback" })}>Save confirmed parameters</button></div>
      <div className="device-console-actions"><button disabled={snapshot.mode === "developer_preview" || pending} onClick={() => void importCameraPresets().then(text => text === null ? undefined : act({ type: "preset_import", jsonText: text })).catch(e => setMessage(String(e)))}>Import preset file…</button><button disabled={snapshot.mode === "developer_preview" || pending || Boolean(snapshot.deviceControls.presets.error)} onClick={() => void dispatch({ type: "preset_export" }).then(response => { const text = response?.deviceControls.exportJson; if (!text) throw new Error("Engine did not return a verified preset export"); return exportCameraPresets(text); }).then(path => { if (path) setMessage(`Presets exported: ${path}`); }).catch(e => setMessage(String(e)))}>Export all presets…</button></div>
      {comparison && <table className="camera-properties"><thead><tr><th>{comparison.name}</th><th>Preset</th><th>Actual</th><th>Comparison</th></tr></thead><tbody>{Object.entries(comparison.parameters).map(([key, value]) => { const actual = camera?.properties[key]?.actual ?? null; return <tr key={key}><td>{key}</td><td>{nativePropertyLabel(key, value)}</td><td>{nativePropertyLabel(key, actual)}</td><td>{actual === null ? "UNKNOWN" : actual === value ? "Match" : "Different"}</td></tr>; })}</tbody></table>}
      {(message || snapshot.deviceControls.presets.error) && <p role="status" className="device-control-reason">{message || snapshot.deviceControls.presets.error}</p>}
    </section>
    <section className="device-console-section"><h3>Independent still capture</h3><div className="device-console-actions"><input aria-label="Independent capture directory" placeholder="Default: Pictures/MicroCT/CameraCaptures" value={directory} onChange={e => setDirectory(e.target.value)} /><button disabled={snapshot.mode === "developer_preview"} onClick={() => void chooseCameraCaptureDirectory(directory).then(path => { if (path) setDirectory(path); }).catch(e => setMessage(String(e)))}>Choose directory…</button><button disabled={Boolean(captureReason)} title={captureReason || "Capture one host file without changing scan frames or manifest"} onClick={() => void act({ type: "camera_capture", ...(directory.trim() ? { directory: directory.trim() } : {}) })}>Single capture</button></div><p>{captureReason || "Independent capture does not start X-ray output and does not enter a scan manifest."}</p><p>Last confirmed file: {camera?.lastCapture || "None"}</p></section>
    <section className="device-console-section"><div className="device-console-actions"><button disabled={Boolean(capabilityReason("liveView")) || Boolean(camera?.liveView.active)} title={capabilityReason("liveView") || undefined} onClick={() => void act({ type: "camera_live_view_start" })}>Start Live View</button><button disabled={!camera?.liveView.active} onClick={() => void act({ type: "camera_live_view_stop" })}>Stop Live View</button><button disabled={Boolean(capabilityReason("autofocus"))} title={capabilityReason("autofocus") || undefined} onClick={() => void act({ type: "camera_autofocus" })}>Single AF</button>{camera?.capabilities.focusStep && <><button disabled={Boolean(reason)} onClick={() => void act({ type: "camera_focus", step: -1 })}>Focus −1</button><button disabled={Boolean(reason)} onClick={() => void act({ type: "camera_focus", step: 1 })}>Focus +1</button></>}</div><p>Stop Live View explicitly before taking a photo. Closing this console sends Stop Live View.</p></section>
    <LiveView camera={camera && !disabledReason?.includes("unavailable") ? camera : null} preview={snapshot.mode === "developer_preview"} />
  </>;
}

function TurntableConsole({ snapshot, reason, connectionReason, act }: { snapshot: EngineSnapshot; reason: string | null; connectionReason: string | null; act: (command: EngineCommand) => Promise<EngineSnapshot | undefined> }) {
  const turntable = snapshot.deviceControls.turntable;
  const confirmedPhase = turntable.confirmedAngleDeg === null ? null : ((-turntable.confirmedAngleDeg % 360) + 360) % 360;
  const [relative, setRelative] = useState("90"); const [absolute, setAbsolute] = useState("0"); const [isolated, setIsolated] = useState(false);
  const angleError = (text: string, rel: boolean) => !text.trim() || !Number.isFinite(Number(text)) || Number(text) < 0 || (rel ? Number(text) <= 0 || Number(text) > 360 : Number(text) >= 360) || Math.abs(Number(text) * 1000 - Math.round(Number(text) * 1000)) > 1e-6;
  const motionReason = reason || (!turntable.connected ? "Connect the turntable" : !turntable.status?.homed || !turntable.status.referenceValid || !turntable.status.rearmed || turntable.status.state !== "IDLE" ? "Confirmed HOME reference and IDLE are required" : turntable.status.microsteps !== 8 || turntable.status.pulsesPerRev !== 96000 ? "Unsupported pulse scale; expected 8 / 96000" : null);
  const xray = snapshot.workstation?.xray;
  const homeReason = reason || (!turntable.connected ? "Connect the turntable" : !xray?.connected && !turntable.benchMode ? "Connect X-ray for confirmed OFF, or confirm physically isolated bench mode" : null);
  return <>
    <section className="device-console-section"><h3>Identity and motion evidence</h3><div className="device-console-actions"><button disabled={Boolean(connectionReason)} onClick={() => void act({ type: turntable.connected ? "turntable_disconnect" : "turntable_connect" })}>{turntable.connected ? "Disconnect" : "Connect turntable"}</button><button onClick={() => void act({ type: "turntable_status" })}>Read status</button><button disabled={Boolean(homeReason)} title={homeReason || "Confirm X-ray OFF before HOME; no automatic beam restart"} onClick={() => void act({ type: "turntable_home" })}>HOME</button></div><dl className="device-evidence"><dt>Device</dt><dd>{turntable.identity?.device || "UNKNOWN"} · {turntable.identity?.version || "UNKNOWN"}</dd><dt>State / reference</dt><dd>{turntable.status?.state || "UNKNOWN"} / {turntable.status?.referenceValid && turntable.status.homed ? "HOME confirmed" : "UNKNOWN / invalid"}</dd><dt>Confirmed angle</dt><dd>{turntable.confirmedAngleDeg === null ? "UNKNOWN" : `${turntable.confirmedAngleDeg}° (signed, unwrapped)`}</dd><dt>Quantized forward phase</dt><dd>{confirmedPhase === null ? "UNKNOWN" : `${confirmedPhase}° · from confirmed pulses`}</dd><dt>Controller pulse counter</dt><dd>{turntable.status?.positionPulses ?? "UNKNOWN"}</dd><dt>Pulse scale</dt><dd>{turntable.status ? `${turntable.status.microsteps} microsteps / ${turntable.status.pulsesPerRev} pulses per turn` : "UNKNOWN · fixed 8 / 96000 required"}</dd></dl><p>Positive input advances only along the prescribed production direction. HOME retains its internal bounded reverse retreat; ordinary reverse controls are unavailable.</p>{(reason || turntable.reason || homeReason) && <p className="device-control-reason">{reason || turntable.reason || homeReason}</p>}</section>
    <section className="device-console-section"><h3>Prescribed-direction motion</h3><div className="device-console-actions"><label>Relative ° <input type="number" min="0.001" max="360" step="0.001" value={relative} onChange={e => setRelative(e.target.value)} /></label><button disabled={Boolean(motionReason) || angleError(relative, true)} title={motionReason || "Positive (0, 360] degrees"} onClick={() => void act({ type: "turntable_relative", angleDeg: Number(relative) })}>Move relative</button><label>Absolute phase ° <input type="number" min="0" max="359.999" step="0.001" value={absolute} onChange={e => setAbsolute(e.target.value)} /></label><button disabled={Boolean(motionReason) || angleError(absolute, false)} title={motionReason || "[0, 360) phase, reached along the prescribed direction"} onClick={() => void act({ type: "turntable_to_angle", angleDeg: Number(absolute) })}>Move to phase</button></div><p>Targets use pulse quantization (0.00375° per pulse). Completion requires confirmed arrival, matching ticket release and IDLE; a phase already reached does not add a turn.</p>{motionReason && <p className="device-control-reason">{motionReason}</p>}</section>
    <section className="device-console-section"><h3>Isolated turntable bench</h3><label><input type="checkbox" checked={isolated} onChange={e => setIsolated(e.target.checked)} disabled={Boolean(reason) || Boolean(xray?.connected)} /> I confirm the X-ray source is physically isolated for this turntable-only bench task.</label><div className="device-console-actions"><button disabled={Boolean(reason) || Boolean(xray?.connected) || !isolated} onClick={() => void act({ type: "turntable_bench_mode", enabled: true, physicallyIsolated: isolated })}>Confirm bench mode</button><button disabled={Boolean(reason) || !turntable.benchMode} onClick={() => void act({ type: "turntable_bench_mode", enabled: false, physicallyIsolated: false })}>Leave bench mode</button></div><p>{turntable.benchMode ? "BENCH MODE · X-ray UNKNOWN / unmonitored · beam commands prohibited" : "Bench mode inactive"}. A previous source connection with unconfirmed OFF cannot be overridden; the engine rejects that case. Connecting the source requires renewed validation.</p></section>
  </>;
}

export function DeviceConsole({ kind, snapshot, busy, stale, dispatch, onClose }: ConsoleProps) {
  const scope = useRef<HTMLElement | null>(null);
  const [localError, setLocalError] = useState<string | null>(null);
  const [abandonConfirmed, setAbandonConfirmed] = useState(false);
  const [restoreConfirmed, setRestoreConfirmed] = useState(false);
  useEffect(() => { setRestoreConfirmed(false); setAbandonConfirmed(false); }, [snapshot.engineSessionId, snapshot.deviceControls.taskLock.taskId, snapshot.deviceControls.taskLock.savePath]);
  const reason = manualControlReason(snapshot, stale) || (busy ? "Command acknowledgement pending" : null);
  const connectionReason = deviceConnectionReason(snapshot, stale) || (busy ? "Command acknowledgement pending" : null);
  const act = async (command: EngineCommand) => { setLocalError(null); const result = await dispatch(command); if (!result) setLocalError("Command failed or was discarded · inspect engine status"); return result; };
  const close = () => { if (kind === "camera") void dispatch({ type: "camera_live_view_stop" }); onClose(); };
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    scope.current?.focus();
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") { if (kind === "camera") void dispatch({ type: "camera_live_view_stop" }); onClose(); }
      if (event.key !== "Tab") return;
      const controls = Array.from(scope.current?.querySelectorAll<HTMLElement>("button:not(:disabled),input:not(:disabled),select:not(:disabled)") || []);
      const index = controls.indexOf(document.activeElement as HTMLElement);
      if (index < 0 || (!event.shiftKey && index === controls.length - 1) || (event.shiftKey && index === 0)) {
        event.preventDefault(); (event.shiftKey ? controls.at(-1) : controls[0])?.focus();
      }
    };
    document.addEventListener("keydown", key);
    return () => { document.removeEventListener("keydown", key); if (previous?.isConnected) previous.focus(); };
  }, [kind]);
  const lock = snapshot.deviceControls.taskLock;
  return <div className="device-console-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) close(); }}><section ref={scope} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="device-console-title" className="device-console">
    <header><div><h2 id="device-console-title">{kind === "camera" ? "Camera" : "Turntable"}</h2><span>{snapshot.mode === "developer_preview" ? "NO HARDWARE · browser preview" : "Offline candidate · hardware acceptance pending"}</span></div><div className="device-console-actions"><button className="device-safe-stop" onClick={() => void dispatch({ type: "stop" })}>Safety Stop</button><button onClick={close} aria-label="Close device console">Close</button></div></header>
    <div className="device-console-body">
      {localError && <p role="alert" className="device-control-reason">{localError}</p>}
      {kind === "camera" ? <CameraConsole snapshot={snapshot} disabledReason={reason} connectionReason={connectionReason} dispatch={dispatch} act={act} pending={busy} /> : <TurntableConsole snapshot={snapshot} reason={reason} connectionReason={connectionReason} act={act} />}
      {lock.locked && <section className="device-console-section"><h3>Unfinished task lock</h3><p>{lock.reason || "Manual controls remain locked"} · {lock.taskId || "Unknown task"} · {lock.savePath || "Unknown path"}</p><label><input type="checkbox" checked={restoreConfirmed} onChange={e => setRestoreConfirmed(e.target.checked)} /> I confirm controlled restore using the retained task&apos;s frozen X-ray voltage/current, Exposure, continuous-output limit and camera profile, including release of USB Auto Shut Down.</label><div className="device-console-actions"><button disabled={!restoreConfirmed || Boolean(connectionReason)} title="Restore the retained task session, apply and verify its frozen settings; no beam is enabled" onClick={() => void act({ type: "preflight", restore: true, confirmUsbAutoShutdownRelease: true })}>Restore preflight</button></div><p>After restore preflight, use the original scan HOME control for controlled recovery, then Restore Previous Scan to verify retained files. Ordinary parameters, focus and motion remain locked.</p><label><input type="checkbox" checked={abandonConfirmed} onChange={e => setAbandonConfirmed(e.target.checked)} /> Explicitly abandon this task and retain every existing file.</label><button disabled={!abandonConfirmed || busy || stale || ["running", "finishing", "stopping"].includes(snapshot.phase) || snapshot.operations.some(op => ["queued", "running", "cancelling"].includes(op.status))} onClick={() => void act({ type: "abandon_scan" })}>Abandon task and retain files</button></section>}
      <section className="device-console-section"><h3>Engine operations</h3>{snapshot.operations.length ? <table className="camera-properties"><thead><tr><th>Operation / device session</th><th>Stage</th><th>Status</th></tr></thead><tbody>{snapshot.operations.slice(-8).reverse().map(op => <tr key={op.operationId}><td>{op.kind}<small>{op.operationId} · {op.deviceSessionId}</small></td><td>{op.stage}</td><td>{op.status}{op.cancelRequested ? " · cancel requested" : ""}{op.error ? ` · ${op.error}` : ""}</td></tr>)}</tbody></table> : <p>No engine operations yet. Accepted commands acknowledge enqueue; completed hardware cleanup is reported separately.</p>}</section>
    </div>
  </section></div>;
}
