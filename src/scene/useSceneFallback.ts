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
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = (): void => {
      if (query.matches) {
        setReason("reduced-motion");
      } else if (!detectWebGlSupport()) {
        setReason("webgl-unavailable");
      } else {
        setReason((current) => (current === "context-lost" ? current : null));
      }
    };
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  return {
    reason,
    setContextLost: () => setReason("context-lost"),
  };
}
