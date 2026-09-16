import {
  Aperture, ArrowClockwise, Camera, Check, CircleNotch, ClockCountdown, Crosshair,
  FloppyDisk, FolderOpen, HouseLine, Image, LockKey, Pause, Play, PlugsConnected,
  Power, Radioactive, ShieldCheck, Stop, Warning,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useState } from "react";
import { useEngine } from "./engine/useEngine";
import type { DeviceId, DeviceState, EngineCommand, EngineSnapshot, LogEntry, ScanParameters } from "./engine/types";

type LogFilter = "聚合" | LogEntry["source"];
type BottomView = LogFilter | "影像";

const deviceIcons: Record<DeviceId, typeof Aperture> = { xray: Radioactive, turntable: Aperture, camera: Camera };
const deviceNames: Record<DeviceId, string> = { xray: "X-Ray Source", turntable: "Turntable / Nano", camera: "Camera / Nikon" };
const stateLabels: Record<DeviceState, string> = {
  offline: "OFFLINE", connected: "CONNECTED", ready: "READY", busy: "RUNNING", locked: "LOCKED", fault: "FAULT",
};
const logLevelLabels: Record<LogEntry["level"], string> = { info: "INFO", success: "PASS", warning: "WARN", error: "STOP" };
const phaseLabels: Record<EngineSnapshot["phase"], string> = {
  idle: "Idle", ready_for_home: "Ready for Home", ready: "Ready", running: "Running",
  paused: "Paused", stopped: "Stopped", completed: "Completed", fault: "Fault",
};
const sourceLabels: Record<LogEntry["source"], string> = { 系统: "SYSTEM", 转台: "TURNTABLE", 相机: "CAMERA", 射线: "X-RAY" };

function formatTime(timestamp: string) {
  try {
    const date = new Date(timestamp);
    const part = (value: number) => String(value).padStart(2, "0");
    return `${date.getFullYear()}-${part(date.getMonth() + 1)}-${part(date.getDate())} ${part(date.getHours())}:${part(date.getMinutes())}:${part(date.getSeconds())}`;
  } catch {
    return "---- -- -- --:--:--";
  }
}

function deviceDetail(device: EngineSnapshot["devices"][number]) {
  if (device.id === "xray") return "Hardware output interlocked";
  if (device.state === "offline") return "Waiting for connection";
  if (device.state === "busy") return device.id === "turntable" ? "Motion in progress" : "Awaiting capture";
  if (device.state === "ready") return device.id === "camera" ? "Online · host storage" : "Online · ready";
  return "Developer preview · no hardware";
}

function DeviceStatusRow({ device, busy, onRefresh }: {
  device: EngineSnapshot["devices"][number];
  busy: boolean;
  onRefresh: (deviceId: DeviceId) => void;
}) {
  const Icon = deviceIcons[device.id];
  return (
    <div className="device-status-row">
      <span className="device-symbol" aria-hidden="true"><Icon size={19} weight="duotone" /></span>
      <span className="device-status-copy"><strong>{deviceNames[device.id]}</strong><small>{deviceDetail(device)}</small></span>
      <span className={`device-state state-${device.state}`}><i aria-hidden="true" />{stateLabels[device.state]}</span>
      <button className="device-refresh" title={`Refresh ${deviceNames[device.id]} connection`} aria-label={`Refresh ${deviceNames[device.id]} connection`} onClick={() => onRefresh(device.id)} disabled={busy}>
        <ArrowClockwise size={16} weight="bold" /><span>Refresh</span>
      </button>
    </div>
  );
}

function LabeledNumber({ label, value, unit }: { label: string; value: string | number; unit?: string }) {
  return <div className="labeled-number"><span>{label}</span><div><strong>{value}</strong>{unit ? <small>{unit}</small> : null}</div></div>;
}

