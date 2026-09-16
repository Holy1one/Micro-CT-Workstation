import { useEffect, useMemo, useState } from "react";
import { useEngine } from "./engine/useEngine";
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

function formatHms(totalSeconds: number): string {
  const s = Math.max(0, Math.min(359999, Math.round(totalSeconds)));
  const p = (value: number) => String(value).padStart(2, "0");
  return `${p(Math.floor(s / 3600))}:${p(Math.floor((s % 3600) / 60))}:${p(s % 60)}`;
}

function parseHms(text: string): number | null {
  const match = /^(\d{1,2}):(\d{2}):(\d{2})$/.exec(text.trim());
  if (!match) return null;
  return Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3]);
}

const toneClass = (tone: string) => `tone-${tone}`;

/* ------------------------------------------------------------------ */
/* Menu bar                                                            */
/* ------------------------------------------------------------------ */

function MenuBar({ theme, onTheme }: { theme: Theme; onTheme: (theme: Theme) => void }) {
  return (
    <header className="menu-bar">
      <nav className="sys-menu" aria-label="Application menu">
        {["File", "Edit", "Tools", "Help"].map((item) => (
          <button key={item} type="button">
            {item}
          </button>
        ))}
      </nav>
      <span className="menu-spacer" />
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
      <span className="badge badge--dev">DEVELOPER PREVIEW</span>
      <span className="badge badge--engine">
        <i aria-hidden="true" />
        ENGINE ONLINE
      </span>
    </header>
  );
}

/* ------------------------------------------------------------------ */
/* Left column                                                         */
/* ------------------------------------------------------------------ */

function DevicePanel({ ws, dispatch }: { ws: WorkstationView; dispatch: (c: EngineCommand) => void }) {
  return (
    <section className="panel">
      <div className="panel__header">
        <h2>Device Connection Status</h2>
        <span className="chip chip--ok">{ws.onlineSummary}</span>
      </div>
      <div className="device-list">
        {ws.devices.map((device) => (
          <div className="device-row" key={device.id}>
            <div className="device-row__top">
              <strong>{device.name}</strong>
              <span className={`device-row__word ${toneClass(device.tone)}`}>{device.word}</span>
              <button
                type="button"
                className="retry-link"
                onClick={() => void dispatch({ type: "retry_device", device: device.id as DeviceId })}
              >
                Retry
              </button>
            </div>
            <div className="device-row__spec">{device.spec}</div>
          </div>
        ))}
      </div>
      <div className="preflight-block">
        <div className="preflight-block__top">
          <span>One-click System Pre-inspection</span>
          <strong className={ws.preflight.tone === "pass" ? "tone-ok" : ws.preflight.tone === "fail" ? "tone-danger" : "tone-warn"}>
            {ws.preflight.word}
          </strong>
        </div>
        <div className={`preflight preflight--${ws.preflight.tone === "running" ? "pass" : ws.preflight.tone}`} role="progressbar" aria-valuenow={ws.preflight.percent} aria-valuemin={0} aria-valuemax={100}>
          <span className="preflight__fill" style={{ width: `${ws.preflight.percent}%` }} />
        </div>
        <div className="preflight-block__sub">{ws.preflight.subline}</div>
      </div>
    </section>
  );
}

