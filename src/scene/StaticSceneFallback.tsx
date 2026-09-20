/**
 * Non-interactive scene replacement for unavailable or lost WebGL contexts.
 * It communicates the graphics limitation while continuing to display the
 * engine-provided equipment state; it does not alter device control.
 */

import type { SceneFallbackReason, SceneViewModel } from "./types";

const REASON_LABELS: Record<SceneFallbackReason, string> = {
  "webgl-unavailable": "WebGL unavailable",
  "context-lost": "Graphics context lost",
};

export function StaticSceneFallback({
  view,
  reason,
}: {
  view: SceneViewModel;
  reason: SceneFallbackReason;
}) {
  const rotated = Math.abs(view.angleDeg) > 0.005;
  const src = `/assets/3D-scene-${view.theme}${rotated ? "-144" : ""}.png`;
  return (
    <div className="scene-fallback" data-reason={reason}>
      <img
        className="scene-render"
        src={src}
        alt="Micro-CT equipment scene fallback"
        draggable={false}
      />
      <span className="scene-fallback__label">STATIC VIEW · {REASON_LABELS[reason]}</span>
    </div>
  );
}
