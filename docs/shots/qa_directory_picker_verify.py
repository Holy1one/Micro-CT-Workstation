"""End-to-end verification of the native directory picker in the real app.

Driving strategy (chosen so the user's desktop is never taken over):

* the app is launched with ``WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>``,
  so the page can be driven through the Chrome DevTools protocol with real
  ``Input.dispatchMouseEvent`` clicks - no ``SetForegroundWindow`` needed;
* the native folder dialog (``#32770``, created by the app process) is operated
  with plain window messages (``BM_CLICK`` on the dialog buttons), so no focus
  steal and no ``SendInput`` is required;
* the app window is restored with ``SW_SHOWNOACTIVATE`` only while the clicks
  land, then handed back minimised.

Evidence is written to ``docs/shots/directory-picker-verify.json``.
"""

from __future__ import annotations

import base64
import ctypes
import json
import os
import re
import socket
import struct
import subprocess
import tempfile
import time
import urllib.request
import zlib
from ctypes import wintypes

APP_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
EXE = os.path.join(APP_DIR, "target", "debug", "ct-workstation.exe")
SHOTS = os.path.join(APP_DIR, "docs", "shots")
OUT_JSON = os.path.join(SHOTS, "directory-picker-verify.json")
PICK_DIR = os.path.join(tempfile.gettempdir(), "microct-qa-pick")
# start = the folder the picker opens on, target = the folder we navigate to.
START_DIR = os.path.join(tempfile.gettempdir(), "microct-qa-pick-a")
TARGET_DIR = os.path.join(tempfile.gettempdir(), "microct-qa-pick-b")
DEBUG_PORT = 9333
DIALOG_TITLE = "Select projection image directory"

WM_COMMAND = 0x0111
WM_CLOSE = 0x0010
WM_SETTEXT = 0x000C
BM_CLICK = 0x00F5
SW_SHOWMINIMIZED = 2
SW_SHOWNOACTIVATE = 4
PW_RENDERFULLCONTENT = 0x2
SRCCOPY = 0x00CC0020
DIB_RGB_COLORS = 0

user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

EnumWindowsProc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
EnumChildProc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows.argtypes = [EnumWindowsProc, wintypes.LPARAM]
user32.EnumChildWindows.argtypes = [wintypes.HWND, EnumChildProc, wintypes.LPARAM]
user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]


def text_of(hwnd: int) -> str:
    buf = ctypes.create_unicode_buffer(512)
    user32.GetWindowTextW(hwnd, buf, 512)
    return buf.value


def class_of(hwnd: int) -> str:
    buf = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, buf, 256)
    return buf.value


def pid_of(hwnd: int) -> int:
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def find_windows(pred) -> list[int]:
    found: list[int] = []

    def cb(hwnd, _lparam):
        if pred(hwnd):
            found.append(hwnd)
        return True

    user32.EnumWindows(EnumWindowsProc(cb), 0)
    return found


def child_windows(hwnd: int, cls: str | None = None) -> list[int]:
    found: list[int] = []

    def cb(child, _lparam):
        if cls is None or class_of(child) == cls:
            found.append(child)
        return True

    user32.EnumChildWindows(hwnd, EnumChildProc(cb), 0)
    return found


def wait_for(pred, timeout: float, interval: float = 0.25):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = pred()
        if value:
            return value
        time.sleep(interval)
    return None


# --------------------------------------------------------------------------
# minimal Chrome DevTools protocol client (hand-rolled WebSocket)
# --------------------------------------------------------------------------


