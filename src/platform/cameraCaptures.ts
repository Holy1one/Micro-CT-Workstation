/** Registered independent NEF preview. The renderer supplies an engine capture ID,
 * never a file path. JPEG bytes stay in memory and are not exported as a file. */
import { invoke } from "@tauri-apps/api/core";
import { isDesktopRuntime } from "./desktopPaths";

export interface IndependentCapturePreview {
  captureId: string;
  sessionId: string;
  operationId: string;
  jpeg: number[];
}

export async function cameraCapturePreview(captureId: string): Promise<IndependentCapturePreview> {
  if (!isDesktopRuntime()) throw new Error("Independent photo preview requires the desktop application");
  if (!captureId || captureId.length > 256 || captureId.includes("\0")) throw new Error("Invalid capture ID");
  const preview = await invoke<IndependentCapturePreview>("camera_capture_preview", { captureId });
  if (preview.captureId !== captureId || !preview.sessionId || !preview.operationId
      || !Array.isArray(preview.jpeg) || preview.jpeg.length < 4 || preview.jpeg.length > 16 * 1024 * 1024
      || preview.jpeg[0] !== 255 || preview.jpeg[1] !== 216
      || preview.jpeg.at(-2) !== 255 || preview.jpeg.at(-1) !== 217
      || preview.jpeg.some(byte => !Number.isInteger(byte) || byte < 0 || byte > 255)) {
    throw new Error("Independent preview is malformed or belongs to another capture");
  }
  return preview;
}

export function capturePreviewDataUrl(preview: IndependentCapturePreview): string {
  let binary = "";
  for (let offset = 0; offset < preview.jpeg.length; offset += 32768) {
    binary += String.fromCharCode(...preview.jpeg.slice(offset, offset + 32768));
  }
  return `data:image/jpeg;base64,${btoa(binary)}`;
}