function ControlDock({ snapshot, busy, dispatch }: {
  snapshot: EngineSnapshot;
  busy: boolean;
  dispatch: (command: EngineCommand) => Promise<void>;
}) {
  const running = snapshot.phase === "running";
  const paused = snapshot.phase === "paused";
  const active = running || paused;
  const ready = snapshot.preflightPassed && snapshot.homed;
  const primary = paused
    ? { label: "Resume", icon: Play, command: { type: "resume" } as EngineCommand, className: "resume" }
    : running
      ? { label: "Pause", icon: Pause, command: { type: "pause" } as EngineCommand, className: "pause" }
      : { label: "Start", icon: Play, command: { type: "start_scan" } as EngineCommand, className: "start" };
  const PrimaryIcon = primary.icon;

  return (
    <div className="control-dock" aria-label="Scan quick controls">
      <button className="dock-button home" title="Reset / Home" aria-label="Reset / Home" onClick={() => void dispatch({ type: "home" })} disabled={busy || snapshot.connectionState !== "connected" || active || !snapshot.preflightPassed}>
        <HouseLine size={23} weight="duotone" /><span>Home</span>
      </button>
      <div className="dock-actions">
        <button className={`dock-button ${primary.className}`} title={primary.label} aria-label={primary.label} onClick={() => void dispatch(primary.command)} disabled={busy || (!active && !ready)}>
          <PrimaryIcon size={23} weight="fill" /><span>{primary.label}</span>
        </button>
        <button className="dock-button restore" title="Load previous scan progress" aria-label="Load previous scan progress" onClick={() => void dispatch({ type: "restore_previous" })} disabled={busy || snapshot.connectionState !== "connected" || active}>
          <ArrowClockwise size={23} weight="bold" /><span>Restore</span>
        </button>
        <button className="dock-button emergency" title="Emergency stop all equipment" aria-label="Emergency stop all equipment" onClick={() => void dispatch({ type: "stop" })} disabled={busy || snapshot.connectionState !== "connected"}>
          <Stop size={23} weight="fill" /><span>E-Stop</span>
        </button>
      </div>
    </div>
  );
}