class Cdp:
    def __init__(self, ws_url: str):
        m = re.match(r"ws://([^:/]+):(\d+)(/\S*)", ws_url)
        if not m:
            raise RuntimeError(f"unexpected ws url: {ws_url}")
        host, port, path = m.group(1), int(m.group(2)), m.group(3)
        self.sock = socket.create_connection((host, port), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(req.encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            buf += self.sock.recv(4096)
        if b"101" not in buf.split(b"\r\n", 1)[0]:
            raise RuntimeError("websocket handshake failed: " + buf[:120].decode("utf-8", "replace"))
        self.seq = 0
        self._rest = b""

    def _read(self, n: int) -> bytes:
        while len(self._rest) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("cdp socket closed")
            self._rest += chunk
        out, self._rest = self._rest[:n], self._rest[n:]
        return out

    def _frame(self) -> bytes:
        payload = b""
        while True:
            b0, b1 = self._read(2)
            fin = b0 & 0x80
            opcode = b0 & 0x0F
            length = b1 & 0x7F
            if length == 126:
                length = struct.unpack(">H", self._read(2))[0]
            elif length == 127:
                length = struct.unpack(">Q", self._read(8))[0]
            data = self._read(length) if length else b""
            if opcode == 0x8:
                raise RuntimeError("cdp socket closed by peer")
            if opcode in (0x1, 0x2):
                payload = data
            elif opcode == 0x0:
                payload += data
            if fin:
                return payload

    def _send(self, payload: bytes) -> None:
        header = bytearray([0x81])
        n = len(payload)
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header += struct.pack(">H", n)
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", n)
        mask = os.urandom(4)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(bytes(header) + mask + masked)

    def call(self, method: str, params: dict | None = None, timeout: float = 20.0) -> dict:
        self.seq += 1
        msg_id = self.seq
        self._send(json.dumps({"id": msg_id, "method": method, "params": params or {}}).encode())
        self.sock.settimeout(timeout)
        deadline = time.time() + timeout
        while time.time() < deadline:
            raw = self._frame()
            try:
                msg = json.loads(raw.decode("utf-8"))
            except ValueError:
                continue
            if msg.get("id") == msg_id:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result", {})
        raise RuntimeError(f"{method}: timed out")

    def evaluate(self, expression: str) -> object:
        out = self.call(
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True, "awaitPromise": False},
        )
        value = out.get("result", {}).get("value")
        if out.get("exceptionDetails"):
            raise RuntimeError(f"page error: {out['exceptionDetails']}")
        return value

    def click(self, x: float, y: float) -> None:
        for kind in ("mousePressed", "mouseReleased"):
            self.call(
                "Input.dispatchMouseEvent",
                {"type": kind, "x": x, "y": y, "button": "left", "clickCount": 1, "buttons": 1 if kind == "mousePressed" else 0},
            )
            time.sleep(0.05)


def http_json(url: str) -> object:
    with urllib.request.urlopen(url, timeout=5) as resp:
        return json.loads(resp.read().decode("utf-8"))


def connect_page(port: int, timeout: float = 40.0) -> Cdp:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            targets = http_json(f"http://127.0.0.1:{port}/json/list")
            for target in targets:
                if target.get("type") == "page" and target.get("webSocketDebuggerUrl"):
                    cdp = Cdp(target["webSocketDebuggerUrl"])
                    cdp.call("Runtime.enable")
                    return cdp
        except Exception:
            time.sleep(0.5)
    raise RuntimeError("devtools endpoint never came up")


# --------------------------------------------------------------------------
# page helpers
# --------------------------------------------------------------------------

JS_INPUT_VALUE = "document.querySelector('.path-control input').value"

JS_SUMMARY_SHOWS = """(path => Array.from(document.querySelectorAll('strong'))
  .some(node => (node.textContent || '').trim() === path))(%r)"""

JS_SET_INPUT = """(() => {
  const el = document.querySelector('.path-control input');
  if (!el) return 'missing';
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  el.focus();
  setter.call(el, %r);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.blur();
  return el.value;
})()"""

JS_STRONG_TEXTS = "Array.from(document.querySelectorAll('strong')).map(node => (node.textContent || '').trim())"

JS_RECT_OF = """(label => {
  const wanted = label.toLowerCase();
  // Top-level buttons first (File/Edit/Tools/Help), then dropdown entries.
  const top = Array.from(document.querySelectorAll('.sys-menu__item'))
    .find(n => (n.textContent || '').trim().toLowerCase() === wanted);
  const nodes = top ? [top] : Array.from(document.querySelectorAll('.menu-dropdown__item'));
  const el = nodes.find(n => (n.textContent || '').trim().toLowerCase().startsWith(wanted));
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) return null;
  return { x: r.x + r.width / 2, y: r.y + r.height / 2, text: (el.textContent || '').trim() };
})(%r)"""

JS_DROPDOWN_OPEN = "Boolean(document.querySelector('.menu-dropdown'))"


def read_path(cdp: Cdp) -> str:
    return str(cdp.evaluate(JS_INPUT_VALUE) or "")


def summary_shows(cdp: Cdp, path: str) -> bool:
    return bool(cdp.evaluate(JS_SUMMARY_SHOWS % path))


def click_label(cdp: Cdp, label: str) -> dict:
    rect = cdp.evaluate(JS_RECT_OF % label)
    if not rect:
        raise RuntimeError(f"element not visible: {label}")
    cdp.click(rect["x"], rect["y"])
    return rect


def open_menu_entry(cdp: Cdp, group: str, label: str) -> None:
    click_label(cdp, group)
    wait_for(lambda: cdp.evaluate(JS_DROPDOWN_OPEN), 5.0, 0.1)
    click_label(cdp, label)


# --------------------------------------------------------------------------
# dialog helpers
# --------------------------------------------------------------------------


def wait_dialog(pid: int, timeout: float = 25.0):
    """Waits for the *real* folder dialog owned by the app process.

    Title and class must both match: the app owns other ``#32770`` windows and
    matching on the class alone produces false positives.
    """

    def find():
        hits = find_windows(
            lambda h: pid_of(h) == pid
            and class_of(h) == "#32770"
            and text_of(h) == DIALOG_TITLE
            and user32.IsWindowVisible(h)
        )
        return hits[0] if hits else None

    return wait_for(find, timeout, 0.2)


def dialog_children(dlg: int) -> list[dict]:
    out = []
    for child in child_windows(dlg):
        out.append(
            {
                "hwnd": child,
                "class": class_of(child),
                "text": text_of(child),
                "id": user32.GetDlgCtrlID(child),
            }
        )
    return out


def dialog_edit(dlg: int) -> int | None:
    """Returns the file-name combo/edit of the dialog, when one exists."""
    for info in dialog_children(dlg):
        if info["class"] in ("Edit", "ComboBoxEx32", "ComboBox"):
            return info["hwnd"]
    return None


def set_dialog_path(dlg: int, path: str) -> bool:
    edit = dialog_edit(dlg)
    if not edit:
        return False
    user32.SendMessageW(edit, WM_SETTEXT, 0, ctypes.c_wchar_p(path))
    return True


def dialog_buttons(dlg: int) -> list[str]:
    return [text_of(h) for h in child_windows(dlg, "Button") if text_of(h)]


def press_dialog_button(dlg: int, control_id: int) -> bool:
    btn = user32.GetDlgItem(dlg, control_id)
    if not btn:
        return False
    user32.PostMessageW(btn, BM_CLICK, 0, 0)
    return True


def dialog_gone(dlg: int, timeout: float = 10.0) -> bool:
    return wait_for(lambda: not user32.IsWindow(dlg), timeout, 0.15) is True


def capture_png(hwnd: int, path: str) -> bool:
    rect = wintypes.RECT()
    if not user32.GetWindowRect(hwnd, ctypes.byref(rect)):
        return False
    width = rect.right - rect.left
    height = rect.bottom - rect.top
    if width <= 0 or height <= 0:
        return False
    hdc = user32.GetWindowDC(hwnd)
    mem = gdi32.CreateCompatibleDC(hdc)
    bitmap = gdi32.CreateCompatibleBitmap(hdc, width, height)
    gdi32.SelectObject(mem, bitmap)
    gdi32.BitBlt(mem, 0, 0, width, height, hdc, 0, 0, SRCCOPY)

    class BMI(ctypes.Structure):
        _fields_ = [
            ("biSize", wintypes.DWORD),
            ("biWidth", wintypes.LONG),
            ("biHeight", wintypes.LONG),
            ("biPlanes", wintypes.WORD),
            ("biBitCount", wintypes.WORD),
            ("biCompression", wintypes.DWORD),
            ("biSizeImage", wintypes.DWORD),
            ("biXPelsPerMeter", wintypes.LONG),
            ("biYPelsPerMeter", wintypes.LONG),
            ("biClrUsed", wintypes.DWORD),
            ("biClrImportant", wintypes.DWORD),
        ]

    bmi = BMI()
    bmi.biSize = ctypes.sizeof(BMI)
    bmi.biWidth = width
    bmi.biHeight = -height
    bmi.biPlanes = 1
    bmi.biBitCount = 32
    bmi.biCompression = 0
    buffer = ctypes.create_string_buffer(width * height * 4)
    gdi32.GetDIBits(mem, bitmap, 0, height, buffer, ctypes.byref(bmi), DIB_RGB_COLORS)
    rows = []
    stride = width * 4
    for y in range(height):
        row = buffer[y * stride : (y + 1) * stride]
        rows.append(b"\x00" + bytes(b for i, b in enumerate(row) if i % 4 != 3))
    raw = b"".join(rows)

    def chunk(tag: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 6))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as handle:
        handle.write(png)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    return True


