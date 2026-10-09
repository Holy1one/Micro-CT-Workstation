/**
 * Unified linear icon set for the workstation console.
 *
 * Every icon is drawn on a 24×24 grid with a single 1.6px round-capped
 * stroke in `currentColor`, so one component covers menus, panel headers,
 * the control dock, form controls and the status area with one visual
 * language. Colour is always supplied by the context (CSS `color`), never
 * baked into the artwork, which keeps light/dark themes and semantic tones
 * in sync automatically.
 */

import type { ReactNode } from "react";

export type LineIconName =
  | "task-new"
  | "folder"
  | "folder-open"
  | "history"
  | "export"
  | "undo"
  | "redo"
  | "reset"
  | "sliders"
  | "shield-check"
  | "crosshair"
  | "pulse"
  | "book"
  | "warning"
  | "info"
  | "question"
  | "play"
  | "pause"
  | "stop"
  | "radiation"
  | "rotate"
  | "camera"
  | "link"
  | "gauge"
  | "terminal"
  | "image"
  | "cube"
  | "sun"
  | "moon"
  | "send";

const STROKE = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

const PATHS: Record<LineIconName, ReactNode> = {
  /* File — new scan task: document with a plus. */
  "task-new": (
    <g {...STROKE}>
      <path d="M14 3.5H7a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5z" />
      <path d="M14 3.5v5h5" />
      <path d="M12 12v5M9.5 14.5h5" />
    </g>
  ),
  /* Closed folder (save-path picker). */
  folder: (
    <g {...STROKE}>
      <path d="M3.5 7a2 2 0 0 1 2-2h3.6l2 2h7.4a2 2 0 0 1 2 2v8.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />
    </g>
  ),
  /* Open folder. */
  "folder-open": (
    <g {...STROKE}>
      <path d="M3.5 7a2 2 0 0 1 2-2h3.6l2 2h7.4a2 2 0 0 1 2 2v1" />
      <path d="M3.5 7v10.5a2 2 0 0 0 2 2h12.3a2 2 0 0 0 1.9-1.4l1.8-5.1a1.5 1.5 0 0 0-1.4-2H7.2a2 2 0 0 0-1.9 1.4L3.5 17" />
    </g>
  ),
  /* Clock with a counter-clockwise arrow: history / restore. */
  history: (
    <g {...STROKE}>
      <path d="M4 12a8 8 0 1 0 2.3-5.7L4 8.5" />
      <path d="M4 4v4.5h4.5" />
      <path d="M12 8v4.2l3 1.8" />
    </g>
  ),
  /* Export: arrow rising out of a tray. */
  export: (
    <g {...STROKE}>
      <path d="M12 15V4.5" />
      <path d="M7.5 8.5 12 4l4.5 4.5" />
      <path d="M4.5 15v3a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-3" />
    </g>
  ),
  undo: (
    <g {...STROKE}>
      <path d="M8.5 5.5 4 10l4.5 4.5" />
      <path d="M4 10h9.5a5.5 5.5 0 0 1 0 11H10" />
    </g>
  ),
  redo: (
    <g {...STROKE}>
      <path d="M15.5 5.5 20 10l-4.5 4.5" />
      <path d="M20 10h-9.5a5.5 5.5 0 0 0 0 11H14" />
    </g>
  ),
  /* Counter-clockwise circular arrow: reset to defaults. */
  reset: (
    <g {...STROKE}>
      <path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3L4.5 8.8" />
      <path d="M4.5 4.3v4.5H9" />
    </g>
  ),
  /* Horizontal sliders: preferences / scan parameters. */
  sliders: (
    <g {...STROKE}>
      <path d="M4 6.5h8M16 6.5h4" />
      <circle cx="14" cy="6.5" r="2" />
      <path d="M4 12h2M10 12h10" />
      <circle cx="8" cy="12" r="2" />
      <path d="M4 17.5h10M18 17.5h2" />
      <circle cx="16" cy="17.5" r="2" />
    </g>
  ),
  /* Shield with check: preflight validation. */
  "shield-check": (
    <g {...STROKE}>
      <path d="M12 3.2 19 5.8v5.1c0 4.4-2.9 8.1-7 9.6-4.1-1.5-7-5.2-7-9.6V5.8z" />
      <path d="m9.2 11.6 2 2 3.6-4" />
    </g>
  ),
  /* Crosshair: homing / axis origin. */
  crosshair: (
    <g {...STROKE}>
      <circle cx="12" cy="12" r="6.5" />
      <path d="M12 2.8v3M12 18.2v3M2.8 12h3M18.2 12h3" />
      <circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none" />
    </g>
  ),
  /* Heartbeat trace: diagnostics. */
  pulse: (
    <g {...STROKE}>
      <path d="M3 12h4l2.5 7 4-14 2.5 7H21" />
    </g>
  ),
  /* Open book: user guide. */
  book: (
    <g {...STROKE}>
      <path d="M12 6.8C10.2 5.3 7.3 4.8 4 4.8v13.4c3.3 0 6.2.5 8 2 1.8-1.5 4.7-2 8-2V4.8c-3.3 0-6.2.5-8 2z" />
      <path d="M12 6.8v13.4" />
    </g>
  ),
  /* Warning triangle. */
  warning: (
    <g {...STROKE}>
      <path d="M12 4.2 3.2 19.4h17.6z" />
      <path d="M12 9.8v4.2" />
      <path d="M12 16.9h.01" />
    </g>
  ),
  info: (
    <g {...STROKE}>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 11v5" />
      <path d="M12 7.8h.01" />
    </g>
  ),
  question: (
    <g {...STROKE}>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M9.6 9.2a2.4 2.4 0 1 1 3.6 2.1c-.9.5-1.2 1-1.2 1.9" />
      <path d="M12 16.6h.01" />
    </g>
  ),
  play: (
    <g {...STROKE}>
      <path d="M8 5.4v13.2a.8.8 0 0 0 1.2.7l10.4-6.6a.8.8 0 0 0 0-1.4L9.2 4.7a.8.8 0 0 0-1.2.7z" />
    </g>
  ),
  pause: (
    <g {...STROKE}>
      <path d="M9 5.5v13" strokeWidth="2.2" />
      <path d="M15 5.5v13" strokeWidth="2.2" />
    </g>
  ),
  stop: (
    <g {...STROKE}>
      <rect x="7" y="7" width="10" height="10" rx="2" />
    </g>
  ),
  /* Ionising-radiation trefoil: X-ray source. */
  radiation: (
    <g {...STROKE}>
      <path d="M10 8.54 7.5 4.21a9 9 0 0 1 9 0L14 8.54a4 4 0 0 0-4 0z" />
      <path d="M14 12h7a9 9 0 0 1-4.5 7.79L14 15.46A4 4 0 0 0 14 12z" />
      <path d="M10 15.46 7.5 19.79A9 9 0 0 1 3 12h7a4 4 0 0 0 0 3.46z" />
      <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
    </g>
  ),
  /* Clockwise circular arrow: turntable rotation. */
  rotate: (
    <g {...STROKE}>
      <path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3l2.2 2.1" />
      <path d="M19.5 4.3v4.5H15" />
    </g>
  ),
  camera: (
    <g {...STROKE}>
      <path d="M4 8.5a1.5 1.5 0 0 1 1.5-1.5h2L9 4.5h6L16.5 7h2A1.5 1.5 0 0 1 20 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 17.5z" />
      <circle cx="12" cy="12.6" r="3.4" />
    </g>
  ),
  /* Chain link: device connection. */
  link: (
    <g {...STROKE}>
      <path d="M10 14.2a4.5 4.5 0 0 0 6.4 0l2.8-2.9a4.5 4.5 0 0 0-6.4-6.4l-1.6 1.6" />
      <path d="M14 9.8a4.5 4.5 0 0 0-6.4 0l-2.8 2.9a4.5 4.5 0 0 0 6.4 6.4l1.6-1.6" />
    </g>
  ),
  /* Gauge: operation status. */
  gauge: (
    <g {...STROKE}>
      <path d="M4.5 19a8.5 8.5 0 1 1 15 0" />
      <path d="m12 14.5 3.8-5.8" />
      <circle cx="12" cy="15" r="1.3" fill="currentColor" stroke="none" />
    </g>
  ),
  /* Terminal prompt: log channels. */
  terminal: (
    <g {...STROKE}>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
      <path d="m7 9 3.5 3L7 15" />
      <path d="M12.5 15H17" />
    </g>
  ),
  image: (
    <g {...STROKE}>
      <rect x="4" y="5" width="16" height="14" rx="2" />
      <circle cx="9.2" cy="9.6" r="1.4" />
      <path d="m4.5 16.5 4-4 3 3 3.5-3.5 4.5 4.5" />
    </g>
  ),
  /* Isometric cube: 3D equipment view. */
  cube: (
    <g {...STROKE}>
      <path d="M12 3.2 19.5 7.4v9.2L12 20.8l-7.5-4.2V7.4z" />
      <path d="M12 12 19.5 7.6M12 12v8.6M12 12 4.5 7.6" />
    </g>
  ),
  sun: (
    <g {...STROKE}>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2.8v2M12 19.2v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2.8 12h2M19.2 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
    </g>
  ),
  moon: (
    <g {...STROKE}>
      <path d="M19.8 13.6A8 8 0 0 1 10.4 4.2a8 8 0 1 0 9.4 9.4z" />
    </g>
  ),
  /* Paper plane: send a setpoint. */
  send: (
    <g {...STROKE}>
      <path d="M20 4 4 10.8l6.5 2.7L13.2 20z" />
      <path d="M20 4l-9.5 9.5" />
    </g>
  ),
};

export function LineIcon({
  name,
  size = 16,
  className,
}: {
  name: LineIconName;
  /** Rendered edge in px; CSS classes may still override it. */
  size?: number;
  className?: string;
}) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      focusable="false"
    >
      {PATHS[name]}
    </svg>
  );
}
