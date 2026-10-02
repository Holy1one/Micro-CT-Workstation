/** Native camera media/preset facade. Endpoints are engine-controlled; callers
 * never supply arbitrary pipe names or read/write paths. Browser preview has no I/O. */
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { isDesktopRuntime } from "./desktopPaths";
export interface CameraLiveFrame { sessionId: string; sequence: number; width: number; height: number; jpeg: number[] }
export function validateLiveFrame(frame: CameraLiveFrame, sessionId: string, afterSeq: number): CameraLiveFrame {
  if (frame.sessionId !== sessionId || !Number.isSafeInteger(frame.sequence) || frame.sequence <= afterSeq
    || ![frame.width, frame.height].every(value => Number.isInteger(value) && value > 0 && value <= 8192)
    || frame.width * frame.height > 16_777_216
    || !Array.isArray(frame.jpeg) || frame.jpeg.length < 4 || frame.jpeg.length > 8 * 1024 * 1024
    || frame.jpeg[0] !== 255 || frame.jpeg[1] !== 216 || frame.jpeg.at(-2) !== 255 || frame.jpeg.at(-1) !== 217
    || frame.jpeg.some(byte => !Number.isInteger(byte) || byte < 0 || byte > 255)) throw new Error("Live View frame invalid, old or unrelated");
  return frame;
}
export async function cameraLiveViewFrame(afterSeq: number): Promise<CameraLiveFrame | null> {
  if (!isDesktopRuntime()) return null;
  return invoke<CameraLiveFrame | null>("camera_live_view_frame", { afterSeq });
}
export async function resolveCameraCaptureDirectory(): Promise<string> {
  if (!isDesktopRuntime()) return "";
  return invoke<string>("resolve_camera_capture_directory");
}
export async function chooseCameraCaptureDirectory(current: string): Promise<string | null> {
  if (!isDesktopRuntime()) return null;
  const selected = await open({ directory: true, multiple: false, defaultPath: current || undefined, title: "Select independent camera capture directory" });
  return typeof selected === "string" ? selected : null;
}
export async function importCameraPresets(): Promise<string | null> {
  if (!isDesktopRuntime()) throw new Error("Preset import requires the desktop application");
  return invoke<string | null>("camera_preset_import");
}
export async function exportCameraPresets(contents: string): Promise<string | null> {
  if (!isDesktopRuntime()) throw new Error("Preset export requires the desktop application");
  return invoke<string | null>("camera_preset_export", { contents });
}
