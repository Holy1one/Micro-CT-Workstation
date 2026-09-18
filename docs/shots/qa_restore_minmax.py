"""Restore / shrink-to-minimum / re-maximize verification (post-guard build)."""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

import qa_window_geometry_final as final

OUT = os.path.join(r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App",
                   "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-restore-minmax.json")

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.GetCurrentThreadId.restype = wt.DWORD
kernel32.GetCurrentThreadId.argtypes = []
user32 = final.user32
GWL_STYLE = -16
user32.GetWindowLongPtrW.argtypes = [wt.HWND, ctypes.c_int]
user32.GetWindowLongPtrW.restype = ctypes.c_ssize_t
user32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
user32.SetCursorPos.restype = wt.BOOL
user32.mouse_event.argtypes = [wt.DWORD, wt.DWORD, wt.DWORD, wt.DWORD, wt.ULONG]
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_MOVE = 0x0001


class PLACEMENT(ctypes.Structure):
    _fields_ = [("length", wt.UINT), ("flags", wt.UINT), ("showCmd", wt.UINT),
                ("ptMinPosition", wt.POINT), ("ptMaxPosition", wt.POINT),
                ("rcNormalPosition", wt.RECT)]


def focus(hwnd_value):
    target = wt.HWND(hwnd_value)
    current = kernel32.GetCurrentThreadId()
    pid = wt.DWORD()
    other = user32.GetWindowThreadProcessId(target, ctypes.byref(pid))
    user32.AttachThreadInput(current, other, True)
    try:
        user32.SetForegroundWindow(target)
    finally:
        user32.AttachThreadInput(current, other, False)
    time.sleep(0.8)


def read_state(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    style = user32.GetWindowLongPtrW(hwnd, GWL_STYLE)
    placement = PLACEMENT()
    placement.length = ctypes.sizeof(PLACEMENT)
    user32.GetWindowPlacement(hwnd, ctypes.byref(placement))
    outer = wt.RECT()
    inner = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(outer))
    user32.GetClientRect(hwnd, ctypes.byref(inner))
    return {
        "isZoomed": bool(user32.IsZoomed(hwnd)),
        "WS_MAXIMIZE": bool(style & 0x01000000),
        "showCmd": placement.showCmd,
        "rcNormalPosition": [placement.rcNormalPosition.left, placement.rcNormalPosition.top,
                             placement.rcNormalPosition.right, placement.rcNormalPosition.bottom],
        "outer": [outer.left, outer.top, outer.right, outer.bottom],
        "outerSize": [outer.right - outer.left, outer.bottom - outer.top],
        "clientSize": [inner.right - inner.left, inner.bottom - inner.top],
    }


def drag_right_edge_inwards(hwnd_value, target_x):
    hwnd = wt.HWND(hwnd_value)
    rect = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    y = rect.top + (rect.bottom - rect.top) // 2
    start_x = rect.right - 6
    user32.SetCursorPos(start_x, y)
    user32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    time.sleep(0.2)
    steps = 12
    for index in range(1, steps + 1):
        x = start_x + int((target_x - start_x) * index / steps)
        user32.SetCursorPos(x, y)
        time.sleep(0.05)
    user32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
    time.sleep(1.0)


def main():
    hwnd_value = final.find_app()
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "hwnd": hwnd_value, "steps": []}
    if not hwnd_value:
        report["error"] = "window not found"
    else:
        focus(hwnd_value)
        report["steps"].append({"step": "maximized", "state": read_state(hwnd_value),
                                "sample": final.sample(hwnd_value, "guard-maximized")})

        user32.ShowWindow(wt.HWND(hwnd_value), final.SW_RESTORE)
        time.sleep(2.0)
        report["steps"].append({"step": "restored", "state": read_state(hwnd_value),
                                "sample": final.sample(hwnd_value, "guard-restored")})

        # Drag the right border far left; the minimum size should clamp the width.
        rect = wt.RECT()
        user32.GetWindowRect(wt.HWND(hwnd_value), ctypes.byref(rect))
        drag_right_edge_inwards(hwnd_value, rect.left + 300)
        report["steps"].append({"step": "after-shrink-drag", "state": read_state(hwnd_value),
                                "sample": final.sample(hwnd_value, "guard-shrunk")})

        user32.ShowWindow(wt.HWND(hwnd_value), final.SW_MAXIMIZE)
        time.sleep(2.0)
        report["steps"].append({"step": "re-maximized", "state": read_state(hwnd_value),
                                "sample": final.sample(hwnd_value, "guard-remaximized")})

    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
