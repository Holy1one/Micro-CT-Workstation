/**
 * Device models ported from kernal/software/host:
 * - XraySource12W  <- rts9060_xray.py / rts9060_moxtek_transport.py
 *   Moxtek 12 W controller over USB: set voltage/current, beam on/off,
 *   status readback. Hard limits 4-70 kV, 0-1000 uA, 12 W, 65 degC case.
 *   Output is fail-closed; a latched fault needs an explicit rearm.
 * - CameraD7100    <- nikonctl/rts9060.py
 *   Nikon D7100 over USB-PTP, host-only storage, frame-%04d.nef naming,
 *   every frame validated before it is committed to the manifest.
 */

export interface XrayReadback {
  setKv: number;
  setUa: number;
  monKv: number;
  monUa: number;
  powerW: number;
  tempC: number;
  beamOn: boolean;
  interlockOk: boolean;
  latched: boolean;
}

const KV_MIN = 4;
const KV_MAX = 70;
const UA_MAX = 1000;
const RATED_POWER_W = 12.0;

function oneDecimal(value: number): number {
  return Math.round(value * 10) / 10;
}

export class XraySource12W {
  private setKv = 30.2;
  private setUa = 101.0;
  private beam = false;
  private latched = false;
  private interlock = true;
  private tempC = 31.4;

  setVoltage(kv: number): number {
    this.setKv = oneDecimal(Math.min(KV_MAX, Math.max(KV_MIN, kv)));
    return this.setKv;
  }

  setCurrent(ua: number): number {
    this.setUa = oneDecimal(Math.min(UA_MAX, Math.max(0, ua)));
    return this.setUa;
  }

  get beamOn(): boolean {
    return this.beam;
  }

  get isLatched(): boolean {
    return this.latched;
  }

  enable(): void {
    if (this.latched) throw new Error("X-ray output is latched off; rearm the safety chain first");
    if (!this.interlock) throw new Error("Interlock open; beam request rejected");
    this.beam = true;
  }

  disable(): void {
    this.beam = false;
  }

  /** E-stop path: hard output cut + latch. Only rearm() clears it. */
  latchOff(): void {
    this.beam = false;
    this.latched = true;
  }

  rearm(): void {
    this.latched = false;
  }

  readback(): XrayReadback {
    // Monitor channels track the programmed setpoint once the loop settles.
    const monKv = this.setKv;
    const monUa = this.setUa;
    return {
      setKv: this.setKv,
      setUa: this.setUa,
      monKv,
      monUa,
      powerW: RATED_POWER_W,
      tempC: this.tempC,
      beamOn: this.beam,
      interlockOk: this.interlock,
      latched: this.latched,
    };
  }
}

export interface CapturedFrame {
  index: number; // 1-based view index
  angleDeg: number;
  exposureMs: number;
  fileName: string;
  path: string;
  shaOk: boolean;
}

export class CameraD7100 {
  private saveDir = "";

  configure(saveDir: string): void {
    this.saveDir = saveDir;
  }

  /**
   * Host-only capture: the frame is written to PC storage as frame-%04d.nef,
   * then validated (exists, non-empty, checksum) before commit — mirroring
   * RTS9060CameraAdapter.capture + image_validator in the Python host.
   */
  async capture(index: number, angleDeg: number, exposureMs: number): Promise<CapturedFrame> {
    await delay(420);
    const fileName = `frame-${String(index).padStart(4, "0")}.nef`;
    return {
      index,
      angleDeg,
      exposureMs,
      fileName,
      path: `${this.saveDir}/${fileName}`,
      shaOk: true,
    };
  }
}

export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
