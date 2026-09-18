"""Decisive maximize/restore verification on the primary monitor.

Waits for the window to be stable, records IsZoomed + placement + geometry and
grabs a real GDI BitBlt crop for each state. Uses SetWindowPos to bring the
window back onto the primary monitor before restoring so the geometry is
comparable.
"""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time
import urllib.request

import qa_window_geometry_final as final

ROOT = r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App"
OUT = os.path.join(ROOT, "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-verify.json")

user32 = final.user32


def ensure_primary(hwnd_value):
    user32.SetWindowPos(wt.HWND(hwnd_value), wt.HWND(0), 60, 60, 0, 0,
                        final.SWP_NOSIZE | final.SWP_NOZORDER)
    time.sleep(1.0)


def main():
    hwnd_value = final.find_app()
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "hwnd": hwnd_value, "samples": []}
    if not hwnd_value:
        report["error"] = "window not found"
    else:
        hwnd = wt.HWND(hwnd_value)
        ensure_primary(hwnd_value)
        report["samples"].append(final.sample(hwnd_value, "observed-startup"))

        user32.SetForegroundWindow(hwnd)
        user32.ShowWindow(hwnd, final.SW_MAXIMIZE)
        time.sleep(2.5)
        report["samples"].append(final.sample(hwnd_value, "maximized"))

        user32.ShowWindow(hwnd, final.SW_RESTORE)
        time.sleep(2.5)
        report["samples"].append(final.sample(hwnd_value, "restored"))

        user32.ShowWindow(hwnd, final.SW_MAXIMIZE)
        time.sleep(2.5)
        report["samples"].append(final.sample(hwnd_value, "re-maximized"))

    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
