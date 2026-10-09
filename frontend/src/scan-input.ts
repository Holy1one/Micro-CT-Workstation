/** Numeric scan-field parsing. Minutes are a UI unit; engine commands use seconds. */
export const MAX_PROJECTIONS = 3600;
export const D7100_EXPOSURE_MIN_MS = 0.125;
export const D7100_EXPOSURE_MAX_MS = 30000;

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

export function minutesToSeconds(text: string): number | null {
  const minutes = decimal(text);
  if (minutes === null || minutes <= 0 || minutes > 10) return null;
  const seconds = minutes * 60;
  return Math.abs(seconds - Math.round(seconds)) < 1e-4 && seconds >= 1 ? Math.round(seconds) : null;
}

export function secondsToMinutes(seconds: number): string {
  return String(Number((seconds / 60).toFixed(6)));
}