function ScanParamsPanel({ ws, dispatch }: { ws: WorkstationView; dispatch: (c: EngineCommand) => void }) {
  const setup = ws.scanSetup;
  const locked = ws.dataState === "scanning" || ws.dataState === "paused";
  const [savePath, setSavePath] = useState(setup.savePath);
  const [taskId, setTaskId] = useState(setup.taskId);
  const [views, setViews] = useState(String(setup.projectionCount));
  const [exposure, setExposure] = useState(String(setup.exposureMs));
  const [maxXray, setMaxXray] = useState(formatHms(setup.maxXraySec));

  useEffect(() => {
    setSavePath(setup.savePath);
    setTaskId(setup.taskId);
    setViews(String(setup.projectionCount));
    setExposure(String(setup.exposureMs));
    setMaxXray(formatHms(setup.maxXraySec));
    // Sync from the workflow only while idle; never clobber an edit mid-scan.
  }, [setup.savePath, setup.taskId, setup.projectionCount, setup.exposureMs, setup.maxXraySec, locked]);

  const commit = (patch: Partial<WorkstationView["scanSetup"]>) => {
    dispatch({
      type: "update_scan_setup",
      setup: {
        savePath,
        taskId,
        projectionCount: setup.projectionCount,
        exposureMs: setup.exposureMs,
        maxXraySec: setup.maxXraySec,
        ...patch,
      },
    });
  };

  return (
    <section className="panel">
      <div className="panel__header">
        <h2>Scan Parameters</h2>
        <span className="chip chip--accent">{setup.projectionCount} PROJECTIONS</span>
      </div>
      <div className="param-stack">
        <label className="param-field">
          <span>Save Path</span>
          <input
            value={savePath}
            disabled={locked}
            onChange={(event) => setSavePath(event.target.value)}
            onBlur={() => savePath.trim() && commit({ savePath })}
          />
        </label>
        <label className="param-field">
          <span>Task ID</span>
          <div className="param-field__row">
            <input
              value={taskId}
              disabled={locked}
              onChange={(event) => setTaskId(event.target.value)}
              onBlur={() => taskId.trim() && commit({ taskId })}
            />
            <button type="button" className="browse-link" disabled={locked}>
              Browse…
            </button>
          </div>
        </label>
        <div className="param-line">
          <span>Total Projection Count</span>
          <div className="param-line__value">
            <input
              className="param-line__input"
              value={views}
              disabled={locked}
              inputMode="numeric"
              onChange={(event) => setViews(event.target.value.replace(/[^0-9]/g, ""))}
              onBlur={() => {
                const count = Math.max(1, Math.min(360, Number(views) || setup.projectionCount));
                setViews(String(count));
                commit({ projectionCount: count });
              }}
            />
            <small>views · {setup.angleStepDeg.toFixed(2)}° / view</small>
          </div>
        </div>
        <div className="param-line">
          <span>Single Shot Exposure</span>
          <div className="param-line__value">
            <input
              className="param-line__input"
              value={exposure}
              disabled={locked}
              inputMode="numeric"
              onChange={(event) => setExposure(event.target.value.replace(/[^0-9]/g, ""))}
              onBlur={() => {
                const ms = Math.max(1, Math.min(10000, Number(exposure) || setup.exposureMs));
                setExposure(String(ms));
                commit({ exposureMs: ms });
              }}
            />
            <small>ms</small>
          </div>
        </div>
        <div className="param-line">
          <span>Max X-ray Duration</span>
          <div className="param-line__value">
            <input
              className="param-line__input param-line__input--wide"
              value={maxXray}
              disabled={locked}
              onChange={(event) => setMaxXray(event.target.value)}
              onBlur={() => {
                const seconds = parseHms(maxXray);
                if (seconds !== null && seconds > 0) {
                  commit({ maxXraySec: seconds });
                  setMaxXray(formatHms(seconds));
                } else {
                  setMaxXray(formatHms(setup.maxXraySec));
                }
              }}
            />
            <small>hh:mm:ss</small>
          </div>
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

function ControlDock({ ws, dispatch }: { ws: WorkstationView; dispatch: (c: EngineCommand) => void }) {
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
        disabled={!dock.home}
        onClick={() => void dispatch({ type: "home" })}
      >
        <DockIcon src="/assets/dock-home.svg" alt="" />
        <span className="dock-key__label">Home</span>
      </button>
      <button
        type="button"
        className="dock-key dock-key--play"
        aria-label={play.label}
        disabled={!dock.play}
        onClick={() => void dispatch(play.command)}
      >
        <DockIcon src={play.src} alt="" />
        <span className="dock-key__label">{play.label}</span>
      </button>
      <button
        type="button"
        className="dock-key dock-key--restore"
        aria-label="Restore"
        disabled={!dock.restore}
        onClick={() => void dispatch({ type: "restore_previous" })}
      >
        <DockIcon src="/assets/dock-restore.svg" alt="" />
        <span className="dock-key__label">Restore</span>
      </button>
      <button
        type="button"
        className="dock-key dock-key--estop"
        aria-label={ws.dataState === "fault" ? "Release E-Stop" : "E-Stop"}
        disabled={!dock.estop}
        onClick={() => void dispatch(ws.dataState === "fault" ? { type: "estop_release" } : { type: "stop" })}
      >
        <DockIcon src="/assets/dock-estop.svg" alt="" />
        <span className="dock-key__label">{ws.dataState === "fault" ? "Release" : "E-Stop"}</span>
      </button>
    </div>
  );
}

function LiveScene({ ws, theme, dispatch }: { ws: WorkstationView; theme: Theme; dispatch: (c: EngineCommand) => void }) {
  const [viewPreset, setViewPreset] = useState<"iso" | "top">("iso");
  const sceneSrc = `/assets/3D-scene-${theme}${ws.scene.rotated ? "-144" : ""}.png`;
  return (
    <section className="panel live-panel">
      <div className="scene-toolbar">
        <span className="chip chip--accent">3D RENDER</span>
        <h2>Live Scene</h2>
        <span className="menu-spacer" />
        {(["iso", "top"] as const).map((preset) => (
          <button
            key={preset}
            type="button"
            className={`scene-view-btn ${viewPreset === preset ? "active" : ""}`}
            aria-pressed={viewPreset === preset}
            onClick={() => setViewPreset(preset)}
          >
            {preset.toUpperCase()}
          </button>
        ))}
      </div>
      <div className="live-scene">
        <img className="scene-render" src={sceneSrc} alt="Micro-CT workspace: camera, turntable with sample, X-ray source" draggable={false} />
        <span className="live-indicator">
          <i aria-hidden="true" />
          LIVE RENDER
        </span>
        <div className="status-floats">
          {ws.floats.map((float) => (
            <span className="status-float" key={float.key}>
              <i className={`status-float__dot ${toneClass(float.tone)}`} aria-hidden="true" />
              {float.key}
              <b>{float.text}</b>
            </span>
          ))}
        </div>
        <div className="scene-readout-block">
          <span className="scene-label">TURNTABLE ANGLE</span>
          <strong className="scene-readout">{ws.scene.angleDeg.toFixed(2)}°</strong>
          <span className={`scene-safety ${ws.safetyBar.tone !== "muted" ? "scene-safety--danger" : ""} ${ws.safetyBar.tone === "dangerBold" ? "scene-safety--bold" : ""}`}>
            {ws.safetyBar.text}
          </span>
        </div>
        <ControlDock ws={ws} dispatch={dispatch} />
      </div>
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* Right column                                                        */
/* ------------------------------------------------------------------ */

function XrayPanel({ ws, dispatch }: { ws: WorkstationView; dispatch: (c: EngineCommand) => void }) {
  const [kvDraft, setKvDraft] = useState(ws.xray.setKv.toFixed(1));
  const [uaDraft, setUaDraft] = useState(ws.xray.setUa.toFixed(1));
  useEffect(() => {
    setKvDraft(ws.xray.setKv.toFixed(1));
    setUaDraft(ws.xray.setUa.toFixed(1));
  }, [ws.xray.setKv, ws.xray.setUa]);

  return (
    <section className="panel">
      <div className="panel__header">
        <h2>12 Watt Controller</h2>
        <span className="chip chip--muted">USB LINK</span>
      </div>
      <div className="xray-channels">
        <div className="xray-channel">
          <span className="xray-channel__label">SET kV</span>
          <div className="xray-channel__row">
            <input
              className="xray-channel__well"
              value={kvDraft}
              inputMode="decimal"
              onChange={(event) => setKvDraft(event.target.value)}
              onBlur={() => setKvDraft((Number(kvDraft) || ws.xray.setKv).toFixed(1))}
            />
            <div className="xray-channel__side">
              <small>MON {ws.xray.monKv.toFixed(1)} kV</small>
              <button type="button" className="send-btn" onClick={() => void dispatch({ type: "send_voltage", kv: Number(kvDraft) || ws.xray.setKv })}>
                SEND V
              </button>
            </div>
          </div>
        </div>
        <div className="xray-channel">
          <span className="xray-channel__label">SET µA</span>
          <div className="xray-channel__row">
            <input
              className="xray-channel__well"
              value={uaDraft}
              inputMode="decimal"
              onChange={(event) => setUaDraft(event.target.value)}
              onBlur={() => setUaDraft((Number(uaDraft) || ws.xray.setUa).toFixed(1))}
            />
            <div className="xray-channel__side">
              <small>MON {ws.xray.monUa.toFixed(1)} µA</small>
              <button type="button" className="send-btn" onClick={() => void dispatch({ type: "send_current", ua: Number(uaDraft) || ws.xray.setUa })}>
                SEND I
              </button>
            </div>
          </div>
        </div>
      </div>
      <div className="xray-meters">
        <div className="xray-meter">
          <span>POWER</span>
          <div className="xray-meter__well">{ws.xray.powerW.toFixed(1)} W</div>
        </div>
        <div className="xray-meter">
          <span>TEMP</span>
          <div className="xray-meter__well">{ws.xray.tempC.toFixed(1)} °C</div>
        </div>
      </div>
      <button
        type="button"
        className={`switch-btn switch-btn--block ${ws.xray.beamOn ? "switch-btn--on" : "switch-btn--idle"}`}
        disabled={ws.dataState === "fault"}
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
          disabled={ws.dataState === "scanning"}
          onClick={() => void dispatch({ type: "timer_toggle" })}
        >
          {ws.xray.timerOn ? "Timer On" : "Timer Off"}
        </button>
      </div>
      <label className="check-row">
        <input
          type="checkbox"
          checked={ws.xray.usbAutoShutDown}
          onChange={() => void dispatch({ type: "usb_auto_shut_down_toggle" })}
        />
        <i aria-hidden="true" />
        USB Auto Shut Down
      </label>
      <p className="panel__footnote">Output fail-closed · interlock checked before every exposure</p>
    </section>
  );
}

function OperationPanel({ ws }: { ws: WorkstationView }) {
  return (
    <section className="panel">
      <div className="panel__header">
        <h2>Operation Status</h2>
        <span className={`chip chip--${ws.phaseTone}`}>{ws.phaseWord}</span>
      </div>
      <div className="op-stats">
        <div className="op-stat">
          <span>CAPTURED</span>
          <strong>
            {ws.progress.captured} / {ws.progress.total}
          </strong>
        </div>
        <div className="op-stat">
          <span>ANGLE</span>
          <strong>{ws.progress.angleDeg.toFixed(2)}°</strong>
        </div>
        <div className="op-stat">
          <span>ETA</span>
          <strong>{ws.progress.etaText}</strong>
        </div>
      </div>
      <div className="op-progress">
        <div className="op-progress__head">
          <span>VIEW PROGRESS</span>
          <small>{ws.progress.barLabel}</small>
        </div>
        <div className={`op-progress__bar op-progress__bar--${ws.progress.barTone}`} role="progressbar" aria-valuenow={ws.progress.percent} aria-valuemin={0} aria-valuemax={100}>
          <span style={{ width: `${ws.progress.percent}%` }} />
        </div>
      </div>
      <div className="op-summary">
        <span className="op-summary__title">PROCESS SUMMARY</span>
        <div>
          <span>Save Path</span>
          <strong>{ws.summary.savePath}</strong>
        </div>
        <div>
          <span>Acquisition</span>
          <strong>{ws.summary.acquisition}</strong>
        </div>
        <div>
          <span>Output</span>
          <strong>{ws.summary.output}</strong>
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

function LogLines({ logs }: { logs: ConsoleLogLine[] }) {
  if (!logs.length) return <div className="log-empty">No records in this channel.</div>;
  return (
    <div className="log-lines">
      {logs.map((entry) => (
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
        ? "AGGREGATED STREAM · 5 SOURCES · FOLLOW TAIL"
        : `${tab === "xray" ? "X-RAY" : tab === "nano" ? "TURNTABLE" : "CAMERA"} STREAM · FOLLOW TAIL`;

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
        <div className="console__head">{header}</div>
        {tab === "images" ? (
          <div className="image-strip">
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
          <LogLines logs={filtered} />
        )}
      </div>
    </section>
  );
}

/* ------------------------------------------------------------------ */
/* App                                                                 */
/* ------------------------------------------------------------------ */

export function App() {
  const { snapshot, error, dispatch } = useEngine();
  const [theme, setTheme] = useTheme();
  const ws: WorkstationView | undefined = snapshot?.workstation;

  useEffect(() => {
    if (ws) document.documentElement.dataset.state = ws.dataState;
  }, [ws]);

  if (!snapshot || !ws) {
    return (
      <main className="boot-screen">
        <strong>Micro-CT Workstation</strong>
        <span>Linking RTS9060 device chain…</span>
      </main>
    );
  }

  return (
    <main className="console-app">
      <MenuBar theme={theme} onTheme={setTheme} />
      <div className="app-divider" />
      {error ? (
        <div className="error-toast" role="alert">
          {error}
        </div>
      ) : null}
      <section className="main-console">
        <aside className="col">
          <DevicePanel ws={ws} dispatch={dispatch} />
          <ScanParamsPanel ws={ws} dispatch={dispatch} />
        </aside>
        <LiveScene ws={ws} theme={theme} dispatch={dispatch} />
        <aside className="col">
          <XrayPanel ws={ws} dispatch={dispatch} />
          <OperationPanel ws={ws} />
        </aside>
      </section>
      <div className="app-divider" />
      <BottomConsole ws={ws} />
      <div className="app-divider" />
      <footer className="status-bar">
        <span className="status-bar__left">
          <i className={`status-dot ${toneClass(ws.statusbar.dotTone)}`} aria-hidden="true" />
          <strong>{ws.statusbar.left}</strong>
        </span>
        <span className="status-bar__right">{ws.statusbar.right}</span>
      </footer>
    </main>
  );
}

export type { EngineSnapshot };
