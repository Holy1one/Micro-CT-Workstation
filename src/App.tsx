/**
 * Top-level workstation presentation and user-interaction composition.
 * The UI renders complete engine snapshots and translates gestures into domain
 * commands. It does not own production scan progress, device truth, or safety
 * decisions; those remain authoritative in ct-engine.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Folder } from "@phosphor-icons/react";
import { computeCanvasLayout, type CanvasLayout } from "./canvas-layout";
import { projectionError, exposureError, minutesToSeconds, secondsToMinutes } from "./scan-input";
import { useEngine } from "./engine/useEngine";
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
      return window.localStorage.getItem(THEME_KEY) === "dark" ? "dark" : "light";
    } catch {
      return "light";
    }
  });
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      window.localStorage.setItem(THEME_KEY, theme);
    } catch {
      /* storage unavailable */
    }
  }, [theme]);
  return [theme, setTheme];
}

function logTime(timestamp: string): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return "--:--:--.---";
  const p = (value: number, size = 2) => String(value).padStart(size, "0");
  return `${p(date.getHours())}:${p(date.getMinutes())}:${p(date.getSeconds())}.${p(date.getMilliseconds(), 3)}`;
}

function HelpTip({ text }: { text: string }) {
  return <button type="button" className="help-tip" title={text} aria-label={text}>?</button>;
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
  const barRef = useRef<HTMLElement | null>(null);

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
    <header className="menu-bar" ref={barRef}>
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
                        {entry.label}
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </span>
          );
        })}
      </nav>
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
        <h2>Device Connection Status</h2>
        <HelpTip text={`Preflight: ${ws.preflight.subline}`} />
      </div>
      <div className="device-list">
        {ws.devices.map((device) => {
          const retrying = pendingDevice === device.id;
          return (
            <div className="device-row" key={device.id}>
              <div className="device-row__top">
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
              <Folder size={18} weight="duotone" aria-hidden="true" />
            </button>
          </div>
          {pathError ? <small className="field-error">{pathError}</small> : null}
        </label>
        <div className="scan-input-grid">
          <label className={`numeric-field ${viewsInvalid ? "is-invalid" : ""}`}>
            <span>Total projections <HelpTip text="Any positive integer up to 3600. The engine calculates the angular step." /></span>
            <input value={views} disabled={locked} inputMode="numeric" aria-label="Total projections" aria-invalid={viewsInvalid} onChange={(event) => setViews(event.target.value)} onBlur={() => commitNumber("projectionCount", views)} />
          </label>
          <label className={`numeric-field ${exposureInvalid ? "is-invalid" : ""}`}>
            <span>Exposure · ms <HelpTip text={`${ws.cameraExposure.known ? "Connected camera" : "Nikon D7100 timed shutter"}: ${ws.cameraExposure.minMs}–${ws.cameraExposure.maxMs} ms. The value must match a supported camera shutter setting; unsupported values are rejected.`} /></span>
            <input value={exposure} disabled={locked} inputMode="decimal" aria-label="Exposure in milliseconds" aria-invalid={exposureInvalid} onChange={(event) => setExposure(event.target.value)} onBlur={() => commitNumber("exposureMs", exposure)} />
          </label>
          <label className={`numeric-field ${durationInvalid ? "is-invalid" : ""}`}>
            <span>Max X-ray · min <HelpTip text="Default 10 min. Maximum continuous output is limited to 10 min, followed by 5 min cooldown. Fractional minutes such as 0.5 are allowed." /></span>
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

function DockIcon({ src, alt }: { src: string; alt: string }) {
  return <img className="dock-icon" src={src} alt={alt} draggable={false} />;
}

function ControlDock({ ws, dispatch, stale = false }: { ws: WorkstationView; dispatch: (c: EngineCommand) => void; stale?: boolean }) {
  const dock = ws.dock;
  const play =
    dock.playMode === "pause"
      ? { src: "/assets/dock-pause.svg", label: "Pause", command: { type: "pause" } as EngineCommand }
      : dock.playMode === "resume"
        ? { src: "/assets/dock-resume.svg", label: "Resume", command: { type: "resume" } as EngineCommand }
        : { src: "/assets/dock-play.svg", label: "Start", command: { type: "start_scan" } as EngineCommand };

  return (
    <div className="control-dock" aria-label="Scan controls">
      <button
        type="button"
        className="dock-key dock-key--home"
        aria-label="Home"
        title={dock.homeReason || "Home turntable"}
        disabled={stale || !dock.home}
        onClick={() => void dispatch({ type: "home" })}
      >
        <DockIcon src="/assets/dock-home.svg" alt="" />
        <span className="dock-key__label">Home</span>
      </button>
      <button
        type="button"
        className="dock-key dock-key--play"
        aria-label={play.label}
        title={dock.playReason || play.label}
        disabled={stale || !dock.play}
        onClick={() => void dispatch(play.command)}
      >
        <DockIcon src={play.src} alt="" />
        <span className="dock-key__label">{play.label}</span>
      </button>
      <button
        type="button"
        className="dock-key dock-key--restore"
        aria-label="Restore"
        disabled={stale || !dock.restore}
        onClick={() => void dispatch({ type: "restore_previous" })}
      >
        <DockIcon src="/assets/dock-restore.svg" alt="" />
        <span className="dock-key__label">Restore</span>
      </button>
      <button
        type="button"
        className="dock-key dock-key--stop"
        aria-label="End scan"
        title={dock.stop ? "End the active scan; already saved projections are retained" : "No active scan to end"}
        disabled={stale || !dock.stop}
        onClick={() => void dispatch({ type: "stop" })}
      >
        <DockIcon src="/assets/dock-stop.svg" alt="" />
        <span className="dock-key__label">Stop</span>
      </button>
    </div>
  );
}

function LiveScene({ ws, theme, dispatch, stale, setupInvalid, feedbackId }: { ws: WorkstationView; theme: Theme; dispatch: (c: EngineCommand) => void; stale: boolean; setupInvalid: boolean; feedbackId: string }) {
  const [viewPreset, setViewPreset] = useState<ViewPreset>("iso");
  const [presetRevision, setPresetRevision] = useState(0);
  const fallback = useSceneFallback();
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
          <LiveSceneCanvas view={sceneView} preset={viewPreset} presetRevision={presetRevision} onContextLost={fallback.setContextLost} />
        )}
        <span className="live-indicator">
          <i aria-hidden="true" />
          GEOMETRY VIEW · {stale ? "STATUS UNAVAILABLE" : "OPTICAL AXIS"}
        </span>
        <div className="status-floats">
          {ws.floats.map((float) => (
            <span className="status-float" key={float.key}>
              <i className={`status-float__dot ${toneClass(stale ? "muted" : float.tone)}`} aria-hidden="true" />
              {float.key}
              <b>{stale ? "UNKNOWN" : float.text}</b>
            </span>
          ))}
        </div>
        <div className="scene-readout-block">
          <span className="scene-label">TURNTABLE ANGLE <HelpTip text="Latest confirmed turntable angle. The model follows these feedback samples with a short constant-speed transition; it does not predict the next position." /></span>
          <strong className="scene-readout">{stale || !ws.scene.angleKnown ? "—" : ws.scene.angleDeg.toFixed(2)}°</strong>
          <span className={`scene-safety ${ws.safetyBar.tone !== "muted" ? "scene-safety--danger" : ""} ${ws.safetyBar.tone === "dangerBold" ? "scene-safety--bold" : ""}`}>
            {stale ? "Control service unavailable · readings are unknown" : ws.safetyBar.text}
          </span>
        </div>
        <ControlDock ws={ws} dispatch={dispatch} stale={stale || setupInvalid} />
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
  const beamState = stale ? "unknown" : ws.xray.beamState;
  const outputText = beamState === "on" ? "ON" : beamState === "off" ? "OFF · VERIFIED" : "UNKNOWN";
  const connected = !stale && ws.xray.connected;
  const sourceHelp = scanLocked
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
        <h2>12 Watt Controller</h2>
        <HelpTip text={sourceHelp} />
        <span className={`chip chip--${beamState === "on" ? "danger" : connected ? "accent" : "muted"}`}>
          {beamState === "on" ? "EMITTING" : connected ? "CONNECTED" : "NOT CONNECTED"}
        </span>
      </div>
      <div className={`xray-mode-banner ${scanLocked ? "xray-mode-banner--locked" : "xray-mode-banner--manual"}`}>
        <span>{scanLocked ? "Scan control · manual locked" : "Manual source control"}</span>
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
          <span className="xray-channel__label">Voltage · kV <HelpTip text="SET is the requested voltage. Measured is device readback; a dash means unknown." /></span>
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
              {voltageDraftConfirmed ? "V SENT" : "SEND V"}
            </button>
          </div>
        </div>
        <div className="xray-channel">
          <span className="xray-channel__label">Current · µA <HelpTip text="SET is the requested current. Measured is device readback; a dash means unknown." /></span>
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
              {currentDraftConfirmed ? "I SENT" : "SEND I"}
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
                : "RELEASED · MANUAL"}
          </span>
        </div>
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

    </section>
  );
}

