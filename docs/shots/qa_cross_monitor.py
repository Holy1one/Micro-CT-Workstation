"""Cross-monitor move + maximize re-calculation check."""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

import qa_window_geometry_final as final

OUT = os.path.join(r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App",
                   "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-cross-monitor.json")

user32 = final.user32
user32.SetWindowPos.argtypes = [wt.HWND, wt.HWND, ctypes.c_int, ctypes.c_int,
                                ctypes.c_int, ctypes.c_int, wt.UINT]
user32.SetWindowPos.restype = wt.BOOL


def main():
    hwnd_value = final.find_app()
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "hwnd": hwnd_value, "steps": []}
    if not hwnd_value:
        report["error"] = "window not found"
    else:
        hwnd = wt.HWND(hwnd_value)
        user32.ShowWindow(hwnd, final.SW_RESTORE)
        time.sleep(1.5)
        report["steps"].append({"step": "restored-primary", "sample": final.sample(hwnd_value, "cross-restored-primary")})

        # Move the restored window onto the secondary monitor (x >= 1920).
        user32.SetWindowPos(hwnd, wt.HWND(0), 2400, 120, 0, 0,
                            final.SWP_NOSIZE | final.SWP_NOZORDER)
        time.sleep(1.8)  # exceed the 160 ms debounce
        report["steps"].append({"step": "moved-secondary", "sample": final.sample(hwnd_value, "cross-moved-secondary")})

        user32.ShowWindow(hwnd, final.SW_MAXIMIZE)
        time.sleep(2.0)
        report["steps"].append({"step": "maximized-secondary", "sample": final.sample(hwnd_value, "cross-maximized-secondary")})

        user32.ShowWindow(hwnd, final.SW_RESTORE)
        time.sleep(1.5)
        user32.SetWindowPos(hwnd, wt.HWND(0), 60, 60, 0, 0,
                            final.SWP_NOSIZE | final.SWP_NOZORDER)
        time.sleep(1.8)
        user32.ShowWindow(hwnd, final.SW_MAXIMIZE)
        time.sleep(2.0)
        report["steps"].append({"step": "back-primary-maximized", "sample": final.sample(hwnd_value, "cross-back-primary")})

    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
