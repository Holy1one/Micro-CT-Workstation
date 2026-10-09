/** Permanent camera panel; native properties and the settings dialog share one draft.
 *  Layout follows the right-column instrument grammar: one bordered device group,
 *  a titled exposure section, a mono readout row, action rows and a footer status
 *  line. Every control keeps its original command, disabled reason and title. */
import { useState } from "react";
import type { CameraProperty, EngineCommand, EngineSnapshot } from "./engine/types";
import type { CameraDraftModel } from "./camera-draft";
import { cameraCapturePreparationReason, cameraControlReason, cameraPropertyLabel, cameraPropertyReason, cameraPropertyUsesSelect, cameraPropertyValueLabel, deviceConnectionReason, manualControlReason } from "./engine/deviceControls";
import { englishDisplayText, unicodeDisplayText, englishOptionLabel, englishReason } from "./engine/native-labels";
import { chooseImageDirectory, resolveDefaultImageDirectory } from "./platform/desktopPaths";
import { LineIcon } from "./icons";

/** One native camera parameter: micro-label, 34px well, and an actual-value caption
 *  that only appears when it carries information (no repeated "no device" noise). */
export function CameraPropertyControl({ property, model, reason, label, hint }: { property?: CameraProperty; model: CameraDraftModel; reason: string | null; label?: string; hint?: string }) {
  const title = label || cameraPropertyLabel(property);
  if (!property) return <label className="device-field"><span>{title}</span><select className="device-control" disabled aria-label={`${title} draft`}><option>UNKNOWN</option></select><small className="device-caption">Unavailable</small></label>;
  const disabled = reason || cameraPropertyReason(property);
  const value = model.draft[property.key] ?? property.target ?? property.actual ?? "";
  // Option and reason text is presented in English; the engine's native value, its
  // supported/enabled/reason flags and the raw record are unchanged.
  const options = property.options.length
    ? property.options.map(option => ({ ...option, label: englishOptionLabel(property.key, option.value, option.label), reason: option.reason ? englishReason(option.reason) : null }))
    : property.supportedValues.map(native => ({ value: native, label: cameraPropertyValueLabel(property, native), supported: true, enabled: property.writable, reason: null }));
  const dirty = model.draft[property.key] !== undefined && model.draft[property.key] !== property.actual;
  return <label className="device-field" title={disabled || [cameraPropertyLabel(property), hint].filter(Boolean).join(" · ")}><span>{title}</span>{cameraPropertyUsesSelect(property) ? <select className="device-control" aria-label={`${cameraPropertyLabel(property)} draft`} disabled={Boolean(disabled)} value={JSON.stringify(value)} onChange={event => model.set(property.key, JSON.parse(event.target.value))}><option value={JSON.stringify("")} disabled>Select</option>{options.map(option => <option key={JSON.stringify(option.value)} value={JSON.stringify(option.value)} disabled={!option.supported || !option.enabled} title={option.reason || undefined}>{option.label}{!option.supported ? " · unsupported" : !option.enabled ? " · inactive" : ""}</option>)}</select> : <input className="device-control" aria-label={`${cameraPropertyLabel(property)} draft`} type="number" disabled={Boolean(disabled)} value={typeof value === "number" ? value : ""} min={property.range?.[0]} max={property.range?.[1]} step={property.range?.[2] || 1} onChange={event => { if (event.target.value) model.set(property.key, Number(event.target.value)); }} />}<small className="device-caption" title={property.confirmed ? cameraPropertyValueLabel(property, property.actual) : "UNKNOWN"}>Actual: {property.confirmed ? cameraPropertyValueLabel(property, property.actual) : "UNKNOWN"}{dirty ? " · draft" : ""}</small></label>;
}

