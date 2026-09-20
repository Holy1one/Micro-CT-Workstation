/**
 * RTS9060 Nano line-protocol helpers for the browser preview.
 * Historical sources explain the vocabulary but are not active firmware.
 * ASCII lines are `\n` terminated and space separated.
 * Command names are contractual and must not change:
 * HEARTBEAT / PING / STATUS / SET_MICROSTEPS / REARM / HOME /
 * MOVE_ABS / MOVE_REL / CAPTURE_DONE / STOP / GET_HALL / INFO / XRAY_WARNING
 */

export const PULSES_PER_REV = 96000; // 1.8 deg motor x 60:1 gearbox x 8 microsteps
export const MICROSTEPS = 8;
export const FIRMWARE_VERSION = "1.0.0.30";
export const LINK_LABEL = "USB COM4 · 115200 8N1";

export function pulsesForDegrees(deg: number): number {
  return Math.round((deg / 360) * PULSES_PER_REV);
}

export function degreesForPulses(pulses: number): number {
  return (pulses / PULSES_PER_REV) * 360;
}

export function milliDeg(deg: number): number {
  return Math.round(deg * 1000);
}

// Command builders accept either a concrete numeric id or the "{id}"
// placeholder that NanoLink.exec substitutes with the allocated id.
type CmdId = number | "{id}";

export const cmd = {
  ping: (id: CmdId) => `PING ${id}`,
  info: (id: CmdId) => `INFO ${id}`,
  status: (id: CmdId) => `STATUS ${id}`,
  getHall: (id: CmdId) => `GET_HALL ${id}`,
  setMicrosteps: (id: CmdId, n: number) => `SET_MICROSTEPS ${id} ${n}`,
  rearm: (id: CmdId) => `REARM ${id}`,
  home: (id: CmdId) => `HOME ${id}`,
  moveAbs: (id: CmdId, mdeg: number) => `MOVE_ABS ${id} ${mdeg}`,
  moveRel: (id: CmdId, mdeg: number) => `MOVE_REL ${id} ${mdeg}`,
  captureDone: (id: CmdId) => `CAPTURE_DONE ${id}`,
  stop: (id: CmdId) => `STOP ${id}`,
  xrayWarning: (id: CmdId, on: boolean) => `XRAY_WARNING ${id} ${on ? "ON" : "OFF"}`,
  heartbeat: (seq: number) => `HEARTBEAT ${seq}`,
};

export interface ParsedLine {
  token: string;
  id: number | null;
  fields: Record<string, string>;
  rest: string[];
  raw: string;
}

export function parseLine(raw: string): ParsedLine {
  const parts = raw.trim().split(/\s+/).filter(Boolean);
  const token = parts[0] ?? "";
  let id: number | null = null;
  const fields: Record<string, string> = {};
  const rest: string[] = [];
  for (const part of parts.slice(1)) {
    if (id === null && /^\d+$/.test(part)) {
      id = Number(part);
      continue;
    }
    const eq = part.indexOf("=");
    if (eq > 0) {
      fields[part.slice(0, eq)] = part.slice(eq + 1);
    } else {
      rest.push(part);
    }
  }
  return { token, id, fields, rest, raw };
}

export interface NanoStatus {
  state: string;
  pos: number;
  target: number;
  microsteps: number;
  ppr: number;
  homed: boolean;
  rearmed: boolean;
  hall: boolean;
}

export function parseStatus(line: ParsedLine): NanoStatus {
  return {
    state: line.fields.state ?? "UNKNOWN",
    pos: Number(line.fields.pos ?? 0),
    target: Number(line.fields.target ?? 0),
    microsteps: Number(line.fields.microsteps ?? MICROSTEPS),
    ppr: Number(line.fields.ppr ?? PULSES_PER_REV),
    homed: line.fields.homed === "1",
    rearmed: line.fields.rearmed === "1",
    hall: line.fields.hall === "1",
  };
}
