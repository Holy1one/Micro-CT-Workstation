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
  reason,
}: {
  view: SceneViewModel;
  reason: SceneFallbackReason;
}) {
  return (
    <div className="scene-fallback" data-reason={reason}>
      <span className="scene-fallback__label">3D VIEW UNAVAILABLE · {REASON_LABELS[reason]}</span>
    </div>
  );
}
