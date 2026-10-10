/**
 * Top-level workstation presentation and user-interaction composition.
 * The UI renders complete engine snapshots and translates gestures into domain
 * commands. It does not own production scan progress, device truth, or safety
 * decisions; those remain authoritative in ct-engine.
 */

import { englishDisplayText, unicodeDisplayText } from "./engine/native-labels";
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { computeCanvasLayout, type CanvasLayout } from "./canvas-layout";
import { TurntableBenchDialog } from "./DeviceConsoles";
import { CameraPanel } from "./CameraPanel";
import { TurntablePanel } from "./TurntablePanel";
import { CameraSettingsDialog } from "./CameraSettingsDialog";
import { FailureBanner } from "./FailureBanner";
import { TroubleshootingPane } from "./TroubleshootingPane";
import { createPreparationRequestScope, runScanPreparationRequest, ScanPreparationDialog, type PreparationStage } from "./ScanPreparationDialog";
import { TaskRecoveryDialog } from "./TaskRecoveryDialog";
import { WorkstationMedia, type WorkstationMediaMode } from "./WorkstationMedia";
import { ReconstructionConsole } from "./ReconstructionConsole";
import { useCameraDraft, useDeviceActions } from "./camera-draft";
import { UpdateDialog } from "./UpdateDialog";
import { WorkstationDialog } from "./WorkstationDialog";
import { getApplicationVersion } from "./platform/updater";
import { deriveFailureReport, deviceConnectionReason, manualControlReason } from "./engine/deviceControls";
import { LineIcon, type LineIconName } from "./icons";
import { projectionError, exposureError } from "./scan-input";
import { hasDeviceSessionEverEstablished, useEngine } from "./engine/useEngine";
import {
  chooseImageDirectory,
  chooseScanProjectDirectory,
  exportSessionLog,
  resolveDefaultImageDirectory,
  revealDirectory,
} from "./platform/desktopPaths";
import {
  MENU_GROUPS,
  type MenuActionId,
  type MenuAvailability,
  type TopMenu,
} from "./menuActions";
import { LiveSceneCanvas } from "./scene/LiveSceneCanvas";
import { ReconstructionPreview, type PreviewVolume } from "./scene/ReconstructionPreview";
import { StaticSceneFallback } from "./scene/StaticSceneFallback";
import { useSceneFallback } from "./scene/useSceneFallback";
import type { ViewPreset } from "./scene/types";
import type {
  CardAcquisition,
  DeviceOperation,
  ConsoleLogLine,
  ConsoleFrame,
  DeviceId,
  EngineCommand,
  EngineSnapshot,
  ReconstructionGeometry,
  ReconstructionMethodId,
  ReconstructionOptions,
  RetainedTaskRecord,
  ScanPromptStage,
  WorkstationView,
  IndependentCapturedImage,
  CameraHealth,
  ScanPreparation,
} from "./engine/types";

type Theme = "light" | "dark";
type BottomTab = "aggregate" | "xray" | "nano" | "camera" | "images" | "reconstruction";
type CenterView = WorkstationMediaMode;

/** The control dock is anchored 18px below the top of the live scene and expands to an
 *  82px key row, so its lane ends at roughly 200 design px. The failure surface starts
 *  below that lane, which keeps the safety Stop reachable while the notice is open; the
 *  layer is portalled outside the canvas, so App scales the lane by the canvas zoom. */
