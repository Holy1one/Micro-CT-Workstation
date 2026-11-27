/** Synchronize native dialog/window copy only; no device or engine commands. */
import { invoke } from "@tauri-apps/api/core";
import { type Language } from "../i18n";
import { isDesktopRuntime } from "./desktopPaths";

let pending = Promise.resolve();
export function syncDesktopLanguage(language: Language): Promise<void> {
  if (!isDesktopRuntime()) return Promise.resolve();
  // Serialize rapid toggles so the native host finishes with the newest choice.
  pending = pending.catch(() => {}).then(() => invoke<void>("set_interface_language", { language }));
  return pending;
}
