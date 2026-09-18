"""Probe the real Tauri folder button with physical Win32 mouse input.

The probe enumerates new windows rather than trusting foreground focus, because
WorkBuddy may retain the foreground lock in this automation session.
"""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

ROOT = r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App"
REPORT = os.path.join(ROOT, "docs", "shots", "qa-dialog-probe.json")
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

SW_MAXIMIZE = 3
VK_ESCAPE = 0x1B
KEYEVENTF_KEYUP = 0x0002
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004


def text(hwnd):
    n = user32.GetWindowTextLengthW(hwnd)
    value = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(hwnd, value, n + 1)
    return value.value


def klass(hwnd):
    value = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, value, 256)
    return value.value


def windows():
    result = []

    @ENUMPROC
    def visit(hwnd, _):
        if user32.IsWindowVisible(hwnd):
            rect = wt.RECT()
            pid = wt.DWORD()
            user32.GetWindowRect(hwnd, ctypes.byref(rect))
            user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
            result.append({
                "hwnd": int(hwnd),
                "pid": pid.value,
                "title": text(hwnd),
                "class": klass(hwnd),
                "rect": [rect.left, rect.top, rect.right, rect.bottom],
            })
        return True

    user32.EnumWindows(visit, 0)
    return result


def find_app(items):
    for item in items:
        if item["title"] == "Micro-CT Workstation" and item["class"] == "Tauri Window":
            return item
    return None


def click_folder(app):
    hwnd = wt.HWND(app["hwnd"])
    user32.ShowWindow(hwnd, SW_MAXIMIZE)
    user32.SetForegroundWindow(hwnd)
    time.sleep(1)
    current = find_app(windows())
    left, top, right, bottom = current["rect"]
    width = right - left
    height = bottom - top
    # Actual control center, derived from the fixed 1600x1000 canvas.
    x = round(left + width * (320 / 1600))
    y = round(top + height * (222 / 1000))
    user32.SetCursorPos(x, y)
    user32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    user32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
    return [x, y], current


def press_escape():
    user32.keybd_event(VK_ESCAPE, 0, 0, 0)
    user32.keybd_event(VK_ESCAPE, 0, KEYEVENTF_KEYUP, 0)


def main():
    before = windows()
    app = find_app(before)
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "app": app}
    if not app:
        report["error"] = "Tauri app window not found"
    else:
        point, app_after_maximize = click_folder(app)
        report["clickPoint"] = point
        report["appAfterMaximize"] = app_after_maximize
        time.sleep(3)
        after = windows()
        before_handles = {item["hwnd"] for item in before}
        report["newWindows"] = [item for item in after if item["hwnd"] not in before_handles]
        report["possibleDialogs"] = [
            item for item in after
            if item["hwnd"] not in before_handles
            and (
                item["class"] == "#32770"
                or "Select projection image directory" in item["title"]
                or "Select Folder" in item["title"]
                or (
                    item["pid"] == app["pid"]
                    and item["hwnd"] != app["hwnd"]
                    and (item["rect"][2] - item["rect"][0]) > 100
                    and (item["rect"][3] - item["rect"][1]) > 100
                )
            )
        ]
        report["foregroundAfterClick"] = int(user32.GetForegroundWindow())
        if report["possibleDialogs"]:
            dialog = report["possibleDialogs"][0]
            user32.SetForegroundWindow(wt.HWND(dialog["hwnd"]))
            press_escape()
            time.sleep(1)
            report["cancelClosedDialog"] = not any(
                item["hwnd"] == dialog["hwnd"] for item in windows()
            )
        else:
            report["cancelClosedDialog"] = False
            report["automationLimitation"] = (
                "Physical click issued, but no native dialog window appeared in EnumWindows; "
                "foreground remained controlled by another desktop process."
            )
    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
