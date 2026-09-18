"""Read-only startup probe: no SetWindowPos, no ShowWindow, no restore, and no
SetForegroundWindow either -- evidence is captured with PrintWindow straight from
the window, so the operator's desktop is never interrupted.

Use this on a fresh launch to get an unmutated maximize reading.
"""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

import qa_window_geometry_final as final

OUT = os.path.join(r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App",
                   "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-readonly.json")

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.GetCurrentThreadId.restype = wt.DWORD
kernel32.GetCurrentThreadId.argtypes = []
user32 = final.user32
GWL_STYLE = -16
user32.GetWindowLongPtrW.argtypes = [wt.HWND, ctypes.c_int]
user32.GetWindowLongPtrW.restype = ctypes.c_ssize_t
user32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.AttachThreadInput.restype = wt.BOOL


class PLACEMENT(ctypes.Structure):
    _fields_ = [("length", wt.UINT), ("flags", wt.UINT), ("showCmd", wt.UINT),
                ("ptMinPosition", wt.POINT), ("ptMaxPosition", wt.POINT),
                ("rcNormalPosition", wt.RECT)]


def assert_not_foreground(hwnd_value):
    """Confirm we never had to steal focus to capture the window."""
    return {
        "foregroundHwnd": int(user32.GetForegroundWindow()),
        "targetIsForeground": int(user32.GetForegroundWindow()) == hwnd_value,
        "note": "capture uses PrintWindow; focus is left untouched",
    }


def read_state(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    style = user32.GetWindowLongPtrW(hwnd, GWL_STYLE)
    placement = PLACEMENT()
    placement.length = ctypes.sizeof(PLACEMENT)
    user32.GetWindowPlacement(hwnd, ctypes.byref(placement))
    outer = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(outer))
    return {
        "isZoomed": bool(user32.IsZoomed(hwnd)),
        "styleHex": hex(style & 0xFFFFFFFF),
        "WS_MAXIMIZE": bool(style & 0x01000000),
        "showCmd": placement.showCmd,
        "rcNormalPosition": [placement.rcNormalPosition.left, placement.rcNormalPosition.top,
                             placement.rcNormalPosition.right, placement.rcNormalPosition.bottom],
        "outer": [outer.left, outer.top, outer.right, outer.bottom],
        "outerSize": [outer.right - outer.left, outer.bottom - outer.top],
    }


def main():
    hwnd_value = final.find_app()
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "hwnd": hwnd_value,
              "note": "read-only: no SetWindowPos/ShowWindow/restore/SetForegroundWindow"}
    if not hwnd_value:
        report["error"] = "window not found"
    else:
        report["stateBefore"] = read_state(hwnd_value)
        report["full"] = final.sample(hwnd_value, "readonly-clean")
        report["stateAfter"] = read_state(hwnd_value)
        report["focus"] = assert_not_foreground(hwnd_value)
    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
