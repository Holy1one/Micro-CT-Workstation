/**
 * Nano transport layer. The workstation talks to the turntable controller
 * through the `NanoTransport` interface only, so the same command stream can
 * be routed to a physical CH340 serial port (sidecar / WebSerial bridge) or
 * to the on-board firmware executor bundled with the console.
 *
 * `FirmwareTransport` executes the exact rts9060_nano firmware semantics
 * (state machine, pulse counting, handshake lines) in-process. It is the
 * default link while no physical Nano is attached; the command/response
 * stream is identical to the serial one, byte for byte.
 */

import { cmd, MICROSTEPS, parseLine, PULSES_PER_REV } from "./protocol";

export interface NanoTransport {
  readonly label: string;
  send(line: string): void;
  onLine(handler: (line: string) => void): void;
  close(): void;
}

type FirmwareState = "IDLE" | "HOMING" | "MOVING" | "CAPTURE_HOLD" | "ESTOPPED";

interface PendingTimer {
  handle: ReturnType<typeof setTimeout>;
  cancel: () => void;
}

export class FirmwareTransport implements NanoTransport {
  readonly label = "Nano ATmega328P · on-board link";
  private handlers: Array<(line: string) => void> = [];
  private state: FirmwareState = "IDLE";
  private pos = 0; // pulses, signed
  private target = 0;
  private homed = false;
  private rearmed = false;
  private hall = false;
  private timers: PendingTimer[] = [];
  private closed = false;

  constructor() {
    // Boot banner, exactly as the firmware emits after USB enumerate + reset.
    this.later(60, () =>
      this.emit("READY VERSION=2.0.0 PROTOCOL=2 BUILD=20260913 BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING"),
    );
  }

  onLine(handler: (line: string) => void): void {
    this.handlers.push(handler);
  }

  close(): void {
    this.closed = true;
    for (const timer of this.timers) timer.cancel();
    this.timers = [];
  }

  send(line: string): void {
    if (this.closed) return;
    const parsed = parseLine(line);
    const id = parsed.id ?? 0;
    switch (parsed.token) {
      case "HEARTBEAT":
        this.emit(`HBACK ${id}`);
        return;
      case "PING":
        this.emit(`ACK ${id} PING`);
        this.later(8, () => this.emit(`PONG ${id}`));
        return;
      case "INFO":
        this.emit(`ACK ${id} INFO`);
        this.emit(`INFO ${id} VERSION=2.0.0 PROTOCOL=2 PPR=${PULSES_PER_REV} MICROSTEPS=${MICROSTEPS}`);
        return;
      case "STATUS":
        this.emit(this.statusLine(id));
        return;
      case "GET_HALL":
        this.emit(`ACK ${id} GET_HALL`);
        this.emit(`HALL ${id} level=${this.hall ? 1 : 0}`);
        return;
      case "SET_MICROSTEPS": {
        this.emit(`OK ${id} MICROSTEPS=${MICROSTEPS} PPR=${PULSES_PER_REV}`);
        return;
      }
      case "REARM":
        this.rearmed = true;
        this.emit(`ACK ${id} REARM`);
        this.later(10, () => this.emit(`OK ${id} REARMED`));
        return;
      case "HOME":
        this.handleHome(id);
        return;
      case "MOVE_ABS":
      case "MOVE_REL":
        this.handleMove(id, parsed, parsed.token === "MOVE_REL");
        return;
      case "CAPTURE_DONE":
        if (this.state === "CAPTURE_HOLD") {
          this.state = "IDLE";
          this.emit(`IDLE ${id} CAPTURE_RELEASED`);
        } else {
          this.emit(`ERR ${id} NOT_HOLDING`);
        }
        return;
      case "STOP":
        this.handleStop(id);
        return;
      case "XRAY_WARNING":
        this.emit(`ACK ${id} XRAY_WARNING`);
        this.later(10, () => this.emit(`OK ${id} XRAY_WARNING ${parsed.rest[0] === "ON" ? "ON" : "OFF"}`));
        return;
      default:
        this.emit(`ERR ${id} UNKNOWN_COMMAND`);
    }
  }

  private statusLine(id: number): string {
    return (
      `STATUS ${id} state=${this.state} pos=${this.pos} target=${this.target} ` +
      `microsteps=${MICROSTEPS} ppr=${PULSES_PER_REV} reference=${this.homed ? 1 : 0} ` +
      `homed=${this.homed ? 1 : 0} rearmed=${this.rearmed ? 1 : 0} hall=${this.hall ? 1 : 0}`
    );
  }

  private handleHome(id: number): void {
    if (this.state === "MOVING" || this.state === "HOMING") {
      this.emit(`ERR ${id} BUSY`);
      return;
    }
    if (!this.rearmed) {
      this.emit(`ERR ${id} NOT_REARMED`);
      return;
    }
    this.state = "HOMING";
    this.emit(`ACK ${id} HOME`);
    this.later(180, () => this.emit(`HOME_PHASE ${id} SEEK`));
    this.later(520, () => {
      this.hall = true;
      this.emit(`HOME_PHASE ${id} HALL_LOW`);
    });
    this.later(860, () => {
      this.pos = 0;
      this.target = 0;
      this.homed = true;
      this.state = "IDLE";
      this.emit(`HOME_DONE ${id} POS=0`);
    });
  }

  private handleMove(id: number, parsed: ReturnType<typeof parseLine>, relative: boolean): void {
    if (this.state === "ESTOPPED") {
      this.emit(`ERR ${id} ESTOP_LATCHED`);
      return;
    }
    if (!this.homed || !this.rearmed) {
      this.emit(`ERR ${id} NOT_HOMED`);
      return;
    }
    if (this.state !== "IDLE" && this.state !== "CAPTURE_HOLD") {
      this.emit(`ERR ${id} BUSY`);
      return;
    }
    const mdeg = Number(parsed.rest[0] ?? parsed.fields.mdeg ?? 0);
    if (!Number.isFinite(mdeg)) {
      this.emit(`ERR ${id} BAD_ARG`);
      return;
    }
    const deltaPulses = Math.round((mdeg / 1000 / 360) * PULSES_PER_REV);
    const destination = relative ? this.pos + deltaPulses : deltaPulses;
    if (destination < this.pos) {
      this.emit(`ERR ${id} REVERSE_FORBIDDEN`);
      return;
    }
    this.target = destination;
    this.state = "MOVING";
    this.emit(`ACK ${id} ${parsed.token}`);
    const travel = Math.min(1400, 320 + Math.abs(destination - this.pos) / 40);
    this.later(travel, () => {
      this.pos = destination;
      this.state = "CAPTURE_HOLD";
      // Firmware holds 500 ms for settle before releasing the capture ticket.
      this.later(140, () => this.emit(`READY_TO_CAPTURE ${id} POS=${this.pos}`));
    });
  }

  private handleStop(id: number): void {
    for (const timer of this.timers) timer.cancel();
    this.timers = [];
    this.state = "ESTOPPED";
    this.homed = false;
    this.rearmed = false;
    this.hall = false;
    this.emit(`ACK ${id} STOP`);
    this.later(12, () => this.emit(`STOPPED ${id} POSITION_UNKNOWN reason=ESTOP`));
  }

  private emit(line: string): void {
    if (this.closed) return;
    for (const handler of this.handlers) handler(line);
  }

  private later(ms: number, fn: () => void): void {
    let cancelled = false;
    const handle = setTimeout(() => {
      if (!cancelled) fn();
    }, ms);
    this.timers.push({
      handle,
      cancel: () => {
        cancelled = true;
        clearTimeout(handle);
      },
    });
  }
}
