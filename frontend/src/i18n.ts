/** Presentation-only localization. Never pass translated text into engine commands,
 * identity checks, editable values, diagnostic JSON or exported evidence. */
import { zhCN } from "./zh-CN";

export type Language = "en" | "zh-CN";
export const LANGUAGE_KEY = "micro-ct-workstation.language";
const listeners = new Set<() => void>();
export function readSavedLanguage(): Language {
  try { return typeof window !== "undefined" && window.localStorage.getItem(LANGUAGE_KEY) === "zh-CN" ? "zh-CN" : "en"; }
  catch { return "en"; }
}
let language: Language = readSavedLanguage();
export const getLanguage = (): Language => language;
export function subscribeLanguage(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function setLanguage(next: Language): void {
  if (next !== "en" && next !== "zh-CN") return;
  try { if (typeof window !== "undefined") window.localStorage.setItem(LANGUAGE_KEY, next); } catch { /* Session choice remains usable when storage is blocked. */ }
  if (language === next) return;
  language = next;
  for (const listener of listeners) listener();
}

const normalize = (text: string): string => text.replace(/\s+/g, " ").trim();
const messages = new Map(Object.entries(zhCN).map(([key, value]) => [normalize(key), value]));
const caseInsensitiveMessages = new Map(Object.entries(zhCN).map(([key, value]) => [normalize(key).toLowerCase(), value]));
const escapePattern = (text: string): string => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const templates = Object.entries(zhCN).filter(([key]) => /\{\d+\}/.test(key))
  .sort(([a], [b]) => b.replace(/\{\d+\}/g, "").length - a.replace(/\{\d+\}/g, "").length)
  .map(([key, value]) => {
    const order: string[] = [];
    const pattern = normalize(key).split(/(\{\d+\})/).map(part => {
      if (/^\{\d+\}$/.test(part)) { order.push(part); return "(.*?)"; }
      return escapePattern(part);
    }).join("");
    const rawArguments = /^(?:CAPTURED IMAGES|Presets exported|PORT |SERIAL |Unmapped diagnostic)/.test(key);
    return { pattern: new RegExp(`^${pattern}$`, "u"), value, order, rawArguments };
  });
const cache = new Map<string, string>();
/** Exact messages and reviewed sentence templates, not word-by-word machine translation.
 * Unrecognized third-party/device details remain verbatim so evidence is never invented. */
export function translate(text: string, locale: Language = language, depth = 0): string {
  if (locale === "en" || !text || depth > 4) return text;
  const key = normalize(text);
  const direct = messages.get(key) ?? caseInsensitiveMessages.get(key.toLowerCase());
  if (direct !== undefined) return text.replace(text.trim(), direct);
  if (cache.has(text)) return cache.get(text)!;
  // Do not rewrite JSON, filenames, paths, URLs, hashes, or protocol error codes.
  if (/^(?:[\[{]|[A-Za-z]:[\\/]|\\\\|https?:\/\/|ENGINE:[A-Z_]+$)/.test(key)) return text;
  let result = text;
  for (const entry of templates) {
    const match = entry.pattern.exec(key);
    if (!match) continue;
    result = text.replace(text.trim(), entry.value.replace(/\{\d+\}/g, token => {
      const index = entry.order.indexOf(token);
      return index < 0 ? token : entry.rawArguments ? match[index + 1] : translate(match[index + 1], locale, depth + 1);
    }));
    break;
  }
  if (result === text) {
    // Display-only joined states are assembled by several view models.
    const parts = text.split(/( · |\n)/);
    if (parts.length > 1) result = parts.map(part => part === " · " || part === "\n" ? part : translate(part, locale, depth + 1)).join("");
  }
  if (cache.size >= 2048) cache.clear();
  cache.set(text, result);
  return result;
}

/** JSX boundary helper: leave React elements, numbers, values and callbacks untouched. */
export function localize<T>(value: T): T {
  return (typeof value === "string" ? translate(value) : value) as T;
}