export function App() {
  const { snapshot, busy, error, refresh, dispatch } = useEngine();
  const [draft, setDraft] = useState<ScanParameters | null>(null);
  const [bottomView, setBottomView] = useState<BottomView>("聚合");
  const [timerEnabled, setTimerEnabled] = useState(false);
  const [timerSeconds, setTimerSeconds] = useState(300);
  const [maxXraySeconds, setMaxXraySeconds] = useState(120);
  const [setKv, setSetKv] = useState(30);
  const [setUa, setSetUa] = useState(100);
  const [refreshingDevice, setRefreshingDevice] = useState<DeviceId | null>(null);

  useEffect(() => { if (snapshot && !draft) setDraft(snapshot.parameters); }, [snapshot, draft]);
  const logs = useMemo(() => {
    if (!snapshot || bottomView === "影像") return [];
    const filtered = bottomView === "聚合" ? snapshot.logs : snapshot.logs.filter((entry) => entry.source === bottomView);
    return filtered.slice(0, 7);
  }, [snapshot, bottomView]);

  if (!snapshot || !draft) {
    return <main className="loading-screen"><CircleNotch size={28} className="spin" /><strong>Initializing CT Engine</strong><span>All physical outputs remain disabled</span></main>;
  }

  const active = snapshot.phase === "running" || snapshot.phase === "paused";
  const connected = snapshot.connectionState === "connected";
  const dirty = JSON.stringify(draft) !== JSON.stringify(snapshot.parameters);
  const preflightPercent = snapshot.preflightPassed ? 100 : connected ? 38 : 0;
  const xrayLocked = !snapshot.safety.xrayAvailable;
  const phaseLabel = phaseLabels[snapshot.phase];
  const updateProjectionCount = (value: number) => {
    const projectionCount = Math.max(4, Math.min(3600, Math.round(value || 4)));
    setDraft((current) => current && { ...current, projectionCount, angleStepDeg: Number((360 / projectionCount).toFixed(4)) });
  };
  const refreshDevice = (deviceId: DeviceId) => {
    setRefreshingDevice(deviceId);
    void refresh().finally(() => window.setTimeout(() => setRefreshingDevice(null), 180));
  };

  return (
    <main className="industrial-app">
      <header className="window-header">
        <nav className="menu-line" aria-label="Application menu"><button>File</button><button>Edit</button><button>Tools</button><button>Help</button><span className="menu-spacer" /><span className="preview-label"><LockKey size={13} />DEVELOPER PREVIEW · NO REAL HARDWARE</span><span className={`runtime-dot ${connected ? "online" : "offline"}`}><i />{connected ? "ENGINE ONLINE" : "ENGINE STANDBY"}</span></nav>
      </header>

      {error ? <div className="error-toast" role="alert"><Warning size={18} weight="fill" /><span>{error}</span></div> : null}

      <section className="main-console">
        <aside className="left-console">
          <section className="industrial-panel device-connection-panel">
            <div className="industrial-panel-title">
              <div><PlugsConnected size={18} weight="duotone" /><h2>Device Connection Status</h2></div>
              <button className={`preview-link-button ${connected ? "disconnect" : "connect"}`} onClick={() => void dispatch(connected ? { type: "disconnect" } : { type: "connect", adapter: "developer_preview" })} disabled={busy || active}>{connected ? "Disconnect Preview" : "Connect Preview"}</button>
            </div>
            <div className="device-status-list">
              {(["xray", "turntable", "camera"] as DeviceId[]).map((id) => {
                const device = snapshot.devices.find((item) => item.id === id);
                return device ? <DeviceStatusRow key={id} device={device} busy={busy || refreshingDevice === id} onRefresh={refreshDevice} /> : null;
              })}
            </div>
            <div className="preflight-block">
              <div className="preflight-copy"><span>One-click System Pre-inspection</span><strong>{snapshot.preflightPassed ? "PASSED" : connected ? "READY" : "WAITING"}</strong></div>
              <div className={`inspection-meter ${snapshot.preflightPassed ? "passed" : connected ? "ready" : "waiting"}`} role="progressbar" aria-valuenow={preflightPercent} aria-valuemin={0} aria-valuemax={100}><span style={{ width: `${preflightPercent}%` }} /></div>
              <button className="preflight-button" onClick={() => void dispatch({ type: "preflight" })} disabled={busy || !connected || active}><ShieldCheck size={17} weight="duotone" />Run System Check</button>
            </div>
          </section>

          <section className="industrial-panel parameter-panel-v2">
            <div className="industrial-panel-title"><div><FloppyDisk size={18} weight="duotone" /><h2>Scan Parameters</h2></div><span className={dirty ? "parameter-state dirty" : "parameter-state"}>{dirty ? "UNSAVED" : "SYNCED"}</span></div>
            <fieldset disabled={busy || active}>
              <label><span>Save Path</span><div className="field-with-icon"><input value={draft.savePath} onChange={(event) => setDraft({ ...draft, savePath: event.target.value })} /><FolderOpen size={16} /></div></label>
              <label><span>Task ID</span><input value={draft.taskId} onChange={(event) => setDraft({ ...draft, taskId: event.target.value })} /></label>
              <div className="parameter-row">
                <label><span>Total Projections</span><input type="number" min="4" max="3600" value={draft.projectionCount} onChange={(event) => updateProjectionCount(Number(event.target.value))} /></label>
                <label><span>Angle Step</span><div className="field-with-unit"><input readOnly value={draft.angleStepDeg} /><small>°</small></div></label>
              </div>
              <div className="parameter-row">
                <label><span>Single Shot Exposure</span><div className="field-with-unit"><input type="number" min="1" max="10000" value={draft.exposureMs} onChange={(event) => setDraft({ ...draft, exposureMs: Number(event.target.value) })} /><small>ms</small></div></label>
                <label><span>Max X-ray Duration</span><div className="field-with-unit"><input type="number" min="1" max="3600" value={maxXraySeconds} onChange={(event) => setMaxXraySeconds(Number(event.target.value))} /><small>s</small></div></label>
              </div>
            </fieldset>
            <button className="apply-button" disabled={busy || active || !dirty} onClick={() => void dispatch({ type: "set_parameters", parameters: draft })}><FloppyDisk size={16} />Apply Parameters</button>
          </section>
        </aside>

        <section className="scene-console">
          <div className="scene-toolbar"><div><span>LIVE WORKSPACE</span><strong>{phaseLabel}</strong></div><div className="scene-toolbar-actions"><button title="Reset view"><Crosshair size={17} /></button><button title="Camera view"><Camera size={17} /></button></div></div>
          <div className="equipment-scene">
            <img src="/assets/micro-ct-equipment-scene-v2.png" alt="Micro-CT camera, turntable and X-ray source workspace" />
            <span className="scene-tag tag-camera"><Camera size={15} />Camera</span>
            <span className="scene-tag tag-turntable"><Aperture size={15} />Turntable · {snapshot.progress.angleDeg.toFixed(2)}°</span>
            <span className="scene-tag tag-xray"><Radioactive size={15} />X-Ray · Locked</span>
            <span className={`phase-indicator phase-${snapshot.phase}`}><i />{phaseLabel}</span>
            <ControlDock snapshot={snapshot} busy={busy} dispatch={dispatch} />
          </div>
        </section>

        <aside className="right-console">
          <section className="industrial-panel xray-controller">
            <div className="industrial-panel-title"><div><Radioactive size={19} weight="duotone" /><h2>12 Watt Controller</h2></div><span className="controller-lock"><LockKey size={13} />LOCKED</span></div>
            <div className="xray-control-grid">
              <label className="xray-setting"><span>Set Voltage</span><div><input type="range" min="20" max="70" value={setKv} onChange={(event) => setSetKv(Number(event.target.value))} disabled={xrayLocked} /><strong>{setKv}<small>kV</small></strong></div></label>
              <label className="xray-setting"><span>Set Current</span><div><input type="range" min="10" max="200" value={setUa} onChange={(event) => setSetUa(Number(event.target.value))} disabled={xrayLocked} /><strong>{setUa}<small>μA</small></strong></div></label>
            </div>
            <div className="xray-readbacks"><LabeledNumber label="Monitor Voltage" value="--" unit="kV" /><LabeledNumber label="Monitor Current" value="--" unit="μA" /><LabeledNumber label="Power" value="--" unit="W" /><LabeledNumber label="Temp" value="--" unit="°C" /></div>
            <button className="xray-enable" disabled={xrayLocked}><Power size={18} weight="fill" />X-ray Enable</button>
            <div className="timer-control"><div><ClockCountdown size={18} /><span>Exposure Timer</span></div><label className="timer-seconds"><span>Auto stop</span><input type="number" min="1" value={timerSeconds} onChange={(event) => setTimerSeconds(Number(event.target.value))} disabled={!timerEnabled} /><small>s</small></label><button className={`toggle ${timerEnabled ? "on" : "off"}`} role="switch" aria-checked={timerEnabled} onClick={() => setTimerEnabled((value) => !value)}><i /><span>{timerEnabled ? "ON" : "OFF"}</span></button></div>
            <p className="xray-safety-copy"><ShieldCheck size={15} />V1 hardware output is fail-closed; Windows device integration is not enabled.</p>
          </section>

          <section className="industrial-panel operation-status">
            <div className="industrial-panel-title"><div><Aperture size={18} weight="duotone" /><h2>Operation Status</h2></div><span className={`operation-badge phase-${snapshot.phase}`}>{phaseLabel}</span></div>
            <div className="progress-header"><span>Acquisition Progress</span><strong>{snapshot.progress.current} / {snapshot.progress.total}<b>{snapshot.progress.percent}%</b></strong></div>
            <div className="scan-progress"><span style={{ width: `${snapshot.progress.percent}%` }} /></div>
            <div className="operation-numbers"><LabeledNumber label="Current Angle" value={snapshot.progress.angleDeg.toFixed(2)} unit="°" /><LabeledNumber label="ETA" value={snapshot.progress.etaSeconds ?? "--"} unit={snapshot.progress.etaSeconds === null ? undefined : "s"} /><LabeledNumber label="Real Images" value={snapshot.imageCount} /></div>
            <div className="process-summary">
              <div className={connected ? "done" : "current"}><i>{connected ? <Check size={12} /> : "1"}</i><span>Device link</span></div>
              <div className={snapshot.preflightPassed ? "done" : connected ? "current" : "pending"}><i>{snapshot.preflightPassed ? <Check size={12} /> : "2"}</i><span>Pre-inspection</span></div>
              <div className={snapshot.homed ? "done" : snapshot.preflightPassed ? "current" : "pending"}><i>{snapshot.homed ? <Check size={12} /> : "3"}</i><span>Turntable home</span></div>
              <div className={active || snapshot.phase === "completed" ? "current" : "pending"}><i>4</i><span>Acquisition</span></div>
            </div>
            <div className="scan-summary" aria-label="Current scan configuration summary">
              <div><span>Task</span><strong>{snapshot.parameters.taskId}</strong></div>
              <div><span>Scan setup</span><strong>{snapshot.parameters.projectionCount} × {snapshot.parameters.angleStepDeg}° · {snapshot.parameters.exposureMs} ms</strong></div>
              <div><span>Safety</span><strong>{snapshot.safety.xrayEnabled ? "X-RAY ACTIVE" : "X-ray locked · motion preview only"}</strong></div>
            </div>
          </section>
        </aside>
      </section>

      <section className="bottom-console">
        <div className="log-tab-rail" role="tablist" aria-label="Logs and image preview">
          <div className="rail-title"><strong>OUTPUT</strong><span>LOGS / CURRENT ROUND</span></div>
          {(["聚合", "射线", "转台", "相机", "影像"] as const).map((view) => {
            const count = view === "影像" ? snapshot.imageCount : view === "聚合" ? snapshot.logs.length : snapshot.logs.filter((entry) => entry.source === view).length;
            const label = view === "聚合" ? "Log Aggregation" : view === "射线" ? "X-ray Log" : view === "转台" ? "Turntable Log" : view === "相机" ? "Camera Log" : "Image Preview";
            return <button key={view} role="tab" aria-selected={bottomView === view} className={bottomView === view ? "active" : ""} onClick={() => setBottomView(view)}>{label}<span>{count}</span></button>;
          })}
        </div>
        {bottomView === "影像" ? (
          <div className="image-preview-pane" role="tabpanel" key="image-preview">
            <div className="bottom-pane-head"><div><Image size={18} /><strong>IMAGE PREVIEW · CURRENT ROUND</strong></div><span>{snapshot.imageCount} real images · preview never fabricates captures</span></div>
            <div className="image-slot-row">{Array.from({ length: 5 }, (_, index) => <div className="image-slot" key={index}><Image size={25} weight="duotone" /><span>{snapshot.imageCount > index ? `Frame ${String(index + 1).padStart(3, "0")}` : "Awaiting capture"}</span></div>)}</div>
          </div>
        ) : (
          <div className="terminal-log" role="tabpanel" aria-label="Runtime log" key="terminal-log">
            <div className="bottom-pane-head"><div><strong>{bottomView === "聚合" ? "AGGREGATED DEVICE LOG" : `${sourceLabels[bottomView]} LOG`}</strong></div><span>Newest entries first · {logs.length} visible</span></div>
            <div className="terminal-lines">
              {logs.length ? logs.map((entry) => <div className={`terminal-line level-${entry.level}`} key={entry.id}><time>[{formatTime(entry.timestamp)}]</time><i>[{logLevelLabels[entry.level]}]</i><span>[{sourceLabels[entry.source]}]</span><p>{entry.message}</p></div>) : <div className="terminal-empty">No records in this channel.</div>}
            </div>
          </div>
        )}
      </section>

      <footer className="status-line"><span><i className={connected ? "online" : "offline"} />{snapshot.adapterLabel}</span><span>Protocol JSONL v1</span><span>Last update {formatTime(snapshot.updatedAt)}</span><span className="status-spacer" /><span>{snapshot.safety.xrayEnabled ? "X-RAY ACTIVE" : "X-RAY SAFE / DISABLED"}</span></footer>
    </main>
  );
}
