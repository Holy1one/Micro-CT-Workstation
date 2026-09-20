/**
 * Small frontend facade over native path, picker, reveal, and save operations.
 * Browser preview calls return safe defaults or explicit unsupported errors;
 * no device or scan state belongs in this platform integration module.
 */

import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

/** Returns whether the page is running inside the Tauri desktop shell. */
export function isDesktopRuntime(): boolean {
  const runtimeWindow = window as Window & {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
  };
  return Boolean(runtimeWindow.__TAURI_INTERNALS__ ?? runtimeWindow.__TAURI__);
}

/** Resolves the current Windows user's configured image directory without creating it. */
export async function resolveDefaultImageDirectory(): Promise<string> {
  if (!isDesktopRuntime()) {
    throw new Error("System image directory is unavailable in developer preview");
  }
  return invoke<string>("resolve_default_image_directory");
}

/** Opens the native single-directory picker. Cancellation returns null. */
export async function chooseImageDirectory(currentPath: string): Promise<string | null> {
  if (!isDesktopRuntime()) {
    return null;
  }
  const selection = await open({
    directory: true,
    multiple: false,
    defaultPath: currentPath.trim() || undefined,
    title: "Select projection image directory",
  });
  return typeof selection === "string" ? selection : null;
}

/** Reveals an existing directory in Windows Explorer. */
export async function revealDirectory(path: string): Promise<string> {
  if (!isDesktopRuntime()) {
    throw new Error("Opening a folder requires the desktop application");
  }
  return invoke<string>("open_directory_in_shell", { path });
}

/**
 * Writes the session log to a path chosen through the native save dialog.
 * Cancellation returns null; the Rust side only accepts .log/.txt targets.
 */
export async function exportSessionLog(suggestedName: string, contents: string): Promise<string | null> {
  if (!isDesktopRuntime()) {
    throw new Error("Exporting a session log requires the desktop application");
  }
  const target = await save({
    defaultPath: suggestedName,
    title: "Export session log",
    filters: [{ name: "Session log", extensions: ["log", "txt"] }],
  });
  if (typeof target !== "string" || target.trim() === "") return null;
  return invoke<string>("export_session_log", { path: target, contents });
}