export function CameraPanel({ snapshot, stale, pending, model, run, onAdvanced, onPhoto, onLive, mediaRevision }: { snapshot: EngineSnapshot; stale: boolean; pending: boolean; model: CameraDraftModel; run: (command: EngineCommand) => Promise<EngineSnapshot>; onAdvanced: () => void; onPhoto: (snapshot: EngineSnapshot, revision: number) => void; onLive: (revision: number) => void; mediaRevision: number }) {
  const camera = snapshot.deviceControls.camera;
  const [selected, setSelected] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const manualReason = manualControlReason(snapshot, stale) || (pending ? "Operation pending" : null);
  const connectionReason = deviceConnectionReason(snapshot, stale) || (pending ? "Operation pending" : null);
  const cameraReason = manualReason || cameraControlReason(camera);
  const applyReason = cameraReason || model.issue || (!model.dirty ? "No draft changes" : null);
  const captureReason = cameraReason || (camera?.capabilities.hostCapture ? null : "Host capture unavailable") || (camera?.liveView.active ? "Stop Live View explicitly before capture" : null) || cameraCapturePreparationReason(camera) || (snapshot.parameters.savePath.trim() ? null : "Choose an image storage root before capture") || (model.dirty ? "Apply or reload the parameter draft first" : null);
  const readiness = snapshot.cameraScanReadiness;
  const scanReady = Boolean(readiness?.ready && !model.dirty);
  const state = !camera || (!camera.connected && !camera.isolated) ? "DISCONNECTED" : !camera.cleanupConfirmed ? "CLEANUP UNKNOWN" : camera.connected && !camera.isolated ? camera.phase.toUpperCase() : "DISCONNECTED";
  const act = (action: () => Promise<unknown>) => { setMessage(null); void action().catch(reason => setMessage(String(reason))); };
  const root = snapshot.parameters.savePath;
  const identity = camera?.identity;
  const reason = message || (!camera?.cleanupConfirmed && camera?.isolated ? "Camera cleanup unconfirmed" : camera?.lastError) || captureReason || (readiness?.ready ? "Confirmed configuration" : readiness?.issues[0]?.remedy || "Read and verify camera configuration");
  return <section className="panel camera-panel" id="camera-panel"><header className="panel__header"><LineIcon name="camera" size={15} className="panel__icon" /><h2>Camera</h2><span className="chip chip--muted">{stale ? "UNKNOWN" : state}</span></header><div className="device-body">
    <div className="device-group">
      <div className="device-group__head"><h3 className="device-group__title">Device</h3><span className="chip chip--muted" title={identity?.deviceId || "Select a camera explicitly"}>{identity ? `${unicodeDisplayText(identity.manufacturer)} ${unicodeDisplayText(identity.model)}` : "Not selected"}</span></div>
      <div className="device-grid device-grid--pick"><button disabled={Boolean(connectionReason) || Boolean(camera?.connected)} onClick={() => act(() => run({ type: "camera_connect" }))}>Discover</button><select className="device-control" aria-label="Active camera device" value={selected} disabled={Boolean(connectionReason) || Boolean(camera?.connected)} onChange={event => setSelected(event.target.value)}><option value="">Select camera</option>{camera?.discovered.map(item => <option key={item.deviceId} value={item.deviceId}>{unicodeDisplayText(item.model)} · {unicodeDisplayText(item.serial || item.deviceId)}</option>)}</select></div>
      <div className="device-grid device-grid--2"><button disabled={Boolean(connectionReason) || (!camera?.connected && !selected)} onClick={() => act(() => run(camera?.connected ? { type: "camera_disconnect" } : { type: "camera_connect", deviceId: selected }))}>{camera?.connected ? "Disconnect" : "Connect"}</button><button onClick={onAdvanced} title="Advanced camera settings, Live View and native properties">Advanced…</button></div>
      <label className="device-field"><span>Image storage root</span><div className="device-well"><span className="device-value" title={unicodeDisplayText(root || "Not configured")}>{unicodeDisplayText(root || "Not configured")}</span><button className="folder-btn" aria-label="Choose image storage root" disabled={Boolean(manualReason) || snapshot.mode === "developer_preview"} onClick={() => act(async () => { const selectedRoot = await chooseImageDirectory(root || await resolveDefaultImageDirectory()); if (selectedRoot) await run({ type: "set_capture_root", savePath: selectedRoot }); })}><LineIcon name="folder" size={15} /></button></div></label>
    </div>
    <div className="device-section">
      <div className="device-section__head"><h3 className="device-section__title">Exposure</h3></div>
      <div className="device-grid device-grid--2">{[["iso", "ISO"], ["shutter", "Native shutter"], ["aperture", "Aperture"], ["whiteBalance", "White balance"]].map(([key, label]) => <CameraPropertyControl key={key} property={camera?.properties[key]} model={model} reason={cameraReason} label={label} />)}</div>
      {/* Color temperature is a normal native parameter with a real parent dependency:
          the engine only marks it writable once the confirmed white balance is
          Color temperature (0x8012), so the control stays disabled until the parent
          mode is applied and read back. No predicted writability. */}
      <div className="device-grid device-grid--2"><CameraPropertyControl property={camera?.properties.whiteBalanceTemperature} model={model} reason={cameraReason} label="Color temperature" hint="Selects a Kelvin value on the camera, sent as its native index 0–30 (index 20 = 5000 K). Preset manual is the camera's own measured/photo-reference white balance, not a software photographic preset." /><CameraPropertyControl property={camera?.properties.longExposureNoiseReduction} model={model} reason={cameraReason} label="Long exposure NR" hint="Must be Off for fixed scans. When On, exposures longer than 1 s require extra processing before the next photo. Apply changes and verify the readback." /></div>
    </div>
    <div className="device-readouts">{[["exposureMode", "Mode"], ["autoIso", "Auto ISO"], ["focusMode", "Focus"]].map(([key, label]) => { const property = camera?.properties[key]; const text = property?.confirmed ? cameraPropertyValueLabel(property, property.actual) : "UNKNOWN"; return <div className="device-readout" key={key}><span>{label}</span><strong className={text === "UNKNOWN" ? "is-unknown" : undefined} title={text}>{text}</strong></div>; })}</div>
    {/* U9: the engine snapshot already publishes camera status continuously, so the
        redundant Status button is gone. Each remaining entry names one duty:
        Refresh settings / Apply changes / Capture NEF / Live View. */}
    <div className="device-grid device-grid--3"><button disabled={Boolean(connectionReason || cameraControlReason(camera))} title="Read the current native values and confirmed readback" onClick={() => act(async () => model.loadConfirmed(await run({ type: "camera_read_properties" })))}>Refresh settings</button><button className={!applyReason || model.dirty ? "primary" : undefined} disabled={Boolean(applyReason)} title={applyReason || "Write the draft serially and verify every readback"} onClick={() => act(async () => { const result = await run({ type: "camera_apply_properties", properties: model.changes, ...(model.presetId ? { presetId: model.presetId } : {}) }); model.loadConfirmed(result); })}>Apply changes</button><button disabled={Boolean(captureReason)} title={captureReason || "Capture one host RAM NEF into the scan-independent photo slot"} onClick={() => act(async () => { const prior = camera?.lastCapturedImage?.captureId; const result = await run({ type: "camera_capture" }); if (!result.deviceControls.camera?.lastCapturedImage || result.deviceControls.camera.lastCapturedImage.captureId === prior) throw new Error("No new confirmed capture registration"); onPhoto(result, mediaRevision); })}>Capture NEF</button></div>
    {/* Stop Live View stays available whenever Live View is running, even while
        manual controls are locked, because ending the preview is a safety action. */}
    <div className="device-grid device-grid--2"><button className={camera?.liveView.active ? undefined : "primary"} disabled={camera?.liveView.active ? false : Boolean(cameraReason) || !camera?.capabilities.liveView} title={camera?.liveView.active ? "Stop Live View explicitly before capture or parameter writes" : "Start the memory-only Live View preview"} onClick={() => act(async () => { if (camera?.liveView.active) { await run({ type: "camera_live_view_stop" }); return; } await run({ type: "camera_live_view_start" }); onLive(mediaRevision); })}>{camera?.liveView.active ? "Stop Live View" : "Start Live View"}</button><button disabled={Boolean(cameraReason) || !camera?.capabilities.autofocus} onClick={() => act(() => run({ type: "camera_autofocus" }))}>AF</button></div>
    {camera?.capabilities.focusStep && <div className="device-grid device-grid--2"><button disabled={Boolean(cameraReason) || !camera.liveView.active} onClick={() => act(() => run({ type: "camera_focus", step: -1 }))}>Focus −1</button><button disabled={Boolean(cameraReason) || !camera.liveView.active} onClick={() => act(() => run({ type: "camera_focus", step: 1 }))}>Focus +1</button></div>}
    <div className="device-footer"><p role="status" title={englishDisplayText(reason)}>{englishDisplayText(reason)}</p><span className={`chip ${captureReason ? "chip--muted" : "chip--accent"}`} title={captureReason || "Single photo ready"}>Single photo {captureReason ? "✕" : "✓"}</span><span className={`chip ${scanReady ? "chip--accent" : "chip--muted"}`} title={englishDisplayText(scanReady ? "Fixed scan ready" : readiness?.issues[0]?.requirement || "Fixed scan not ready")}>Fixed scan {scanReady ? "✓" : "✕"}</span></div>
  </div></section>;
}
