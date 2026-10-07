/** Latest decoded preview in memory only. A missed poll preserves the image;
 * session retirement releases it and rejects late work. No filesystem API exists. */
import type { CameraLiveFrame } from "./cameraControls";

export interface LiveViewImage { url: string; sequence: number; receivedAt: number; error: string | null }
/** Data URLs avoid Chromium's Blob storage, which may spill blobs to disk under
 * memory pressure. The bounded JPEG lives only in the renderer's JS/image heap. */
export function liveViewDataUrl(jpeg: number[]): string {
  if (!jpeg.length || jpeg.length > 4 * 1024 * 1024) throw new Error("Live View JPEG size invalid");
  const chunks: string[] = [];
  for (let offset = 0; offset < jpeg.length; offset += 32768) chunks.push(String.fromCharCode(...jpeg.slice(offset, offset + 32768)));
  return `data:image/jpeg;base64,${btoa(chunks.join(""))}`;
}
interface DecodedImage { url: string; width: number; height: number }
interface LiveViewMedia {
  fetch: (afterSeq: number) => Promise<CameraLiveFrame | null>;
  validate: (frame: CameraLiveFrame, session: string, afterSeq: number) => unknown;
  decode: (frame: CameraLiveFrame) => Promise<DecodedImage>;
  release: (url: string) => void;
  update: (image: LiveViewImage | null, error?: string | null) => void;
  now: () => number;
}
export class MemoryLiveViewFeed {
  private stopped = false;
  private pending = false;
  private image: LiveViewImage | null = null;
  private retired: string | null = null;
  constructor(readonly session: string, private media: LiveViewMedia) {}
  async poll(): Promise<void> {
    if (this.stopped || this.pending || this.retired) return;
    this.pending = true;
    try {
      const afterSeq = this.image?.sequence ?? 0;
      const frame = await this.media.fetch(afterSeq);
      if (this.stopped || !frame) return;
      this.media.validate(frame, this.session, afterSeq);
      const decoded = await this.media.decode(frame);
      if (this.stopped || decoded.width !== frame.width || decoded.height !== frame.height) {
        this.media.release(decoded.url);
        if (!this.stopped) throw new Error("Live View JPEG dimensions disagree with frame metadata");
        return;
      }
      this.retired = this.image?.url ?? null;
      this.image = { url: decoded.url, sequence: frame.sequence, receivedAt: this.media.now(), error: null };
      this.media.update(this.image);
    } catch (reason) {
      if (!this.stopped) {
        // Transport errors never turn an older image into current evidence.
        this.image = this.image ? { ...this.image, error: String(reason) } : null;
        this.media.update(this.image, String(reason));
      }
    } finally { this.pending = false; }
  }
  presented(sequence: number): void {
    if (this.image?.sequence === sequence && this.retired) {
      this.media.release(this.retired); this.retired = null;
    }
  }
  stop(): void {
    this.stopped = true;
    if (this.retired) this.media.release(this.retired);
    if (this.image) this.media.release(this.image.url);
    this.retired = null; this.image = null; this.media.update(null);
  }
}
