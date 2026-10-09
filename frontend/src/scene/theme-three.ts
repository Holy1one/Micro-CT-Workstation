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

function readOpacity(name: string, fallback: number): number {
  const value = Number.parseFloat(readCssVariable(name));
  return Number.isFinite(value) ? value : fallback;
}

export function readSceneTheme(): SceneTheme {
  return {
    accent: readCssVariable("--accent"),
    beam: readCssVariable("--beamGlow"),
    sceneLine: readCssVariable("--sceneLine"),
    optics: readCssVariable("--optics"),
    textPrimary: readCssVariable("--textPrimary"),
    panel: readCssVariable("--panel"),
    panelHeader: readCssVariable("--panelHeader"),
    well: readCssVariable("--well"),
    chassis: readCssVariable("--chassis"),
    stageTable: readCssVariable("--stageTable"),
    stageWall: readCssVariable("--stageWall"),
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
    window.addEventListener("resize", update);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", update);
    };
  }, []);

  return theme;
}
