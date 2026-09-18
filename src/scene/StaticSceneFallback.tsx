import type { SceneFallbackReason, SceneViewModel } from "./types";

const REASON_LABELS: Record<SceneFallbackReason, string> = {
  "webgl-unavailable": "WebGL unavailable",
  "reduced-motion": "Reduced motion",
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
