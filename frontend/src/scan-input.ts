/** Numeric scan-field parsing. The X-ray limit is a whole-second contract; the UI may
 *  express it as exact seconds, decimal minutes or mm:ss. */
import type { ScintillatorLayer } from "./engine/types";
export interface LayerDraft { materialName: string; emissionColor: string; thicknessMm: string }
export const blankLayer = (): LayerDraft => ({ materialName: "", emissionColor: "", thicknessMm: "" });
/** Blank rows are ignored; a partial row invalidates the complete submission. */
export function parseLayerDrafts(enabled: boolean, rows: LayerDraft[]): ScintillatorLayer[] | null {
  if (!enabled) return [];
  if (rows.length > 5) return null;
  const filled = rows.filter(row => Object.values(row).some(value => value.trim()));
  if (!filled.length || filled.some(row => !row.materialName.trim() || row.materialName.trim().length > 128
    || !["red", "green", "blue", "yellow", "other"].includes(row.emissionColor)
    || !/^(?:\d+\.?\d*|\.\d+)$/.test(row.thicknessMm.trim())
    || Number(row.thicknessMm) <= 0 || Number(row.thicknessMm) > 100)) return null;
  return filled.map(row => ({ materialName: row.materialName.trim(), emissionColor: row.emissionColor as ScintillatorLayer["emissionColor"], thicknessMm: Number(row.thicknessMm) }));
}
export const MAX_PROJECTIONS = 3600;
export const D7100_EXPOSURE_MIN_MS = 0.125;
export const D7100_EXPOSURE_MAX_MS = 30000;
/** Unified scan continuous-X-ray limit; keep in step with ct-engine `scan::SCAN_MAX_XRAY_SEC`. */
export const MIN_XRAY_LIMIT_SEC = 1;
export const MAX_XRAY_LIMIT_SEC = 1000;

const decimal = (text: string): number | null => {
  const value = text.trim();
  if (!/^(?:\d+\.?\d*|\.\d+)$/.test(value)) return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
};

export function projectionError(text: string): string | null {
  return /^\d+$/.test(text.trim()) && Number(text) >= 1 && Number(text) <= MAX_PROJECTIONS
    ? null : "Total projections must be an integer from 1 to 3600.";
}

export function exposureError(text: string, minMs = D7100_EXPOSURE_MIN_MS, maxMs = D7100_EXPOSURE_MAX_MS): string | null {
  const value = decimal(text);
  return value !== null && value >= minMs && value <= maxMs
    ? null : `Exposure must be between ${minMs} and ${maxMs} ms.`;
}

/** Exact whole seconds for a maximum-continuous-X-ray draft. Decimal minutes ("10", "0.5")
 *  and mm:ss ("16:40") are both accepted. Anything that is not an exact second count inside
 *  1..1000 returns null, so "16.7" or "17:00" can never silently become 1000 s. */
export function xrayLimitSeconds(text: string): number | null {
  const value = text.trim();
  const clock = /^(\d{1,2}):([0-5]\d)$/.exec(value);
  const seconds = clock ? Number(clock[1]) * 60 + Number(clock[2]) : minutesToExactSeconds(value);
  return seconds !== null && seconds >= MIN_XRAY_LIMIT_SEC && seconds <= MAX_XRAY_LIMIT_SEC ? seconds : null;
}

function minutesToExactSeconds(value: string): number | null {
  const minutes = decimal(value);
  if (minutes === null) return null;
  const seconds = minutes * 60;
  return Math.abs(seconds - Math.round(seconds)) < 1e-4 ? Math.round(seconds) : null;
}

/** Exact mm:ss rendering of a frozen limit: 1000 -> "16:40", 600 -> "10:00". */
export function xrayLimitText(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** Review text that keeps the whole-second contract and its mm:ss form together. */
export function xrayLimitSummary(seconds: number): string {
  return `${Math.round(seconds)} s (${xrayLimitText(seconds)})`;
}
