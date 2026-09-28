/**
 * Top-level workstation presentation and user-interaction composition.
 * The UI renders complete engine snapshots and translates gestures into domain
 * commands. It does not own production scan progress, device truth, or safety
 * decisions; those remain authoritative in ct-engine.
 */

import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { computeCanvasLayout, type CanvasLayout } from "./canvas-layout";
import { LineIcon, type LineIconName } from "./icons";
import { projectionError, exposureError, minutesToSeconds, secondsToMinutes } from "./scan-input";
import { hasDeviceSessionEverEstablished, useEngine } from "./engine/useEngine";
import {
  chooseImageDirectory,
  exportSessionLog,
  resolveDefaultImageDirectory,
  revealDirectory,
} from "./platform/desktopPaths";
import {
  DEFAULT_SCAN_SETUP,
  MENU_GROUPS,
  type MenuActionId,
  type MenuAvailability,
  type TopMenu,
} from "./menuActions";
import { LiveSceneCanvas } from "./scene/LiveSceneCanvas";
import { StaticSceneFallback } from "./scene/StaticSceneFallback";
import { useSceneFallback } from "./scene/useSceneFallback";
import type { ViewPreset } from "./scene/types";
import type {
  ConsoleLogLine,
  ConsoleFrame,
  DeviceId,
  EngineCommand,
  EngineSnapshot,
  WorkstationView,
} from "./engine/types";

type Theme = "light" | "dark";
type BottomTab = "aggregate" | "xray" | "nano" | "camera" | "images";

const THEME_KEY = "micro-ct-workstation.theme";

function useTheme(): [Theme, (theme: Theme) => void] {
  const [theme, setTheme] = useState<Theme>(() => {
    try {
      const saved = window.localStorage.getItem(THEME_KEY);
      if (saved === "light" || saved === "dark") return saved;
    } catch { /* use the operating-system preference */ }
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  });
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    const themeColor = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
    if (themeColor) themeColor.content = theme === "dark" ? "#111821" : "#E8EDF2";
  }, [theme]);
  useEffect(() => {
    const preference = window.matchMedia("(prefers-color-scheme: dark)");
    const syncFromOperatingSystem = (event: MediaQueryListEvent): void => {
      try {
        const saved = window.localStorage.getItem(THEME_KEY);
        if (saved === "light" || saved === "dark") return;
      } catch { /* a blocked store behaves as no manual override */ }
      setTheme(event.matches ? "dark" : "light");
    };
    preference.addEventListener("change", syncFromOperatingSystem);
    return () => preference.removeEventListener("change", syncFromOperatingSystem);
  }, []);
  const chooseTheme = useCallback((next: Theme): void => {
    try { window.localStorage.setItem(THEME_KEY, next); } catch { /* the active session still follows the selection */ }
    setTheme(next);
  }, []);
  return [theme, chooseTheme];
}

function logTime(timestamp: string): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return "--:--:--.---";
  const p = (value: number, size = 2) => String(value).padStart(size, "0");
  return `${p(date.getHours())}:${p(date.getMinutes())}:${p(date.getSeconds())}.${p(date.getMilliseconds(), 3)}`;
}

function HelpTip({ text, align = "start", side = "down" }: { text: string; align?: "start" | "end"; side?: "down" | "up" }) {
  const id = useId();
  const button = useRef<HTMLButtonElement>(null);
  const bubble = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(false);
  const [position, setPosition] = useState({ top: 0, left: 0 });
  const show = useCallback(() => setVisible(true), []);
  useLayoutEffect(() => {
    if (!visible) return;
    const rect = button.current?.getBoundingClientRect();
    const tip = bubble.current;
    if (!rect || !tip) return;
    const margin = 8;
    const width = tip.offsetWidth;
    const height = tip.offsetHeight;
    const preferredLeft = align === "end" ? rect.right + margin - width : rect.left - margin;
    const preferredTop = side === "up" ? rect.top - margin - height : rect.bottom + margin;
    setPosition({
      top: Math.max(margin, Math.min(preferredTop, window.innerHeight - height - margin)),
      left: Math.max(margin, Math.min(preferredLeft, window.innerWidth - width - margin)),
    });
  }, [visible, align, side, text]);
  return (
    <>
      <button
        ref={button}
        type="button"
        className="help-tip"
        aria-label={text}
        aria-describedby={visible ? id : undefined}
        onPointerEnter={show}
        onPointerLeave={() => setVisible(false)}
        onFocus={show}
        onBlur={() => setVisible(false)}
      >
        <LineIcon name="question" size={12} />
      </button>
      {visible && createPortal(
        <span
          ref={bubble}
          id={id}
          className="help-tip__bubble"
          role="tooltip"
          style={{ top: position.top, left: position.left }}
        >{text}</span>,
        document.body,
      )}
    </>
  );
}

const toneClass = (tone: string) => `tone-${tone}`;
const measured = (value: number | null, stale = false): string =>
  stale || value == null ? "—" : value.toFixed(1);

/* ------------------------------------------------------------------ */
/* Menu bar + floating drop-down submenu                               */
/* ------------------------------------------------------------------ */

/** Plain-text session log for `File → Export Session Log…`. */
function sessionLogText(ws: WorkstationView, adapterKind: "developer_preview" | "tauri"): string {
  const header = [
    "Micro-CT Workstation session log",
    `exported    : ${new Date().toISOString()}`,
    `engine      : ${adapterKind === "tauri" ? "ct-engine sidecar (JSONL stdio IPC)" : "browser developer preview"}`,
    `task        : ${ws.scanSetup.taskId || "(not configured)"}`,
    `save path   : ${ws.scanSetup.savePath || "(not configured)"}`,
    `acquisition : ${ws.summary.acquisition}`,
    `output      : ${ws.summary.output}`,
    `progress    : ${ws.progress.captured} / ${ws.progress.total} views · ${ws.progress.percent}%`,
    `phase       : ${ws.phaseWord}`,
    "",
  ].join("\n");
  const lines = ws.consoleLogs
    .map((entry) => `[${entry.timestamp}] ${entry.level} · ${entry.source} · ${entry.message}`)
    .join("\n");
  return `${header}${lines}\n`;
}

