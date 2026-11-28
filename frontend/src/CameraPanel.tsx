import { localize } from "./i18n";
/** Permanent camera panel; native properties and the settings dialog share one draft.
 *  Layout follows the right-column instrument grammar: an open device group,
 *  a titled exposure section, a tabular readout row, action rows and a footer status
 *  line. Every control keeps its original command, disabled reason and title. */
import { useState } from "react";
import type { CameraProperty, EngineCommand, EngineSnapshot } from "./engine/types";
import type { CameraDraftModel } from "./camera-draft";
import { cameraCapturePreparationReason, cameraControlReason, cameraParameterReadout, cameraPropertyLabel, cameraPropertyReason, cameraPropertyUsesSelect, cameraPropertyValueLabel, deviceConnectionReason, manualControlReason } from "./engine/deviceControls";
import { deviceConnectionLabel, englishDisplayText, unicodeDisplayText, englishOptionLabel, englishReason } from "./engine/native-labels";
import { chooseImageDirectory, resolveDefaultImageDirectory } from "./platform/desktopPaths";
import { LineIcon } from "./icons";

/** One native camera parameter: shared label and control, and an actual-value caption
 *  that only appears when it carries information (no repeated "no device" noise). */
export { cameraParameterReadout } from "./engine/deviceControls";

export function CameraPropertyControl({ property, model, reason, label, hint, stale = false }: { property?: CameraProperty; model: CameraDraftModel; reason: string | null; label?: string; hint?: string; stale?: boolean }) {
  const title = label || cameraPropertyLabel(property);
  if (!property) return <label className="device-field" title={localize(`${title} · Unavailable; no confirmed readback`)}><span>{localize(title)}</span><select className="device-control is-unknown" disabled aria-label={localize(`${title} draft · Unknown`)}><option>–</option></select><small className="device-caption">{localize("Unavailable")}</small></label>;
  const disabled = reason || cameraPropertyReason(property);
  const actual = cameraParameterReadout(property, stale);
  const value = stale ? "" : model.draft[property.key] ?? property.target ?? (actual === "–" ? "" : property.actual);
  // Option and reason text is presented in English; the engine's native value, its
  // supported/enabled/reason flags and the raw record are unchanged.
  const options = property.options.length
    ? property.options.map(option => ({ ...option, label: englishOptionLabel(property.key, option.value, option.label), reason: option.reason ? englishReason(option.reason) : null }))
    : property.supportedValues.map(native => ({ value: native, label: cameraPropertyValueLabel(property, native), supported: true, enabled: property.writable, reason: null }));
  const dirty = model.draft[property.key] !== undefined && model.draft[property.key] !== property.actual;
  const readbackTitle = actual === "–" ? "Unknown; no current confirmed readback" : actual;
  return <label className="device-field" title={localize([disabled, title, hint, `Actual: ${readbackTitle}`].filter(Boolean).join(" · "))}>
    <span>{localize(title)}</span>
    {cameraPropertyUsesSelect(property) ? <select className={`device-control${value === "" ? " is-unknown" : ""}`} aria-label={localize(`${title} draft${value === "" ? " · Unknown" : ""}`)} disabled={Boolean(disabled)} value={JSON.stringify(value)} onChange={event => model.set(property.key, JSON.parse(event.target.value))}>
      <option value={JSON.stringify("")} disabled>–</option>
      {options.map(option => <option key={JSON.stringify(option.value)} value={JSON.stringify(option.value)} disabled={!option.supported || !option.enabled} title={localize(option.reason || undefined)}>{localize(option.label)}{localize(!option.supported ? " · unsupported" : !option.enabled ? " · inactive" : "")}</option>)}
    </select> : <input className="device-control" aria-label={localize(`${title} draft${typeof value !== "number" ? " · Unknown" : ""}`)} placeholder="–" type="number" disabled={Boolean(disabled)} value={typeof value === "number" ? value : ""} min={property.range?.[0]} max={property.range?.[1]} step={property.range?.[2] || 1} onChange={event => { if (event.target.value) model.set(property.key, Number(event.target.value)); }} />}
    <small className="device-caption" title={localize(readbackTitle)}>{localize("Actual: ")}{localize(actual)}{localize(dirty ? " · draft" : "")}</small>
  </label>;
}

