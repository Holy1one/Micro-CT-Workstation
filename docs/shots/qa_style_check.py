"""Read WS_MAXIMIZE / placement state to resolve the maximize contradiction."""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

import qa_window_geometry_final as final

OUT = os.path.join(r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App",
                   "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-style.json")

user32 = final.user32
GWL_STYLE = -16
user32.GetWindowLongPtrW.argtypes = [wt.HWND, ctypes.c_int]
user32.GetWindowLongPtrW.restype = ctypes.c_ssize_t


class PLACEMENT(ctypes.Structure):
    _fields_ = [("length", wt.UINT), ("flags", wt.UINT), ("showCmd", wt.UINT),
                ("ptMinPosition", wt.POINT), ("ptMaxPosition", wt.POINT),
                ("rcNormalPosition", wt.RECT)]


def describe(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    style = user32.GetWindowLongPtrW(hwnd, GWL_STYLE)
    placement = PLACEMENT()
    placement.length = ctypes.sizeof(PLACEMENT)
    user32.GetWindowPlacement(hwnd, ctypes.byref(placement))
    rect = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    return {
        "hwnd": hwnd_value,
        "styleHex": hex(style & 0xFFFFFFFF),
        "WS_MAXIMIZE": bool(style & 0x01000000),
        "isZoomed": bool(user32.IsZoomed(hwnd)),
        "showCmd": placement.showCmd,
        "rcNormalPosition": [placement.rcNormalPosition.left, placement.rcNormalPosition.top,
                             placement.rcNormalPosition.right, placement.rcNormalPosition.bottom],
        "outer": [rect.left, rect.top, rect.right, rect.bottom],
    }


def main():
    hwnd_value = final.find_app()
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "observed": None,
              "afterShowMaximize": None, "afterShowRestore": None, "afterShowMaximizeAgain": None}
    if hwnd_value:
        report["observed"] = describe(hwnd_value)
        user32.ShowWindow(wt.HWND(hwnd_value), final.SW_MAXIMIZE)
        time.sleep(2.0)
        report["afterShowMaximize"] = describe(hwnd_value)
        user32.ShowWindow(wt.HWND(hwnd_value), final.SW_RESTORE)
        time.sleep(2.0)
        report["afterShowRestore"] = describe(hwnd_value)
        user32.ShowWindow(wt.HWND(hwnd_value), final.SW_MAXIMIZE)
        time.sleep(2.0)
        report["afterShowMaximizeAgain"] = describe(hwnd_value)
    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
