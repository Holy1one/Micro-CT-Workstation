"""Isolate the maximize behaviour: sample without touching window state, then
explicitly request maximize/restore and re-sample."""

import ctypes
import ctypes.wintypes as wt
import json
import os
import time

ROOT = r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App"
OUT = os.path.join(ROOT, "docs", "shots")
REPORT = os.path.join(OUT, "window-geometry-isolate.json")

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.GetWindowTextLengthW.argtypes = [wt.HWND]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.IsZoomed.argtypes = [wt.HWND]
user32.IsZoomed.restype = wt.BOOL
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.GetClientRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
user32.ShowWindow.restype = wt.BOOL
user32.IsWindowVisible.argtypes = [wt.HWND]
ENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
user32.EnumWindows.argtypes = [ENUMPROC, wt.LPARAM]


class WINDOWPLACEMENT(ctypes.Structure):
    _fields_ = [("length", wt.UINT), ("flags", wt.UINT), ("showCmd", wt.UINT),
                ("ptMinPosition", wt.POINT), ("ptMaxPosition", wt.POINT),
                ("rcNormalPosition", wt.RECT)]


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
        if user32.IsWindowVisible(hwnd) and class_name(hwnd) == "Tauri Window":
            found.append(int(hwnd))
        return True

    user32.EnumWindows(visit, 0)
    return found


def style_bits(hwnd):
    GWL_STYLE = -16
    get_long = getattr(user32, "GetWindowLongPtrW", user32.GetWindowLongW)
    get_long.argtypes = [wt.HWND, ctypes.c_int]
    get_long.restype = ctypes.c_longlong if hasattr(user32, "GetWindowLongPtrW") else ctypes.c_long
    style = get_long(wt.HWND(hwnd), GWL_STYLE)
    return {
        "styleHex": hex(style & 0xFFFFFFFF),
        "WS_MAXIMIZE": bool(style & 0x01000000),
        "WS_MAXIMIZEBOX": bool(style & 0x00010000),
        "WS_MINIMIZEBOX": bool(style & 0x00020000),
        "WS_THICKFRAME": bool(style & 0x00040000),
        "WS_CAPTION": bool(style & 0x00C00000),
        "WS_POPUP": bool(style & 0x80000000),
        "WS_OVERLAPPEDWINDOW": bool(style & 0x00CF0000),
    }


def snapshot(label):
    samples = []
    for hwnd_value in find_app():
        rect = wt.RECT()
        client = wt.RECT()
        user32.GetWindowRect(wt.HWND(hwnd_value), ctypes.byref(rect))
        user32.GetClientRect(wt.HWND(hwnd_value), ctypes.byref(client))
        placement = WINDOWPLACEMENT()
        placement.length = ctypes.sizeof(WINDOWPLACEMENT)
        user32.GetWindowPlacement(wt.HWND(hwnd_value), ctypes.byref(placement))
        samples.append({
            "label": label,
            "hwnd": hwnd_value,
            "title": window_text(wt.HWND(hwnd_value))[:60],
            "isZoomed": bool(user32.IsZoomed(wt.HWND(hwnd_value))),
            "showCmd": placement.showCmd,
            "rcNormalPosition": [placement.rcNormalPosition.left, placement.rcNormalPosition.top,
                                 placement.rcNormalPosition.right, placement.rcNormalPosition.bottom],
            "outer": [rect.left, rect.top, rect.right, rect.bottom],
            "outerSize": [rect.right - rect.left, rect.bottom - rect.top],
            "clientSize": [client.right - client.left, client.bottom - client.top],
            "style": style_bits(hwnd_value),
        })
    return samples


def main():
    report = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "steps": []}
    windows = find_app()
    report["windowCount"] = len(windows)
    report["steps"].append({"step": "observed-no-touch", "windows": snapshot("observed")})
    time.sleep(0.5)
    report["steps"].append({"step": "observed-again", "windows": snapshot("observed2")})

    hwnd_value = windows[0] if windows else None
    if hwnd_value:
        result = user32.ShowWindow(wt.HWND(hwnd_value), 3)  # SW_MAXIMIZE
        report["showMaximizeResult"] = bool(result)
        time.sleep(1.5)
        report["steps"].append({"step": "after-SW_MAXIMIZE", "windows": snapshot("maximized")})

        result = user32.ShowWindow(wt.HWND(hwnd_value), 9)  # SW_RESTORE
        report["showRestoreResult"] = bool(result)
        time.sleep(1.5)
        report["steps"].append({"step": "after-SW_RESTORE", "windows": snapshot("restored")})

    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)


if __name__ == "__main__":
    main()
