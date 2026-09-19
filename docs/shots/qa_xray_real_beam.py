# qa_xray_real_beam.py — 真实 Moxtek 12W 出束验收（不伪造任何射线信息）
#
# 直接驱动 target/debug/ct-engine.exe（生产模式），JSONL stdio：
#   1. 连接：引擎默认 USB Auto Shut Down = ARMED（勾选）→ 手动开束必须被拒绝
#   2. 释放勾选 → 设备 0x74 0x00 → SEND V/I → 真实出束并保持 8s（无需持续发包）
#   3. 手动关束 → 重新勾选（ARMED）→ 开束再次被拒
#   4. EOF 终止（模拟窗口关闭）→ 重连验证束流已关
#   5. taskkill /F（释放态出束中）→ 束流物理保持 → 重连必须检测到红色状态 → 手动关闭
# 证据写入 docs/shots/xray-real-beam-verify.json，最终状态必须是束流 OFF。
import json
import os
import queue
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
sys.stderr.reconfigure(encoding="utf-8", errors="replace")

ROOT = os.path.dirname(os.path.abspath(__file__))
ENGINE = os.path.join(ROOT, "..", "..", "target", "debug", "ct-engine.exe")
EVIDENCE = os.path.join(ROOT, "xray-real-beam-verify.json")

KV, UA = 20.0, 50.0  # 1 W，最低安全验证功率


def ts():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


