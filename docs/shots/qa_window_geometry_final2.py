"""Final decisive geometry + BitBlt evidence with a clean, unforced startup.

Does not resize or maximize anything. Relaunch the app first so this reflects
the real startup state.
"""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

import qa_window_geometry_final as final

OUT = os.path.join(r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App",
                   "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-clean-startup.json")

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.GetCurrentThreadId.restype = wt.DWORD
user32 = final.user32
user32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.BringWindowToTop.argtypes = [wt.HWND]
HWND_TOPMOST = wt.HWND(-1)
SWP_SHOWWINDOW = 0x0040
SWP_NOZORDER = 0x0004


def force_foreground(hwnd_value):
    target = wt.HWND(hwnd_value)
    current = kernel32.GetCurrentThreadId()
    pid = wt.DWORD()
    other = user32.GetWindowThreadProcessId(target, ctypes.byref(pid))
    user32.AttachThreadInput(current, other, True)
    try:
        user32.BringWindowToTop(target)
        user32.SetWindowPos(target, HWND_TOPMOST, 0, 0, 0, 0,
                            final.SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW)
        user32.SetForegroundWindow(target)
    finally:
        user32.AttachThreadInput(current, other, False)
    time.sleep(1.2)
    return int(user32.GetForegroundWindow())


def main():
    hwnd_value = final.find_app()
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
              "hwnd": hwnd_value, "note": "no resize/maximize command issued"}
    if not hwnd_value:
        report["error"] = "window not found"
    else:
        report["foregroundAfterAttach"] = force_foreground(hwnd_value)
        time.sleep(1.5)
        report["sample"] = final.sample(hwnd_value, "clean-startup")
    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
