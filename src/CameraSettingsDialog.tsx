/** Full native catalog and presets use the permanent camera panel's session draft. */
import { useState } from "react";
import type { CameraPreset, CameraProperty, CameraValue, EngineCommand, EngineSnapshot } from "./engine/types";
import type { CameraDraftModel } from "./camera-draft";
import { cameraControlReason, cameraPropertyLabel, cameraPropertyReason, cameraPropertyUsesSelect, cameraPropertyValueLabel, confirmedCameraDraft, manualControlReason, nativePropertyLabel } from "./engine/deviceControls";
import { englishOptionLabel, englishReason } from "./engine/native-labels";
import { importCameraPresets, exportCameraPresets } from "./platform/cameraControls";
import { WorkstationDialog } from "./WorkstationDialog";
/** A retained value is not confirmed readback evidence. Preset comparison never uses drafts. */
export function presetReadbackEvidence(property: CameraProperty | undefined, expected: CameraValue): { actual: CameraValue | null; comparison: "UNKNOWN" | "Match" | "Different" } {
  if (!property || !property.supported || !property.readable || !property.confirmed || property.actual == null || property.error) return { actual: null, comparison: "UNKNOWN" };
  return { actual: property.actual, comparison: property.actual === expected ? "Match" : "Different" };
}
/** The five photographic groups of docs/frontend-functional-spec.md §4.1: 9 + 5 + 10 + 7 + 3 = 34 keys.
 *  Classification is presentation only. `other` is compiled from the live descriptor, so a
 *  property a future engine build adds can never be hidden behind these static groups. */
export const CAMERA_PROPERTY_GROUPS = [
  { id: "exposure", label: "Exposure & metering", keys: [
    "exposureMode", "iso", "autoIso", "autoIsoHighLimit", "shutter", "aperture",
    "exposureCompensation", "meteringMode", "centerWeightedArea"] },
  { id: "whiteBalance", label: "White balance & colour", keys: [
    "whiteBalance", "whiteBalanceFluorescent", "whiteBalanceTemperature", "colorSpace", "pictureControl"] },
  { id: "file", label: "File & capture", keys: [
    "rawFormat", "rawBitDepth", "rawCompression", "crop", "imageSize", "driveMode",
    "burstNumber", "recordingMedia", "focusMode", "exposureDelay"] },
  { id: "processing", label: "Automatic processing", keys: [
    "bracketing", "activeDLighting", "hdrMode", "hdrExposureDifference", "hdrSmoothing",
    "longExposureNoiseReduction", "highIsoNoiseReduction"] },
  { id: "flash", label: "Flash settings", keys: ["flashMode", "flashCompensation", "flashSyncSpeed"] },
] as const;
type CameraGroupId = (typeof CAMERA_PROPERTY_GROUPS)[number]["id"] | "other" | "presets" | "diagnostics";