export function CameraPanel({ snapshot, stale, pending, model, run, onAdvanced, onPhoto, onLive, mediaRevision, cardImportReason = null }: { snapshot: EngineSnapshot; stale: boolean; pending: boolean; model: CameraDraftModel; run: (command: EngineCommand) => Promise<EngineSnapshot>; onAdvanced: () => void; onPhoto: (snapshot: EngineSnapshot, revision: number) => void; onLive: (revision: number) => void; mediaRevision: number; cardImportReason?: string | null }) {
  const camera = snapshot.deviceControls.camera;
  const [selected, setSelected] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  // A running card import owns the camera: capture, Live View and parameter writes
  // stay refused here as well as in the menu and the dialog.
  const manualReason = cardImportReason || manualControlReason(snapshot, stale) || (pending ? "Operation pending" : null);
  const connectionReason = cardImportReason || deviceConnectionReason(snapshot, stale) || (pending ? "Operation pending" : null);
  const cameraReason = manualReason || cameraControlReason(camera);
  const applyReason = cameraReason || model.issue || (!model.dirty ? "No draft changes" : null);
  const cardPhoto = camera?.properties.recordingMedia?.confirmed && !camera.properties.recordingMedia.error && camera.properties.recordingMedia.actual === 0;
  const captureReason = cameraReason || ((cardPhoto ? camera?.capabilities.cardCapture : camera?.capabilities.hostCapture) ? null : "Selected photo destination unavailable") || (camera?.liveView.active ? "Stop Live View explicitly before capture" : null) || cameraCapturePreparationReason(camera) || (cardPhoto || snapshot.parameters.savePath.trim() ? null : "Choose an image storage root before capture") || (model.dirty ? "Apply or reload the parameter draft first" : null);
  const readiness = snapshot.cameraScanReadiness;
  const scanReady = Boolean(readiness?.ready && !model.dirty);
  const connected = Boolean(camera?.connected && !camera.isolated);
  const cleanupUnknown = Boolean(camera && (camera.connected || camera.isolated) && !camera.cleanupConfirmed);
  const state = !stale && cleanupUnknown ? "CLEANUP UNKNOWN" : deviceConnectionLabel(connected, stale);
  const act = (action: () => Promise<unknown>) => { setMessage(null); void action().catch(reason => setMessage(String(reason))); };
  const root = snapshot.parameters.savePath;
  const identity = camera?.identity;
  const reason = message || (!camera?.cleanupConfirmed && camera?.isolated ? "Camera cleanup unconfirmed" : camera?.lastError) || captureReason || (readiness?.ready ? "Confirmed configuration" : readiness?.issues[0]?.remedy || "Read and verify camera configuration");
  return <section className="panel camera-panel" id="camera-panel"><header className="panel__header"><LineIcon name="camera" size={15} className="panel__icon" /><h2>{localize("Camera")}</h2><span className={`chip ${stale || cleanupUnknown ? "chip--warn" : "chip--muted"}`} title={localize(`Camera state: ${stale ? "UNKNOWN" : englishDisplayText(camera?.phase?.toUpperCase() || "UNKNOWN")}`)}>{localize(state)}</span></header><div className="device-body">
    <div className="device-group">
      <div className="device-group__head"><h3 className="device-group__title">{localize("Device")}</h3><span className="device-identity" title={localize(identity?.deviceId || "Select a camera explicitly")}>{identity ? `${unicodeDisplayText(identity.manufacturer)} ${unicodeDisplayText(identity.model)}` : localize("Not selected")}</span></div>
      <div className="device-grid device-grid--pick"><button className="primary" disabled={Boolean(connectionReason) || Boolean(camera?.connected)} onClick={() => act(() => run({ type: "camera_connect" }))}>{localize("Discover")}</button><select className="device-control" aria-label={localize("Active camera device")} value={selected} disabled={Boolean(connectionReason) || Boolean(camera?.connected)} onChange={event => setSelected(event.target.value)}><option value="">{localize("Select camera")}</option>{localize(camera?.discovered.map(item => <option key={item.deviceId} value={item.deviceId}>{unicodeDisplayText(item.model)} · {unicodeDisplayText(item.serial || item.deviceId)}</option>))}</select></div>
      <div className="device-grid device-grid--2"><button disabled={Boolean(connectionReason) || (!camera?.connected && !selected)} onClick={() => act(() => run(camera?.connected ? { type: "camera_disconnect" } : { type: "camera_connect", deviceId: selected }))}>{localize(camera?.connected ? "Disconnect" : "Connect")}</button><button onClick={onAdvanced} title={localize("Advanced camera settings, Live View and native properties")}>{localize("Advanced…")}</button></div>
      <label className="device-field"><span>{localize("Image storage root")}</span><div className="device-well"><span className="device-value" title={localize(unicodeDisplayText(root || "Not configured"))}>{root ? unicodeDisplayText(root) : localize("Not configured")}</span><button className="folder-btn" aria-label={localize("Choose image storage root")} disabled={Boolean(manualReason) || snapshot.mode === "developer_preview"} onClick={() => act(async () => { const selectedRoot = await chooseImageDirectory(root || await resolveDefaultImageDirectory()); if (selectedRoot) await run({ type: "set_capture_root", savePath: selectedRoot }); })}><LineIcon name="folder" size={15} /></button></div></label>
    </div>
    <div className="device-section">
      <div className="device-section__head"><h3 className="device-section__title">{localize("Exposure")}</h3></div>
      <div className="device-grid device-grid--2">{[["iso", "ISO"], ["shutter", "Native shutter"], ["aperture", "Aperture"], ["whiteBalance", "White balance"]].map(([key, label]) => <CameraPropertyControl key={key} property={camera?.properties[key]} model={model} reason={cameraReason} label={label} stale={stale} />)}</div>
      {/* Color temperature is a normal native parameter with a real parent dependency:
          the engine only marks it writable once the confirmed white balance is
          Color temperature (0x8012), so the control stays disabled until the parent
          mode is applied and read back. No predicted writability. */}
      <div className="device-grid device-grid--2 camera-destination-grid"><CameraPropertyControl property={camera?.properties.whiteBalanceTemperature} model={model} reason={cameraReason} stale={stale} label="Color temperature" hint="Selects a Kelvin value on the camera, sent as its native index 0–30 (index 20 = 5000 K). Preset manual is the camera's own measured/photo-reference white balance, not a software photographic preset." /><CameraPropertyControl property={camera?.properties.recordingMedia} model={model} reason={cameraReason} stale={stale} label="Photo destination" hint="Default: keep every photo on the camera SD card. Apply and verify before capture or Preflight. Local downloads require explicitly selecting Host RAM and Apply in this session. Scans import after X-ray OFF and remove their SD images after every local file is verified." /></div>
    </div>
    <div className="camera-capture">
    <div className="device-readouts">{[["exposureMode", "Mode"], ["autoIso", "Auto ISO"], ["focusMode", "Focus"]].map(([key, label]) => { const property = camera?.properties[key]; const text = cameraParameterReadout(property, stale); return <div className="device-readout" key={key}><span>{localize(label)}</span><strong className={text === "–" ? "is-unknown" : undefined} title={localize(text === "–" ? `${label} · Unknown; no current confirmed readback` : text)} aria-label={localize(text === "–" ? `${label} unknown` : undefined)}>{localize(text)}</strong></div>; })}</div>
    {/* U9: the engine snapshot already publishes camera status continuously, so the
        redundant Status button is gone. Each remaining entry names one duty:
        Refresh settings / Apply changes / Capture NEF / Live View. */}
    <div className="device-grid device-grid--3"><button disabled={Boolean(connectionReason || cameraControlReason(camera))} title={localize("Read the current native values and confirmed readback")} onClick={() => act(async () => model.loadConfirmed(await run({ type: "camera_read_properties" })))}>{localize("Refresh settings")}</button><button className={!applyReason || model.dirty ? "primary" : undefined} disabled={Boolean(applyReason)} title={localize(applyReason || "Write the draft serially and verify every readback")} onClick={() => act(async () => { const result = await run({ type: "camera_apply_properties", properties: model.changes, ...(model.presetId ? { presetId: model.presetId } : {}) }); model.loadConfirmed(result); })}>{localize("Apply changes")}</button><button disabled={Boolean(captureReason)} title={localize(captureReason || (cardPhoto ? "Capture one NEF to the camera SD card without downloading it" : "Capture one host RAM NEF into the scan-independent photo slot"))} onClick={() => act(async () => { const prior = camera?.lastCapturedImage?.captureId; const priorCard = camera?.lastCardCaptureTiming?.operationId; const result = await run({ type: "camera_capture" }); if (cardPhoto) { const health = result.deviceControls.camera; const receipt = health?.lastCardCaptureTiming; if (!receipt?.completed || receipt.operationId === priorCard || receipt.sessionId !== health?.sessionId) throw new Error("No new confirmed card capture"); setMessage("NEF saved to camera SD card · no local image transfer"); return; } if (!result.deviceControls.camera?.lastCapturedImage || result.deviceControls.camera.lastCapturedImage.captureId === prior) throw new Error("No new confirmed capture registration"); onPhoto(result, mediaRevision); })}>{localize("Capture NEF")}</button></div>
    {/* Stop Live View stays available whenever Live View is running, even while
        manual controls are locked, because ending the preview is a safety action. */}
    <div className={`device-grid device-grid--2 camera-view-actions${camera?.capabilities.focusStep ? " has-focus-steps" : ""}`}>
      <button disabled={camera?.liveView.active ? false : Boolean(cameraReason) || !camera?.capabilities.liveView} title={localize(camera?.liveView.active ? "Stop Live View explicitly before capture or parameter writes" : "Start the memory-only Live View preview")} onClick={() => act(async () => { if (camera?.liveView.active) { await run({ type: "camera_live_view_stop" }); return; } await run({ type: "camera_live_view_start" }); onLive(mediaRevision); })}>{localize(camera?.liveView.active ? "Stop Live View" : "Start Live View")}</button>
      <button disabled={Boolean(cameraReason) || !camera?.capabilities.autofocus} onClick={() => act(() => run({ type: "camera_autofocus" }))}>{localize("AF")}</button>
      {camera?.capabilities.focusStep && <>
        <button disabled={Boolean(cameraReason) || !camera.liveView.active} onClick={() => act(() => run({ type: "camera_focus", step: -1 }))}>{localize("Focus −1")}</button>
        <button disabled={Boolean(cameraReason) || !camera.liveView.active} onClick={() => act(() => run({ type: "camera_focus", step: 1 }))}>{localize("Focus +1")}</button>
      </>}
    </div>
    <div className="device-footer"><p role="status" title={localize(englishDisplayText(reason))}>{localize(englishDisplayText(reason))}</p><span className="camera-capability" title={localize(englishDisplayText(captureReason || "Single photo ready"))} aria-label={`${localize("Single photo ")}${captureReason ? "\u2715" : "\u2713"} \u00b7 ${localize(englishDisplayText(captureReason || "Single photo ready"))}`}>{localize("Single photo ")}{localize(captureReason ? "✕" : "✓")}</span><span className="camera-capability" title={localize(englishDisplayText(scanReady ? "Fixed scan ready" : readiness?.issues[0]?.requirement || "Fixed scan not ready"))} aria-label={`${localize("Fixed scan ")}${scanReady ? "\u2713" : "\u2715"} \u00b7 ${localize(englishDisplayText(scanReady ? "Fixed scan ready" : readiness?.issues[0]?.requirement || "Fixed scan not ready"))}`}>{localize("Fixed scan ")}{localize(scanReady ? "✓" : "✕")}</span></div>
    </div>
  </div></section>;
}