const FAILURE_TOP_LANE_DESIGN_PX = 208;

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
    const favicon = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
    if (favicon) favicon.href = theme === "dark" ? "/assets/micro-ct-logo-dark.png" : "/assets/micro-ct-logo.png";
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
        aria-label={englishDisplayText(text)}
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
        >{englishDisplayText(text)}</span>,
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
  version,
  theme,
}: {
  version: string;
  theme: Theme;
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
        <img src={theme === "dark" ? "/assets/micro-ct-logo-dark.png" : "/assets/micro-ct-logo.png"} alt="" draggable={false} />
        <strong>Micro-CT Workstation</strong>
        <span>{version}</span>
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
                        title={englishDisplayText(state.reason ?? entry.label)}
                        onClick={() => {
                          setOpenMenu(null);
                          onAction(entry.id);
                        }}
                      >
                        <LineIcon name={entry.icon} size={13} className="menu-dropdown__icon" />
                        <span>{state.label || entry.label}</span>
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
      title={englishDisplayText(title)}
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
  "engineSessionId", "snapshotSequence", "operationId", "operations", "deviceControls",
  "cameraScanReadiness", "softwareUpdatePreparation", "cameraCaptureDescriptor",
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
  "scanFlow", "reconstruction",
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
const FIRST_START_CARD_KEYS = [
  "destination", "active", "acquisitionStatus", "dataStatus", "recordCount",
  "pendingImportTasks", "currentTaskPendingImport", "importRequired",
];
const FIRST_START_CARD_IMPORT_KEYS = [
  "importStatus", "importOperationId", "importError", "importedRecords", "verifiedRecords",
  "acquiredProjectionCount", "importedProjectionCount", "verifiedProjectionCount",
];
const FIRST_START_CARD_FULL_KEYS = [...FIRST_START_CARD_KEYS, ...FIRST_START_CARD_IMPORT_KEYS];
const CARD_IMPORT_STATUSES = ["idle", "running", "cancelled", "failed", "verified"] as const;
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
  // A disconnected startup has no capture or maintenance transaction. The
  // camera gate explicitly reports the missing session, never readiness.
  if (snapshot.softwareUpdatePreparation !== null || snapshot.cameraCaptureDescriptor !== null
    || !hasExactOwnKeys(snapshot.cameraScanReadiness, ["ready", "issues"])
    || snapshot.cameraScanReadiness.ready !== false
    || !hasExactArrayEntries(snapshot.cameraScanReadiness.issues)
    || snapshot.cameraScanReadiness.issues.length !== 1) return false;
  const sessionIssue = snapshot.cameraScanReadiness.issues[0];
  if (!hasExactOwnKeys(sessionIssue, ["key", "actual", "requirement", "remedy"])
    || sessionIssue.key !== "session" || sessionIssue.actual !== null
    || !isString(sessionIssue.requirement) || !sessionIssue.requirement.trim()
    || !isString(sessionIssue.remedy) || !sessionIssue.remedy.trim()
    || snapshot.deviceControls?.camera?.lastCapturedImage != null) return false;
  if (!isString(snapshot.engineSessionId) || !snapshot.engineSessionId || !Number.isSafeInteger(snapshot.snapshotSequence)
    || snapshot.snapshotSequence < 0 || snapshot.operationId !== null || !Array.isArray(snapshot.operations)
    || snapshot.operations.length !== 0 || !snapshot.deviceControls
    || snapshot.deviceControls.taskLock.locked || snapshot.deviceControls.turntable.connected
    || snapshot.deviceControls.turntable.confirmedAngleDeg !== null
    || snapshot.deviceControls.camera?.connected || snapshot.deviceControls.presets.error
    || !hasExactOwnKeys(snapshot.deviceControls, ["camera", "turntable", "taskLock", "presets", "cameraDraft", "exportJson", "presetComparison"])
    || !hasExactOwnKeys(snapshot.deviceControls.turntable, ["connected", "identity", "status", "confirmedAngleDeg", "benchMode", "hardwareCertified", "reason"])
    || snapshot.deviceControls.turntable.identity !== null || snapshot.deviceControls.turntable.status !== null
    || snapshot.deviceControls.turntable.benchMode || snapshot.deviceControls.turntable.hardwareCertified
    || !hasExactOwnKeys(snapshot.deviceControls.taskLock, ["locked", "reason", "taskId", "savePath", "status", "retained"])
    || snapshot.deviceControls.taskLock.status !== "unlocked"
    || !hasExactOwnKeys(snapshot.deviceControls.presets, ["schemaVersion", "presets", "error"])
    || snapshot.deviceControls.presets.schemaVersion !== 1 || !Array.isArray(snapshot.deviceControls.presets.presets)) return false;
  const hasWorkstation = Object.prototype.hasOwnProperty.call(snapshot, "workstation");
  const rootKeys = hasWorkstation ? [...FIRST_START_SNAPSHOT_KEYS, "workstation"] : FIRST_START_SNAPSHOT_KEYS;
  if (!hasExactOwnKeys(snapshot, rootKeys)) return false;
  // Additive card state: a snapshot captured before the card migration may omit it,
  // but any published value must be complete and consistent.
  if (snapshot.cardAcquisition !== undefined && (
    !(hasExactOwnKeys(snapshot.cardAcquisition, FIRST_START_CARD_KEYS)
      || hasExactOwnKeys(snapshot.cardAcquisition, FIRST_START_CARD_FULL_KEYS))
    || !["cameraCard", "hostRam", "unknown"].includes(snapshot.cardAcquisition.destination)
    || ![snapshot.cardAcquisition.active, snapshot.cardAcquisition.currentTaskPendingImport, snapshot.cardAcquisition.importRequired].every((value) => typeof value === "boolean")
    || ![snapshot.cardAcquisition.recordCount, snapshot.cardAcquisition.pendingImportTasks].every((value) => Number.isInteger(value) && value >= 0)
    || ![snapshot.cardAcquisition.acquisitionStatus, snapshot.cardAcquisition.dataStatus].every((value) => value === null || typeof value === "string"))) return false;
  // The USB-import extension is additive: the engine publishes the base block until
  // its import work lands, and once any import key appears the whole extension must
  // be present and consistent.
  if (snapshot.cardAcquisition !== undefined && snapshot.cardAcquisition.importStatus !== undefined && (
    !hasExactOwnKeys(snapshot.cardAcquisition, FIRST_START_CARD_FULL_KEYS)
    || !CARD_IMPORT_STATUSES.includes(snapshot.cardAcquisition.importStatus)
    || ![snapshot.cardAcquisition.importOperationId, snapshot.cardAcquisition.importError].every((value) => value === null || typeof value === "string")
    || ![snapshot.cardAcquisition.importedRecords, snapshot.cardAcquisition.verifiedRecords,
      snapshot.cardAcquisition.acquiredProjectionCount, snapshot.cardAcquisition.importedProjectionCount,
      snapshot.cardAcquisition.verifiedProjectionCount].every((value) => Number.isInteger(value) && value >= 0))) return false;
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
  if (!hasExactOwnKeys(ws.scanFlow, ["stage", "prompt", "references", "geometry", "checkpointPath"]) ||
    ws.scanFlow.stage !== "idle" || ws.scanFlow.prompt !== null || ws.scanFlow.geometry !== null ||
    (ws.scanFlow.checkpointPath !== null && !isString(ws.scanFlow.checkpointPath)) ||
    !hasExactOwnKeys(ws.scanFlow.references, ["preDark", "preFlat", "postFlat", "postDark", "required"]) ||
    [ws.scanFlow.references.preDark, ws.scanFlow.references.preFlat, ws.scanFlow.references.postFlat, ws.scanFlow.references.postDark].some(value => value !== 0) ||
    ws.scanFlow.references.required !== 10) return false;
  if (!hasExactOwnKeys(ws.reconstruction, ["unlocked", "methods", "selectedMethod", "status", "percent", "message", "cachePath"]) ||
    ws.reconstruction.unlocked !== false || ws.reconstruction.selectedMethod !== null || ws.reconstruction.status !== "idle" ||
    ws.reconstruction.percent !== 0 || ws.reconstruction.cachePath !== null || !isString(ws.reconstruction.message) ||
    !hasExactArrayEntries(ws.reconstruction.methods) ||
    !ws.reconstruction.methods.every(method => hasExactOwnKeys(method, ["id", "label", "enabled", "reason"]) &&
      ["fdk", "sirt", "cgls"].includes(method.id) && isString(method.label) && typeof method.enabled === "boolean" &&
      (method.reason === null || isString(method.reason)))) return false;
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
 * Fresh startup is one neutral disconnected case; confirmed offline project
 * viewing is checked separately below. Every signal that can disprove startup
 * is checked here. Parameter drafts do not count as device readbacks or scan activity,
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

/** Historical image counts are data evidence, not evidence of a device session. */
function isConfirmedOfflineProject(snapshot: EngineSnapshot, ws: WorkstationView): boolean {
  if (!Array.isArray(snapshot.operations) || !Array.isArray(snapshot.logs) || !Array.isArray(ws.consoleLogs)) return false;
  const project = snapshot.operations?.filter(op => op.device === "task_project" && op.kind === "open_project" && op.status === "completed" && !op.cancelRequested && op.deviceSessionId === snapshot.engineSessionId).at(-1);
  const count = snapshot.imageCount;
  if (snapshot.operations?.some(op => !(op.device === "task_project" && op.kind === "open_project"
    || op.device === "task_metadata" && op.kind === "metadata_probe"
    || op.device === "system" && op.kind === "reconstruction_prepare")
    || !["completed", "cancelled", "rejected"].includes(op.status))) return false;
  if (!project || !Number.isInteger(count) || count < 3 || count !== snapshot.parameters.projectionCount
    || snapshot.progress.current !== count || snapshot.progress.percent !== 100 || ws.frames.length !== count
    || ws.progress.captured !== count || ws.progress.total !== count || ws.progress.percent !== 100
    || ws.scanFlow.stage !== "completed" || ws.scanFlow.prompt !== null || ws.scanFlow.geometry?.measurement !== "measured"
    || ws.scanFlow.references.preDark !== 10 || ws.scanFlow.references.preFlat !== 10) return false;
  // Reuse every device/startup check, excluding confirmed historical metadata,
  // reconstruction fields and audited data-only receipts. No state is changed.
  const deviceLogs = snapshot.logs.filter(entry => !(entry.source === "reconstruction" && ["INFO", "PASS"].includes(entry.level)));
  const consoleLogs = ws.consoleLogs.filter(entry => !(entry.source === "reconstruction" && ["INFO", "PASS"].includes(entry.level)));
  const view = { ...ws, consoleLogs, frames: [], progress: { ...ws.progress, captured: 0, percent: 0, barLabel: `0 / ${count} · 0%` },
    scanFlow: { ...ws.scanFlow, stage: "idle" as const, geometry: null, references: { ...ws.scanFlow.references, preDark: 0, preFlat: 0, postDark: 0, postFlat: 0 } },
    reconstruction: { ...ws.reconstruction, unlocked: false, selectedMethod: null, status: "idle" as const, percent: 0, cachePath: null },
    summary: { ...ws.summary, acquisition: "No acquisition" } };
  return hasConsistentFirstStartProjection({ ...snapshot, logs: deviceLogs, imageCount: 0, operationId: null, operations: [],
    progress: { ...snapshot.progress, current: 0, percent: 0 }, workstation: view }, view);
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
  const independentCamera = snapshot.deviceControls?.camera;
  if (snapshot.connectionState === "disconnected" && !stale && snapshot.phase === "idle"
    && snapshot.lastError === null && independentCamera?.connected === true
    && independentCamera.identity?.deviceId && independentCamera.sessionId
    && !independentCamera.isolated && independentCamera.cleanupConfirmed
    && !snapshot.deviceControls.turntable.connected && !snapshot.deviceControls.taskLock.locked
    && !ws.xray.beamOn && !ws.xray.latched && !snapshot.safety.xrayEnabled
    && !["danger", "dangerBold"].includes(ws.safetyBar.tone)) {
    return { state: "Stopped", tone: "muted", active: false,
      detail: "Camera session connected · scan remains locked until turntable and X-ray evidence is verified · unconnected device states remain UNKNOWN" };
  }
  // Exhaustive partition: disconnected is neutral only for a fully verified
  // initial snapshot. No disconnected combination can reach the phase switch or
  // its final ordinary Stopped fallback.
  if (snapshot.connectionState === "disconnected") {
    if (sessionEverEstablished === false && !stale && isConfirmedOfflineProject(snapshot, ws)) {
      return { ...offlineFeedback(), detail: "Offline project view · device states unknown" };
    }
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
        detail: `Ending the active task · pose frozen at ${ws.scene.angleKnown ? angleText : "an unknown reading"} · awaiting confirmed output OFF, worker cleanup and task closure`,
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
        detail: snapshot.deviceControls.taskLock.locked
          ? "Interrupted task retained · use controlled recovery or Stop to end it and retain every file"
          : "Task ended · acquired files retained · prepare a new task, then repeat Preflight and HOME",
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

/* ------------------------------------------------------------------ */
/* Camera-card acquisition and USB import presentation.                */
/* Every value below is read from the engine snapshot; the UI never    */
/* invents an import state, a file count or an import percentage.      */
/* ------------------------------------------------------------------ */

const CARD_DESTINATION_LABELS: Record<CardAcquisition["destination"], string> = {
  cameraCard: "Camera SD card",
  hostRam: "Host RAM",
  unknown: "Unknown",
};

/** One operator-facing state word derived from the engine's two status fields. */
function cardAcquisitionStateWord(card: CardAcquisition): string {
  if (card.importStatus === "verified" || card.dataStatus === "verified") return "Verified · sealed";
  if (card.importStatus === "running" || card.dataStatus === "importing") return "Importing";
  if (card.importStatus === "failed" || card.dataStatus === "import_failed") return "Import failed";
  if (card.importStatus === "cancelled") return "Import cancelled · import required";
  if (card.dataStatus === "pending_import") return "Acquired · pending import";
  switch (card.acquisitionStatus) {
    case "acquired": return "Acquired · pending import";
    case "running": return "Acquiring";
    case "paused": return "Acquisition paused";
    case "prepared": return "Prepared";
    case "stopped": return "Acquisition stopped";
    case "fault": return "Acquisition fault";
    default: return "Unknown";
  }
}

/** The USB transfer owns the camera; single capture, Live View, parameter writes
 *  and a new scan against the same camera stay refused until it settles. */
function cardImportLockReason(card: CardAcquisition | undefined): string | null {
  if (card?.importStatus !== "running") return null;
  return "Camera card import in progress · the USB transfer owns this camera until it is cancelled or verified";
}

/** Why reconstruction and project opening must refuse this data set, or null. */
function cardReconstructionGate(card: CardAcquisition | undefined): string | null {
  if (!card || card.destination !== "cameraCard") return null;
  if (card.importStatus === "verified") return null;
  if (card.importStatus === "running") {
    return "Camera card import is running · reconstruction stays locked until every imported file is verified and the data set is sealed";
  }
  if (card.importStatus === "failed" || card.dataStatus === "import_failed") {
    return "Acquisition complete · import required · the USB import failed, so reconstruction stays locked. Resume the card import and wait for verification.";
  }
  if (card.importStatus === "cancelled") {
    return "Acquisition complete · import required · the USB import was cancelled, so reconstruction stays locked. Resume the card import and wait for verification.";
  }
  if (["prepared", "running", "paused"].includes(String(card.acquisitionStatus))) {
    return "The camera-card acquisition is still in progress · only a verified, sealed data set can be reconstructed";
  }
  return "Acquisition complete · import required · the data is still on the camera SD card, so reconstruction stays locked until the USB import is verified and sealed.";
}

/** Retained tasks the engine still holds with their data on the card. */
function pendingImportTaskRecords(snapshot: EngineSnapshot): RetainedTaskRecord[] {
  return snapshot.deviceControls.taskLock.retained.filter(record => /pending_import/i.test(record.status));
}

/** Compare a picker result with one task directory without trusting either separator. */
function sameTaskDirectory(candidate: string, savePath: string, taskId: string): boolean {
  const normalize = (value: string) => value.trim().replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  const chosen = normalize(candidate);
  return Boolean(chosen) && Boolean(savePath.trim()) && Boolean(taskId.trim())
    && chosen === normalize(`${savePath}/${taskId}`);
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
  // A running camera-card import owns the camera, so this dock must not offer a
  // scan start. Home and Restore stay independent of the card transfer.
  const cardImportReason = cardImportLockReason(snapshot.cardAcquisition);
  const playBlocked = setupBlocked || cardImportReason !== null;
  const announcement = englishDisplayText(`${feedback.state}. ${feedback.detail} Progress ${progressReadout}.`);

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
          title={englishDisplayText(`${feedback.state} · ${feedback.detail}`)}
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
          title={cardImportReason || (setupInvalid ? "Complete valid scan parameters first" : dock.playReason || play.label)}
          disabled={playBlocked || !dock.play}
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
          title="End this task · stop devices, confirm cleanup and retain every file. Use Pause to continue this scan later."
          disabled={false}
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
        aria-label={englishDisplayText(`Console feedback state: ${active}. ${feedback.detail}`)}
      >
        <span className="op-feedback__word">{active}</span>
        <span className="op-feedback__detail">{englishDisplayText(detailLead)}</span>
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

function LiveScene({ snapshot, ws, theme, dispatch, stale, setupInvalid, feedbackId, centerView, volume, previewError, volumeMode, volumeResetRevision, onVolumeAvailable, camera, photo, requestRevision, onCloseMedia, runDevice, pending, liveBlockedReason = null }: { snapshot: EngineSnapshot; ws: WorkstationView; theme: Theme; dispatch: (c: EngineCommand) => void; stale: boolean; setupInvalid: boolean; feedbackId: string; centerView: CenterView; volume: PreviewVolume | null; previewError: string | null; volumeMode: "mip" | "opacity"; volumeResetRevision: number; onVolumeAvailable: (available: boolean) => void; camera: CameraHealth | null; photo: IndependentCapturedImage | null; requestRevision: number; onCloseMedia: () => void; runDevice: (c: EngineCommand) => Promise<EngineSnapshot>; pending: boolean; liveBlockedReason?: string | null }) {
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
        <h2>{centerView === "equipment" ? "Equipment View" : centerView === "photo" ? "Independent Photo" : centerView === "live" ? "Live View" : "Reconstruction Preview"}</h2>
        <HelpTip text={centerView === "equipment" ? `Read-only geometry view. ${setupInvalid ? "Complete valid scan parameters first." : ws.dock.playReason || "Ready for the next operation."}` : "Three linked slices and a rotatable volume from the saved reconstruction."} />
        <span className="menu-spacer" />
        {centerView === "equipment" && (["iso", "front", "top"] as const).map((preset) => (
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
      <div className={`live-scene ${centerView === "reconstruction" ? "live-scene--reconstruction" : ""}`}>
        {/* Keep the live canvas and its orbit state mounted across result navigation. */}
        <div className="equipment-scene-layer" aria-hidden={centerView !== "equipment"} style={{ visibility: centerView === "equipment" ? "visible" : "hidden", pointerEvents: centerView === "equipment" ? "auto" : "none" }}>
          {fallback.reason ? (
            <StaticSceneFallback view={sceneView} reason={fallback.reason} />
          ) : (
            <LiveSceneCanvas view={sceneView} status={feedback} preset={viewPreset} presetRevision={presetRevision} onContextLost={fallback.setContextLost} />
          )}
        </div>
        {centerView === "reconstruction" && (
          volume ? <ReconstructionPreview key={ws.reconstruction.cachePath || "preview"} volume={volume} mode={volumeMode} resetRevision={volumeResetRevision} onVolumeAvailable={onVolumeAvailable} /> : <div className="recon-empty" role="status">{previewError ? reconstructionMessageEn(previewError, "failed") : ws.reconstruction.status === "running" ? "Reconstructing…" : "Waiting for result data"}</div>
        )}
        {centerView === "equipment" && <><span className="live-indicator">
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
        </>}
        <WorkstationMedia mode={centerView} camera={camera} photo={photo} requestRevision={requestRevision} onClose={onCloseMedia} run={runDevice} controlReason={manualControlReason(snapshot, stale) || (pending ? "Operation pending" : null)} readReason={deviceConnectionReason(snapshot, stale) || (pending ? "Operation pending" : null)} liveBlockedReason={liveBlockedReason} />
        {centerView === "equipment" && <ControlDock
          snapshot={snapshot}
          ws={ws}
          dispatch={dispatch}
          stale={stale}
          setupInvalid={setupInvalid}
        />}
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
    <section className="panel xray-panel" id="xray-panel">
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
          <span className="xray-channel__label">Voltage · kV <HelpTip align="end" text="SET is your draft. Applied is the last device-confirmed sent value, available before beam ON. Measured is output telemetry. A dash means unconfirmed or unavailable." /></span>
          <div className="xray-channel__row">
            <div className="xray-channel__control">
              <label className="xray-channel__draft"><span className="xray-channel__key">SET</span>
              <input
                className="xray-channel__well"
                aria-label="Voltage setpoint in kV"
                value={kvDraft}
                disabled={busy || !ws.xray.setpointControlsEnabled}
                inputMode="decimal"
                onChange={(event) => setKvDraft(event.target.value)}
              />
              </label>
              <span className="xray-channel__monitor xray-channel__applied" aria-label="Applied voltage">Applied <b>{measured(ws.xray.connected && ws.xray.voltageConfirmed ? ws.xray.setKv : null, stale)}</b></span>
              <span className="xray-channel__monitor">Measured <b>{measured(ws.xray.monKv, stale)}</b></span>
            </div>
            <button type="button" className="send-btn" disabled={busy || !ws.xray.setpointControlsEnabled || !Number.isFinite(Number(kvDraft))} onClick={() => void dispatch({ type: "send_voltage", kv: Number(kvDraft) })}>
              SEND V
            </button>
          </div>
        </div>
        <div className="xray-channel">
          <span className="xray-channel__label">Current · µA <HelpTip align="end" text="SET is your draft. Applied is the last device-confirmed sent value, available before beam ON. Measured is output telemetry. A dash means unconfirmed or unavailable." /></span>
          <div className="xray-channel__row">
            <div className="xray-channel__control">
              <label className="xray-channel__draft"><span className="xray-channel__key">SET</span>
              <input
                className="xray-channel__well"
                value={uaDraft}
                aria-label="Current setpoint in microamps"
                disabled={busy || !ws.xray.setpointControlsEnabled}
                inputMode="decimal"
                onChange={(event) => setUaDraft(event.target.value)}
              />
              </label>
              <span className="xray-channel__monitor xray-channel__applied" aria-label="Applied current">Applied <b>{measured(ws.xray.connected && ws.xray.currentConfirmed ? ws.xray.setUa : null, stale)}</b></span>
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
        disabled={beamState === "off" && (busy || ws.dataState === "fault" || !ws.xray.manualControlsEnabled)}
        title={faultLocked ? "Device fault latched · inspect the log, then repeat Preflight and HOME" : scanLocked ? "The scan owns X-ray output" : !ws.xray.connected ? "Connect the X-ray source first" : busy ? "Wait for the current command" : "Manual X-ray output control"}
        onClick={() => void dispatch({ type: beamState === "off" ? "xray_toggle" : "xray_off" })}
      >
        {beamState === "off" ? "Xray Enable" : "Xray OFF"}
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

function OperationPanel({ snapshot, ws, stale, onImport }: { snapshot: EngineSnapshot; ws: WorkstationView; stale: boolean; onImport: () => void }) {
  const feedback = deriveConsoleFeedback(snapshot, ws, stale, hasDeviceSessionEverEstablished());
  const card = snapshot.cardAcquisition;
  // Card state is additive: an older snapshot without the block renders no row.
  const cardState = card ? cardAcquisitionStateWord(card) : null;
  const canImport = Boolean(card && (card.currentTaskPendingImport || card.pendingImportTasks > 0))
    && card?.importStatus !== "running" && !stale;
  return (
    <section className="panel operation-panel">
      <div className="panel__header">
        <LineIcon name="gauge" size={15} className="panel__icon" />
        <h2>Operation Status</h2>
        <span className={`chip chip--${stale ? "muted" : ws.phaseTone}`}>{stale ? "UNKNOWN" : ws.phaseWord}</span>
      </div>
      <StateFeedback feedback={feedback} stale={stale} />
      {card && <div className="op-card">
        <div className="op-card__row">
          <span className="op-card__tag" title="Where this acquisition stores its images">CARD · {CARD_DESTINATION_LABELS[card.destination].toUpperCase()}</span>
          <strong role="status">{stale ? "UNKNOWN" : cardState}</strong>
          <button type="button" disabled={!canImport} title={stale ? "Control service unavailable" : card.importStatus === "running" ? "Import already running" : card.currentTaskPendingImport || card.pendingImportTasks > 0 ? "Open the USB card import and verification controls" : "No task is waiting for a card import"} onClick={onImport}>Card import…</button>
        </div>
        <div className="op-card__counts">
          <span>Pending imports <b>{stale ? "—" : String(card.pendingImportTasks)}</b></span>
          <span>Acquired <b>{stale ? "—" : String(card.acquiredProjectionCount)}</b></span>
          <span>Imported <b>{stale ? "—" : String(card.importedProjectionCount)}</b></span>
          <span>Verified <b>{stale ? "—" : String(card.verifiedProjectionCount)}</b></span>
        </div>
      </div>}
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
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* Camera card import dialog (overlay; no fixed panel changes)          */
/* ------------------------------------------------------------------ */

/**
 * One explicit operator entry for the USB card import. It starts/resumes and
 * cancels the engine's import and renders only engine-reported progress; the
 * preview and a task without a card profile never reach this state.
 */
function CardImportDialog({ snapshot, pending, stale, run, onClose, onStop }: { snapshot: EngineSnapshot; pending: boolean; stale: boolean; run: (command: EngineCommand) => void; onClose: () => void; onStop: () => void }) {
  const card = snapshot.cardAcquisition;
  const retained = pendingImportTaskRecords(snapshot);
  const defaultTaskId = retained.find(record => record.parameters.taskId === snapshot.parameters.taskId)?.parameters.taskId
    ?? (card?.currentTaskPendingImport ? snapshot.parameters.taskId : retained[0]?.parameters.taskId ?? snapshot.parameters.taskId);
  const [taskId, setTaskId] = useState(defaultTaskId);
  useEffect(() => { setTaskId(defaultTaskId); }, [defaultTaskId]);
  const operation = card?.importOperationId
    ? snapshot.operations.find(item => item.operationId === card.importOperationId) ?? null
    : null;
  const progress = operation?.progress;
  const running = card?.importStatus === "running";
  const verified = card?.importStatus === "verified";
  const readyToImport = Boolean(card) && !running && !pending && !stale
    && (card!.currentTaskPendingImport || card!.pendingImportTasks > 0 || retained.length > 0);
  const determinate = Boolean(progress && progress.totalBytes > 0);
  const percent = determinate ? Math.max(0, Math.min(100, progress!.percent)) : null;
  const eta = progress && progress.estimatedRemainingSec != null
    ? `About ${Math.ceil(progress.estimatedRemainingSec / 10) * 10}s remaining`
    : "Estimating remaining time…";
  const statusLine = !card ? "No camera-card acquisition state is published by the engine"
    : verified ? "Import verified · every imported file is sealed and reconstruction is unlocked"
    : running ? "Acquisition complete · transferring via USB"
    : "Acquisition complete · import required";
  const inProgressFile = progress && progress.totalFiles > 0
    ? Math.min(progress.verifiedFiles + 1, progress.totalFiles)
    : null;
  return <WorkstationDialog title="Camera Card Import" onClose={onClose} onStop={onStop}>
    <p className={`card-import__status ${verified ? "is-verified" : running ? "is-running" : "is-pending"}`} role="status">{statusLine}</p>
    <dl className="modal-facts">
      <div><dt>Destination</dt><dd>{card ? CARD_DESTINATION_LABELS[card.destination] : "UNKNOWN"}</dd></div>
      <div><dt>Acquisition state</dt><dd>{card && !stale ? cardAcquisitionStateWord(card) : "UNKNOWN"}</dd></div>
      <div><dt>Pending import tasks</dt><dd>{card && !stale ? card.pendingImportTasks : "UNKNOWN"}</dd></div>
      <div><dt>Acquired projections</dt><dd>{card && !stale ? card.acquiredProjectionCount : "UNKNOWN"}</dd></div>
      <div><dt>Imported projections</dt><dd>{card && !stale ? card.importedProjectionCount : "UNKNOWN"}</dd></div>
      <div><dt>Verified projections</dt><dd>{card && !stale ? card.verifiedProjectionCount : "UNKNOWN"}</dd></div>
      <div><dt>Camera objects imported / recorded</dt><dd>{card && !stale ? `${card.importedRecords} / ${card.recordCount}` : "UNKNOWN"}</dd></div>
      <div><dt>Camera objects verified</dt><dd>{card && !stale ? card.verifiedRecords : "UNKNOWN"}</dd></div>
    </dl>
    {running && <div className="card-import__progress">
      <div className="recon-workspace__load-summary">
        <span>{operation ? englishDisplayText(`USB transfer · ${operation.stage}`) : "Waiting for the engine's transfer operation…"}</span>
        <b>{determinate ? `${percent}%` : "—"}</b>
      </div>
      <div className={`recon-workspace__progress-track${determinate ? "" : " is-indeterminate"}`} role="progressbar" aria-label="Camera card USB import progress" aria-valuenow={percent ?? undefined} aria-valuemin={0} aria-valuemax={100} aria-valuetext={determinate ? `${percent}% of file data transferred` : "Preparing transfer"}>
        <i style={{ width: determinate ? `${percent}%` : undefined }} />
      </div>
      <div className="recon-workspace__load-summary">
        <span>{progress && progress.totalFiles > 0
          ? `${progress.verifiedFiles} / ${progress.totalFiles} files done${inProgressFile && progress.verifiedFiles < progress.totalFiles ? ` · file ${inProgressFile} in progress` : ""}`
          : "File counts are not yet reported by the engine"}</span>
        <b role="status">{determinate ? eta : "—"}</b>
      </div>
      {determinate && <p className="card-import__note">{progress!.processedBytes} / {progress!.totalBytes} bytes read by the engine</p>}
    </div>}
    {card?.importError && <p className="card-import__error" role="alert">{englishDisplayText(card.importError)}</p>}
    {retained.length > 1 && <label className="card-import__task">Task waiting for import
      <select aria-label="Task waiting for a camera card import" value={taskId} disabled={running || pending} onChange={event => setTaskId(event.target.value)}>
        {retained.map(record => <option key={`${record.parameters.savePath}/${record.parameters.taskId}`} value={record.parameters.taskId}>{unicodeDisplayText(record.parameters.taskId)}</option>)}
      </select>
    </label>}
    <div className="permanent-actions">
      <button className="primary" disabled={!readyToImport} title={stale ? "Control service unavailable" : running ? "An import is already running" : pending ? "Operation pending" : readyToImport ? "Start or resume the USB import for the selected task" : "No task is waiting for a card import"} onClick={() => run({ type: "start_card_import", taskId })}>{card?.importStatus === "failed" || card?.importStatus === "cancelled" ? "Resume USB import" : "Start USB import"}</button>
      <button disabled={!running || stale} title={running ? "Stop the USB transfer; every verified file and the acquisition ledger are kept" : "No import is running"} onClick={() => run({ type: "cancel_card_import" })}>Cancel import</button>
    </div>
    <p className="card-import__note">The camera card keeps every original file. Only files the engine imports and verifies are sealed for reconstruction; an unverified task is never shown as completed or reconstructable.</p>
    {!card && <p className="card-import__error" role="alert">This engine build publishes no camera-card acquisition state.</p>}
  </WorkstationDialog>;
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
  { id: "reconstruction", label: "Reconstruction", icon: "cube" },
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
            {entry.level} · {englishDisplayText(entry.source)}
          </span>
          <span className="log-line__msg">{englishDisplayText(entry.message)}</span>
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
  return <div className="image-tile image-tile--captured" ref={element} title={englishDisplayText(failure || frame.fileName)}>
    <div className="image-tile__image">
      {url ? <img src={url} alt={`Camera projection ${frame.index}`} /> : <span>{failure ? "Preview unavailable" : production ? "Loading image…" : "Offline preview"}</span>}
    </div>
    <div className="image-tile__caption"><strong>VIEW {String(frame.index).padStart(2, "0")}</strong><span>{frame.angleDeg.toFixed(2)}° · {frame.exposureMs} ms</span></div>
  </div>;
}


function reconstructionMessageEn(message: string, status: WorkstationView["reconstruction"]["status"]): string {
  const known: Record<string, string> = {
    "\u51c6\u5907\u91cd\u6784": "Preparing reconstruction",
    "\u6838\u5bf9\u626b\u63cf\u6570\u636e": "Validating scan data",
    "\u5904\u7406\u6821\u6b63\u5e27": "Correcting reference frames",
    "\u4fdd\u5b58\u7ed3\u679c": "Saving result",
    "\u91cd\u6784\u5b8c\u6210": "Reconstruction complete",
    "\u91cd\u6784\u5df2\u7f13\u5b58": "Verified result loaded from cache",
    "GPU \u5931\u8d25，\u6539\u7528 CPU \u91cd\u7b97": "GPU failed; restarting on CPU",
    "GPU \u8bfb\u53d6\u5931\u8d25，\u6539\u7528 CPU \u91cd\u7b97": "GPU readback failed; restarting on CPU",
  };
  if (known[message]) return known[message];
  const views = /^\u91cd\u6784 (\d+)\/(\d+)$/.exec(message);
  if (views) return `Processed ${views[1]} / ${views[2]} views`;
  if (/[\u3400-\u9fff]/u.test(message)) return status === "failed" ? "Reconstruction failed; check the scan data and compute environment" : "Reconstruction status updated";
  return message;
}

function BottomConsole({ tab, onTab, ws, card, projectOperation, openingProject, production, centerView, volumeReady, volume3DAvailable, busy, stale, volumeMode, onVolumeMode, onResetVolume, onOpenReconstruction, onShowEquipment, onStartReconstruction, onCardImport, cardGateReason }: { tab: BottomTab; onTab: (tab: BottomTab) => void; ws: WorkstationView; card?: CardAcquisition; projectOperation: DeviceOperation | null; openingProject: boolean; production: boolean; centerView: CenterView; volumeReady: boolean; volume3DAvailable: boolean; busy: boolean; stale: boolean; volumeMode: "mip" | "opacity"; onVolumeMode: (mode: "mip" | "opacity") => void; onResetVolume: () => void; onOpenReconstruction: () => void; onShowEquipment: () => void; onStartReconstruction: (method: ReconstructionMethodId, options: ReconstructionOptions) => void; onCardImport: () => void; cardGateReason: string | null }) {

  const [follow, setFollow] = useState(true);
  const logs = ws.consoleLogs;
  const frames = useMemo(() => [...ws.frames].sort((a, b) => a.index - b.index), [ws.frames]);
  const filtered = useMemo(() => {
    if (tab === "aggregate" || tab === "images" || tab === "reconstruction") return logs;
    return logs.filter(entry => entry.source === tab);
  }, [logs, tab]);
  const counts = useMemo(() => ({
    aggregate: logs.length,
    xray: logs.filter(entry => entry.source === "xray").length,
    nano: logs.filter(entry => entry.source === "nano").length,
    camera: logs.filter(entry => entry.source === "camera").length,
    images: ws.frames.length,
    reconstruction: ws.reconstruction.status === "completed" ? 1 : 0,
  }), [logs, ws.frames.length, ws.reconstruction.status]);
  // While the acquisition data is still on the card, only frames the engine has
  // already imported and verified may be requested and shown. A frame without a
  // verified local file is never fetched and no thumbnail is fabricated from
  // Live View or any other live source.
  const cardPending = card?.destination === "cameraCard" && card.importStatus !== "verified";
  const unverifiedFrames = cardPending ? frames.filter(frame => frame.index > card!.verifiedProjectionCount) : [];
  const visibleFrames = cardPending ? frames.filter(frame => frame.index <= card!.verifiedProjectionCount) : frames;
  const header = tab === "images"
    ? `CAPTURED IMAGES · ${ws.scanSetup.taskId} · ${ws.progress.captured} / ${ws.progress.total} VIEWS · ${ws.scanSetup.angleStepDeg.toFixed(2)}° STEP · ${ws.scanSetup.exposureMs} ms`
    : tab === "aggregate" ? "SESSION EVENTS · ALL SOURCES"
    : `${tab === "xray" ? "X-RAY" : tab === "nano" ? "TURNTABLE" : "CAMERA"} EVENTS`;

  return <section className="console">
    <div className="console__tabs" role="tablist" aria-label="Logs and image preview">
      {bottomTabs.map(item => <button key={item.id} type="button" role="tab" aria-selected={tab === item.id} className="console__tab" onClick={() => onTab(item.id)}>
        <LineIcon name={item.icon} size={13} className="console__tab-icon" /><em>{item.label}</em><span>{counts[item.id]}</span>
      </button>)}
    </div>
    <div className="console__body" role="tabpanel">
      <div className="recon-workspace-mount" hidden={tab !== "reconstruction"}><ReconstructionConsole ws={ws} projectOperation={projectOperation} openingProject={openingProject} centerView={centerView} volumeReady={volumeReady} volume3DAvailable={volume3DAvailable} busy={busy} stale={stale} volumeMode={volumeMode} onVolumeMode={onVolumeMode} onResetVolume={onResetVolume} onOpenReconstruction={onOpenReconstruction} onShowEquipment={onShowEquipment} onStartReconstruction={onStartReconstruction} cardGateReason={cardGateReason} /></div>
      {tab !== "reconstruction" && <>
        <div className="console__head"><span>{header}</span>{tab !== "images" && <button type="button" className="follow-toggle" aria-pressed={follow} onClick={() => setFollow(!follow)}>{follow ? "Following latest" : "Resume live log"}</button>}</div>
        {tab === "images" ? <div className="image-strip">
          {cardPending && <div className="image-strip__card-note" role="status">
            <strong>Images are on the camera SD card</strong>
            <span>{`Acquired ${card!.acquiredProjectionCount} · imported ${card!.importedProjectionCount} · verified ${card!.verifiedProjectionCount}`}</span>
            {unverifiedFrames.length > 0 && <span>{`${unverifiedFrames.length} captured view${unverifiedFrames.length === 1 ? "" : "s"} have no verified local file yet and show no thumbnail.`}</span>}
            <button type="button" disabled={card!.importStatus === "running" || stale} onClick={onCardImport}>{card!.importStatus === "running" ? "Import running…" : "Card import…"}</button>
          </div>}
          {visibleFrames.length === 0 && !cardPending && <div className="image-strip__empty">No projections yet. Configure a scan task to view its image sequence.</div>}
          {visibleFrames.length === 0 && cardPending && unverifiedFrames.length === 0 && <div className="image-strip__empty">No projection is registered locally yet. Import the card data to preview it.</div>}
          {visibleFrames.map(frame => <FrameThumbnail frame={frame} production={production} key={`${ws.scanSetup.taskId}:${frame.index}:${frame.sha256 ?? "preview"}`} />)}
        </div> : <LogLines logs={filtered} follow={follow} onFollow={setFollow} />}
      </>}
    </div>
  </section>;
}

/* ------------------------------------------------------------------ */
/* Menu dialogs (overlay on the fixed design canvas, layout unchanged) */
/* ------------------------------------------------------------------ */

type DialogKind = "guide" | "safety" | "troubleshooting" | "about" | "preferences" | "diagnostics";

const DIALOG_TITLES: Record<DialogKind, string> = {
  guide: "User Guide",
  safety: "Safety Notes",
  troubleshooting: "Troubleshooting",
  about: "About Micro-CT Workstation",
  preferences: "Preferences",
  diagnostics: "Device Diagnostics",
};

const DIALOG_ICONS: Record<DialogKind, LineIconName> = {
  guide: "book",
  safety: "warning",
  troubleshooting: "pulse",
  about: "info",
  preferences: "sliders",
  diagnostics: "pulse",
};



function InfoDialog({
  kind,
  version,
  ws,
  snapshot,
  adapterKind,
  theme,
  troubleshootingGroup,
  onTheme,
  onClose,
}: {
  kind: DialogKind;
  version: string;
  ws: WorkstationView | null;
  snapshot: EngineSnapshot | null;
  adapterKind: "developer_preview" | "tauri";
  theme: Theme;
  troubleshootingGroup?: string;
  onTheme: (theme: Theme) => void;
  onClose: () => void;
}) {
  let body: ReactNode = null;
  if (kind === "guide") {
    body = (
      <ol className="modal-list">
        <li><strong>Connect each device independently.</strong> Use its permanent panel. Choose the camera explicitly; read actual parameters and set the common image root.</li>
        <li><strong>Apply camera targets explicitly.</strong> Basic and Advanced Settings share one draft. Apply and verify finishes only after its engine operation and readback.</li>
        <li><strong>Independent photo / Live View.</strong> Neither starts X-ray output. Stop Live View explicitly before a photo. Confirmed photos stay visible until Close; preview errors do not delete the saved NEF.</li>
        <li><strong>Prepare a fixed scan.</strong> Task → Scan Preparation / Preflight opens task ID, projection count, continuous-output limit and all geometry in one window. Resolve every fixed-configuration issue and rerun preflight.</li>
        <li><strong>Acquire.</strong> Establish the required HOME reference, follow dark/flat and sample confirmations, then Start. Pause commits a safe boundary; Stop waits for confirmed cleanup.</li>
        <li><strong>Recover or abandon.</strong> Task → Restore uses frozen settings and verified retained files. Explicit abandonment preserves every file and unlocks only after cleanup confirmation.</li>
        <li><strong>Reconstruct and view.</strong> Reconstruction remains in the bottom tab. Equipment/Result only change display; a result appears after validated data loading.</li>
        <li><strong>Update.</strong> Help → Check Update or Import Signed Offline Update verifies packages. Installation requires separate confirmation and an engine-issued maintenance-ready state.</li>
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
          <strong>{snapshot?.mode === "developer_preview" ? "This preview drives no hardware." : "Native device controls are an offline candidate pending hardware acceptance."}</strong>{" "}
          {snapshot?.modeLabel || "Control service unavailable"}. Independent camera and turntable controls never enable X-ray output. Unknown readback is not OFF; complete acquisition requires engine preflight and verified device evidence.
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
          <dd>{version}</dd>
        </div>
        <div>
          <dt>Engine</dt>
          <dd>{snapshot?.modeLabel || "Control service unavailable"}</dd>
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
          <dd>{snapshot?.connectionState.toUpperCase() || "UNKNOWN"}</dd>
        </div>
        <div>
          <dt>Devices</dt>
          <dd>{ws?.onlineSummary || "UNKNOWN"}</dd>
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
            <dd>1920 × 1080 design baseline; fills the actual client with one control scale</dd>
          </div>
          <div>
            <dt>Safety model</dt>
            <dd>Engine-owned scan state · device fault protection · preflight then HOME</dd>
          </div>
        </dl>
      </div>
    );
  } else if (kind === "troubleshooting") {
    body = <TroubleshootingPane snapshot={snapshot} initialGroup={troubleshootingGroup} />;
  } else {
    body = (
      <div className="modal-stack">
        <p className="modal-note">
          Read-only device and operation evidence. Opening this dialog does not reconnect devices.
        </p>
        <dl className="modal-facts">
          {ws?.devices.map((device) => (
            <div key={device.id}>
              <dt>{device.name}</dt>
              <dd>
                <span className={`tone-${device.tone}`}>{device.word}</span> · {englishDisplayText(device.spec)}
              </dd>
            </div>
          ))}
          <div>
            <dt>Link summary</dt>
            <dd>{ws?.onlineSummary || "UNKNOWN"}</dd>
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

const SCAN_PROMPT_COPY: Record<ScanPromptStage, { title: string; body: string; confirm: string }> = {
  geometry: { title: "Scan Geometry", body: "Enter measured scan geometry.", confirm: "Save and Continue" },
  preReferences: { title: "Reference Frames", body: "Remove the sample. Capture 10 dark frames, then enable X-ray for 10 flat frames.", confirm: "Start Capture" },
  placeSample: { title: "Place Sample", body: "X-ray is off. Place the sample; scanning will enable X-ray after confirmation.", confirm: "Sample Placed" },
};

function ScanStageDialog({ stage, recorded, beamState, busy, onConfirm, onCancel }: {
  stage: ScanPromptStage;
  recorded: ReconstructionGeometry | null;
  beamState: "on" | "off" | "unknown";
  busy: boolean;
  onConfirm: (geometry?: ReconstructionGeometry) => Promise<void>;
  onCancel: () => Promise<void>;
}) {
  const dialogRef = useRef<HTMLElement>(null);
  const [submitting, setSubmitting] = useState(false);
  const locked = busy || submitting;
  const [values, setValues] = useState<Record<string, string>>(() => ({
    sodMm: recorded?.sodMm.toString() ?? "",
    objectToDetectorMm: recorded?.objectToDetectorMm.toString() ?? "",
    detectorWidthMm: recorded?.detectorWidthMm.toString() ?? "",
    centerOffsetXMm: recorded?.centerOffsetXMm.toString() ?? "",
    centerOffsetYMm: recorded?.centerOffsetYMm.toString() ?? "",
    mirrorX: recorded ? String(recorded.mirrorX) : "",
  }));
  const [attempted, setAttempted] = useState(false);
  const geometry = useMemo<ReconstructionGeometry | null>(() => {
    const numbers = ["sodMm", "objectToDetectorMm", "detectorWidthMm", "centerOffsetXMm", "centerOffsetYMm"] as const;
    if (numbers.some(key => values[key].trim() === "" || !Number.isFinite(Number(values[key])))) return null;
    const [sodMm, objectToDetectorMm, detectorWidthMm, centerOffsetXMm, centerOffsetYMm] = numbers.map(key => Number(values[key]));
    if (sodMm <= 0 || objectToDetectorMm <= 0 || detectorWidthMm <= 0 || !["true", "false"].includes(values.mirrorX)) return null;
    return { sodMm, objectToDetectorMm, detectorWidthMm, centerOffsetXMm, centerOffsetYMm,
      rotationDirection: "counterclockwise", mirrorX: values.mirrorX === "true", measurement: "measured" };
  }, [values]);
  const copy = SCAN_PROMPT_COPY[stage];
  const sampleAccessBlocked = stage === "placeSample" && beamState !== "off";
  useEffect(() => {
    const priorFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const initial = dialogRef.current?.querySelector<HTMLInputElement>("input") ?? dialogRef.current?.querySelector<HTMLButtonElement>(".scan-stage-confirm");
    initial?.focus();
    return () => priorFocus?.focus();
  }, []);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); if (!locked) { setSubmitting(true); void onCancel().finally(() => setSubmitting(false)); } }
      if (event.key === "Tab") {
        const controls = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled), select:not(:disabled)") ?? []);
        if (!controls.length) { event.preventDefault(); return; }
        const current = controls.indexOf(document.activeElement as HTMLElement);
        const next = event.shiftKey ? (current <= 0 ? controls.length - 1 : current - 1) : (current >= controls.length - 1 ? 0 : current + 1);
        event.preventDefault(); controls[next]?.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [locked, onCancel]);
  const cancel = () => { if (!locked) { setSubmitting(true); void onCancel().finally(() => setSubmitting(false)); } };
  const field = (key: string, label: string, hint: string) => <label className="scan-geometry-field" key={key}>
    <span>{label}</span><input type="number" inputMode="decimal" step="any" value={values[key]} aria-label={label} placeholder={hint} disabled={locked} onChange={event => setValues(current => ({ ...current, [key]: event.target.value }))} />
  </label>;
  const select = (key: string, label: string, options: Array<[string, string]>) => <label className="scan-geometry-field" key={key}>
    <span>{label}</span><select value={values[key]} disabled={locked} onChange={event => setValues(current => ({ ...current, [key]: event.target.value }))}><option value="">Select</option>{options.map(([value, text]) => <option value={value} key={value}>{text}</option>)}</select>
  </label>;
  return <div className="modal-backdrop scan-stage-backdrop" onClick={cancel}>
    <section ref={dialogRef} className="modal-card scan-stage-card" role="dialog" aria-modal="true" aria-labelledby="scan-stage-title" onClick={event => event.stopPropagation()}>
      <header className="modal-card__head"><LineIcon name="warning" size={15} className="panel__icon" /><h2 id="scan-stage-title">{copy.title}</h2><button type="button" className="modal-card__close" aria-label="Cancel this scan" disabled={locked} onClick={cancel}>×</button></header>
      <div className="modal-card__body"><p className="scan-stage-copy">{copy.body}</p>
        {stage === "geometry" && <div className="scan-geometry-grid">
          {field("sodMm", "Source to axis · mm", "Above 0")}
          {field("objectToDetectorMm", "Axis to detector · mm", "Above 0")}
          {field("detectorWidthMm", "Active detector width · mm", "Above 0")}
          {field("centerOffsetXMm", "Horizontal offset · mm", "May be 0")}
          {field("centerOffsetYMm", "Vertical offset · mm", "May be 0")}
          {select("mirrorX", "Mirror image horizontally", [["false", "No"], ["true", "Yes"]])}
        </div>}
        {attempted && stage === "geometry" && !geometry && <p className="scan-stage-error" role="alert">Enter complete, valid geometry values.</p>}
        {sampleAccessBlocked && <p className="scan-stage-error" role="alert">X-ray OFF is unconfirmed. Do not handle the sample.</p>}
        <div className="scan-stage-actions"><button type="button" disabled={locked} onClick={cancel}>Cancel Scan</button><button type="button" className="scan-stage-confirm" disabled={locked || sampleAccessBlocked} onClick={() => { if (stage === "geometry") { setAttempted(true); if (!geometry) return; } setSubmitting(true); void onConfirm(stage === "geometry" ? geometry ?? undefined : undefined).finally(() => setSubmitting(false)); }}>{copy.confirm}</button></div>
      </div>
    </section>
  </div>;
}

/* ------------------------------------------------------------------ */
/* App                                                                 */
/* ------------------------------------------------------------------ */

export function App() {
  const { adapterKind, snapshot, busy, error, transportError, dispatch: engineDispatch } = useEngine();
  const actions = useDeviceActions(snapshot, engineDispatch);
  const model = useCameraDraft(snapshot);
  const dispatch = useCallback(async (command: EngineCommand): Promise<void> => { await engineDispatch(command); }, [engineDispatch]);
  const [theme, setTheme] = useTheme();
  const [version, setVersion] = useState("Version unavailable");
  const [dialog, setDialog] = useState<DialogKind | null>(null);
  const [auxiliary, updateAuxiliary] = useState<"camera" | "bench" | "prepare" | "recovery" | "card" | null>(null);
  const preparationScope = useRef(createPreparationRequestScope());
  const [preparationStage, setPreparationStage] = useState<PreparationStage>(null);
  const [preparationError, setPreparationError] = useState<string | null>(null);
  const auxiliaryCurrent = useRef(auxiliary); auxiliaryCurrent.current = auxiliary;
  const setAuxiliary = (next: typeof auxiliary, expectedRevision?: number) => {
    if (!preparationScope.current.replace(expectedRevision)) return;
    if (next === "prepare") setPreparationError(null);
    auxiliaryCurrent.current = next;
    updateAuxiliary(next);
  };
  const auxiliaryRevision = preparationScope.current.revision;
  const [update, setUpdate] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [openingProject, setOpeningProject] = useState(false);
  const [troubleshootingGroup, setTroubleshootingGroup] = useState<string | undefined>(undefined);
  // Dismissal is keyed, never a state wipe: the 400 ms poll cannot re-open a hidden
  // notice, while a new attempt or a new operation produces a new key and re-arms it.
  const [dismissedFailures, setDismissedFailures] = useState<ReadonlySet<string>>(() => new Set());
  const [centerView, setCenterView] = useState<CenterView>("equipment");
  const [photo, setPhoto] = useState<IndependentCapturedImage | null>(null);
  const [bottomTab, setBottomTab] = useState<BottomTab>("aggregate");
  const [volumeMode, setVolumeMode] = useState<"mip" | "opacity">("mip");
  const [volumeResetRevision, setVolumeResetRevision] = useState(0);
  const [volume3DAvailable, setVolume3DAvailable] = useState(false);
  const [previewData, setPreviewData] = useState<{ key: string; volume: PreviewVolume } | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [previewRetry, setPreviewRetry] = useState(0);
  const [preparationRevision, setPreparationRevision] = useState(0);
  const [requestRevision, setRequestRevision] = useState(0);
  const presentation = useRef(0);
  const latest = useRef(snapshot); latest.current = snapshot;
  const [canvasLayout, setCanvasLayout] = useState<CanvasLayout>(() => computeCanvasLayout(window.innerWidth, window.innerHeight));
  const ws = snapshot?.workstation;
  const stale = Boolean(transportError);
  const failure = useMemo(
    () => deriveFailureReport(snapshot, error, actionError, actions.errorAttempt),
    [snapshot, error, actionError, actions.errorAttempt],
  );
  const preparationNotice = auxiliary === "prepare" && !stale && failure && failure.kind !== "cleanup_unconfirmed"
    && (failure.stage.startsWith("preflight ·") || (preparationError !== null && failure.kind === "engine"));
  const visibleFailure = failure && !preparationNotice && !dismissedFailures.has(failure.key) ? failure : null;
  const dismissFailure = useCallback((key: string) => {
    setDismissedFailures(previous => { const next = new Set(previous); next.add(key); return next; });
    setActionError(null);
  }, []);
  const openTroubleshooting = useCallback((group: string) => { setTroubleshootingGroup(group); setDialog("troubleshooting"); }, []);
  const pending = busy || actions.pending || preparationStage !== null;
  const preparationContext = useRef<{ engineSessionId: string; taskId: string } | null>(null);
  preparationContext.current = snapshot && ws && !stale ? { engineSessionId: snapshot.engineSessionId, taskId: ws.scanSetup.taskId } : null;
  const previewKey = ws?.reconstruction.status === "completed" && ws.reconstruction.cachePath && ws.reconstruction.selectedMethod ? `${ws.reconstruction.selectedMethod}|${ws.reconstruction.cachePath}` : null;
  const activeVolume = previewKey && previewData?.key === previewKey ? previewData.volume : null;
  const setupInvalid = model.dirty || !snapshot?.cameraScanReadiness?.ready;
  const card = snapshot?.cardAcquisition;
  const cardImportReason = cardImportLockReason(card);
  const cardGateReason = cardReconstructionGate(card);
  const navigate = (view: CenterView) => { presentation.current += 1; setRequestRevision(presentation.current); setCenterView(view); };
  useEffect(() => { let active = true; void getApplicationVersion().then(value => { if (active) setVersion(value); }).catch(() => {}); return () => { active = false; }; }, []);
  useEffect(() => { const resize = () => setCanvasLayout(computeCanvasLayout(window.innerWidth, window.innerHeight)); resize(); window.addEventListener("resize", resize); return () => window.removeEventListener("resize", resize); }, []);
  useEffect(() => { if (ws) document.documentElement.dataset.state = ws.dataState; }, [ws?.dataState]);
  const context = `${snapshot?.engineSessionId || "none"}/${ws?.scanSetup.taskId || "none"}`;
  const previousContext = useRef(context);
  const previousPreparationContext = useRef(preparationContext.current);
  useEffect(() => {
    if (previousContext.current === context) return;
    const previous = previousPreparationContext.current;
    previousPreparationContext.current = preparationContext.current;
    previousContext.current = context; navigate("equipment"); setPhoto(null);
    if (preparationScope.current.acceptsTaskTransition(previous, preparationContext.current)) return;
    if (preparationScope.current.acceptsContext(preparationContext.current)) return;
    preparationScope.current.invalidate(); setPreparationStage(null); setAuxiliary(null);
  }, [context]);
  useEffect(() => {
    if (!stale) return;
    preparationScope.current.invalidate(); setPreparationStage(null); setAuxiliary(null);
  }, [stale]);
  useEffect(() => () => { preparationScope.current.invalidate(); }, []);
  useEffect(() => {
    if (centerView === "live" && (cardImportReason !== null || !snapshot?.deviceControls.camera?.liveView.active || !snapshot.deviceControls.camera.connected || snapshot.deviceControls.camera.isolated || !snapshot.deviceControls.camera.cleanupConfirmed || stale)) navigate("equipment");
  }, [snapshot?.deviceControls.camera?.liveView.active, snapshot?.deviceControls.camera?.connected, snapshot?.deviceControls.camera?.isolated, snapshot?.deviceControls.camera?.cleanupConfirmed, stale, cardImportReason]);
  useEffect(() => {
    if (!previewKey || adapterKind !== "tauri" || previewData?.key === previewKey) return;
    let active = true; const revision = presentation.current; setPreviewError(null);
    void invoke<PreviewVolume>("reconstruction_preview", { method: ws?.reconstruction.selectedMethod }).then(data => {
      if (!active) return;
      const [z,y,x] = data.shape ?? [];
      if (![z,y,x].every(value => Number.isSafeInteger(value) && value > 0 && value <= 256) || z*y*x > 256 ** 3 || !Array.isArray(data.voxels) || data.voxels.length !== z*y*x || !Array.isArray(data.spacingMm) || data.spacingMm.length !== 3 || data.spacingMm.some(value => !Number.isFinite(value) || value <= 0) || !Number.isFinite(data.windowMin) || !Number.isFinite(data.windowMax) || data.windowMax <= data.windowMin || data.voxels.some(value => !Number.isInteger(value) || value < 0 || value > 255)) throw new Error("Invalid preview volume data");
      if (data.colors != null && (!Array.isArray(data.colors) || data.colors.length !== z*y*x*3 || data.colors.some(value => !Number.isInteger(value) || value < 0 || value > 255) || !Array.isArray(data.colorLegend) || data.colorLegend.length < 1 || data.colorLegend.length > 3 || data.colorLegend.some(value => typeof value !== "string") || typeof data.materialLabels !== "boolean")) throw new Error("Invalid colour preview data");
      setPreviewData({ key: previewKey, volume: data });
      if (revision === presentation.current) setCenterView("reconstruction");
    }).catch(reason => { if (active) { setPreviewError(String(reason)); setActionError(`Reconstruction preview: ${String(reason)}`); } });
    return () => { active = false; };
  }, [adapterKind, previewData?.key, previewKey, previewRetry, ws?.reconstruction.selectedMethod]);
  const openReconstruction = () => { if (activeVolume) navigate("reconstruction"); else if (previewKey) { presentation.current += 1; setRequestRevision(presentation.current); setPreviewRetry(value => value + 1); } };
  const startReconstruction = (method: ReconstructionMethodId, options: ReconstructionOptions) => {
    // A card acquisition that is not imported and verified is never reconstructable,
    // even if a stale entry point is reached.
    if (cardGateReason) { setActionError(cardGateReason); return; }
    setPreviewError(null); setPreviewData(null); navigate("equipment"); void actions.run({ type: "start_reconstruction", method, options }).catch(reason => setActionError(String(reason)));
  };
  const onPhoto = (result: EngineSnapshot, revision: number) => {
    if (revision !== presentation.current || result.engineSessionId !== latest.current?.engineSessionId) return;
    const record = result.deviceControls.camera?.lastCapturedImage;
    if (!record || !record.exposureEnded || !record.captureId || !record.bytes || !/^[a-f0-9]{64}$/i.test(record.sha256)) throw new Error("Capture file registration is unconfirmed");
    setPhoto(record); navigate("photo");
  };
  const closeMedia = () => { if (centerView === "live") void actions.run({ type: "camera_live_view_stop" }).catch(reason => setActionError(String(reason))); navigate("equipment"); };
  const stop = () => { preparationScope.current.invalidate(true); setPreparationStage(null); navigate("equipment"); void actions.run({ type: "stop" }).catch(reason => setActionError(String(reason))); };
  const focusPanel = (id: string) => document.querySelector<HTMLElement>(`#${id} button:not(:disabled)`)?.focus();
  const openPreparation = () => { setPreparationError(null); if (snapshot?.deviceControls.taskLock.locked) setAuxiliary("recovery"); else setAuxiliary("prepare"); };
  const availability: MenuAvailability = useMemo(() => {
    const result = Object.fromEntries(MENU_GROUPS.flatMap(group => group.entries.map(entry => [entry.id,{ available:true,reason:null }]))) as MenuAvailability;
    const block = (id: MenuActionId, reason: string) => { result[id] = { available:false,reason }; };
    const unavailable = !snapshot || !ws || stale;
    const occupied = pending || Boolean(snapshot && manualControlReason(snapshot,stale));
    if (unavailable || occupied || openingProject || adapterKind !== "tauri") block("task.openProject", unavailable ? "Control service unavailable" : openingProject ? "Validating scan project" : occupied ? "Finish or abandon the current task before opening another project" : "Desktop application required");
    if (openingProject) result["task.openProject"].label = "Opening Scan Project…";
    for (const id of ["task.new","task.prepare","task.resetDraft","devices.turntableHome"] as const) if (unavailable || occupied) block(id,unavailable ? "Control service unavailable" : "Task or device operation occupied");
    if (snapshot?.deviceControls.taskLock.locked) { block("task.new","Retained task locked"); block("task.resetDraft","Retained task locked"); }
    for (const id of ["task.play","task.stop","task.restore","task.abandon","devices.camera","devices.cameraSettings","devices.turntable","devices.turntableBench","devices.xray"] as const) if (unavailable) block(id,"Control service unavailable");
    if (!ws?.dock.play || model.dirty || !snapshot?.cameraScanReadiness?.ready) block("task.play",ws?.dock.playReason || "Confirmed scan readiness required");
    if (ws) result["task.play"].label = ws.dock.playMode === "pause" ? "Pause Scan" : ws.dock.playMode === "resume" ? "Resume Scan" : "Start Scan";
    if (!snapshot?.deviceControls.taskLock.locked && !ws?.checkpointAvailable) block("task.restore","No retained task/checkpoint");
    if (!snapshot?.deviceControls.taskLock.locked) block("task.abandon","No retained task");
    if (!snapshot?.deviceControls.turntable.connected || occupied) block("devices.turntableHome","Confirmed turntable safety conditions required");
    if (!ws?.scanSetup.savePath || !ws.scanSetup.taskId || adapterKind !== "tauri") block("task.openFolder","No desktop task folder");
    if (!previewKey) block("view.result","No verified completed result");
    if (!photo && !snapshot?.deviceControls.camera?.lastCapturedImage) block("view.photo","No confirmed independent capture");
    if (!snapshot?.deviceControls.camera?.liveView.active) block("view.live","Live View is stopped");
    if (!ws) for (const id of ["view.images","view.reconstruction","view.logs","tools.exportLog","tools.deviceDiagnostics"] as const) block(id,"No engine data available");
    if (adapterKind !== "tauri") for (const id of ["tools.exportLog","help.checkUpdate"] as const) block(id,"Desktop platform required");
    if (preparationStage !== null) for (const group of MENU_GROUPS) for (const entry of group.entries) if (entry.id !== "task.stop") block(entry.id,"Preparation pending · use Stop before changing the current request");
    // A running USB card import owns the camera: no single capture, Live View,
    // parameter write or new scan against the same camera is offered.
    if (cardImportReason) for (const id of ["task.play","task.prepare","task.new","task.resetDraft","devices.camera","devices.cameraSettings","view.live"] as const) block(id, cardImportReason);
    if (cardGateReason && card?.destination === "cameraCard") block("view.result", cardGateReason);
    return result;
  }, [snapshot,ws,pending,stale,model.dirty,adapterKind,previewKey,photo,preparationStage,openingProject,cardImportReason,cardGateReason,card]);
  const handleMenuAction = async (id: MenuActionId): Promise<void> => {
    if (preparationScope.current.pending && id !== "task.stop") return;
    setActionError(null);
    try {
      switch(id) {
        case "help.userGuide": setDialog("guide"); break;
        case "help.safetyNotes": setDialog("safety"); break;
        case "help.troubleshooting": setDialog("troubleshooting"); break;
        case "help.about": setDialog("about"); break;
        case "help.checkUpdate": setUpdate(true); break;
        case "tools.preferences": setDialog("preferences"); break;
        case "task.new": case "task.resetDraft": setPreparationRevision(value => value + 1); setAuxiliary("prepare"); break;
        case "task.openProject": {
          setOpeningProject(true);
          try {
            const path = await chooseScanProjectDirectory(ws?.scanSetup.savePath || "");
            if (!path) break;
            // A task whose data is still on the camera card is not a sealed project.
            // Refuse it here with the same reason the reconstruction entry uses; the
            // engine still validates the chosen directory for every other case.
            if (cardGateReason && sameTaskDirectory(path, ws?.scanSetup.savePath || "", ws?.scanSetup.taskId || "")) { setActionError(cardGateReason); break; }
            setBottomTab("reconstruction");
            await actions.run({ type:"open_scan_project", path });
            setPreviewData(null); setPreviewError(null); setPhoto(null);
            setPreparationRevision(value => value + 1);
            navigate("equipment"); setBottomTab("reconstruction");
          } finally { setOpeningProject(false); }
          break;
        }
        case "task.prepare": openPreparation(); break;
        case "task.restore": case "task.abandon": setAuxiliary("recovery"); break;
        case "task.stop": stop(); break;
        case "task.play": if (ws) await actions.run({ type:ws.dock.playMode === "pause" ? "pause" : ws.dock.playMode === "resume" ? "resume" : "start_scan" }); break;
        case "task.openFolder": if (ws) await revealDirectory(`${ws.scanSetup.savePath}/${ws.scanSetup.taskId}`); break;
        case "devices.camera": focusPanel("camera-panel"); break;
        case "devices.cameraSettings": setAuxiliary("camera"); break;
        case "devices.turntable": focusPanel("turntable-panel"); break;
        case "devices.turntableHome": await actions.run({ type:"turntable_home" }); break;
        case "devices.turntableBench": setAuxiliary("bench"); break;
        case "devices.xray": focusPanel("xray-panel"); break;
        case "view.equipment": navigate("equipment"); break;
        case "view.result": openReconstruction(); break;
        case "view.photo": { const registered = photo || snapshot?.deviceControls.camera?.lastCapturedImage; if (registered) { setPhoto(registered); navigate("photo"); } break; }
        case "view.live": navigate("live"); break;
        case "view.images": setBottomTab("images"); break;
        case "view.reconstruction": setBottomTab("reconstruction"); break;
        case "view.logs": setBottomTab("aggregate"); break;
        case "tools.exportLog": if(ws) await exportSessionLog(`session-${ws.scanSetup.taskId || "log"}.log`,sessionLogText(ws,adapterKind)); break;
        case "tools.deviceDiagnostics": setDialog("diagnostics"); break;
      }
    } catch(reason) { setActionError(String(reason)); }
  };
  const info = dialog ? <InfoDialog kind={dialog} version={version} ws={ws || null} snapshot={snapshot} adapterKind={adapterKind} theme={theme} troubleshootingGroup={troubleshootingGroup} onTheme={setTheme} onClose={() => setDialog(null)} /> : null;
  const failureBanner = <FailureBanner report={visibleFailure} onDismiss={dismissFailure} onTroubleshooting={openTroubleshooting} onDiagnostics={() => setDialog("diagnostics")} topOffset={Math.round(FAILURE_TOP_LANE_DESIGN_PX * canvasLayout.zoom)} />;
  if (!snapshot || !ws) return <main className="viewport-shell"><div className="design-canvas" style={{zoom:canvasLayout.zoom,width:`${canvasLayout.designWidth}px`,height:`${canvasLayout.designHeight}px`}}><MenuBar theme={theme} availability={availability} onAction={id => void handleMenuAction(id)} version={version} /><div className="app-divider" /><main className="boot-screen"><strong>Micro-CT Workstation</strong><span>{englishDisplayText(error || "Starting control service…")}</span><span>Device readings are UNKNOWN. Help and desktop platform actions remain available.</span></main><div className="app-divider" /><footer className="status-bar">Control service unavailable · no device action permitted</footer>{failureBanner}{info}<UpdateDialog open={update} onClose={() => setUpdate(false)} /></div></main>;
  const submitPreparation = async (draft: ScanPreparation) => {
    if (auxiliaryCurrent.current !== "prepare") return false;
    setPreparationError(null);
    return runScanPreparationRequest(preparationScope.current, draft, () => preparationContext.current, actions.run, setPreparationStage, setPreparationError);
  };
  return <main className="viewport-shell"><div className="design-canvas" style={{zoom:canvasLayout.zoom,width:`${canvasLayout.designWidth}px`,height:`${canvasLayout.designHeight}px`}}><MenuBar theme={theme} availability={availability} onAction={id => void handleMenuAction(id)} version={version} /><div className="app-divider" />
    {failureBanner}
    <section className="workspace-body">
      <aside className="col device-column"><CameraPanel snapshot={snapshot} stale={stale} pending={pending} model={model} run={actions.run} cardImportReason={cardImportReason} onAdvanced={() => setAuxiliary("camera")} onPhoto={onPhoto} mediaRevision={requestRevision} onLive={revision => { if(revision === presentation.current) navigate("live"); }} /><TurntablePanel snapshot={snapshot} stale={stale} pending={pending} run={actions.run} onPreflight={openPreparation} onRecovery={() => setAuxiliary("recovery")} onBench={() => setAuxiliary("bench")} /></aside>
      <section className="main-console"><LiveScene snapshot={snapshot} ws={ws} theme={theme} dispatch={dispatch} stale={stale} setupInvalid={setupInvalid} feedbackId={snapshot.updatedAt} centerView={centerView} volume={activeVolume} previewError={previewError} volumeMode={volumeMode} volumeResetRevision={volumeResetRevision} onVolumeAvailable={setVolume3DAvailable} camera={snapshot.deviceControls.camera} photo={photo} requestRevision={requestRevision} onCloseMedia={closeMedia} runDevice={actions.run} pending={pending} liveBlockedReason={cardImportReason} /></section>
      <BottomConsole tab={bottomTab} onTab={setBottomTab} ws={ws} card={card} projectOperation={snapshot.operations.filter(operation => operation.device === "task_project" && operation.kind === "open_project").at(-1) || null} openingProject={openingProject} production={adapterKind === "tauri"} centerView={centerView} volumeReady={Boolean(activeVolume)} volume3DAvailable={volume3DAvailable} busy={pending} stale={stale} volumeMode={volumeMode} onVolumeMode={setVolumeMode} onResetVolume={() => setVolumeResetRevision(value => value + 1)} onOpenReconstruction={openReconstruction} onShowEquipment={() => navigate("equipment")} onStartReconstruction={startReconstruction} onCardImport={() => setAuxiliary("card")} cardGateReason={cardGateReason} />
      <aside className="col col--right"><XrayPanel ws={ws} busy={pending || stale} dispatch={dispatch} stale={stale} /><OperationPanel snapshot={snapshot} ws={ws} stale={stale} onImport={() => setAuxiliary("card")} /></aside>
    </section><div className="app-divider" /><footer className="status-bar"><span className="status-bar__left"><i className={`status-dot ${toneClass(ws.statusbar.dotTone)}`} /><strong>{stale ? "Control service unavailable · device readings UNKNOWN" : englishDisplayText(snapshot.lastError || ws.dock.playReason || ws.phaseWord)}</strong></span><div className="status-bar__right"><span>{ws.scanSetup.taskId || "No task selected"}</span>{snapshot.mode === "developer_preview" && <span className="badge badge--dev">Offline preview · no hardware</span>}<span className={`badge badge--engine badge--${stale ? "lost" : "connected"}`}>Control service · {stale ? "Unavailable" : "Ready"}</span><div className="theme-toggle" role="group" aria-label="Theme">{(["light","dark"] as const).map(value => <button key={value} className="theme-toggle__seg" aria-pressed={theme === value} onClick={() => setTheme(value)}>{value === "light" ? "Light" : "Dark"}</button>)}</div></div></footer>
    {info}<UpdateDialog open={update} onClose={() => setUpdate(false)} />
    {auxiliary === "camera" && <CameraSettingsDialog snapshot={snapshot} pending={pending} stale={stale} model={model} run={actions.run} onClose={() => setAuxiliary(null, auxiliaryRevision)} onStop={stop} />}
    {auxiliary === "bench" && <TurntableBenchDialog snapshot={snapshot} pending={pending} stale={stale} run={actions.run} onClose={() => setAuxiliary(null, auxiliaryRevision)} onStop={stop} />}
    {auxiliary === "recovery" && <TaskRecoveryDialog snapshot={snapshot} pending={pending} stale={stale} run={actions.run} onClose={() => setAuxiliary(null, auxiliaryRevision)} onStop={stop} />}
    {auxiliary === "prepare" && <ScanPreparationDialog key={preparationRevision} snapshot={snapshot} dirtyCamera={model.dirty} resetRevision={preparationRevision} stage={preparationStage} submissionError={(preparationNotice && failure ? `${failure.reason}\n${failure.nextAction}` : null) || preparationError} onSubmit={submitPreparation} onClose={() => setAuxiliary(null, auxiliaryRevision)} onAdvanced={() => setAuxiliary("camera", auxiliaryRevision)} onStop={stop} />}
    {auxiliary === "card" && <CardImportDialog snapshot={snapshot} pending={pending} stale={stale} run={dispatch} onClose={() => setAuxiliary(null, auxiliaryRevision)} onStop={stop} />}
    {ws.scanFlow.prompt && <ScanStageDialog key={`${ws.scanSetup.taskId}:${ws.scanFlow.prompt.stage}`} stage={ws.scanFlow.prompt.stage} recorded={ws.scanFlow.geometry} beamState={ws.xray.beamState} busy={pending} onConfirm={geometry => dispatch({ type:"confirm_scan_stage",stage:ws.scanFlow.prompt!.stage,geometry })} onCancel={() => dispatch({ type:"cancel_scan_stage",stage:ws.scanFlow.prompt!.stage })} />}
  </div></main>;
}
export type { EngineSnapshot };
