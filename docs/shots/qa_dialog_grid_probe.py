"""Try a bounded grid around the visible folder icon using physical clicks."""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

ROOT = r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App"
REPORT = os.path.join(ROOT, "docs", "shots", "qa-dialog-grid-probe.json")
user32 = ctypes.WinDLL("user32", use_last_error=True)
ENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
user32.EnumWindows.argtypes = [ENUMPROC, wt.LPARAM]
user32.GetWindowTextLengthW.argtypes = [wt.HWND]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.SetForegroundWindow.argtypes = [wt.HWND]
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
user32.mouse_event.argtypes = [wt.DWORD, wt.DWORD, wt.DWORD, wt.DWORD, wt.ULONG]
user32.keybd_event.argtypes = [wt.BYTE, wt.BYTE, wt.DWORD, wt.ULONG]
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
MOUSEEVENTF_LEFTDOWN = 2
MOUSEEVENTF_LEFTUP = 4
VK_ESCAPE = 0x1B
KEYEVENTF_KEYUP = 2
SW_RESTORE = 9


def value(hwnd, class_name=False):
    if class_name:
        text = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(hwnd, text, 256)
    else:
        size = user32.GetWindowTextLengthW(hwnd)
        text = ctypes.create_unicode_buffer(size + 1)
        user32.GetWindowTextW(hwnd, text, size + 1)
    return text.value


def enumerate_windows():
    found = []
    @ENUMPROC
    def visit(hwnd, _):
        if user32.IsWindowVisible(hwnd):
            rect = wt.RECT(); pid = wt.DWORD()
            user32.GetWindowRect(hwnd, ctypes.byref(rect))
            user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
            found.append({"hwnd": int(hwnd), "pid": pid.value, "title": value(hwnd),
                          "class": value(hwnd, True),
                          "rect": [rect.left, rect.top, rect.right, rect.bottom]})
        return True
    user32.EnumWindows(visit, 0)
    return found


def app_window(items):
    return next((x for x in items if x["title"] == "Micro-CT Workstation"), None)


def physical_click(x, y):
    user32.SetCursorPos(x, y)
    time.sleep(0.15)
    user32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    user32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)


def escape():
    user32.keybd_event(VK_ESCAPE, 0, 0, 0)
    user32.keybd_event(VK_ESCAPE, 0, KEYEVENTF_KEYUP, 0)


def main():
    initial = enumerate_windows()
    app = app_window(initial)
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "app": app,
              "attempts": []}
    if not app:
        report["error"] = "app missing"
    else:
        hwnd = wt.HWND(app["hwnd"])
        user32.ShowWindow(hwnd, SW_RESTORE)
        user32.SetForegroundWindow(hwnd)
        time.sleep(1)
        app = app_window(enumerate_windows())
        left, top, right, bottom = app["rect"]
        # Around the folder icon visible at design x~=322, y~=236 after title bar.
        x_candidates = [left + round((right-left) * f / 1600) for f in (305, 320, 335)]
        y_candidates = [top + round((bottom-top) * f / 1000) for f in (220, 235, 250, 265, 280)]
        known = {item["hwnd"] for item in enumerate_windows()}
        for y in y_candidates:
            for x in x_candidates:
                user32.SetForegroundWindow(hwnd)
                physical_click(x, y)
                time.sleep(0.8)
                current = enumerate_windows()
                created = [item for item in current if item["hwnd"] not in known]
                real_dialogs = [item for item in created if item["class"] == "#32770"
                                or "Select projection image directory" in item["title"]
                                or (item["pid"] == app["pid"] and
                                    item["rect"][2]-item["rect"][0] > 100 and
                                    item["rect"][3]-item["rect"][1] > 100)]
                report["attempts"].append({"point": [x, y], "newWindows": created,
                                           "dialogs": real_dialogs})
                if real_dialogs:
                    dialog = real_dialogs[0]
                    user32.SetForegroundWindow(wt.HWND(dialog["hwnd"]))
                    escape(); time.sleep(1)
                    report["opened"] = dialog
                    report["cancelClosed"] = not any(
                        item["hwnd"] == dialog["hwnd"] for item in enumerate_windows())
                    break
            if report.get("opened"):
                break
        if not report.get("opened"):
            report["result"] = "UNVERIFIED: no dialog detected after bounded physical click grid"
    with open(REPORT, "w", encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=2)


if __name__ == "__main__":
    main()
