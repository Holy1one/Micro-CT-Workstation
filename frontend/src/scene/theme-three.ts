/**
 * Translate CSS design tokens into Three.js material colors and opacities.
 * Keeping this conversion centralized prevents scene components from drifting
 * away from the light/dark workstation themes.
 */

import { useEffect, useState } from "react";
import type { SceneTheme } from "./types";

function readCssVariable(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function readColor(name: string): string {
  const value = readCssVariable(name);
  if (!/^(Canvas|CanvasText|Highlight|HighlightText)$/i.test(value)) return value;
  // Three.js cannot parse OS system-color keywords. Let the browser resolve
  // the current high-contrast palette without changing normal theme colors.
  const probe = document.createElement("span");
  probe.hidden = true;
  probe.style.color = value;
  probe.style.forcedColorAdjust = "none";
  document.documentElement.append(probe);
  const resolved = getComputedStyle(probe).color;
  probe.remove();
  return resolved;
}

function readOpacity(name: string, fallback: number): number {
  const value = Number.parseFloat(readCssVariable(name));
  return Number.isFinite(value) ? value : fallback;
}

export function readSceneTheme(): SceneTheme {
  return {
    accent: readColor("--accent"),
    beam: readColor("--beamGlow"),
    sceneLine: readColor("--sceneLine"),
    optics: readColor("--optics"),
    textPrimary: readColor("--textPrimary"),
    panel: readColor("--panel"),
    panelHeader: readColor("--panelHeader"),
    well: readColor("--well"),
    chassis: readColor("--chassis"),
    stageTable: readColor("--stageTable"),
    stageWall: readColor("--stageWall"),
    stageGlow: readOpacity("--stageGlow", 0.3),
    rayReady: readOpacity("--rayReady", 0.12),
    rayScanning: readOpacity("--rayScanning", 0.85),
    rayPaused: readOpacity("--rayPaused", 0.14),
    rayFault: readOpacity("--rayFault", 0),
  };
}

export function useSceneTheme(): SceneTheme {
  const [theme, setTheme] = useState<SceneTheme>(() => readSceneTheme());

  useEffect(() => {
    const update = (): void => setTheme(readSceneTheme());
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const forcedColors = window.matchMedia("(forced-colors: active)");
    forcedColors.addEventListener("change", update);
    window.addEventListener("resize", update);
    return () => {
      observer.disconnect();
      forcedColors.removeEventListener("change", update);
      window.removeEventListener("resize", update);
    };
  }, []);

  return theme;
}
