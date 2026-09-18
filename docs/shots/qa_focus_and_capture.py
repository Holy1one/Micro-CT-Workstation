"""Force the Tauri window to the foreground, then capture and test maximize.

WorkBuddy keeps the foreground lock in this automation session, so this probe
attaches the input queues before calling SetForegroundWindow.
"""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

import qa_window_geometry_final as final

ROOT = r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App"
OUT = os.path.join(ROOT, "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-focus.json")

user32 = final.user32
user32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
user32.AttachThreadInput.restype = wt.BOOL
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.GetWindowThreadProcessId.restype = wt.DWORD
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.GetCurrentThreadId.restype = wt.DWORD
user32.BringWindowToTop.argtypes = [wt.HWND]
user32.SetWindowPos.argtypes = [wt.HWND, wt.HWND, ctypes.c_int, ctypes.c_int,
                                ctypes.c_int, ctypes.c_int, wt.UINT]
user32.SetWindowPos.restype = wt.BOOL
HWND_TOPMOST = wt.HWND(-1)
SWP_SHOWWINDOW = 0x0040


def force_foreground(hwnd_value):
    target = wt.HWND(hwnd_value)
    current_thread = kernel32.GetCurrentThreadId()
    pid = wt.DWORD()
    target_thread = user32.GetWindowThreadProcessId(target, ctypes.byref(pid))
    user32.AttachThreadInput(current_thread, target_thread, True)
    try:
        user32.BringWindowToTop(target)
        user32.SetWindowPos(target, HWND_TOPMOST, 0, 0, 0, 0,
                            final.SWP_NOSIZE | SWP_SHOWWINDOW)
        user32.ShowWindow(target, final.SW_RESTORE)
        user32.SetForegroundWindow(target)
    finally:
        user32.AttachThreadInput(current_thread, target_thread, False)
    time.sleep(1.0)
    return int(user32.GetForegroundWindow())


def main():
    hwnd_value = final.find_app()
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "hwnd": hwnd_value, "steps": []}
    if not hwnd_value:
        report["error"] = "window not found"
    else:
        report["foregroundAfterForce"] = force_foreground(hwnd_value)
        time.sleep(1.0)
        report["steps"].append({"step": "foreground", "sample": final.sample(hwnd_value, "foreground")})

        user32.ShowWindow(wt.HWND(hwnd_value), final.SW_MAXIMIZE)
        time.sleep(2.5)
        report["steps"].append({"step": "maximize", "sample": final.sample(hwnd_value, "maximize-attempt")})

        user32.ShowWindow(wt.HWND(hwnd_value), final.SW_RESTORE)
        time.sleep(2.5)
        report["steps"].append({"step": "restore", "sample": final.sample(hwnd_value, "restore-attempt")})

    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