# --------------------------------------------------------------------------


def main() -> int:
    os.makedirs(START_DIR, exist_ok=True)
    os.makedirs(TARGET_DIR, exist_ok=True)
    os.makedirs(SHOTS, exist_ok=True)
    result: dict = {"startDir": START_DIR, "targetDir": TARGET_DIR, "checks": {}}
    hwnd = None

    # Only one instance may own the debug port, and a previous run leaves its
    # window behind, so start from a clean slate.
    subprocess.run(["taskkill", "/IM", "ct-workstation.exe", "/F"], capture_output=True)
    time.sleep(1.0)

    env = dict(os.environ)
    env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={DEBUG_PORT}"
    proc = subprocess.Popen([EXE], env=env, cwd=os.path.dirname(EXE))
    result["pid"] = proc.pid
    try:
        hwnd = wait_for(lambda: (find_windows(lambda h: pid_of(h) == proc.pid and text_of(h)) or [None])[0], 45.0)
        if not hwnd:
            result["error"] = "app window never appeared"
            return 1
        result["mainWindow"] = {"hwnd": hwnd, "title": text_of(hwnd), "class": class_of(hwnd)}
        # visible but NOT activated: CDP clicks need geometry, not OS focus.
        user32.ShowWindow(hwnd, SW_SHOWNOACTIVATE)
        time.sleep(1.0)

        cdp = connect_page(DEBUG_PORT)
        result["pageUrl"] = cdp.evaluate("location.href")

        def commit_ui_path(path: str) -> bool:
            """Types a path into Save Path and waits for the engine to echo it."""
            cdp.evaluate(JS_SET_INPUT % path)
            return bool(wait_for(lambda: read_path(cdp) == path and summary_shows(cdp, path), 8.0, 0.3))

        result["checks"]["baselinePath"] = read_path(cdp)
        if not commit_ui_path(START_DIR):
            result["error"] = f"could not preset save path (now {read_path(cdp)!r})"
            return 1
        result["checks"]["startDirApplied"] = True

        # ---- scenario 1: open the picker, then cancel -----------------------
        open_menu_entry(cdp, "File", "Open Image Folder")
        dlg = wait_dialog(proc.pid)
        if not dlg:
            result["error"] = "native dialog never opened"
            return 1
        result["dialog"] = {
            "hwnd": dlg,
            "title": text_of(dlg),
            "class": class_of(dlg),
            "buttons": dialog_buttons(dlg),
            "children": dialog_children(dlg),
        }
        capture_png(dlg, os.path.join(SHOTS, "directory-picker-dialog.png"))
        result["checks"]["dialogOpened"] = class_of(dlg) == "#32770"

        result["checks"]["cancelClicked"] = press_dialog_button(dlg, 2)  # IDCANCEL
        result["checks"]["dialogClosedAfterCancel"] = dialog_gone(dlg)
        time.sleep(0.6)
        after_cancel = read_path(cdp)
        result["afterCancelPath"] = after_cancel
        result["checks"]["cancelKeepsPath"] = after_cancel == START_DIR

        # ---- scenario 2: open the picker, then choose a different folder ----
        open_menu_entry(cdp, "File", "Open Image Folder")
        dlg2 = wait_dialog(proc.pid)
        if not dlg2:
            result["error"] = "native dialog did not reopen"
            return 1
        result["secondDialog"] = {
            "hwnd": dlg2,
            "title": text_of(dlg2),
            "buttons": dialog_buttons(dlg2),
            "children": dialog_children(dlg2),
        }
        # Navigate to the target folder so "selected == shown" cannot pass by
        # accident: the dialog starts on START_DIR, we ask for TARGET_DIR.
        navigated = set_dialog_path(dlg2, TARGET_DIR)
        result["checks"]["targetTypedIntoDialog"] = navigated
        expected = TARGET_DIR if navigated else START_DIR

        result["checks"]["okClicked"] = press_dialog_button(dlg2, 1)  # IDOK
        result["checks"]["dialogClosedAfterOk"] = dialog_gone(dlg2)
        settled = wait_for(lambda: read_path(cdp) == expected, 10.0, 0.3)
        summary_ok = wait_for(lambda: summary_shows(cdp, expected), 6.0, 0.5)
        chosen = read_path(cdp)
        result["afterChoosePath"] = chosen
        result["expectedPath"] = expected
        result["summaryTexts"] = cdp.evaluate(JS_STRONG_TEXTS)
        result["checks"]["selectedMatchesInput"] = bool(settled) and chosen == expected
        result["checks"]["selectedMatchesSummary"] = bool(summary_ok)
        # The dialog opened on START_DIR, so seeing TARGET_DIR afterwards can
        # only come from the picker result being applied.
        result["checks"]["inputMovedOffStartDir"] = chosen != START_DIR

        # The menu entry also reveals the chosen directory in Explorer.
        folder = os.path.basename(expected)
        explorer = wait_for(
            lambda: (find_windows(lambda h: class_of(h) in ("CabinetWClass", "ExploreWClass")
                                  and folder.lower() in text_of(h).lower()) or [None])[0],
            10.0,
        )
        result["checks"]["explorerRevealed"] = explorer is not None
        if explorer:
            result["explorerTitle"] = text_of(explorer)
            user32.PostMessageW(explorer, WM_CLOSE, 0, 0)

        result["verdict"] = all(
            [
                result["checks"]["dialogOpened"],
                result["checks"]["cancelKeepsPath"],
                result["checks"]["selectedMatchesInput"],
                result["checks"]["selectedMatchesSummary"],
                result["checks"]["inputMovedOffStartDir"],
            ]
        )
    finally:
        if hwnd:
            user32.ShowWindow(hwnd, SW_SHOWMINIMIZED)  # hand the desktop back
        with open(OUT_JSON, "w", encoding="utf-8") as handle:
            json.dump(result, handle, indent=2, ensure_ascii=False)
        print(json.dumps(result, indent=2, ensure_ascii=False))
        for folder in (START_DIR, TARGET_DIR):
            try:
                os.rmdir(folder)
            except OSError:
                pass

    return 0 if result.get("verdict") else 1


if __name__ == "__main__":
    raise SystemExit(main())
