"""Definitive maximize check: call ShowWindow(SW_MAXIMIZE) and observe whether
the OS actually enters the maximized state."""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

import qa_window_geometry_final as final

OUT = os.path.join(r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App",
                   "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-maximize-definitive.json")

user32 = final.user32
user32.IsZoomed.argtypes = [wt.HWND]
user32.IsZoomed.restype = wt.BOOL
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
user32.ShowWindow.restype = wt.BOOL


def quick(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    rect = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    return {
        "isZoomed": bool(user32.IsZoomed(hwnd)),
        "outer": [rect.left, rect.top, rect.right, rect.bottom],
        "outerSize": [rect.right - rect.left, rect.bottom - rect.top],
    }


def main():
    hwnd_value = final.find_app()
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "hwnd": hwnd_value}
    if not hwnd_value:
        report["error"] = "window not found"
    else:
        report["before"] = quick(hwnd_value)
        result = user32.ShowWindow(wt.HWND(hwnd_value), final.SW_MAXIMIZE)
        report["showWindowMaximizeReturned"] = bool(result)
        time.sleep(1.0)
        report["after1s"] = quick(hwnd_value)
        time.sleep(2.0)
        report["after3s"] = quick(hwnd_value)
        report["after3sFull"] = final.sample(hwnd_value, "after-maximize-command")
    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