class Engine:
    def __init__(self):
        self.proc = subprocess.Popen(
            [os.path.abspath(ENGINE)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
        )
        self.seq = 0
        self.lines = queue.Queue()
        threading.Thread(target=self._reader, daemon=True).start()

    def _reader(self):
        for line in self.proc.stdout:
            self.lines.put(line)

    def send(self, command, payload=None, timeout=30.0):
        self.seq += 1
        request = {
            "protocol_version": 1,
            "request_id": f"qa-{self.seq}",
            "command": command,
            "payload": payload if payload is not None else {"type": command},
            "timestamp": ts(),
            "sequence": self.seq,
            "error_code": None,
        }
        self.proc.stdin.write(json.dumps(request) + "\n")
        self.proc.stdin.flush()
        line = self.lines.get(timeout=timeout)
        return json.loads(line)

    def xray(self):
        snap = self.send("snapshot", {})
        return snap["payload"]["workstation"]["xray"]

    def eof_shutdown(self, timeout=20.0):
        self.proc.stdin.close()
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.proc.poll() is not None:
                return self.proc.returncode
            time.sleep(0.1)
        self.proc.kill()
        raise TimeoutError("engine did not exit after stdin EOF")

    def force_kill(self):
        subprocess.run(["taskkill", "/F", "/PID", str(self.proc.pid)], capture_output=True)
        self.proc.wait(timeout=10)


def expect(condition, step, evidence, **detail):
    evidence["steps"].append({"step": step, "pass": bool(condition), **detail})
    print(("PASS " if condition else "FAIL ") + step + (" · " + json.dumps(detail, ensure_ascii=False) if detail else ""))
    if not condition:
        raise SystemExit(f"step failed: {step}")


def connect_xray(engine, evidence, label):
    response = engine.send("retry_device", {"type": "retry_device", "device": "xray"})
    expect(response["error_code"] is None, f"{label}: retry_device xray", evidence)
    xray = engine.xray()
    return xray["beamOn"], xray


def set_setpoints(engine, evidence, label):
    expect(engine.send("send_voltage", {"type": "send_voltage", "kv": KV})["error_code"] is None,
           f"{label}: SEND V {KV} kV", evidence)
    expect(engine.send("send_current", {"type": "send_current", "ua": UA})["error_code"] is None,
           f"{label}: SEND I {UA} µA", evidence)


def beam_on_and_hold(engine, evidence, label, hold_s=8.0):
    response = engine.send("xray_toggle", {"type": "xray_toggle"}, timeout=30.0)
    expect(response["error_code"] is None, f"{label}: xray_toggle ON accepted", evidence)
    samples = []
    t0 = time.time()
    while time.time() - t0 < hold_s:
        time.sleep(1.0)
        xray = engine.xray()
        samples.append((xray["beamOn"], xray["monKv"], xray["monUa"]))
    expect(all(on for on, _, _ in samples), f"{label}: beam HELD {hold_s:.0f}s without re-enable", evidence,
           samples=[{"on": on, "kv": round(kv, 2), "ua": round(ua, 1)} for on, kv, ua in samples])
    last = samples[-1]
    expect(abs(last[1] - KV) <= 0.5 and abs(last[2] - UA) <= 5.0,
           f"{label}: steady output matches setpoint (REAL beam)", evidence,
           monKv=last[1], monUa=last[2])


def main():
    evidence = {"startedAt": ts(), "engine": os.path.abspath(ENGINE),
                "setpoint": {"kv": KV, "ua": UA}, "steps": []}
    if not os.path.exists(ENGINE):
        raise SystemExit(f"ct-engine not built: {ENGINE}")

    # ---- 阶段 1：ARMED 锁定 → 释放 → 真实出束保持 → 手动关 → 重新锁定 ----
    engine = Engine()
    beam_on, xray = connect_xray(engine, evidence, "P1")
    evidence["initialBeamStateDetected"] = beam_on
    evidence["initialUsbAutoShutDown"] = xray["usbAutoShutDown"]
    if beam_on:
        # 上次遗留的出束：先释放锁再手动关闭
        engine.send("usb_auto_shut_down_toggle", {"type": "usb_auto_shut_down_toggle"})
        engine.send("xray_toggle", {"type": "xray_toggle"})
        engine.send("usb_auto_shut_down_toggle", {"type": "usb_auto_shut_down_toggle"})
        beam_on = engine.xray()["beamOn"]
        expect(beam_on is False, "P1: leftover beam disabled after detection", evidence)

    response = engine.send("xray_toggle", {"type": "xray_toggle"})
    expect(response["error_code"] == "SAFETY_LOCK_REQUIRED",
           "P1: beam enable REJECTED while USB Auto Shut Down armed (locked)", evidence,
           errorCode=response["error_code"])

    expect(engine.send("usb_auto_shut_down_toggle", {"type": "usb_auto_shut_down_toggle"})["error_code"] is None,
           "P1: release checkbox → device deadman RELEASED (0x74 0x00)", evidence)
    set_setpoints(engine, evidence, "P1")
    beam_on_and_hold(engine, evidence, "P1", hold_s=8.0)

    expect(engine.send("xray_toggle", {"type": "xray_toggle"})["error_code"] is None,
           "P1: xray_toggle OFF", evidence)
    expect(engine.xray()["beamOn"] is False, "P1: beam OFF confirmed", evidence)

    expect(engine.send("usb_auto_shut_down_toggle", {"type": "usb_auto_shut_down_toggle"})["error_code"] is None,
           "P1: re-arm checkbox → device deadman ARMED (0x74 0x01)", evidence)
    response = engine.send("xray_toggle", {"type": "xray_toggle"})
    expect(response["error_code"] == "SAFETY_LOCK_REQUIRED",
           "P1: beam enable rejected again after re-arm", evidence, errorCode=response["error_code"])

    # ---- 阶段 2：释放态出束 + EOF 终止 → 必须关束 ----
    expect(engine.send("usb_auto_shut_down_toggle", {"type": "usb_auto_shut_down_toggle"})["error_code"] is None,
           "P2: release again", evidence)
    beam_on_and_hold(engine, evidence, "P2", hold_s=3.0)
    code = engine.eof_shutdown()
    expect(code == 0, "P2: stdin EOF with beam ON → engine exited cleanly", evidence, exitCode=code)

    engine = Engine()
    beam_on, _ = connect_xray(engine, evidence, "P2-reconnect")
    expect(beam_on is False, "P2: beam was OFF after EOF termination (shutdown cleanup works)", evidence)
    code = engine.eof_shutdown()
    expect(code == 0, "P2: second engine EOF", evidence, exitCode=code)

    # ---- 阶段 3：释放态出束 + taskkill /F → 束流物理保持 → 重连检测红色 ----
    engine = Engine()
    beam_on, _ = connect_xray(engine, evidence, "P3")
    expect(beam_on is False, "P3: beam OFF at start", evidence)
    expect(engine.send("usb_auto_shut_down_toggle", {"type": "usb_auto_shut_down_toggle"})["error_code"] is None,
           "P3: release", evidence)
    set_setpoints(engine, evidence, "P3")
    beam_on_and_hold(engine, evidence, "P3", hold_s=3.0)
    engine.force_kill()
    evidence["steps"].append({"step": "P3: taskkill /F with beam ON (released mode)", "pass": True})
    print("PASS P3: taskkill /F with beam ON (released mode)")

    engine = Engine()
    beam_on, xray = connect_xray(engine, evidence, "P3-reconnect")
    expect(beam_on is True, "P3: beam left ON by force-kill is DETECTED at connect (red state)", evidence,
           monKv=xray["monKv"], monUa=xray["monUa"])
    # 重连后引擎默认 ARMED 已下发 0x74 0x01：死开关约 0.27s 内会自动关束；
    # 等 1.5s 后轮询应看到设备已自行关束（硬件保险）
    time.sleep(1.5)
    beam_on = engine.xray()["beamOn"]
    evidence["steps"].append({"step": "P3: armed deadman auto-killed leftover beam", "pass": beam_on is False})
    print(("PASS " if beam_on is False else "INFO ") + "P3: armed deadman auto-killed leftover beam" if beam_on is False else "INFO P3: leftover beam still on, disabling manually")
    if beam_on:
        engine.send("usb_auto_shut_down_toggle", {"type": "usb_auto_shut_down_toggle"})
        expect(engine.send("xray_toggle", {"type": "xray_toggle"})["error_code"] is None,
               "P3: leftover beam manually disabled", evidence)
        expect(engine.xray()["beamOn"] is False, "P3: beam confirmed OFF", evidence)
    code = engine.eof_shutdown()
    expect(code == 0, "P3: final EOF shutdown", evidence, exitCode=code)

    evidence["finishedAt"] = ts()
    evidence["verdict"] = True
    with open(EVIDENCE, "w", encoding="utf-8") as file:
        json.dump(evidence, file, ensure_ascii=False, indent=2)
    print(f"VERDICT true · evidence → {EVIDENCE}")


if __name__ == "__main__":
    main()
