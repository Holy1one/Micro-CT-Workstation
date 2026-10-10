/** Ordered physical detector metadata. Empty rows are not detector layers. */
import { parseLayerDrafts, type LayerDraft } from "./scan-input";
export { blankLayer, parseLayerDrafts, type LayerDraft } from "./scan-input";
export function ScintillatorStackFields({ enabled, rows, disabled, onEnabled, onRows }: {
  enabled: boolean; rows: LayerDraft[]; disabled: boolean; onEnabled: (enabled: boolean) => void; onRows: (rows: LayerDraft[]) => void;
}) {
  const update = (index: number, key: keyof LayerDraft, value: string) => onRows(rows.map((row, i) => i === index ? { ...row, [key]: value } : row));
  return <section className="scan-prep-card">
    <label className="scintillator-stack-toggle"><input type="checkbox" checked={enabled} disabled={disabled} onChange={event => onEnabled(event.target.checked)} />Stacked scintillator scan</label>
    {enabled && <>
      <p className="device-note">Order from the X-ray incident side. Complete up to five rows; blank rows are ignored. Unknown glass composition may be recorded as “scintillator glass”.</p>
      <div className="scintillator-stack-grid" role="group" aria-label="Scintillator layers">
        {rows.map((row, index) => <div className="scintillator-stack-row" key={index}>
          <span>Layer {index + 1}</span>
          <label className="scan-geometry-field"><span>Material</span><input value={row.materialName} disabled={disabled} onChange={event => update(index, "materialName", event.target.value)} /></label>
          <label className="scan-geometry-field"><span>Emission colour</span><select value={row.emissionColor} disabled={disabled} onChange={event => update(index, "emissionColor", event.target.value)}><option value="">Select</option>{["green", "blue", "red", "yellow", "other"].map(colour => <option key={colour} value={colour}>{colour[0].toUpperCase() + colour.slice(1)}</option>)}</select></label>
          <label className="scan-geometry-field"><span>Thickness · mm</span><input inputMode="decimal" value={row.thicknessMm} disabled={disabled} onChange={event => update(index, "thicknessMm", event.target.value)} /></label>
        </div>)}
      </div>
      <p className="device-note">{parseLayerDrafts(enabled, rows)?.length ?? "Incomplete"} configured layers. RGB provides up to three colour observations; individually resolving layers and identifying materials require measured calibration.</p>
    </>}
  </section>;
}
