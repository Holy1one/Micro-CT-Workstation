/**
 * Detect WebGL support and convert runtime context loss into a UI fallback.
 * This hook changes visualization only; it never changes engine safety state
 * or attempts to recover production hardware.
 */

import { useEffect, useState } from "react";
import type { SceneFallbackReason } from "./types";

export function detectWebGlSupport(): boolean {
  try {
    const canvas = document.createElement("canvas");
    return Boolean(canvas.getContext("webgl2") ?? canvas.getContext("webgl"));
  } catch {
    return false;
  }
}

export function useSceneFallback(): {
  reason: SceneFallbackReason | null;
  setContextLost: () => void;
} {
  const [reason, setReason] = useState<SceneFallbackReason | null>(() =>
    detectWebGlSupport() ? null : "webgl-unavailable",
  );

  useEffect(() => {
    // The live scene is the operator's only view of the bench, so it falls back
    // to the static render only when WebGL is genuinely unusable. A
    // "prefers-reduced-motion" setting asks for less animation, not for a
    // different picture, so it must never swap the 3D view out.
    if (!detectWebGlSupport()) {
      setReason("webgl-unavailable");
    } else {
      setReason((current) => (current === "context-lost" ? current : null));
    }
  }, []);

  return {
    reason,
    setContextLost: () => setReason("context-lost"),
  };
}