export function CameraSettingsDialog({ snapshot, pending, stale, model, run, onClose, onStop }: { snapshot: EngineSnapshot; pending: boolean; stale: boolean; model: CameraDraftModel; run: (command: EngineCommand) => Promise<EngineSnapshot>; onClose: () => void; onStop: () => void }) {
  const camera = snapshot.deviceControls.camera;
  const draft = model.draft; const draftPresetId = model.presetId;
  const [presetId, setPresetId] = useState(""); const [name,setName] = useState(""); const [description,setDescription] = useState("");
  const [comparison,setComparison] = useState<CameraPreset | null>(null); const [message,setMessage] = useState<string | null>(null);
  // Category selection is the only local state this page adds; switching it never runs a command.
  const [group,setGroup] = useState<CameraGroupId>("exposure");
  const reason = manualControlReason(snapshot,stale) || cameraControlReason(camera) || (pending ? "Operation pending" : null);
  const propertyReason = reason || (camera?.capabilities.properties ? null : "Property capability unavailable");
  const readReason = propertyReason; const draftError = model.issue; const changes = model.changes;
  const applyReason = propertyReason || draftError || (!model.dirty ? "No parameter changes" : null);
  const preset = snapshot.deviceControls.presets.presets.find(p => p.id === presetId);
  const dispatch = run;
  const act = async (command: EngineCommand) => { setMessage(null); try { const result = await run(command); if(command.type === "camera_apply_properties") { model.loadConfirmed(result); setMessage("Apply completed with confirmed readback"); } return result; } catch(error) { setMessage(String(error)); return undefined; } };
  const read = async () => { const result = await act({ type: "camera_read_properties" }); if(result) model.loadConfirmed(result); };
  const loadPreset = () => { if(!preset) return; try { model.loadPreset(preset); setMessage("Preset loaded into the shared draft; Apply explicitly"); } catch(error) { setMessage(String(error)); } };
  const properties = camera?.properties ?? {};
  const known = new Set<string>(CAMERA_PROPERTY_GROUPS.flatMap(entry => [...entry.keys]));
  const otherKeys = Object.keys(properties).filter(key => !known.has(key));
  const activeKeys = group === "other" ? otherKeys
    : group === "presets" || group === "diagnostics" ? []
    : [...(CAMERA_PROPERTY_GROUPS.find(entry => entry.id === group)?.keys ?? [])];
  const dirtyKeys = new Set(Object.keys(changes));
  const rows = activeKeys.map(key => properties[key]).filter((property): property is CameraProperty => Boolean(property));
  const tab = (id: CameraGroupId, label: string, count?: number) => <button key={id} type="button" role="tab" aria-selected={group === id} className={`camera-advanced__tab${group === id ? " is-active" : ""}`} onClick={() => setGroup(id)}><span>{label}</span>{count === undefined ? null : <span className="camera-advanced__count">{count}</span>}{id !== "other" && id !== "presets" && id !== "diagnostics" && CAMERA_PROPERTY_GROUPS.find(entry => entry.id === id)!.keys.some(key => dirtyKeys.has(key)) ? <i className="camera-advanced__dot" aria-label="has unapplied changes" /> : null}</button>;
  return <WorkstationDialog title="Camera Advanced Settings" onClose={onClose} onStop={onStop} className="camera-settings-dialog"
    footer={<div className="camera-advanced__actions">
      <button disabled={Boolean(readReason)} title={readReason || "Read actual properties into the shared draft"} onClick={() => void read()}>Refresh settings</button>
      <button disabled={Boolean(applyReason)} title={applyReason || "Write serially and verify every readback"} onClick={() => void act({ type: "camera_apply_properties", properties: changes, ...(draftPresetId ? { presetId: draftPresetId } : {}) })}>Apply changes</button>
      <span className="camera-advanced__dirty" role="status">{model.dirty ? `${Object.keys(changes).length} change(s) not applied` : "No unapplied changes"}</span>
    </div>}>
    <p className="scan-prep__lead">Shutter and scan Exposure share one target. Apply a mode change and read it back to unlock its dependent controls. Disabled parameters show the camera's reason.</p>
    <div className="camera-advanced">
      <nav className="camera-advanced__nav" role="tablist" aria-label="Camera advanced categories">
        {CAMERA_PROPERTY_GROUPS.map(entry => tab(entry.id, entry.label, entry.keys.filter(key => properties[key]).length))}
        {tab("other", "Other described parameters", otherKeys.length)}
        {tab("presets", "Presets", snapshot.deviceControls.presets.presets.length)}
        {tab("diagnostics", "Read-only diagnostics")}
      </nav>
      <div className="camera-advanced__pane" role="tabpanel" aria-label="Camera advanced content">
        {group === "presets" ? <section className="device-console-section"><h3>Camera presets <small>Camera parameters only</small></h3>
          <div className="device-console-actions"><select aria-label="Camera preset" value={presetId} onChange={e => setPresetId(e.target.value)}><option value="">Select named preset</option>{snapshot.deviceControls.presets.presets.map(p => <option key={p.id} value={p.id}>{p.name} · {p.models.join(", ")}</option>)}</select><button disabled={!preset || Boolean(reason)} onClick={loadPreset}>Load draft</button><button disabled={!preset || !camera} onClick={() => setComparison(preset || null)}>Compare readback</button><button disabled={!preset || pending} onClick={() => void act({ type: "preset_delete", id: presetId })}>Delete preset</button></div>
          <div className="device-console-actions"><input aria-label="Preset name" placeholder="Preset name" value={name} onChange={e => setName(e.target.value)} /><input aria-label="Preset description" placeholder="Description" value={description} onChange={e => setDescription(e.target.value)} /><button disabled={!name.trim() || Boolean(propertyReason) || !Object.keys(confirmedCameraDraft(camera?.properties || {})).length} onClick={() => void act({ type: "preset_save", name: name.trim(), description, source: "native-camera-confirmed-readback" })}>Save confirmed parameters</button></div>
          <div className="device-console-actions"><button disabled={snapshot.mode === "developer_preview" || pending} onClick={() => void importCameraPresets().then(text => text === null ? undefined : act({ type: "preset_import", jsonText: text })).catch(e => setMessage(String(e)))}>Import preset file…</button><button disabled={snapshot.mode === "developer_preview" || pending || Boolean(snapshot.deviceControls.presets.error)} onClick={() => void dispatch({ type: "preset_export" }).then(response => { const text = response?.deviceControls.exportJson; if (!text) throw new Error("Engine did not return a verified preset export"); return exportCameraPresets(text); }).then(path => { if (path) setMessage(`Presets exported: ${path}`); }).catch(e => setMessage(String(e)))}>Export all presets…</button></div>
          {comparison && <table className="camera-properties"><thead><tr><th>{comparison.name}</th><th>Preset</th><th>Actual</th><th>Comparison</th></tr></thead><tbody>{Object.entries(comparison.parameters).map(([key, value]) => { const property = camera?.properties[key]; const evidence = presetReadbackEvidence(property, value); return <tr key={key}><td>{key}</td><td>{nativePropertyLabel(key, value, property?.code)}</td><td>{nativePropertyLabel(key, evidence.actual, property?.code)}</td><td>{evidence.comparison}</td></tr>; })}</tbody></table>}
        </section>
        : group === "diagnostics" ? <details open><summary>Read-only runtime and cleanup diagnostics</summary><p>Physical focus position: {camera?.focusPosition ?? "UNKNOWN; not stored in presets"}</p><pre className="diagnostic-json">{JSON.stringify({ identity:camera?.identity, capabilities:camera?.capabilities, runtime:camera?.runtimeProperties, capture:camera?.lastCaptureEventDiagnostic, cleanup:camera?.lastCleanupDiagnostic },null,2)}</pre><p>Raw status and transport closure do not substitute for physical exposure-end evidence.</p></details>
        : rows.length ? <>{group === "whiteBalance" && <p className="device-note">Colour temperature selects a Kelvin value on the camera, sent as its native index 0–30 (index 20 = 5000 K). Preset manual is the camera's own measured/photo-reference white balance, not a software photographic preset.</p>}<table className="camera-properties"><thead><tr><th>Parameter / capability</th><th>Draft target</th><th>Actual readback</th><th>Evidence</th></tr></thead><tbody>
          {rows.map(property => {
            const unavailable = cameraPropertyReason(property);
            const disabled = Boolean(propertyReason || unavailable);
            // English presentation of the engine's option labels and reasons; the native
            // value, supported/enabled flags and the raw record are untouched.
            const options = property.options.length ? property.options.map(option => ({ ...option, label: englishOptionLabel(property.key, option.value, option.label), reason: option.reason ? englishReason(option.reason) : null })) : property.supportedValues.map(value => ({ value, label: nativePropertyLabel(property.key, value, property.code), supported: true, enabled: property.writable, reason: null }));
            return <tr key={property.key} className={unavailable ? "camera-property--disabled" : undefined} title={unavailable || undefined}><td>{cameraPropertyLabel(property)}<small>{property.supported ? property.effective ? property.nativeWritable ? "Available" : "Read only" : "Inactive" : "Not supported"}</small></td><td>
              {cameraPropertyUsesSelect(property) ? <select aria-label={`${cameraPropertyLabel(property)} draft`} title={propertyReason || unavailable || undefined} disabled={disabled} value={JSON.stringify(draft[property.key] ?? property.target ?? property.actual ?? "")} onChange={event => { model.set(property.key, JSON.parse(event.target.value) as number | string); }}><option value={JSON.stringify("")} disabled>Select value</option>{options.map(option => <option key={JSON.stringify(option.value)} value={JSON.stringify(option.value)} disabled={!option.supported || !option.enabled} title={option.reason || undefined}>{option.label}{option.supported ? option.enabled ? "" : " · unavailable" : " · not supported"}</option>)}</select>
              : property.range ? <input type="number" aria-label={`${cameraPropertyLabel(property)} draft`} title={propertyReason || unavailable || undefined} min={property.range[0]} max={property.range[1]} step={property.range[2] || 1} disabled={disabled} value={draft[property.key] ?? ""} onChange={e => { model.set(property.key, Number(e.target.value)); }} /> : <input aria-label={`${cameraPropertyLabel(property)} draft`} disabled value={cameraPropertyValueLabel(property, property.actual)} />}
            </td><td>{cameraPropertyValueLabel(property, property.actual)}</td><td>{unavailable || property.error || (property.confirmed ? "Readback confirmed" : "UNKNOWN / not confirmed")}</td></tr>;
          })}
        </tbody></table></>
        : <p className="camera-advanced__empty">{camera ? "This group has no described parameters in the current session." : "No described camera properties. Connect a selected device and read its capabilities."}</p>}
      </div>
    </div>
    {(propertyReason || draftError) && <p className="device-control-reason">{propertyReason || draftError}</p>}
    {(message || snapshot.deviceControls.presets.error) && <p role="status" className="device-control-reason">{message || snapshot.deviceControls.presets.error}</p>}
  </WorkstationDialog>;
}