function MenuBar({
  availability,
  onAction,
}: {
  availability: MenuAvailability;
  onAction: (id: MenuActionId) => void;
}) {
  const [openMenu, setOpenMenu] = useState<TopMenu | null>(null);
  const [windowMaximized, setWindowMaximized] = useState(true);
  const [windowMaximizable, setWindowMaximizable] = useState(false);
  const barRef = useRef<HTMLElement | null>(null);
  const desktopWindow = useMemo(() => isTauri() ? getCurrentWindow() : null, []);

  useEffect(() => {
    if (!desktopWindow) return;
    let disposed = false;
    let stopResize: (() => void) | undefined;
    const syncWindowState = async (): Promise<void> => {
      try {
        const [maximized, maximizable] = await Promise.all([
          desktopWindow.isMaximized(),
          desktopWindow.isMaximizable(),
        ]);
        if (!disposed) {
          setWindowMaximized(maximized);
          setWindowMaximizable(maximizable);
        }
      } catch { /* the chrome remains visible if the host is shutting down */ }
    };
    void syncWindowState();
    void desktopWindow.onResized(() => void syncWindowState()).then((stop) => {
      if (disposed) stop();
      else stopResize = stop;
    });
    return () => {
      disposed = true;
      stopResize?.();
    };
  }, [desktopWindow]);

  useEffect(() => {
    if (!openMenu) return;
    const onPointerDown = (event: MouseEvent): void => {
      if (!barRef.current?.contains(event.target as Node)) setOpenMenu(null);
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") setOpenMenu(null);
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [openMenu]);

  return (
    <header
      className="menu-bar"
      ref={barRef}
      onDoubleClick={(event) => {
        if ((event.target as HTMLElement).closest("button, nav, .window-controls")) return;
        if (desktopWindow && windowMaximizable) void desktopWindow.toggleMaximize().catch(() => undefined);
      }}
    >
      <div className="workstation-brand" data-tauri-drag-region>
        <img src="/assets/micro-ct-logo.png" alt="" draggable={false} />
        <strong>Micro-CT Workstation</strong>
        <span>v0.7.8</span>
      </div>
      <nav className="sys-menu" aria-label="Application menu">
        {MENU_GROUPS.map((group) => {
          const open = openMenu === group.label;
          return (
            <span className="menu-group" key={group.label}>
              <button
                type="button"
                className={`sys-menu__item ${open ? "is-open" : ""}`}
                aria-haspopup="menu"
                aria-expanded={open}
                onClick={() => setOpenMenu(open ? null : group.label)}
                onMouseEnter={() => {
                  if (openMenu) setOpenMenu(group.label);
                }}
              >
                {group.label}
              </button>
              {open ? (
                <div className="menu-dropdown" role="menu" aria-label={`${group.label} menu`}>
                  {group.entries.map((entry) => {
                    const state = availability[entry.id];
                    return (
                      <button
                        key={entry.id}
                        type="button"
                        role="menuitem"
                        className="menu-dropdown__item"
                        disabled={!state.available}
                        aria-disabled={!state.available}
                        title={state.reason ?? entry.label}
                        onClick={() => {
                          setOpenMenu(null);
                          onAction(entry.id);
                        }}
                      >
                        <LineIcon name={entry.icon} size={13} className="menu-dropdown__icon" />
                        <span>{entry.label}</span>
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </span>
          );
        })}
      </nav>
      <span className="title-drag-space" data-tauri-drag-region aria-hidden="true" />
      <div className="window-controls" aria-label="Window controls">
        <button type="button" aria-label="Minimise window" title="Minimise" onClick={() => void desktopWindow?.minimize().catch(() => undefined)}>
          <span className="window-glyph window-glyph--minimise" aria-hidden="true" />
        </button>
        <button
          type="button"
          aria-label={windowMaximized ? "Restore window" : "Maximise window"}
          title={windowMaximizable ? (windowMaximized ? "Restore" : "Maximise") : "Window remains maximised at this display size"}
          disabled={!windowMaximizable}
          onClick={() => void desktopWindow?.toggleMaximize().catch(() => undefined)}
        >
          <span className={`window-glyph ${windowMaximized ? "window-glyph--restore" : "window-glyph--maximise"}`} aria-hidden="true" />
        </button>
        <button type="button" className="window-controls__close" aria-label="Close window" title="Close" onClick={() => void desktopWindow?.close().catch(() => undefined)}>
          <span className="window-glyph window-glyph--close" aria-hidden="true" />
        </button>
      </div>
    </header>
  );
}

/* ------------------------------------------------------------------ */
/* Left column                                                         */
/* ------------------------------------------------------------------ */

function DevicePanel({
  ws,
  busy,
  hardwareConnected,
  desktopRuntime,
  dispatch,
  stale,
  setupInvalid,
}: {
  ws: WorkstationView;
  busy: boolean;
  hardwareConnected: boolean;
  desktopRuntime: boolean;
  dispatch: (c: EngineCommand) => Promise<void>;
  stale: boolean;
  setupInvalid: boolean;
}) {
  const [pendingDevice, setPendingDevice] = useState<DeviceId | null>(null);
  const setup = ws.scanSetup;
  const setupComplete =
    setup.taskId.trim().length > 0 &&
    setup.savePath.trim().length > 0 &&
    Number.isInteger(setup.projectionCount) &&
    setup.projectionCount >= 1 &&
    setup.projectionCount <= 3600 &&
    Number.isFinite(setup.exposureMs) &&
    setup.exposureMs >= ws.cameraExposure.minMs &&
    setup.exposureMs <= ws.cameraExposure.maxMs &&
    Number.isInteger(setup.maxXraySec) &&
    setup.maxXraySec >= 1 &&
    setup.maxXraySec <= 600 && !setupInvalid;
  const runRetry = async (device: DeviceId): Promise<void> => {
    setPendingDevice(device);
    try {
      if (device === "turntable" && !hardwareConnected) {
        await dispatch({ type: "connect", adapter: "real_hardware" });
      } else {
        await dispatch({ type: "retry_device", device });
      }
    } finally {
      setPendingDevice(null);
    }
  };

  return (
    <section className="panel device-panel">
      <div className="panel__header">
        <LineIcon name="link" size={15} className="panel__icon" />
        <h2>Device Connection Status</h2>
        <HelpTip text={`Preflight: ${ws.preflight.subline}`} />
      </div>
      <div className="device-list">
        {ws.devices.map((device) => {
          const retrying = pendingDevice === device.id;
          const deviceIcon: LineIconName =
            device.id === "xray" ? "radiation" : device.id === "camera" ? "camera" : "rotate";
          return (
            <div className="device-row" key={device.id}>
              <div className="device-row__top">
                <LineIcon name={deviceIcon} size={15} className="device-row__icon" />
                <strong>{device.name}</strong>
                <HelpTip text={stale ? "Device status unavailable" : device.spec} />
                <span className={`device-row__word ${toneClass(stale ? "muted" : device.tone)}`}>{stale ? "UNKNOWN" : device.word}</span>
                <button
                  type="button"
                  className="retry-btn"
                  disabled={busy || pendingDevice !== null}
                  aria-busy={retrying}
                  onClick={() => void runRetry(device.id as DeviceId)}
                >
                  {retrying ? "Connecting…" : (device.id === "turntable" && !hardwareConnected) || device.id === "camera" || device.id === "xray" ? "Connect" : "Retry"}
                </button>
                {device.id === "camera" && device.word === "ONLINE" && desktopRuntime ? (
                  <button
                    type="button"
                    className="retry-btn"
                    disabled={busy || pendingDevice !== null || !setup.savePath.trim() || !setup.taskId.trim()}
                    title="Capture one dark test frame into Save Path / Task ID / frames"
                    onClick={() => void dispatch({ type: "camera_test_capture" })}
                  >
                    Test capture
                  </button>
                ) : null}
              </div>
            </div>
          );
        })}
      </div>
      <div className="preflight-block">
        <div className="preflight-block__top">
          <button
            type="button"
            className="preflight-btn"
            disabled={!setupComplete || busy || ws.dataState === "scanning" || ws.dataState === "paused"}
            aria-busy={ws.preflight.tone === "running"}
            onClick={() => void dispatch({ type: "preflight" })}
          >
            {ws.preflight.tone === "running" ? "Inspecting…" : "Run Preflight"}
          </button>
          <strong className={ws.preflight.tone === "pass" ? "tone-ok" : ws.preflight.tone === "fail" ? "tone-danger" : "tone-warn"}>
            {ws.preflight.word}
          </strong>
        </div>
        <div className={`preflight preflight--${ws.preflight.tone === "running" ? "pass" : ws.preflight.tone}`} role="progressbar" aria-valuenow={ws.preflight.percent} aria-valuemin={0} aria-valuemax={100}>
          <span className="preflight__fill" style={{ width: `${ws.preflight.percent}%` }} />
        </div>
      </div>
      <div className="device-online-summary">
        <span>LINK SUMMARY</span>
        <strong>{ws.onlineSummary}</strong>
      </div>
    </section>
  );
}

function ScanParamsPanel({
  ws,
  busy,
  desktopRuntime,
  dispatch,
  syncRevision,
  onInvalidChange,
  onValidationError,
}: {
  ws: WorkstationView;
  busy: boolean;
  desktopRuntime: boolean;
  dispatch: (c: EngineCommand) => Promise<void>;
  /** Bumped when a menu command changed the setup, so drafts refill from the engine. */
  syncRevision: number;
  onInvalidChange: (invalid: boolean) => void;
  onValidationError: (message: string | null) => void;
}) {
  const setup = ws.scanSetup;
  const locked = busy || ws.dataState === "scanning" || ws.dataState === "paused";
  const defaultPathRequested = useRef(false);
  const taskIdRef = useRef<HTMLInputElement | null>(null);
  const [savePath, setSavePath] = useState("");
  const [taskId, setTaskId] = useState("");
  const [views, setViews] = useState("");
  const [exposure, setExposure] = useState("");
  const [maxXray, setMaxXray] = useState(() => secondsToMinutes(setup.maxXraySec || 600));
  const [pathError, setPathError] = useState<string | null>(null);

  useEffect(() => {
    if (!desktopRuntime || defaultPathRequested.current || savePath.trim()) return;
    defaultPathRequested.current = true;
    void resolveDefaultImageDirectory()
      .then((path) => {
        setSavePath(path);
        setPathError(null);
        return dispatch({ type: "update_scan_setup", setup: { savePath: path } });
      })
      .catch((reason: unknown) => {
        setSavePath("");
        setPathError(reason instanceof Error ? reason.message : String(reason));
      });
  }, [desktopRuntime, dispatch, savePath]);

  const commitText = (field: "savePath" | "taskId", value: string): void => {
    const trimmed = value.trim();
    if (trimmed) void dispatch({ type: "update_scan_setup", setup: { [field]: trimmed } });
  };

  const commitNumber = (field: "projectionCount" | "exposureMs", value: string): void => {
    const invalid = field === "projectionCount" ? projectionError(value)
      : exposureError(value, ws.cameraExposure.minMs, ws.cameraExposure.maxMs);
    if (invalid) return;
    const parsed = Number(value);
    void dispatch({ type: "update_scan_setup", setup: { [field]: parsed } });
  };

  const commitDuration = (): void => {
    const seconds = minutesToSeconds(maxXray);
    if (seconds !== null && seconds >= 1 && seconds <= 600) {
      void dispatch({ type: "update_scan_setup", setup: { maxXraySec: seconds } });
    }
  };

  const chooseDirectory = async (): Promise<void> => {
    try {
      const selected = await chooseImageDirectory(savePath);
      if (!selected) return;
      setSavePath(selected);
      setPathError(null);
      await dispatch({ type: "update_scan_setup", setup: { savePath: selected } });
    } catch (reason: unknown) {
      setPathError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const viewsIssue = views === "" ? null : projectionError(views);
  const exposureIssue = exposure === "" ? null : exposureError(exposure, ws.cameraExposure.minMs, ws.cameraExposure.maxMs);
  const durationSeconds = minutesToSeconds(maxXray);
  const durationIssue = maxXray !== "" && durationSeconds === null ? "Max X-ray time must be 0–10 min, positive and in whole-second increments." : null;
  const viewsInvalid = Boolean(viewsIssue);
  const exposureInvalid = Boolean(exposureIssue);
  const durationInvalid = Boolean(durationIssue);
  const invalidDraft = !taskId.trim() || !savePath.trim() || !views || !exposure || !maxXray || viewsInvalid || exposureInvalid || durationInvalid;
  useEffect(() => { onInvalidChange(invalidDraft); }, [invalidDraft, onInvalidChange]);
  useEffect(() => {
    onValidationError([viewsIssue, exposureIssue, durationIssue].filter(Boolean).join(" ") || null);
  }, [viewsIssue, exposureIssue, durationIssue, onValidationError]);

  // New Scan Task / Reset Parameters go through the engine, so the drafts are
  // refilled from the engine snapshot once the next poll lands.
  useEffect(() => {
    if (syncRevision === 0) return;
    setTaskId(setup.taskId);
    setSavePath(setup.savePath);
    setViews(setup.projectionCount > 0 ? String(setup.projectionCount) : "");
    setExposure(setup.exposureMs > 0 ? String(setup.exposureMs) : "");
    setMaxXray(secondsToMinutes(setup.maxXraySec || 600));
    taskIdRef.current?.focus();
  }, [syncRevision]);

  return (
    <section className="panel scan-panel">
      <div className="panel__header">
        <LineIcon name="sliders" size={15} className="panel__icon" />
        <h2>Scan Parameters</h2>
        <span className="chip chip--accent">
          {setup.projectionCount > 0 ? `${setup.projectionCount} PROJECTIONS` : "NOT CONFIGURED"}
        </span>
      </div>
      <div className="param-stack">
        <label className="param-field param-field--full">
          <span>Task ID</span>
          <input
            ref={taskIdRef}
            placeholder="Enter a task name"
            value={taskId}
            disabled={locked}
            onChange={(event) => setTaskId(event.target.value)}
            onBlur={() => commitText("taskId", taskId)}
          />
        </label>
        <label className="param-field param-field--full">
          <span>Save Path</span>
          <div className={`path-control ${pathError ? "is-invalid" : ""}`}>
            <input
              value={savePath}
              title={savePath}
              disabled={locked}
              aria-invalid={Boolean(pathError)}
              placeholder={desktopRuntime ? "Windows image directory unavailable" : "Unavailable in developer preview"}
              onChange={(event) => {
                setSavePath(event.target.value);
                setPathError(null);
              }}
              onBlur={() => commitText("savePath", savePath)}
            />
            <button
              type="button"
              className="folder-btn"
              disabled={locked || !desktopRuntime}
              title={desktopRuntime ? "Select image directory" : "Native directory selection is unavailable in developer preview"}
              aria-label="Select image directory"
              onClick={() => void chooseDirectory()}
            >
              <LineIcon name="folder" size={15} />
            </button>
          </div>
          {pathError ? <small className="field-error">{pathError}</small> : null}
        </label>
        <div className="scan-input-grid">
          <label className={`numeric-field ${viewsInvalid ? "is-invalid" : ""}`}>
            <span>Total projections <HelpTip align="end" text="Any positive integer up to 3600. The engine calculates the angular step." /></span>
            <input value={views} disabled={locked} inputMode="numeric" aria-label="Total projections" aria-invalid={viewsInvalid} onChange={(event) => setViews(event.target.value)} onBlur={() => commitNumber("projectionCount", views)} />
          </label>
          <label className={`numeric-field ${exposureInvalid ? "is-invalid" : ""}`}>
            <span>Exposure · ms <HelpTip align="end" text={`${ws.cameraExposure.known ? "Connected camera" : "Nikon D7100 timed shutter"}: ${ws.cameraExposure.minMs}–${ws.cameraExposure.maxMs} ms. The value must match a supported camera shutter setting; unsupported values are rejected.`} /></span>
            <input value={exposure} disabled={locked} inputMode="decimal" aria-label="Exposure in milliseconds" aria-invalid={exposureInvalid} onChange={(event) => setExposure(event.target.value)} onBlur={() => commitNumber("exposureMs", exposure)} />
          </label>
          <label className={`numeric-field ${durationInvalid ? "is-invalid" : ""}`}>
            <span>Max X-ray · min <HelpTip align="end" text="Default 10 min. Maximum continuous output is limited to 10 min, followed by 5 min cooldown. Fractional minutes such as 0.5 are allowed." /></span>
            <input value={maxXray} disabled={locked} inputMode="decimal" aria-label="Maximum X-ray time in minutes" aria-invalid={durationInvalid} onChange={(event) => setMaxXray(event.target.value)} onBlur={commitDuration} />
          </label>
        </div>
      </div>
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* Center: live scene                                                  */
/* ------------------------------------------------------------------ */

function DockKey({
  variant,
  icon,
  label,
  title,
  disabled,
  onClick,
}: {
  variant: string;
  icon: LineIconName;
  label: string;
  title: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`dock-key dock-key--${variant}`}
      aria-label={label}
      title={title}
      disabled={disabled}
      onClick={onClick}
    >
      <LineIcon name={icon} size={28} className="dock-icon" />
      <span className="dock-key__label">{label}</span>
    </button>
  );
}

/* ------------------------------------------------------------------ */
/* Console feedback: the eight states the operator must be able to     */
/* read off the console. Every field is derived in React from the       */
/* existing EngineSnapshot; no IPC field, DTO or device command is      */
/* added, and the engine stays the only source of production truth.     */
/* ------------------------------------------------------------------ */

/** The eight console feedback states. Each one renders its own name. */
const CONSOLE_FEEDBACK_STATES = [
  "Running",
  "Paused",
  "Cooling",
  "Finishing",
  "Stopping",
  "Stopped",
  "Completed",
  "Fault",
] as const;

type ConsoleStateName = (typeof CONSOLE_FEEDBACK_STATES)[number];
type ConsoleStateTone = "accent" | "warn" | "danger" | "muted" | "ok";

interface ConsoleFeedback {
  state: ConsoleStateName;
  tone: ConsoleStateTone;
  /** One-line reading. Never claims more than the snapshot proves. */
  detail: string;
  /** True while the scan itself is still in flight. */
  active: boolean;
}

/**
 * Evidence checks combine snapshot/workstation fields with the device-session
 * result supplied by useEngine. Exported so acceptance tests can drive the real
 * derivation instead of a copy of it.
 */
export interface FeedbackEvidence {
  linkLost: boolean;
  phaseFault: boolean;
  beamUnconfirmed: boolean;
  /** A scan is running or has run, so a cooldown could legitimately be in play. */
  scanAttempted: boolean;
}

/**
 * One-line reading for the benign startup state. It states plainly that nothing
 * is connected and that every device and output reading is unavailable, so the
 * neutral banner can never be read as a claim that the machine is ready.
 */
function offlineFeedback(): ConsoleFeedback {
  return {
    state: "Stopped",
    tone: "muted",
    detail: "Not connected · no device link has been established yet · device and output states are unavailable and no scan has run",
    active: false,
  };
}

/**
 * These key sets mirror every object layer in EngineSnapshot and WorkstationView.
 * Requiring exact own-key coverage means a new contract field starts closed
 * until this first-start projection classifies it. Existing fields are checked
 * below by their value, type, or relationship to the other projection.
 */
const FIRST_START_SNAPSHOT_KEYS = [
  "mode", "modeLabel", "connectionState", "adapterLabel", "phase", "phaseLabel",
  "preflightPassed", "homed", "requiresPreflight", "requiresHome", "safety", "devices",
  "parameters", "progress", "imageCount", "logs", "lastError", "updatedAt",
];
const FIRST_START_SAFETY_KEYS = ["xrayAvailable", "xrayEnabled", "interlockOk", "lockReason"];
const FIRST_START_DEVICE_KEYS = ["id", "label", "state", "detail"];
const FIRST_START_PARAMETER_KEYS = ["taskId", "savePath", "projectionCount", "angleStepDeg", "exposureMs"];
const FIRST_START_PROGRESS_KEYS = ["current", "total", "percent", "angleDeg", "etaSeconds"];
const FIRST_START_LOG_KEYS = ["id", "timestamp", "level", "source", "message"];
const FIRST_START_WORKSTATION_KEYS = [
  "cameraExposure", "dataState", "phaseWord", "phaseTone", "devices", "onlineSummary", "preflight",
  "floats", "safetyBar", "scene", "xray", "progress", "summary", "statusbar", "dock", "scanSetup",
  "consoleLogs", "frames", "checkpointAvailable",
];
const FIRST_START_CONSOLE_DEVICE_KEYS = ["id", "name", "word", "tone", "spec"];
const FIRST_START_FLOAT_KEYS = ["key", "text", "tone"];
const FIRST_START_XRAY_KEYS = [
  "connected", "setKv", "setUa", "monKv", "monUa", "powerW", "tempC", "beamState", "beamOn", "latched",
  "onSec", "offSec", "timerOn", "usbAutoShutDown", "usbAutoShutDownKnown", "usbShutdownDelay",
  "manualControlsEnabled", "timerControlsEnabled", "setpointControlsEnabled", "voltageConfirmed",
  "currentConfirmed", "setpointConfirmed",
];
const FIRST_START_CONSOLE_PROGRESS_KEYS = ["captured", "total", "percent", "angleDeg", "etaText", "barTone", "barLabel"];
const FIRST_START_FRAME_KEYS = ["index", "angleDeg", "exposureMs", "fileName"];
const FIRST_START_DEVICE_IDS = ["turntable", "camera", "xray"] as const;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactOwnKeys(value: unknown, expectedKeys: readonly string[]): value is Record<string, unknown> {
  if (!isRecord(value)) return false;
  const actualKeys = Reflect.ownKeys(value);
  return actualKeys.length === expectedKeys.length && actualKeys.every((key) =>
    typeof key === "string" && expectedKeys.includes(key),
  );
}

function hasExactArrayEntries(value: unknown): value is unknown[] {
  if (!Array.isArray(value)) return false;
  const ownKeys = Reflect.ownKeys(value).filter((key) => key !== "length");
  return ownKeys.length === value.length && ownKeys.every((key, index) =>
    typeof key === "string" && key === String(index),
  );
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function hasDeviceIds(devices: unknown, expectedIds: readonly string[]): devices is Record<string, unknown>[] {
  if (!hasExactArrayEntries(devices) || devices.length !== expectedIds.length) return false;
  const ids = devices.map((device) => isRecord(device) ? device.id : undefined);
  return expectedIds.every((id) => ids.filter((candidate) => candidate === id).length === 1);
}

function sameStructuredValue(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left) && Array.isArray(right) &&
      hasExactArrayEntries(left) && hasExactArrayEntries(right) &&
      left.length === right.length && left.every((value, index) => sameStructuredValue(value, right[index]));
  }
  if (!isRecord(left) || !isRecord(right)) return false;
  const leftKeys = Reflect.ownKeys(left);
  const rightKeys = Reflect.ownKeys(right);
  return leftKeys.length === rightKeys.length && leftKeys.every((key) =>
    rightKeys.includes(key) && typeof key === "string" && sameStructuredValue(left[key], right[key]),
  );
}

function hasCompleteFirstStartLogs(logs: unknown): boolean {
  if (!hasExactArrayEntries(logs)) return false;
  return logs.every((entry) =>
    hasExactOwnKeys(entry, FIRST_START_LOG_KEYS) &&
    isString(entry.id) && entry.id.length > 0 &&
    isString(entry.timestamp) &&
    ["INFO", "WARN"].includes(String(entry.level)) &&
    ["system", "xray", "nano", "camera", "preflight", "operator"].includes(String(entry.source)) &&
    isString(entry.message),
  );
}

function hasCompleteFirstStartConsoleLogs(logs: unknown): boolean {
  // Rust projects the same production log records into both snapshot logs and
  // workstation console logs. Keep one format contract so they cannot drift.
  return hasCompleteFirstStartLogs(logs);
}

function hasCompleteFirstStartSnapshot(snapshot: EngineSnapshot): boolean {
  const hasWorkstation = Object.prototype.hasOwnProperty.call(snapshot, "workstation");
  const rootKeys = hasWorkstation ? [...FIRST_START_SNAPSHOT_KEYS, "workstation"] : FIRST_START_SNAPSHOT_KEYS;
  if (!hasExactOwnKeys(snapshot, rootKeys)) return false;
  if (!hasExactOwnKeys(snapshot.safety, FIRST_START_SAFETY_KEYS)) return false;
  if (![snapshot.safety.xrayAvailable, snapshot.safety.xrayEnabled, snapshot.safety.interlockOk].every((value) => typeof value === "boolean") ||
    !isString(snapshot.safety.lockReason)) return false;
  if (!hasExactOwnKeys(snapshot.parameters, FIRST_START_PARAMETER_KEYS)) return false;
  if (!hasExactOwnKeys(snapshot.progress, FIRST_START_PROGRESS_KEYS)) return false;
  if (!hasExactArrayEntries(snapshot.devices) || !hasDeviceIds(snapshot.devices, FIRST_START_DEVICE_IDS)) return false;
  if (!snapshot.devices.every((device) =>
    hasExactOwnKeys(device, FIRST_START_DEVICE_KEYS) &&
    isString(device.label) && isString(device.state) && isString(device.detail),
  )) return false;
  if (!hasCompleteFirstStartLogs(snapshot.logs)) return false;
  if (!isString(snapshot.modeLabel) || !isString(snapshot.adapterLabel) || !isString(snapshot.phaseLabel) || !isString(snapshot.updatedAt)) return false;
  if (!isString(snapshot.parameters.taskId) || !isString(snapshot.parameters.savePath)) return false;
  if (![snapshot.parameters.projectionCount, snapshot.parameters.angleStepDeg, snapshot.parameters.exposureMs].every(isFiniteNumber)) return false;
  if (![snapshot.progress.current, snapshot.progress.total, snapshot.progress.percent, snapshot.progress.angleDeg].every(isFiniteNumber)) return false;
  if (snapshot.progress.etaSeconds !== null && !isFiniteNumber(snapshot.progress.etaSeconds)) return false;
  if (!isFiniteNumber(snapshot.imageCount) || !Number.isInteger(snapshot.imageCount)) return false;
  if (snapshot.lastError !== null && !isString(snapshot.lastError)) return false;
  if (hasWorkstation && (snapshot.workstation === undefined || !hasCompleteFirstStartWorkstation(snapshot.workstation))) return false;
  return true;
}

function hasCompleteFirstStartWorkstation(ws: WorkstationView): boolean {
  if (!hasExactOwnKeys(ws, FIRST_START_WORKSTATION_KEYS)) return false;
  if (!hasExactOwnKeys(ws.cameraExposure, ["minMs", "maxMs", "known"])) return false;
  if (!hasDeviceIds(ws.devices, FIRST_START_DEVICE_IDS)) return false;
  if (!ws.devices.every((device) =>
    hasExactOwnKeys(device, FIRST_START_CONSOLE_DEVICE_KEYS) &&
    isString(device.name) && isString(device.word) && isString(device.tone) && isString(device.spec),
  )) return false;
  if (!hasExactOwnKeys(ws.preflight, ["word", "percent", "tone", "subline"])) return false;
  if (!isString(ws.preflight.word) || !isFiniteNumber(ws.preflight.percent) || !isString(ws.preflight.tone) || !isString(ws.preflight.subline)) return false;
  if (!hasExactArrayEntries(ws.floats) || ws.floats.length !== 3) return false;
  if (!ws.floats.every((item) =>
    hasExactOwnKeys(item, FIRST_START_FLOAT_KEYS) && isString(item.key) && isString(item.text) && isString(item.tone),
  )) return false;
  if (!hasExactOwnKeys(ws.safetyBar, ["text", "tone"]) || !isString(ws.safetyBar.text) || !isString(ws.safetyBar.tone)) return false;
  if (!hasExactOwnKeys(ws.scene, ["angleDeg", "rotated", "angleKnown", "rotationDirection"])) return false;
  if (!hasExactOwnKeys(ws.xray, FIRST_START_XRAY_KEYS)) return false;
  if (![ws.xray.setKv, ws.xray.setUa, ws.xray.onSec, ws.xray.offSec].every(isFiniteNumber)) return false;
  if (![ws.xray.connected, ws.xray.beamOn, ws.xray.latched, ws.xray.timerOn, ws.xray.usbAutoShutDown,
    ws.xray.usbAutoShutDownKnown, ws.xray.manualControlsEnabled, ws.xray.timerControlsEnabled,
    ws.xray.setpointControlsEnabled, ws.xray.voltageConfirmed, ws.xray.currentConfirmed,
    ws.xray.setpointConfirmed].every((value) => typeof value === "boolean")) return false;
  if (![ws.xray.monKv, ws.xray.monUa, ws.xray.powerW, ws.xray.tempC, ws.xray.usbShutdownDelay].every((value) => value === null || isFiniteNumber(value))) return false;
  if (!hasExactOwnKeys(ws.progress, FIRST_START_CONSOLE_PROGRESS_KEYS)) return false;
  if (![ws.progress.captured, ws.progress.total, ws.progress.percent, ws.progress.angleDeg].every(isFiniteNumber)) return false;
  if (!isString(ws.progress.etaText) || !isString(ws.progress.barTone) || !isString(ws.progress.barLabel)) return false;
  if (!hasExactOwnKeys(ws.summary, ["savePath", "acquisition", "output"]) ||
    ![ws.summary.savePath, ws.summary.acquisition, ws.summary.output].every(isString)) return false;
  if (!hasExactOwnKeys(ws.statusbar, ["left", "right", "dotTone"]) ||
    ![ws.statusbar.left, ws.statusbar.right, ws.statusbar.dotTone].every(isString)) return false;
  if (!hasExactOwnKeys(ws.dock, ["home", "play", "restore", "stop", "playMode", "homeReason", "restoreReason", "playReason"])) return false;
  if (![ws.dock.home, ws.dock.play, ws.dock.restore, ws.dock.stop].every((value) => typeof value === "boolean")) return false;
  if (!isString(ws.dock.playMode) || !isString(ws.dock.homeReason) || !isString(ws.dock.restoreReason) || !isString(ws.dock.playReason)) return false;
  if (!hasExactOwnKeys(ws.scanSetup, ["savePath", "taskId", "projectionCount", "angleStepDeg", "exposureMs", "maxXraySec"])) return false;
  if (![ws.scanSetup.savePath, ws.scanSetup.taskId].every(isString) ||
    ![ws.scanSetup.projectionCount, ws.scanSetup.angleStepDeg, ws.scanSetup.exposureMs, ws.scanSetup.maxXraySec].every(isFiniteNumber)) return false;
  if (!hasCompleteFirstStartConsoleLogs(ws.consoleLogs)) return false;
  if (!hasExactArrayEntries(ws.frames) || !ws.frames.every((frame) =>
    hasExactOwnKeys(frame, FIRST_START_FRAME_KEYS) &&
    [frame.index, frame.angleDeg, frame.exposureMs].every(isFiniteNumber) && isString(frame.fileName),
  )) return false;
  if (typeof ws.cameraExposure.known !== "boolean" || !isFiniteNumber(ws.cameraExposure.minMs) || !isFiniteNumber(ws.cameraExposure.maxMs)) return false;
  if (!isString(ws.dataState) || !isString(ws.phaseWord) || !isString(ws.phaseTone) || !isString(ws.onlineSummary)) return false;
  if (typeof ws.scene.rotated !== "boolean" || typeof ws.scene.angleKnown !== "boolean" ||
    !isFiniteNumber(ws.scene.angleDeg) || ![-1, 1].includes(ws.scene.rotationDirection)) return false;
  return typeof ws.checkpointAvailable === "boolean";
}

function hasConsistentFirstStartProjection(snapshot: EngineSnapshot, ws: WorkstationView): boolean {
  if (!hasCompleteFirstStartSnapshot(snapshot) || !hasCompleteFirstStartWorkstation(ws)) return false;
  if (snapshot.workstation !== undefined && !sameStructuredValue(snapshot.workstation, ws)) return false;

  if (snapshot.mode !== "production_locked" || snapshot.connectionState !== "disconnected" || snapshot.phase !== "idle") return false;
  if (!/locked|disconnected/i.test(snapshot.modeLabel) || !/idle/i.test(snapshot.phaseLabel)) return false;
  if (snapshot.preflightPassed !== false || snapshot.homed !== false ||
    snapshot.requiresPreflight !== true || snapshot.requiresHome !== true) return false;
  if (snapshot.safety.xrayAvailable !== false || snapshot.safety.xrayEnabled !== false || snapshot.safety.interlockOk !== false) return false;
  if (snapshot.lastError !== null || !hasCompleteFirstStartLogs(snapshot.logs)) return false;
  if (snapshot.devices.some((device) =>
    device.id === "xray"
      ? device.state !== "offline" && device.state !== "locked"
      : device.state !== "offline",
  )) return false;

  const snapshotDevices = new Map(snapshot.devices.map((device) => [device.id, device]));
  const viewDevices = new Map(ws.devices.map((device) => [device.id, device]));
  if (ws.xray.connected !== false || ws.xray.latched !== false || ws.xray.beamOn !== false) return false;
  if (ws.xray.beamState !== "off" && ws.xray.beamState !== "unknown") return false;
  if (snapshot.safety.xrayEnabled !== ws.xray.beamOn) return false;
  if (ws.xray.beamOn) return false;
  if (ws.xray.setpointConfirmed || ws.xray.voltageConfirmed || ws.xray.currentConfirmed) return false;
  if (ws.xray.monKv !== null || ws.xray.monUa !== null || ws.xray.powerW !== null || ws.xray.tempC !== null) return false;
  if (ws.xray.usbAutoShutDownKnown || ws.xray.usbShutdownDelay !== null || ws.xray.timerOn ||
    ws.xray.manualControlsEnabled || ws.xray.timerControlsEnabled || ws.xray.setpointControlsEnabled) return false;
  for (const id of FIRST_START_DEVICE_IDS) {
    const snapshotDevice = snapshotDevices.get(id);
    const viewDevice = viewDevices.get(id);
    if (!snapshotDevice || !viewDevice || viewDevice.tone !== "muted") return false;
    if (/\b(?:online|connected|ready|moving|emitting|fault|failed|busy|scanning|running)\b/i.test(viewDevice.word)) return false;
    if (/\b(?:online|connected|ready|moving|emitting|fault|failed|busy|scanning|running)\b|\bbeam on\b/i.test(viewDevice.spec)) return false;
    if (id !== "xray" && !/offline|disconnected/i.test(viewDevice.word)) return false;
    if (id === "xray") {
      const expectedWords = ws.xray.beamState === "unknown" ? /unknown|offline/i : /off|offline/i;
      if (!expectedWords.test(viewDevice.word)) return false;
    }
  }

  if (ws.dataState !== "ready" || ws.phaseTone !== "accent" || !/^(idle|ready)$/i.test(ws.phaseWord)) return false;
  if (ws.preflight.tone !== "warn" || ws.preflight.percent !== 0 || !/^waiting$/i.test(ws.preflight.word)) return false;
  if (/passed|complete|fault|running/i.test(ws.preflight.subline)) return false;
  if (ws.dock.home || ws.dock.play || ws.dock.restore || ws.dock.stop || ws.dock.playMode !== "start") return false;
  if (!/\b0 real devices\b/i.test(ws.onlineSummary) || !/locked|disconnected/i.test(ws.statusbar.right)) return false;
  if (ws.statusbar.dotTone !== "muted" || /fault|scanning|running|paused|finishing|stopping/i.test(ws.statusbar.left)) return false;
  if (/fault|scanning|running|paused|finishing|stopping|capturing|captured/i.test(ws.summary.acquisition)) return false;

  const xrayFloat = ws.floats.find((item) => item.key === "X-RAY");
  const cameraFloat = ws.floats.find((item) => item.key === "CAMERA");
  const sampleFloat = ws.floats.find((item) => item.key === "SAMPLE");
  if (!xrayFloat || !cameraFloat || !sampleFloat) return false;
  if (new Set(ws.floats.map((item) => item.key)).size !== 3) return false;
  if (xrayFloat.tone !== "muted" || cameraFloat.tone !== "muted" || sampleFloat.tone !== "muted") return false;
  if (ws.xray.beamState === "unknown" ? !/unknown|readback|offline/i.test(xrayFloat.text) : !/off|disabled|offline/i.test(xrayFloat.text)) return false;
  if (!/offline|disconnected|standby|no real camera/i.test(cameraFloat.text) || !/unknown/i.test(sampleFloat.text)) return false;

  if (ws.safetyBar.tone !== "muted" || /\b(on|enabled|active|fault|latched|trip)\b/i.test(ws.safetyBar.text)) return false;
  if (ws.xray.beamState === "unknown" ? !/unknown|readback|unavailable/i.test(ws.safetyBar.text) : !/off|disabled|unavailable/i.test(ws.safetyBar.text)) return false;
  if (ws.summary.output.length === 0 || /\b(on|enabled|active|fault|latched|trip)\b/i.test(ws.summary.output)) return false;
  if (ws.xray.beamState === "unknown" ? !/unknown|readback|unavailable|offline/i.test(ws.summary.output) : !/off|disabled|unavailable|offline/i.test(ws.summary.output)) return false;

  const parameterKeys = ["taskId", "savePath", "projectionCount", "angleStepDeg", "exposureMs"];
  if (parameterKeys.some((key) => snapshot.parameters[key as keyof typeof snapshot.parameters] !== ws.scanSetup[key as keyof typeof ws.scanSetup])) return false;
  if (ws.summary.savePath !== ws.scanSetup.savePath) return false;
  if (!Number.isFinite(snapshot.progress.total) || snapshot.progress.total < 0) return false;
  if (snapshot.progress.current !== 0 || snapshot.progress.percent !== 0 || snapshot.progress.angleDeg !== 0) return false;
  if (snapshot.progress.etaSeconds !== null) return false;
  if (snapshot.progress.total !== snapshot.parameters.projectionCount ||
    ws.progress.total !== snapshot.progress.total || ws.progress.captured !== snapshot.progress.current ||
    ws.progress.percent !== snapshot.progress.percent || ws.progress.angleDeg !== snapshot.progress.angleDeg) return false;
  if (snapshot.imageCount !== snapshot.progress.current || snapshot.imageCount !== 0 ||
    ws.frames.length !== snapshot.imageCount || ws.checkpointAvailable !== false) return false;
  if (ws.progress.barTone !== "accent" || ws.scene.angleDeg !== snapshot.progress.angleDeg ||
    ws.scene.angleKnown !== false || ws.scene.rotated !== false) return false;
  if (ws.scene.angleDeg !== 0 || ws.progress.captured !== 0 || ws.progress.percent !== 0) return false;
  if (ws.progress.barLabel.trim() !== `${ws.progress.captured} / ${ws.progress.total} · ${ws.progress.percent}%` ||
    /\d/.test(ws.progress.etaText)) return false;
  if (ws.cameraExposure.known !== false) return false;
  return true;
}

/**
 * The only neutral disconnected snapshot is a positively identified fresh
 * engine startup. Every signal that can disprove that initial state is checked
 * here, then the caller sends all other disconnected snapshots directly to
 * Fault. Parameter drafts do not count as device readbacks or scan activity,
 * but their duplicated snapshot and workstation values must still agree.
 */
function isStrictlyInitialDisconnected(
  snapshot: EngineSnapshot,
  ws: WorkstationView,
  stale: boolean,
  sessionEverEstablished: unknown,
): boolean {
  return (
    sessionEverEstablished === false &&
    !stale &&
    snapshot.connectionState === "disconnected" &&
    snapshot.phase === "idle" &&
    hasConsistentFirstStartProjection(snapshot, ws)
  );
}

/**
 * Phases that prove the engine actually engaged in a scan. A merely CONFIGURED
 * projection count is not evidence of one: entering a count into the scan form
 * must never make the console announce a cooldown.
 */
const SCAN_ENGAGED_PHASES: ReadonlySet<string> = new Set([
  "running",
  "paused",
  "finishing",
  "stopping",
  "stopped",
  "completed",
]);

export function evaluateFeedbackEvidence(
  snapshot: EngineSnapshot,
  ws: WorkstationView,
  stale: boolean,
  sessionEverEstablished: unknown = "unknown",
): FeedbackEvidence {
  const connection = snapshot.connectionState;
  const capturedFrames = Number.isFinite(ws.progress.captured) ? ws.progress.captured : 0;
  return {
    // A lost link, a degraded link or a stale snapshot means every reading below
    // is unknown, so the console must not present it as a normal state. A
    // `disconnected` link is part of that set unless it is the benign
    // never-connected startup state, which is not a fault and must not be
    // mislabelled as one — it relaxes no gate, it only stops the mislabelling.
    linkLost:
      stale ||
      connection === "lost" ||
      connection === "degraded" ||
      (connection === "disconnected" && sessionEverEstablished !== false),
    phaseFault: snapshot.phase === "fault" || ws.dataState === "fault",
    // Fail closed: a beam that is not a CONFIRMED off/on reading stays a warning.
    beamUnconfirmed: ws.xray.beamState === "unknown" || (!ws.xray.setpointConfirmed && ws.xray.beamOn),
    // Evidence of a real scan only: the engine reached a phase that implies one
    // engaged, or at least one projection was actually committed. A configured
    // projection count alone deliberately does not qualify.
    scanAttempted: SCAN_ENGAGED_PHASES.has(snapshot.phase) || capturedFrames > 0,
  };
}

/** Projection progress is only "known" when the engine reported a target count. */
function progressPercent(snapshot: EngineSnapshot, ws: WorkstationView, stale: boolean): number | null {
  if (stale) return null;
  if (!Number.isFinite(ws.progress.total) || ws.progress.total <= 0) return null;
  const percent = Number.isFinite(ws.progress.percent) ? ws.progress.percent : snapshot.progress.percent;
  if (!Number.isFinite(percent)) return null;
  return Math.max(0, Math.min(100, Math.round(percent)));
}

const feedbackPercentText = (percent: number | null): string => (percent === null ? "UNKNOWN" : `${percent}%`);

function feedbackToneClass(tone: ConsoleStateTone): string {
  return tone === "ok" ? "tone-accent" : `tone-${tone}`;
}

/**
 * Derive the single console feedback state from a complete snapshot.
 *
 * Ordering is deliberate. A fail-closed condition always wins; 100% projection
 * progress still reads as Finishing until the engine itself reports `completed`;
 * a Stopping pose is frozen (the snapshot angle is repeated, never advanced);
 * and Cooling is only ever reported from real cooldown evidence — the snapshot
 * exposes none today, so it reports UNKNOWN instead of inventing a cooldown.
 *
 * A disconnected snapshot is partitioned in full: only a positively identified
 * fresh idle startup is neutral; every other disconnected case is Fault. This
 * keeps unknown device and output conditions out of the ordinary phase mapping.
 */
export function deriveConsoleFeedback(
  snapshot: EngineSnapshot,
  ws: WorkstationView,
  stale: boolean,
  sessionEverEstablished: unknown = "unknown",
): ConsoleFeedback {
  // Exhaustive partition: disconnected is neutral only for a fully verified
  // initial snapshot. No disconnected combination can reach the phase switch or
  // its final ordinary Stopped fallback.
  if (snapshot.connectionState === "disconnected") {
    if (isStrictlyInitialDisconnected(snapshot, ws, stale, sessionEverEstablished)) {
      return offlineFeedback();
    }
    return {
      state: "Fault",
      tone: "danger",
      detail: "Device session evidence does not confirm a never-connected startup · state is unknown",
      active: false,
    };
  }

  const evidence = evaluateFeedbackEvidence(snapshot, ws, stale, sessionEverEstablished);
  if (evidence.linkLost) {
    return {
      state: "Fault",
      tone: "danger",
      detail: "Control service unavailable · device states and progress are unknown",
      active: false,
    };
  }
  if (evidence.phaseFault || ws.xray.latched) {
    return {
      state: "Fault",
      tone: "danger",
      detail: snapshot.lastError
        ? `Device fault latched · ${snapshot.lastError} · inspect the log and recover with Preflight then HOME`
        : "Device fault latched · inspect the log and recover with Preflight then HOME",
      active: false,
    };
  }
  const outputContradictory =
    (ws.xray.beamState === "on" && !ws.xray.beamOn) ||
    (ws.xray.beamState === "off" && ws.xray.beamOn);
  const outputActiveOutsideAcquisition = ws.xray.beamOn && snapshot.phase !== "running";
  if (evidence.beamUnconfirmed || outputContradictory || outputActiveOutsideAcquisition) {
    return {
      state: "Fault",
      tone: "danger",
      detail: outputContradictory
        ? "X-ray output readings contradict each other · treated as unsafe until the engine reports a consistent state"
        : outputActiveOutsideAcquisition
          ? "X-ray output is active outside acquisition · treated as unsafe until the engine confirms it is disabled"
          : "X-ray output state is not confirmed · treated as unsafe until the engine reports it",
      active: false,
    };
  }

  const percent = progressPercent(snapshot, ws, stale);
  const percentText = feedbackPercentText(percent);
  const captured = ws.progress.captured;
  const total = ws.progress.total;
  const angleText = ws.scene.angleKnown ? `${ws.scene.angleDeg.toFixed(2)}°` : "unknown pose";

  switch (snapshot.phase) {
    case "running":
      return {
        state: "Running",
        tone: "accent",
        detail: `Acquiring view ${Math.min(captured + 1, Math.max(total, 1))} of ${total} · ${percentText} committed · ${angleText}`,
        active: true,
      };
    case "paused":
      return {
        state: "Paused",
        tone: "warn",
        detail: `Held after a committed projection · ${captured} of ${total} retained · ${percentText} · ${angleText}`,
        active: true,
      };
    case "finishing":
      // Projection progress may read 100% while final position and manifest
      // confirmation are still outstanding.
      return {
        state: "Finishing",
        tone: "warn",
        detail: snapshot.mode === "developer_preview"
          ? `All ${total} projections committed · simulated return and cleanup in progress`
          : `All ${total} projections committed · verifying the final ${angleText} position, output OFF and manifest · not complete yet`,
        active: true,
      };
    case "stopping":
      return {
        state: "Stopping",
        tone: "warn",
        detail: `Ending the active scan · output disabled · pose frozen at ${ws.scene.angleKnown ? angleText : "an unknown reading"} · awaiting beam-off and task exit`,
        active: true,
      };
    case "completed":
      return {
        state: "Completed",
        tone: "ok",
        detail: snapshot.mode === "developer_preview"
          ? `Simulated return and cleanup confirmed · ${total} of ${total} projections stored`
          : `Final position ${angleText} and output OFF confirmed · manifest complete · ${total} of ${total} projections stored`,
        active: false,
      };
    case "stopped":
      return {
        state: "Stopped",
        tone: "muted",
        detail: `Scan ended by the operator · ${captured} of ${total} committed · preflight and HOME are required before another scan`,
        active: false,
      };
    default:
      break;
  }

  // The engine is in a phase that does not itself describe a scan, but a scan
  // has genuinely engaged (see SCAN_ENGAGED_PHASES), so a cooldown could be in
  // play. The snapshot does carry a temperature readback (ws.xray.tempC), but
  // temperature alone cannot prove an active cooldown, and the engine's cooldown
  // deadline is private and never projected. So the state is reported as UNKNOWN
  // rather than assumed active or assumed absent — never as a confirmed
  // "cooling" or "ready".
  if (evidence.scanAttempted) {
    return {
      state: "Cooling",
      tone: "muted",
      detail: `Cooling state unknown · the snapshot projects no cooldown deadline, and a temperature reading alone cannot confirm one · verify at the source before exposure`,
      active: false,
    };
  }
  return {
    state: "Stopped",
    tone: "muted",
    detail: `Queue idle · no scan in flight · ${total > 0 ? `${total} projections configured` : "scan setup not configured"}`,
    active: false,
  };
}

/** Ring progress read-out. Unknown progress is never drawn as 0%. */
function RingReadout({
  percent,
  tone,
  label,
  known,
}: {
  percent: number;
  tone: ConsoleStateTone;
  label: string;
  known: boolean;
}) {
  const radius = 13;
  const circumference = 2 * Math.PI * radius;
  const offset = known ? circumference * (1 - percent / 100) : 0;
  return (
    <span className={`dock-ring dock-ring--${known ? "known" : "unknown"} ${feedbackToneClass(tone)}`}>
      <svg className="dock-ring__svg" viewBox="0 0 36 36" aria-hidden="true" focusable="false">
        <circle className="dock-ring__track" cx="18" cy="18" r={radius} fill="none" strokeWidth="3" />
        <circle
          className="dock-ring__value"
          cx="18"
          cy="18"
          r={radius}
          fill="none"
          strokeWidth="3"
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          transform="rotate(-90 18 18)"
        />
      </svg>
      <span className="dock-ring__text">{known ? `${percent}` : label}</span>
    </span>
  );
}

/**
 * Dynamic-Island control dock.
 *
 * Collapsed it is a small frosted capsule with a ring progress indicator and a
 * readable numeric read-out. It expands into the four frosted keys through
 * hover, keyboard focus, an explicit click and touch. `aria-expanded` is bound
 * to the shipped state; the toggle keeps its footprint while expanding, so a
 * mis-tap between the two states is not possible; and only the committed
 * snapshot progress reaches the ring.
 */
function ControlDock({
  snapshot,
  ws,
  dispatch,
  stale = false,
  setupInvalid = false,
}: {
  snapshot: EngineSnapshot;
  ws: WorkstationView;
  dispatch: (c: EngineCommand) => void;
  stale?: boolean;
  setupInvalid?: boolean;
}) {
  const [hoverOpen, setHoverOpen] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  const [focusInside, setFocusInside] = useState(false);
  const [approaching, setApproaching] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  // An explicit click pins the dock open and wins over hover, so hovering a
  // collapsed capsule can never leave the pointer sitting on a key it did not
  // aim at, and an explicit click can always close the dock again.
  const [clickOpen, setClickOpen] = useState(false);
  // A touch tap has no hover: pointerdown expands the capsule and the click that
  // the same tap generates must not immediately fold it away again.
  const touchHandled = useRef(false);
  const expandTimer = useRef<number | null>(null);
  const feedback = deriveConsoleFeedback(snapshot, ws, stale, hasDeviceSessionEverEstablished());
  const dock = ws.dock;

  const expanded = hoverOpen || focusInside || clickOpen;
  const percent = progressPercent(snapshot, ws, stale);
  const percentKnown = percent !== null;
  const progressReadout = percentKnown ? `${percent}% · ${ws.progress.captured}/${ws.progress.total}` : "UNKNOWN";

  const clearExpandTimer = useCallback((): void => {
    if (expandTimer.current !== null) {
      window.clearTimeout(expandTimer.current);
      expandTimer.current = null;
    }
  }, []);

  useEffect(() => clearExpandTimer, [clearExpandTimer]);

  useEffect(() => {
    const preference = window.matchMedia("(prefers-reduced-motion: reduce)");
  const syncReducedMotion = (event: MediaQueryListEvent): void => {
      setReducedMotion(event.matches);
      setHoverOpen(event.matches);
      if (event.matches) {
        clearExpandTimer();
        setApproaching(false);
        setHoverOpen(true);
      }
    };
    if (preference.matches) setHoverOpen(true);
    preference.addEventListener("change", syncReducedMotion);
    return () => preference.removeEventListener("change", syncReducedMotion);
  }, [clearExpandTimer]);

  const openForHover = useCallback((): void => {
    clearExpandTimer();
    if (reducedMotion) {
      setApproaching(false);
      setHoverOpen(true);
      return;
    }
    setApproaching(true);
  }, [clearExpandTimer, reducedMotion]);

  // Keep the approach ripple alive briefly as the pointer crosses the capsule.
  const scheduleHoverClose = useCallback((): void => {
    clearExpandTimer();
    expandTimer.current = window.setTimeout(() => {
      expandTimer.current = null;
      setApproaching(false);
    }, 260);
  }, [clearExpandTimer]);

  const collapse = useCallback((): void => {
    clearExpandTimer();
    setApproaching(false);
    setHoverOpen(false);
    setFocusInside(false);
    setClickOpen(false);
  }, [clearExpandTimer]);

  // A click on the capsule first pins the panel open; only a later capsule click
  // collapses it, so the pointer never lands on a newly revealed action key.
  const toggleOpen = useCallback((): void => {
    clearExpandTimer();
    setApproaching(false);
    if (touchHandled.current) {
      // The tap already expanded the dock on pointerdown.
      touchHandled.current = false;
      return;
    }
    if (clickOpen) {
      collapse();
      return;
    }
    setClickOpen(true);
  }, [clickOpen, collapse, clearExpandTimer]);

  // Escape collapses the dock from anywhere, not only while one of its own keys
  // holds focus: the dock can also be expanded by hover, and requiring focus
  // would leave the pointer as the only way back to the collapsed state.
  useEffect(() => {
    if (!expanded) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") collapse();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [collapse, expanded]);

  const play =
    dock.playMode === "pause"
      ? { icon: "pause" as LineIconName, label: "Pause", command: { type: "pause" } as EngineCommand }
      : dock.playMode === "resume"
        ? { icon: "play" as LineIconName, label: "Resume", command: { type: "resume" } as EngineCommand }
        : { icon: "play" as LineIconName, label: "Start", command: { type: "start_scan" } as EngineCommand };

  // Availability comes from the engine's own `ws.dock` flags, plus the stale
  // (control-service-lost) override. On top of that, a locally invalid setup
  // draft must not be able to drive an operation that consumes scan parameters,
  // which is the same rule the engineering menu applies. Stop is deliberately
  // exempt: ending a scan must never depend on parameter-validity checks.
  const keysDisabled = stale;
  const setupBlocked = keysDisabled || setupInvalid;
  const announcement = `${feedback.state}. ${feedback.detail} Progress ${progressReadout}.`;

  return (
    <div
      className={`control-dock ${expanded ? "is-expanded" : "is-collapsed"} ${approaching ? "is-approaching" : ""} ${reducedMotion ? "is-reduced-motion" : ""} dock-state--${feedback.state.toLowerCase()}`}
      aria-label="Scan controls"
      aria-expanded={expanded}
      onMouseEnter={openForHover}
      onMouseLeave={scheduleHoverClose}
      onFocusCapture={() => {
        clearExpandTimer();
        setApproaching(false);
        setFocusInside(true);
      }}
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocusInside(false);
      }}
      // Touch has no hover: a tap on the collapsed capsule expands it, and the
      // four keys are only reachable once the capsule has visibly expanded.
      onPointerDown={(event) => {
        if (event.pointerType !== "touch" || !(event.target as HTMLElement).closest(".dock-capsule")) return;
        clearExpandTimer();
        setApproaching(false);
        setHoverOpen(true);
        setClickOpen(true);
        touchHandled.current = true;
      }}
    >
      <div className="dock-key-row" id="control-dock-keys">
        <button
          type="button"
          className="dock-capsule"
          aria-expanded={expanded}
          aria-controls="control-dock-keys"
          aria-label={
            expanded
              ? `Collapse scan controls. ${feedback.state}. Progress ${progressReadout}.`
              : `Expand scan controls. ${feedback.state}. Progress ${progressReadout}.`
          }
          title={`${feedback.state} · ${feedback.detail}`}
          onClick={toggleOpen}
        >
          <span className="dock-capsule__lead">
            <RingReadout percent={percent ?? 0} tone={feedback.tone} label="–" known={percentKnown} />
            <span className="dock-capsule__text">
              <span className="dock-capsule__state">{feedback.state}</span>
              <span className="dock-capsule__progress">{progressReadout}</span>
            </span>
          </span>
          <span className="dock-capsule__chevron" aria-hidden="true" />
        </button>
        <DockKey
          variant="home"
          icon="crosshair"
          label="Home"
          title={setupInvalid ? "Complete valid scan parameters first" : dock.homeReason || "Home turntable"}
          disabled={setupBlocked || !dock.home}
          onClick={() => void dispatch({ type: "home" })}
        />
        <DockKey
          variant="play"
          icon={play.icon}
          label={play.label}
          title={setupInvalid ? "Complete valid scan parameters first" : dock.playReason || play.label}
          disabled={setupBlocked || !dock.play}
          onClick={() => void dispatch(play.command)}
        />
        <DockKey
          variant="restore"
          icon="history"
          label="Restore"
          title={setupInvalid ? "Complete valid scan parameters first" : dock.restoreReason || "After Preflight and a valid Nano reference, verify saved frames; Start continues at the next view"}
          disabled={setupBlocked || !dock.restore}
          onClick={() => void dispatch({ type: "restore_previous" })}
        />
        <DockKey
          variant="stop"
          icon="stop"
          label="Stop"
          title={dock.stop ? "End the active scan; already saved projections are retained" : "No active scan to end"}
          disabled={keysDisabled || !dock.stop}
          onClick={() => void dispatch({ type: "stop" })}
        />
      </div>
      <span className="sr-only" role="status" aria-live="polite">{announcement}</span>
    </div>
  );
}

/**
 * Read-only eight-state console feedback strip. The active state is emphasised;
 * its detail line always carries the meaning as text, so no reading depends on
 * the colour of a dot. The strip scrolls horizontally inside its own row when
 * the column is narrow, which never clips a fixed column.
 */
function StateFeedback({ feedback, stale }: { feedback: ConsoleFeedback; stale: boolean }) {
  const active = stale ? "Fault" : feedback.state;
  // The strip is one glance, not a paragraph: only the leading segment of the
  // detail stays on the row; the full sentence lives in the help bubble.
  const detailLead = feedback.detail.split(" · ")[0];
  return (
    <div className={`op-feedback ${feedback.active ? "is-hot" : ""}`}>
      <div
        className={`op-feedback__row ${feedbackToneClass(feedback.tone)}`}
        role="status"
        aria-live="polite"
        aria-label={`Console feedback state: ${active}. ${feedback.detail}`}
      >
        <span className="op-feedback__word">{active}</span>
        <span className="op-feedback__detail">{detailLead}</span>
        <HelpTip text={feedback.detail} align="end" />
      </div>
      <div className="op-feedback__states">
        {CONSOLE_FEEDBACK_STATES.map((state) => (
          <span
            key={state}
            className={`op-feedback__chip ${state.toLowerCase()} ${state === active ? "is-active" : "is-idle"}`}
          >
            {state}
          </span>
        ))}
      </div>
    </div>
  );
}

function LiveScene({ snapshot, ws, theme, dispatch, stale, setupInvalid, feedbackId }: { snapshot: EngineSnapshot; ws: WorkstationView; theme: Theme; dispatch: (c: EngineCommand) => void; stale: boolean; setupInvalid: boolean; feedbackId: string }) {
  const [viewPreset, setViewPreset] = useState<ViewPreset>("iso");
  const [presetRevision, setPresetRevision] = useState(0);
  const fallback = useSceneFallback();
  const feedback = deriveConsoleFeedback(snapshot, ws, stale, hasDeviceSessionEverEstablished());
  const sceneView = {
    dataState: ws.dataState,
    angleDeg: ws.scene.angleDeg,
    feedbackId,
    feedbackValid: !stale && ws.scene.angleKnown,
    rotationDirection: ws.scene.rotationDirection,
    taskId: ws.scanSetup.taskId,
    theme,
    beamOn: !stale && ws.xray.beamState === "on",
    xrayLatched: ws.xray.latched,
  } as const;
  return (
    <section className="panel live-panel">
      <div className="scene-toolbar">
        <span className="chip chip--accent">3D RENDER</span>
        <LineIcon name="cube" size={15} className="panel__icon" />
        <h2>Equipment View</h2>
        <HelpTip text={`Read-only geometry view. ${setupInvalid ? "Complete valid scan parameters first." : ws.dock.playReason || "Ready for the next operation."}`} />
        <span className="menu-spacer" />
        {(["iso", "front", "top"] as const).map((preset) => (
          <button
            key={preset}
            type="button"
            className={`scene-view-btn ${viewPreset === preset ? "active" : ""}`}
            aria-pressed={viewPreset === preset}
            onClick={() => { setViewPreset(preset); setPresetRevision(value => value + 1); }}
          >
            {preset.toUpperCase()}
          </button>
        ))}
      </div>
      <div className="live-scene">
        {fallback.reason ? (
          <StaticSceneFallback view={sceneView} reason={fallback.reason} />
        ) : (
          <LiveSceneCanvas view={sceneView} status={feedback} preset={viewPreset} presetRevision={presetRevision} onContextLost={fallback.setContextLost} />
        )}
        <span className="live-indicator">
          <i aria-hidden="true" />
          GEOMETRY VIEW · {stale ? "STATUS UNAVAILABLE" : "OPTICAL AXIS"}
        </span>
        <div className="scene-readout-block">
          <span className="scene-label">TURNTABLE ANGLE <HelpTip side="up" text="Latest confirmed turntable angle. The model follows confirmed feedback without predicting a position; reduced motion jumps directly between samples." /></span>
          <strong className="scene-readout">{stale || !ws.scene.angleKnown ? "—" : ws.scene.angleDeg.toFixed(2)}°</strong>
          <span className={`scene-safety ${ws.safetyBar.tone !== "muted" ? "scene-safety--danger" : ""} ${ws.safetyBar.tone === "dangerBold" ? "scene-safety--bold" : ""}`}>
            {stale ? "Control service unavailable · readings are unknown" : ws.safetyBar.text}
          </span>
        </div>
        <ControlDock
          snapshot={snapshot}
          ws={ws}
          dispatch={dispatch}
          stale={stale}
          setupInvalid={setupInvalid}
        />
      </div>
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* Right column                                                        */
/* ------------------------------------------------------------------ */

function XrayPanel({ ws, busy, dispatch, stale }: { ws: WorkstationView; busy: boolean; dispatch: (c: EngineCommand) => Promise<void>; stale: boolean }) {
  const [kvDraft, setKvDraft] = useState(ws.xray.setKv.toFixed(1));
  const [uaDraft, setUaDraft] = useState(ws.xray.setUa.toFixed(1));
  const [delayDraft, setDelayDraft] = useState(String(ws.xray.usbShutdownDelay ?? 5));
  useEffect(() => {
    setKvDraft(ws.xray.setKv.toFixed(1));
    setUaDraft(ws.xray.setUa.toFixed(1));
  }, [ws.xray.setKv, ws.xray.setUa]);
  useEffect(() => {
    if (ws.xray.usbShutdownDelay != null) {
      setDelayDraft(String(ws.xray.usbShutdownDelay));
    }
  }, [ws.xray.usbShutdownDelay]);
  const voltageDraftConfirmed =
    ws.xray.voltageConfirmed && Number(kvDraft).toFixed(1) === ws.xray.setKv.toFixed(1);
  const currentDraftConfirmed =
    ws.xray.currentConfirmed && Number(uaDraft).toFixed(1) === ws.xray.setUa.toFixed(1);
  const pairConfirmed = voltageDraftConfirmed && currentDraftConfirmed;
  const scanLocked = ws.dataState === "scanning";
  const faultLocked = ws.dataState === "fault" || ws.xray.latched;
  const beamState = stale ? "unknown" : ws.xray.beamState;
  const outputText = beamState === "on" ? "ON" : beamState === "off" ? "OFF · VERIFIED" : "UNKNOWN";
  const connected = !stale && ws.xray.connected;
  const sourceHelp = faultLocked
          ? "Device fault latched · manual beam enable is locked until device checks, Preflight and HOME complete"
          : scanLocked
          ? "CT scan owns beam commands only · USB AUTO SHUT DOWN remains operator-controlled"
          : !ws.xray.connected
            ? "Connect the Moxtek before changing setpoints or device safety settings"
            : !ws.xray.usbAutoShutDownKnown
              ? "Choose USB AUTO SHUT DOWN manually before Preflight"
          : ws.xray.usbAutoShutDown
            ? "Device timer armed · continuous output is bounded if the host stops responding"
            : pairConfirmed
              ? "Released by operator · CT scan will not restore it automatically"
              : "Released by operator · confirm both setpoints before standalone Xray Enable";

  return (
    <section className="panel xray-panel">
      <div className="panel__header">
        <LineIcon name="radiation" size={15} className="panel__icon" />
        <h2>12 Watt Controller</h2>
        <HelpTip text={sourceHelp} />
        <span className={`chip chip--${beamState === "on" ? "danger" : connected ? "accent" : "muted"}`}>
          {beamState === "on" ? "EMITTING" : connected ? "CONNECTED" : "NOT CONNECTED"}
        </span>
      </div>
      <div className="xray-panel__body">
      <div className={`xray-mode-banner ${scanLocked || faultLocked ? "xray-mode-banner--locked" : "xray-mode-banner--manual"}`}>
        <span>{faultLocked ? "Device fault · manual locked" : scanLocked ? "Scan control · manual locked" : "Manual source control"}</span>
        <strong className={beamState === "on" ? "tone-danger" : beamState === "off" ? "tone-ok" : "tone-muted"}>{outputText}</strong>
      </div>
      <button
        type="button"
        className={`switch-btn switch-btn--block xray-connection-btn ${ws.xray.connected ? "xray-connection-btn--connected" : "switch-btn--idle"}`}
        disabled={busy || scanLocked || ws.xray.beamOn}
        onClick={() => void dispatch(ws.xray.connected ? { type: "xray_disconnect" } : { type: "retry_device", device: "xray" })}
      >
        {ws.xray.connected ? "Xray Disconnect" : "Xray Connect"}
      </button>
      <div className="xray-channels">
        <div className="xray-channel">
          <span className="xray-channel__label">Voltage · kV <HelpTip align="end" text="SET is the requested voltage. Measured is device readback; a dash means unknown." /></span>
          <div className="xray-channel__row">
            <div className="xray-channel__control">
              <span className="xray-channel__key">SET</span>
              <input
                className="xray-channel__well"
                aria-label="Voltage setpoint in kV"
                value={kvDraft}
                disabled={busy || !ws.xray.setpointControlsEnabled}
                inputMode="decimal"
                onChange={(event) => setKvDraft(event.target.value)}
              />
              <span className="xray-channel__monitor">Measured <b>{measured(ws.xray.monKv, stale)}</b></span>
            </div>
            <button type="button" className="send-btn" disabled={busy || !ws.xray.setpointControlsEnabled || !Number.isFinite(Number(kvDraft))} onClick={() => void dispatch({ type: "send_voltage", kv: Number(kvDraft) })}>
              SEND V
            </button>
          </div>
        </div>
        <div className="xray-channel">
          <span className="xray-channel__label">Current · µA <HelpTip align="end" text="SET is the requested current. Measured is device readback; a dash means unknown." /></span>
          <div className="xray-channel__row">
            <div className="xray-channel__control">
              <span className="xray-channel__key">SET</span>
              <input
                className="xray-channel__well"
                value={uaDraft}
                aria-label="Current setpoint in microamps"
                disabled={busy || !ws.xray.setpointControlsEnabled}
                inputMode="decimal"
                onChange={(event) => setUaDraft(event.target.value)}
              />
              <span className="xray-channel__monitor">Measured <b>{measured(ws.xray.monUa, stale)}</b></span>
            </div>
            <button type="button" className="send-btn" disabled={busy || !ws.xray.setpointControlsEnabled || !Number.isFinite(Number(uaDraft))} onClick={() => void dispatch({ type: "send_current", ua: Number(uaDraft) })}>
              SEND I
            </button>
          </div>
        </div>
      </div>
      <div className="xray-meters">
        <div className="xray-meter">
          <span>Measured power</span>
          <div className="xray-meter__well">{measured(ws.xray.powerW, stale)} <small>W</small></div>
        </div>
        <div className="xray-meter">
          <span>Temperature</span>
          <div className="xray-meter__well">{measured(ws.xray.tempC, stale)} <small>°C</small></div>
        </div>
        <div className="xray-meter">
          <span>Output state</span>
          <div className={`xray-meter__well ${beamState === "on" ? "xray-meter__well--danger" : beamState === "off" ? "xray-meter__well--safe" : "xray-meter__well--unknown"}`}>
            {beamState === "on" ? "ON" : beamState === "off" ? "OFF" : "UNKNOWN"}
          </div>
        </div>
      </div>
      <button
        type="button"
        className={`switch-btn switch-btn--block ${ws.xray.beamOn ? "switch-btn--on" : "switch-btn--idle"}`}
        disabled={busy || ws.dataState === "fault" || !ws.xray.manualControlsEnabled}
        title={faultLocked ? "Device fault latched · inspect the log, then repeat Preflight and HOME" : scanLocked ? "The scan owns X-ray output" : !ws.xray.connected ? "Connect the X-ray source first" : busy ? "Wait for the current command" : "Manual X-ray output control"}
        onClick={() => void dispatch({ type: "xray_toggle" })}
      >
        {ws.xray.beamOn ? "Xray Disable" : "Xray Enable"}
      </button>
      <div className="panel__divider" />
      <div className="timer-row">
        <div className="timer-field">
          <span>On Sec</span>
          <div className="timer-field__well">{ws.xray.onSec}</div>
        </div>
        <div className="timer-field">
          <span>Off Sec</span>
          <div className="timer-field__well">{ws.xray.offSec}</div>
        </div>
        <button
          type="button"
          className={`switch-btn switch-btn--chip ${ws.xray.timerOn ? "switch-btn--on" : "switch-btn--idle"}`}
          disabled={busy || ws.dataState === "scanning" || !ws.xray.timerControlsEnabled}
          onClick={() => void dispatch({ type: "timer_toggle" })}
        >
          {ws.xray.timerOn ? "Timer On" : "Timer Off"}
        </button>
      </div>
      <div className="safety-block">
        <div className="safety-block__head">
          <span className="safety-block__title">DEVICE SAFETY</span>
          <span className={`chip chip--${stale || !ws.xray.usbAutoShutDownKnown ? "muted" : ws.xray.usbAutoShutDown ? "ok" : "warn"}`}>
            {stale || !ws.xray.usbAutoShutDownKnown
              ? "NOT VERIFIED"
              : ws.xray.usbAutoShutDown
              ? "TIMER ARMED"
              : scanLocked
                ? "RELEASED · SCAN OWNED"
                : faultLocked
                  ? "RELEASED · FAULT LOCKED"
                : "RELEASED · MANUAL"}
          </span>
        </div>
        {/* Unknown readback is already indicated by NOT VERIFIED and the indeterminate
            control. Never claim the hardware timer is armed until confirmed. */}
        <label className="check-row check-row--inset">
          <input
            type="checkbox"
            checked={!stale && ws.xray.usbAutoShutDownKnown && ws.xray.usbAutoShutDown}
            ref={(element) => { if (element) element.indeterminate = stale || !ws.xray.usbAutoShutDownKnown; }}
            disabled={busy || ws.dataState === "scanning" || !ws.xray.connected}
            onChange={() => void dispatch({ type: "usb_auto_shut_down_toggle" })}
          />
          <i aria-hidden="true" />
          USB Auto Shut Down
        </label>
        <div className="delay-row">
          <div className="delay-row__field">
            <span>Shut Down Delay</span>
            <input
              value={delayDraft}
              inputMode="numeric"
              disabled={busy || ws.dataState === "scanning" || !ws.xray.manualControlsEnabled}
              onChange={(event) => setDelayDraft(event.target.value)}
            />
          </div>
          <button
            type="button"
            className="send-btn"
            disabled={busy || ws.dataState === "scanning" || !ws.xray.manualControlsEnabled || !/^\d+$/.test(delayDraft.trim()) || Number(delayDraft) <= 0 || Number(delayDraft) > 65535}
            onClick={() => void dispatch({ type: "set_usb_shutdown_delay", delay: Number(delayDraft) })}
          >
            SET
          </button>
          <span className="delay-row__device">Device {stale ? "—" : ws.xray.usbShutdownDelay ?? "—"}</span>
        </div>
      </div>
      </div>

    </section>
  );
}

function OperationPanel({ snapshot, ws, stale }: { snapshot: EngineSnapshot; ws: WorkstationView; stale: boolean }) {
  const feedback = deriveConsoleFeedback(snapshot, ws, stale, hasDeviceSessionEverEstablished());
  return (
    <section className="panel operation-panel">
      <div className="panel__header">
        <LineIcon name="gauge" size={15} className="panel__icon" />
        <h2>Operation Status</h2>
        <span className={`chip chip--${stale ? "muted" : ws.phaseTone}`}>{stale ? "UNKNOWN" : ws.phaseWord}</span>
      </div>
      <StateFeedback feedback={feedback} stale={stale} />
      <div className="op-stats">
        <div className="op-stat">
          <span>CAPTURED</span>
          <strong>
            {stale || ws.progress.total === 0 ? "— / —" : `${ws.progress.captured} / ${ws.progress.total}`}
          </strong>
        </div>
        <div className="op-stat">
          <span>ANGLE</span>
          <strong>{stale || !ws.scene.angleKnown ? "—" : ws.progress.angleDeg.toFixed(2)}°</strong>
        </div>
        <div className="op-stat">
          <span title="Estimated from completed projection cycles, including X-ray cooling">EST. REMAINING</span>
          <strong>{stale ? "—" : ws.progress.etaText}</strong>
        </div>
      </div>
      <div className="op-progress">
        <div className="op-progress__head">
          <span>VIEW PROGRESS</span>
          <small>{stale ? "Status unavailable" : ws.progress.total ? ws.progress.barLabel : "Awaiting scan setup"}</small>
        </div>
        <div className={`op-progress__bar op-progress__bar--${ws.progress.barTone}`} role="progressbar" aria-valuenow={ws.progress.percent} aria-valuemin={0} aria-valuemax={100}>
          <span style={{ width: `${ws.progress.percent}%` }} />
        </div>
      </div>
      <div className="op-summary">
        <span className="op-summary__title">PROCESS SUMMARY</span>
        <div>
          <span>Acquisition</span>
          <strong>{ws.summary.acquisition}</strong>
        </div>
        <div>
          <span>Output</span>
          <strong title={ws.summary.output}>{stale ? "UNKNOWN · service unavailable" : ws.summary.output}</strong>
        </div>
      </div>
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* Bottom console                                                      */
/* ------------------------------------------------------------------ */

const bottomTabs: Array<{ id: BottomTab; label: string; icon: LineIconName }> = [
  { id: "aggregate", label: "Log Aggregation", icon: "terminal" },
  { id: "xray", label: "X-ray Log", icon: "radiation" },
  { id: "nano", label: "Turntable Log", icon: "rotate" },
  { id: "camera", label: "Camera Log", icon: "camera" },
  { id: "images", label: "Image Preview", icon: "image" },
];

const logLevelClass: Record<ConsoleLogLine["level"], string> = {
  PASS: "log-line--pass",
  INFO: "log-line--info",
  OK: "log-line--info",
  WARN: "log-line--warn",
  ERR: "log-line--err",
  ACTION: "log-line--pass",
};

function LogLines({ logs, follow, onFollow }: { logs: ConsoleLogLine[]; follow: boolean; onFollow: (value: boolean) => void }) {
  const list = useRef<HTMLDivElement | null>(null);
  const ordered = useMemo(() => [...logs].sort((a, b) => a.timestamp.localeCompare(b.timestamp)), [logs]);
  useEffect(() => {
    if (follow && list.current) list.current.scrollTop = list.current.scrollHeight;
  }, [ordered, follow]);
  if (!logs.length) return <div className="log-empty">No records in this channel.</div>;
  return (
    <div className="log-lines" ref={list} onScroll={(event) => {
      const node = event.currentTarget;
      onFollow(node.scrollHeight - node.scrollTop - node.clientHeight < 8);
    }}>
      {ordered.map((entry) => (
        <div className={`log-line ${logLevelClass[entry.level]}`} key={entry.id}>
          <time className="log-line__time">[{logTime(entry.timestamp)}]</time>
          <span className="log-line__tag">
            {entry.level} · {entry.source}
          </span>
          <span className="log-line__msg">{entry.message}</span>
        </div>
      ))}
    </div>
  );
}

function FrameThumbnail({ frame, production }: { frame: ConsoleFrame; production: boolean }) {
  const element = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);
  const [url, setUrl] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  useEffect(() => {
    if (!production || !element.current) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) { setVisible(true); observer.disconnect(); }
    });
    observer.observe(element.current);
    return () => observer.disconnect();
  }, [production]);
  useEffect(() => {
    if (!production || !visible) return;
    let active = true;
    let objectUrl: string | null = null;
    void invoke<number[]>("frame_preview", { index: frame.index }).then(bytes => {
      if (!active) return;
      objectUrl = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
      setUrl(objectUrl);
    }).catch(reason => { if (active) setFailure(String(reason)); });
    return () => { active = false; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [frame.index, frame.sha256, production, visible]);
  return <div className="image-tile image-tile--captured" ref={element} title={failure || frame.fileName}>
    <div className="image-tile__image">
      {url ? <img src={url} alt={`Camera projection ${frame.index}`} /> : <span>{failure ? "Preview unavailable" : production ? "Loading image…" : "Offline preview"}</span>}
    </div>
    <div className="image-tile__caption"><strong>VIEW {String(frame.index).padStart(2, "0")}</strong><span>{frame.angleDeg.toFixed(2)}° · {frame.exposureMs} ms</span></div>
  </div>;
}

function BottomConsole({ ws, production }: { ws: WorkstationView; production: boolean }) {
  const [tab, setTab] = useState<BottomTab>("aggregate");
  const [follow, setFollow] = useState(true);
  const logs = ws.consoleLogs;
  const frames = useMemo(() => [...ws.frames].sort((a, b) => a.index - b.index), [ws.frames]);
  const filtered = useMemo(() => {
    if (tab === "aggregate" || tab === "images") return logs;
    return logs.filter((entry) => entry.source === tab);
  }, [logs, tab]);
  const counts = useMemo(
    () => ({
      aggregate: logs.length,
      xray: logs.filter((entry) => entry.source === "xray").length,
      nano: logs.filter((entry) => entry.source === "nano").length,
      camera: logs.filter((entry) => entry.source === "camera").length,
      images: ws.frames.length,
    }),
    [logs, ws.frames.length],
  );
  const header =
    tab === "images"
      ? `CAPTURED IMAGES · ${ws.scanSetup.taskId} · ${ws.progress.captured} / ${ws.progress.total} VIEWS · ${ws.scanSetup.angleStepDeg.toFixed(2)}° STEP · ${ws.scanSetup.exposureMs} ms`
      : tab === "aggregate"
        ? "SESSION EVENTS · ALL SOURCES"
        : `${tab === "xray" ? "X-RAY" : tab === "nano" ? "TURNTABLE" : "CAMERA"} EVENTS`;

  return (
    <section className="console">
      <div className="console__tabs" role="tablist" aria-label="Logs and image preview">
        {bottomTabs.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            className="console__tab"
            onClick={() => setTab(item.id)}
          >
            <LineIcon name={item.icon} size={13} className="console__tab-icon" />
            <em>{item.label}</em>
            <span>{counts[item.id]}</span>
          </button>
        ))}
      </div>
      <div className="console__body" role="tabpanel">
        <div className="console__head"><span>{header}</span>{tab !== "images" && (
          <button type="button" className="follow-toggle" aria-pressed={follow} onClick={() => setFollow(!follow)}>
            {follow ? "Following latest" : "Resume live log"}
          </button>
        )}</div>
        {tab === "images" ? (
          <div className="image-strip">
            {frames.length === 0 && <div className="image-strip__empty">No projections yet. Configure a scan task to view its image sequence.</div>}
            {frames.map((frame) => (
              <FrameThumbnail frame={frame} production={production} key={`${ws.scanSetup.taskId}:${frame.index}:${frame.sha256 ?? "preview"}`} />
            ))}
          </div>
        ) : (
          <LogLines logs={filtered} follow={follow} onFollow={setFollow} />
        )}
      </div>
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* Menu dialogs (overlay on the fixed design canvas, layout unchanged) */
/* ------------------------------------------------------------------ */

type DialogKind = "guide" | "safety" | "about" | "preferences" | "diagnostics";

const DIALOG_TITLES: Record<DialogKind, string> = {
  guide: "User Guide",
  safety: "Safety Notes",
  about: "About Micro-CT Workstation",
  preferences: "Preferences",
  diagnostics: "Device Diagnostics",
};

const DIALOG_ICONS: Record<DialogKind, LineIconName> = {
  guide: "book",
  safety: "warning",
  about: "info",
  preferences: "sliders",
  diagnostics: "pulse",
};

const APP_VERSION = "0.7.8";

function InfoDialog({
  kind,
  ws,
  snapshot,
  adapterKind,
  theme,
  onTheme,
  onClose,
}: {
  kind: DialogKind;
  ws: WorkstationView;
  snapshot: EngineSnapshot;
  adapterKind: "developer_preview" | "tauri";
  theme: Theme;
  onTheme: (theme: Theme) => void;
  onClose: () => void;
}) {
  let body: ReactNode = null;
  if (kind === "guide") {
    body = (
      <ol className="modal-list">
        <li>
          <strong>Configure the scan.</strong> Task ID, Save Path, Total Projections, Exposure and Max X-ray
          (minutes, default 10) must be valid. Projections accept 1–3600; exposure follows the camera's timed shutter limits.
        </li>
        <li>
          <strong>Run preflight</strong> (Tools → Run Preflight). Eight checks run over the link; the engine
          rejects exposure until they pass.
        </li>
        <li>
          <strong>Establish a turntable reference.</strong> Run HOME once. Later scans in the same valid Nano
          session may reuse it after live STATUS and Preflight confirm the controller is idle and referenced.
        </li>
        <li>
          <strong>Start the scan.</strong> Pause takes effect after a safe projection boundary, Resume continues,
          and Stop ends the active scan while retaining committed images.
        </li>
        <li>
          <strong>After Stop</strong> run Preflight and HOME again before starting a new scan.
        </li>
        <li>
          <strong>Continue an unfinished scan.</strong> Keep the same task ID, save path, projection count,
          exposure and X-ray setpoints. Run Preflight; HOME is needed if Nano reports an invalid reference.
          Restore verifies the manifest and each saved image. Start continues at the next missing
          view without replacing previously committed images.
        </li>
      </ol>
    );
  } else if (kind === "safety") {
    body = (
      <ul className="modal-list">
        <li>
          <strong>Device faults fail closed.</strong> The engine attempts to turn off X-rays and stop motion,
          then invalidates Preflight and HOME. A physical emergency stop remains part of the hardware safety system.
        </li>
        <li>
          <strong>Preflight before exposure.</strong> The engine requires a live, valid Nano reference.
          HOME establishes it when the controller reports none; STOP or a lost reference requires HOME again.
        </li>
        <li>
          <strong>Any parameter change invalidates the safety chain.</strong> Editing Task ID, Save Path,
          projections, exposure or Max X-ray resets preflight; a still valid Nano reference is checked again.
        </li>
        <li>
          <strong>{snapshot.mode === "developer_preview" ? "This build drives no hardware." : "Stage 1 controls the Nano turntable only."}</strong>{" "}
          {snapshot.modeLabel}. Camera and X-ray remain locked; never treat Nano preflight as a verified X-ray interlock.
        </li>
      </ul>
    );
  } else if (kind === "about") {
    body = (
      <dl className="modal-facts">
        <div>
          <dt>Application</dt>
          <dd>Micro-CT Workstation</dd>
        </div>
        <div>
          <dt>Version</dt>
          <dd>{APP_VERSION}</dd>
        </div>
        <div>
          <dt>Engine</dt>
          <dd>{snapshot.modeLabel}</dd>
        </div>
        <div>
          <dt>Control chain</dt>
          <dd>
            {adapterKind === "tauri"
              ? "React → Tauri invoke → Rust EngineClient → ct-engine (JSONL stdio IPC)"
              : "Browser developer preview (no ct-engine sidecar)"}
          </dd>
        </div>
        <div>
          <dt>Connection</dt>
          <dd>{snapshot.connectionState.toUpperCase()}</dd>
        </div>
        <div>
          <dt>Devices</dt>
          <dd>{ws.onlineSummary}</dd>
        </div>
      </dl>
    );
  } else if (kind === "preferences") {
    body = (
      <div className="modal-stack">
        <div className="modal-row">
          <span>Interface theme</span>
          <div className="theme-toggle" role="group" aria-label="Theme">
            {(["light", "dark"] as const).map((value) => (
              <button
                key={value}
                type="button"
                className="theme-toggle__seg"
                aria-pressed={theme === value}
                onClick={() => onTheme(value)}
              >
                {value === "light" ? "Light" : "Dark"}
              </button>
            ))}
          </div>
        </div>
        <dl className="modal-facts">
          <div>
            <dt>Engine bridge</dt>
            <dd>{adapterKind === "tauri" ? "TAURI / CT-ENGINE" : "BROWSER PREVIEW"}</dd>
          </div>
          <div>
            <dt>Canvas</dt>
            <dd>Fixed 1920 × 1080 design canvas, scaled as one unit</dd>
          </div>
          <div>
            <dt>Safety model</dt>
            <dd>Engine-owned scan state · device fault protection · preflight then HOME</dd>
          </div>
        </dl>
      </div>
    );
  } else {
    body = (
      <div className="modal-stack">
        <p className="modal-note">
          Reconnect requested for every device; the engine reports the result in the log stream below.
        </p>
        <dl className="modal-facts">
          {ws.devices.map((device) => (
            <div key={device.id}>
              <dt>{device.name}</dt>
              <dd>
                <span className={`tone-${device.tone}`}>{device.word}</span> · {device.spec}
              </dd>
            </div>
          ))}
          <div>
            <dt>Link summary</dt>
            <dd>{ws.onlineSummary}</dd>
          </div>
        </dl>
      </div>
    );
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <section
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-labelledby="menu-dialog-title"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="modal-card__head">
          <LineIcon name={DIALOG_ICONS[kind]} size={15} className="panel__icon" />
          <h2 id="menu-dialog-title">{DIALOG_TITLES[kind]}</h2>
          <button type="button" className="modal-card__close" onClick={onClose} aria-label="Close dialog">
            ×
          </button>
        </header>
        <div className="modal-card__body">{body}</div>
      </section>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* App                                                                 */
/* ------------------------------------------------------------------ */

export function App() {
  const { adapterKind, snapshot, busy, error, transportError, dispatch } = useEngine();
  const [theme, setTheme] = useTheme();
  const [dialog, setDialog] = useState<DialogKind | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [setupDraftInvalid, setSetupDraftInvalid] = useState(true);
  const [setupDraftError, setSetupDraftError] = useState<string | null>(null);
  const [canvasLayout, setCanvasLayout] = useState<CanvasLayout>(() =>
    computeCanvasLayout(window.innerWidth, window.innerHeight),
  );
  const ws: WorkstationView | undefined = snapshot?.workstation;
  const [setupSync, setSetupSync] = useState(0);
  const setupPendingSync = useRef(false);
  const setupSignature = ws
    ? [ws.scanSetup.taskId, ws.scanSetup.savePath, ws.scanSetup.projectionCount,
       ws.scanSetup.exposureMs, ws.scanSetup.maxXraySec].join("|")
    : "";
  const lastSetupSignature = useRef(setupSignature);

  // Menu commands mutate the setup through the engine; refill the panel drafts
  // only once the engine snapshot actually changed.
  useEffect(() => {
    if (setupSignature === lastSetupSignature.current) return;
    lastSetupSignature.current = setupSignature;
    if (!setupPendingSync.current) return;
    setupPendingSync.current = false;
    setSetupSync((value) => value + 1);
  }, [setupSignature]);

  useEffect(() => {
    const updateCanvasLayout = (): void => {
      setCanvasLayout(computeCanvasLayout(window.innerWidth, window.innerHeight));
    };
    updateCanvasLayout();
    window.addEventListener("resize", updateCanvasLayout);
    return () => window.removeEventListener("resize", updateCanvasLayout);
  }, []);

  useEffect(() => {
    if (ws) document.documentElement.dataset.state = ws.dataState;
  }, [ws]);

  const availability: MenuAvailability = useMemo(() => {
    const ok = { available: true, reason: null };
    const blocked = (reason: string) => ({ available: false, reason });
    const setup = ws?.scanSetup;
    const running = ws?.dataState === "scanning" || ws?.dataState === "paused";
    const configured = Boolean(
      setup &&
        setup.taskId.trim() &&
        setup.savePath.trim() &&
        setup.projectionCount > 0 &&
        setup.exposureMs > 0 &&
        setup.maxXraySec > 0,
    );
    const desktop = adapterKind === "tauri";
    const faulted = ws?.dataState === "fault";
    const entries: MenuAvailability = {
      "file.newTask": running ? blocked("scan active") : ok,
      "file.openImageFolder": desktop
        ? running
          ? blocked("scan active")
          : ok
        : blocked("desktop only"),
      "file.openLastResult": setup?.savePath.trim() ? ok : blocked("no save path"),
      "file.exportLog": desktop ? ok : blocked("desktop only"),
      // The engine owns scan state; it has no undo history to walk back.
      "edit.undo": blocked("engine has no undo stack"),
      "edit.redo": blocked("engine has no undo stack"),
      "edit.resetParameters": running ? blocked("scan active") : ok,
      "edit.preferences": ok,
      "tools.runPreflight": configured
        ? ok
        : blocked("setup incomplete"),
      "tools.homeAllAxes": faulted
        ? blocked("run Preflight to revalidate device safety")
        : snapshot?.preflightPassed
          ? ok
          : blocked("preflight required"),
      "tools.restorePrevious": ws?.dock.restore
        ? ok
        : blocked(ws?.dock.restoreReason || "no checkpoint"),
      "tools.deviceDiagnostics": ok,
      "help.userGuide": ok,
      "help.safetyNotes": ok,
      "help.about": ok,
    };
    if (transportError || busy) {
      for (const id of ["file.newTask", "file.openImageFolder", "edit.resetParameters", "tools.runPreflight", "tools.homeAllAxes", "tools.restorePrevious"] as const) {
        entries[id] = blocked(transportError ? "control service unavailable" : "command in progress");
      }
    }
    if (setupDraftInvalid) {
      for (const id of ["tools.runPreflight", "tools.homeAllAxes", "tools.restorePrevious"] as const) {
        entries[id] = blocked("complete valid scan parameters");
      }
    }
    return entries;
  }, [adapterKind, snapshot?.preflightPassed, ws, transportError, busy, setupDraftInvalid]);

  const handleMenuAction = useCallback(
    async (id: MenuActionId): Promise<void> => {
      if (!ws) return;
      setActionError(null);
      try {
        switch (id) {
          case "file.newTask": {
            const now = new Date();
            const p = (value: number): string => String(value).padStart(2, "0");
            const stamp =
              `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}` +
              `-${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}`;
            setupPendingSync.current = true;
            // The engine invalidates preflight and HOME on every setup change.
            await dispatch({ type: "update_scan_setup", setup: { taskId: `scan-${stamp}` } });
            break;
          }
          case "file.openImageFolder": {
            const selected = await chooseImageDirectory(ws.scanSetup.savePath);
            if (!selected) break;
            // Refill the Scan Parameters drafts from the engine so the Save Path
            // field shows the folder the picker returned.
            setupPendingSync.current = true;
            await dispatch({ type: "update_scan_setup", setup: { savePath: selected } });
            await revealDirectory(selected);
            break;
          }
          case "file.openLastResult":
            await revealDirectory(ws.scanSetup.savePath);
            break;
          case "file.exportLog": {
            const name = `session-${ws.scanSetup.taskId.trim() || "log"}.log`;
            await exportSessionLog(name, sessionLogText(ws, adapterKind));
            break;
          }
          case "edit.resetParameters":
            setupPendingSync.current = true;
            await dispatch({ type: "update_scan_setup", setup: { ...DEFAULT_SCAN_SETUP } });
            break;
          case "edit.undo":
          case "edit.redo":
            // The engine owns scan state and exposes no undo history, so these
            // entries stay permanently disabled (see `availability`).
            break;
          case "edit.preferences":
            setDialog("preferences");
            break;
          case "tools.runPreflight":
            await dispatch({ type: "preflight" });
            break;
          case "tools.homeAllAxes":
            await dispatch({ type: "home" });
            break;
          case "tools.restorePrevious":
            await dispatch({ type: "restore_previous" });
            break;
          case "tools.deviceDiagnostics":
            for (const device of ["xray", "turntable", "camera"] as const) {
              await dispatch({ type: "retry_device", device });
            }
            setDialog("diagnostics");
            break;
          case "help.userGuide":
            setDialog("guide");
            break;
          case "help.safetyNotes":
            setDialog("safety");
            break;
          case "help.about":
            setDialog("about");
            break;
          default:
            break;
        }
      } catch (reason: unknown) {
        setActionError(reason instanceof Error ? reason.message : String(reason));
      }
    },
    [adapterKind, dispatch, ws],
  );

  useEffect(() => {
    if (!dialog) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") setDialog(null);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [dialog]);

  if (!snapshot) {
    return (
      <main className="boot-screen">
        <strong>Micro-CT Workstation</strong>
        <span>{error ? `Engine unavailable · ${error}` : "Starting ct-engine control chain…"}</span>
      </main>
    );
  }

  if (!ws) {
    return (
      <main className="boot-screen boot-screen--error">
        <strong>Engine protocol unavailable</strong>
        <span>{error ?? "The engine returned no workstation view. Restart the desktop application."}</span>
      </main>
    );
  }

  return (
    <main className="viewport-shell">
      <div
        className="design-canvas"
        style={{ zoom: canvasLayout.zoom, width: `${canvasLayout.designWidth}px`, height: `${canvasLayout.designHeight}px` }}
      >
        <MenuBar
          availability={availability}
          onAction={(id) => void handleMenuAction(id)}
        />
        <div className="app-divider" />
        {error || actionError || setupDraftError ? (
          <div className="error-toast" role="alert">
            <LineIcon name="warning" size={13} />
            {setupDraftError ?? actionError ?? error}
          </div>
        ) : null}
        {/* Main row and log row share one grid: the right column reaches the
            bottom of the workspace and the console covers left + centre only. */}
        <section className="workspace-body">
          <section className="main-console">
            <aside className="col">
              <DevicePanel
                ws={ws}
                busy={busy || Boolean(transportError)}
                stale={Boolean(transportError)}
                setupInvalid={setupDraftInvalid}
                hardwareConnected={snapshot.connectionState === "connected"}
                desktopRuntime={adapterKind === "tauri"}
                dispatch={dispatch}
              />
              <ScanParamsPanel
                ws={ws}
                busy={busy || Boolean(transportError)}
                desktopRuntime={adapterKind === "tauri"}
                dispatch={dispatch}
                syncRevision={setupSync}
                onInvalidChange={setSetupDraftInvalid}
                onValidationError={setSetupDraftError}
              />
            </aside>
            <LiveScene snapshot={snapshot} ws={ws} theme={theme} dispatch={dispatch} stale={Boolean(transportError)} setupInvalid={setupDraftInvalid} feedbackId={snapshot.updatedAt} />
          </section>
          <BottomConsole ws={ws} production={adapterKind === "tauri"} />
          <aside className="col col--right">
            <XrayPanel ws={ws} busy={busy || Boolean(transportError)} dispatch={dispatch} stale={Boolean(transportError)} />
            <OperationPanel snapshot={snapshot} ws={ws} stale={Boolean(transportError)} />
          </aside>
        </section>
        <div className="app-divider" />
        <footer className="status-bar">
          <span className="status-bar__left">
            <i className={`status-dot ${toneClass(ws.statusbar.dotTone)}`} aria-hidden="true" />
            <strong>{transportError ? "Control service unavailable · device states unknown" : error || (ws.dataState === "fault" && snapshot.lastError) || ws.dock.playReason || ws.phaseWord}</strong>
          </span>
          <div className="status-bar__right">
            <span>{ws.scanSetup.taskId || "No task selected"}</span>
            {snapshot.mode === "developer_preview" && <span className="badge badge--dev">Offline preview</span>}
            <span className={`badge badge--engine badge--${transportError ? "lost" : "connected"}`}>
              <i aria-hidden="true" />Control service · {transportError ? "Unavailable" : "Ready"}
            </span>
            <div className="theme-toggle" role="group" aria-label="Theme">
              {(["light", "dark"] as const).map(value => (
                <button type="button" key={value} className="theme-toggle__seg" aria-pressed={theme === value} onClick={() => setTheme(value)}>
                  <LineIcon name={value === "light" ? "sun" : "moon"} size={11} />
                  {value === "light" ? "Light" : "Dark"}
                </button>
              ))}
            </div>
          </div>
        </footer>
        {dialog ? (
          <InfoDialog
            kind={dialog}
            ws={ws}
            snapshot={snapshot}
            adapterKind={adapterKind}
            theme={theme}
            onTheme={setTheme}
            onClose={() => setDialog(null)}
          />
        ) : null}
      </div>
    </main>
  );
}

export type { EngineSnapshot };
