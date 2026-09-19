# qa_xray_setpoint_sweep.py — 设备端深度探查
# 1) 多设定点稳态扫描（喂狗保持）：20/50、30/100、40/150、60/200（≤12W）
#    每个点：关束→等 2s（灯丝保护）→写设定→喂狗出束 4s→采样稳态→关束
# 2) 完整 19 字节状态包转储（含未解析字节 12-18，查故障码）
# 3) 判定：测量/设定比值是否恒定（校准特性）还是随设定恶化（管子问题）
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

import serial
from serial.tools import list_ports

HDR = 0x1B
DAC = 13107.2
POINTS = [(20.0, 50.0), (30.0, 100.0), (40.0, 150.0), (60.0, 200.0)]


def find_port():
    for p in list_ports.comports():
        if p.vid == 0x277B and p.pid == 0x07D1 and (p.serial_number or "").startswith("168249"):
            return p.device
    raise SystemExit("Moxtek not found")


def tx(ser, cmd, payload=b"", resp_len=4, retries=3):
    last = b""
    for _ in range(retries):
        ser.reset_input_buffer()
        ser.write(bytes([HDR, cmd, len(payload)]) + payload)
        resp = ser.read(resp_len)
        last = resp
        if len(resp) == resp_len and resp[0] == HDR and resp[1] == cmd:
            return resp
        time.sleep(0.05)
    raise AssertionError(f"bad resp for {cmd:#x}: {last.hex()}")


def status_raw(ser):
    return tx(ser, 0x80, b"", 19)


def parse_status(r):
    kv = int.from_bytes(r[3:5], "little") / DAC * 15.0
    ua = int.from_bytes(r[5:7], "little") / DAC * 250.0
    temp = (int.from_bytes(r[8:10], "little") - 98.3) / 3.19
    tail = r[12:19].hex()
    return kv, ua, r[7], temp, r[10], r[11], tail


def enable(ser, on):
    tx(ser, 0x42, bytes([1 if on else 0]), 4)


def write_setpoint(ser, kv, ua):
    raw = lambda v, s: round(v / s * DAC).to_bytes(2, "little")
    tx(ser, 0x41, raw(0.0, 250.0), 5)
    tx(ser, 0x40, raw(kv, 15.0), 5)
    tx(ser, 0x41, raw(ua, 250.0), 5)


def main():
    port = find_port()
    print("port:", port)
    results = []
    with serial.Serial(port, 57600, timeout=1) as ser:
        enable(ser, False)
        time.sleep(2.2)
        for kv_set, ua_set in POINTS:
            write_setpoint(ser, kv_set, ua_set)
            # 喂狗出束 4s，每秒采一次
            samples = []
            t0 = time.time()
            next_sample = 1.0
            while time.time() - t0 < 4.0:
                enable(ser, True)
                now = time.time() - t0
                if now >= next_sample:
                    kv, ua, locked, temp, kven, uaen, tail = parse_status(status_raw(ser))
                    samples.append((kv, ua))
                    print(
                        f"  set {kv_set:5.1f}kV/{ua_set:5.1f}uA | t={now:3.1f}s "
                        f"mon={kv:6.2f}kV/{ua:6.2f}uA lock={locked} T={temp:4.1f}C "
                        f"kven={kven} uaen={uaen} tail[12-18]={tail}"
                    )
                    next_sample = now + 1.0
                time.sleep(0.08)
            enable(ser, False)
            time.sleep(0.3)
            kv, ua, *_ = parse_status(status_raw(ser))
            print(f"  off-check: {kv:.3f}kV/{ua:.2f}uA")
            time.sleep(2.2)  # 灯丝保护
            steady = samples[-1] if samples else (0.0, 0.0)
            results.append((kv_set, ua_set, *steady))

        print("\n===== 稳态汇总（最后 1s 采样）=====")
        print(f"{'set kV':>8} {'set uA':>8} {'mon kV':>8} {'mon uA':>8} {'kV比':>6} {'uA比':>6}")
        for skv, sua, mkv, mua in results:
            print(f"{skv:8.1f} {sua:8.1f} {mkv:8.2f} {mua:8.2f} {mkv/skv:6.3f} {mua/sua:6.3f}")
        enable(ser, False)
        kv, ua, *_ = parse_status(status_raw(ser))
        print(f"FINAL: {kv:.3f}kV/{ua:.2f}uA (must be ~0)")


if __name__ == "__main__":
    main()