function OperationPanel({ ws, stale }: { ws: WorkstationView; stale: boolean }) {
  return (
    <section className="panel operation-panel">
      <div className="panel__header">
        <h2>Operation Status</h2>
        <span className={`chip chip--${stale ? "muted" : ws.phaseTone}`}>{stale ? "UNKNOWN" : ws.phaseWord}</span>
      </div>
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
          <span>Save Path</span>
          <strong title={ws.summary.savePath}>{ws.summary.savePath || "Not selected"}</strong>
        </div>
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

const bottomTabs: Array<{ id: BottomTab; label: string }> = [
  { id: "aggregate", label: "Log Aggregation" },
  { id: "xray", label: "X-ray Log" },
  { id: "nano", label: "Turntable Log" },
  { id: "camera", label: "Camera Log" },
  { id: "images", label: "Image Preview" },
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

function BottomConsole({ ws }: { ws: WorkstationView }) {
  const [tab, setTab] = useState<BottomTab>("aggregate");
  const [follow, setFollow] = useState(true);
  const logs = ws.consoleLogs;
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
            {item.label}
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
            {ws.progress.total === 0 && <div className="log-empty">No projections yet. Configure a scan task to view its image sequence.</div>}
            {Array.from({ length: ws.progress.total }, (_, index) => {
              const frame = ws.frames.find((item) => item.index === index + 1);
              return (
                <div className={`image-tile ${frame ? "image-tile--captured" : ""}`} key={index}>
                  <strong>VIEW {String(index + 1).padStart(2, "0")}</strong>
                  <span>{frame ? `${frame.angleDeg.toFixed(2)}° · ${frame.exposureMs} ms · 16-bit` : "queued"}</span>
                </div>
              );
            })}
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

const APP_VERSION = "0.1.0";

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
          <strong>Home all axes</strong> (Tools → Home All Axes, or the dock). HOME is required before any
          exposure and is invalidated whenever a parameter changes.
        </li>
        <li>
          <strong>Start the scan.</strong> Pause takes effect after a safe projection boundary, Resume continues,
          and Stop ends the active scan while retaining committed images.
        </li>
        <li>
          <strong>After Stop</strong> run Preflight and HOME again before starting a new scan.
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
          <strong>Preflight before HOME, HOME before exposure.</strong> The engine rejects HOME without
          preflight and rejects exposure without HOME.
        </li>
        <li>
          <strong>Any parameter change invalidates the safety chain.</strong> Editing Task ID, Save Path,
          projections, exposure or Max X-ray resets preflight and HOME.
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
      "tools.restorePrevious": ws?.checkpointAvailable
        ? ok
        : blocked("no checkpoint"),
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
            <LiveScene ws={ws} theme={theme} dispatch={dispatch} stale={Boolean(transportError)} setupInvalid={setupDraftInvalid} feedbackId={snapshot.updatedAt} />
          </section>
          <div className="app-divider" />
          <BottomConsole ws={ws} />
          <aside className="col col--right">
            <XrayPanel ws={ws} busy={busy || Boolean(transportError)} dispatch={dispatch} stale={Boolean(transportError)} />
            <OperationPanel ws={ws} stale={Boolean(transportError)} />
          </aside>
        </section>
        <div className="app-divider" />
        <footer className="status-bar">
          <span className="status-bar__left">
            <i className={`status-dot ${toneClass(ws.statusbar.dotTone)}`} aria-hidden="true" />
            <strong>{transportError ? "Control service unavailable · device states unknown" : ws.dock.playReason || ws.phaseWord}</strong>
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
