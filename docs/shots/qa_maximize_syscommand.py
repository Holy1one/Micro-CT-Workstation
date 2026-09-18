"""Try the OS maximize path explicitly and record monitors from the shell."""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

ROOT = r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App"
OUT = os.path.join(ROOT, "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-syscommand.json")

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.GetWindowTextLengthW.argtypes = [wt.HWND]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.IsZoomed.argtypes = [wt.HWND]
user32.IsZoomed.restype = wt.BOOL
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.SendMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
user32.SetForegroundWindow.argtypes = [wt.HWND]
ENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
user32.EnumWindows.argtypes = [ENUMPROC, wt.LPARAM]
MONITORENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HMONITOR, wt.HDC, ctypes.POINTER(wt.RECT), wt.LPARAM)
user32.EnumDisplayMonitors.argtypes = [wt.HDC, ctypes.c_void_p, MONITORENUMPROC, wt.LPARAM]
user32.GetMonitorInfoW.argtypes = [wt.HMONITOR, ctypes.c_void_p]

WM_SYSCOMMAND = 0x0112
SC_MAXIMIZE = 0xF030
SC_RESTORE = 0xF120


class MONITORINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("rcMonitor", wt.RECT), ("rcWork", wt.RECT), ("dwFlags", wt.DWORD)]


def window_text(hwnd):
    size = user32.GetWindowTextLengthW(hwnd)
    value = ctypes.create_unicode_buffer(size + 1)
    user32.GetWindowTextW(hwnd, value, size + 1)
    return value.value


def class_name(hwnd):
    value = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, value, 256)
    return value.value


def find_app():
    found = []

    @ENUMPROC
    def visit(hwnd, _):
        if user32.IsWindowVisible(hwnd) and class_name(hwnd) == "Tauri Window" \
                and window_text(hwnd) == "Micro-CT Workstation":
            found.append(int(hwnd))
        return True

    user32.EnumWindows(visit, 0)
    return found[0] if found else None


def monitors():
    result = []

    @MONITORENUMPROC
    def visit(handle, _hdc, _rect, _param):
        info = MONITORINFO()
        info.cbSize = ctypes.sizeof(MONITORINFO)
        user32.GetMonitorInfoW(handle, ctypes.byref(info))
        result.append({
            "handle": int(handle),
            "rcMonitor": [info.rcMonitor.left, info.rcMonitor.top, info.rcMonitor.right, info.rcMonitor.bottom],
            "rcWork": [info.rcWork.left, info.rcWork.top, info.rcWork.right, info.rcWork.bottom],
            "primary": bool(info.dwFlags & 1),
        })
        return True

    user32.EnumDisplayMonitors(None, None, visit, 0)
    return result


def state(hwnd_value):
    rect = wt.RECT()
    user32.GetWindowRect(wt.HWND(hwnd_value), ctypes.byref(rect))
    return {
        "isZoomed": bool(user32.IsZoomed(wt.HWND(hwnd_value))),
        "outer": [rect.left, rect.top, rect.right, rect.bottom],
    }


def main():
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "monitors": monitors()}
    hwnd_value = find_app()
    report["hwnd"] = hwnd_value
    if hwnd_value:
        report["initial"] = state(hwnd_value)
        user32.SetForegroundWindow(wt.HWND(hwnd_value))
        user32.SendMessageW(wt.HWND(hwnd_value), WM_SYSCOMMAND, SC_MAXIMIZE, 0)
        time.sleep(2.0)
        report["afterScMaximize"] = state(hwnd_value)
        user32.SendMessageW(wt.HWND(hwnd_value), WM_SYSCOMMAND, SC_RESTORE, 0)
        time.sleep(2.0)
        report["afterScRestore"] = state(hwnd_value)
    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
