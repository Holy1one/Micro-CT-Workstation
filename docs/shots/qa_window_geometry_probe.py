"""Real desktop geometry + GDI BitBlt probe for the Tauri Micro-CT window.

Collects, for maximized / restored / re-maximized:
  - HWND, IsZoomed, GetWindowPlacement.showCmd
  - MonitorFromWindow -> MONITORINFO rcMonitor / rcWork
  - DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS) visible frame
  - GetClientRect + ClientToScreen -> client rect in screen coordinates
  - GetDpiForWindow / GetDpiForMonitor / devicePixelRatio basis
  - expected canvas zoom + scaled canvas + symmetric gutters computed from client size
  - full virtual-desktop BitBlt PNG (GetDC(NULL) + BitBlt SRCCOPY), plus window crop

It never uses PrintWindow.
"""

import ctypes
import ctypes.wintypes as wt
import json
import os
import struct
import time
import urllib.request
import zlib

ROOT = r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App"
OUT = os.path.join(ROOT, "docs", "shots")
DOM_URL = "http://127.0.0.1:4173/__qa_dom_geometry__"

user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi", use_last_error=True)
shcore = ctypes.WinDLL("shcore", use_last_error=True)

SW_MAXIMIZE = 3
SW_RESTORE = 9
SW_SHOWNORMAL = 1
SW_SHOWMAXIMIZED = 3
SRCCOPY = 0x00CC0020
DIB_RGB_COLORS = 0
DWMWA_EXTENDED_FRAME_BOUNDS = 9
MDT_EFFECTIVE_DPI = 0
MONITOR_DEFAULTTONEAREST = 2
SM_XVIRTUALSCREEN = 76
SM_YVIRTUALSCREEN = 77
SM_CXVIRTUALSCREEN = 78
SM_CYVIRTUALSCREEN = 79
SM_CMONITORS = 80

user32.IsZoomed.argtypes = [wt.HWND]
user32.IsZoomed.restype = wt.BOOL
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.GetClientRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.ClientToScreen.argtypes = [wt.HWND, ctypes.POINTER(wt.POINT)]
user32.GetWindowPlacement.argtypes = [wt.HWND, ctypes.c_void_p]
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
user32.SetForegroundWindow.argtypes = [wt.HWND]
user32.GetForegroundWindow.restype = wt.HWND
user32.MonitorFromWindow.argtypes = [wt.HWND, wt.DWORD]
user32.MonitorFromWindow.restype = wt.HMONITOR
user32.GetMonitorInfoW.argtypes = [wt.HMONITOR, ctypes.c_void_p]
user32.GetSystemMetrics.argtypes = [ctypes.c_int]
user32.GetWindowTextLengthW.argtypes = [wt.HWND]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.GetDC.argtypes = [wt.HWND]
user32.GetDC.restype = wt.HDC
user32.ReleaseDC.argtypes = [wt.HWND, wt.HDC]
user32.GetDpiForWindow.argtypes = [wt.HWND]
user32.GetDpiForWindow.restype = ctypes.c_uint
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]

ENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
user32.EnumWindows.argtypes = [ENUMPROC, wt.LPARAM]

dwmapi.DwmGetWindowAttribute.argtypes = [wt.HWND, wt.DWORD, ctypes.c_void_p, wt.DWORD]
gdi32.CreateCompatibleDC.argtypes = [wt.HDC]
gdi32.CreateCompatibleDC.restype = wt.HDC
gdi32.CreateCompatibleBitmap.argtypes = [wt.HDC, ctypes.c_int, ctypes.c_int]
gdi32.CreateCompatibleBitmap.restype = wt.HBITMAP
gdi32.SelectObject.argtypes = [wt.HDC, wt.HGDIOBJ]
gdi32.SelectObject.restype = wt.HGDIOBJ
gdi32.BitBlt.argtypes = [wt.HDC, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                         wt.HDC, ctypes.c_int, ctypes.c_int, wt.DWORD]
gdi32.BitBlt.restype = wt.BOOL
gdi32.GetDIBits.argtypes = [wt.HDC, wt.HBITMAP, ctypes.c_uint, ctypes.c_uint,
                            ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint]
gdi32.GetDIBits.restype = ctypes.c_int
gdi32.DeleteObject.argtypes = [wt.HGDIOBJ]
gdi32.DeleteDC.argtypes = [wt.HDC]

shcore.GetDpiForMonitor.argtypes = [wt.HMONITOR, ctypes.c_int,
                                    ctypes.POINTER(ctypes.c_uint),
                                    ctypes.POINTER(ctypes.c_uint)]


class WINDOWPLACEMENT(ctypes.Structure):
    _fields_ = [("length", wt.UINT), ("flags", wt.UINT), ("showCmd", wt.UINT),
                ("ptMinPosition", wt.POINT), ("ptMaxPosition", wt.POINT),
                ("rcNormalPosition", wt.RECT)]


class MONITORINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("rcMonitor", wt.RECT), ("rcWork", wt.RECT),
                ("dwFlags", wt.DWORD)]


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [("biSize", wt.DWORD), ("biWidth", ctypes.c_long), ("biHeight", ctypes.c_long),
                ("biPlanes", wt.WORD), ("biBitCount", wt.WORD), ("biCompression", wt.DWORD),
                ("biSizeImage", wt.DWORD), ("biXPelsPerMeter", ctypes.c_long),
                ("biYPelsPerMeter", ctypes.c_long), ("biClrUsed", wt.DWORD),
                ("biClrImportant", wt.DWORD)]


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
            title = window_text(hwnd)
            if title == "Micro-CT Workstation" or title.startswith("QA_GEOM|"):
                found.append(int(hwnd))
        return True

    user32.EnumWindows(visit, 0)
    return found[0] if found else None


def rect_to_list(rect):
    return [rect.left, rect.top, rect.right, rect.bottom]


def os_outer(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    rect = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    return rect_to_list(rect)


def dwm_frame(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    rect = wt.RECT()
    result = dwmapi.DwmGetWindowAttribute(
        hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, ctypes.byref(rect), ctypes.sizeof(rect))
    if result != 0:
        return None
    return rect_to_list(rect)


def client_screen_rect(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    rect = wt.RECT()
    if not user32.GetClientRect(hwnd, ctypes.byref(rect)):
        return None, None
    size = [rect.right - rect.left, rect.bottom - rect.top]
    top_left = wt.POINT(rect.left, rect.top)
    user32.ClientToScreen(hwnd, ctypes.byref(top_left))
    return [top_left.x, top_left.y, top_left.x + size[0], top_left.y + size[1]], size


def monitor_info(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    monitor = user32.MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
    info = MONITORINFO()
    info.cbSize = ctypes.sizeof(MONITORINFO)
    user32.GetMonitorInfoW(monitor, ctypes.byref(info))
    dpi_x = ctypes.c_uint()
    dpi_y = ctypes.c_uint()
    shcore.GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI,
                            ctypes.byref(dpi_x), ctypes.byref(dpi_y))
    return {
        "handle": int(monitor),
        "rcMonitor": rect_to_list(info.rcMonitor),
        "rcWork": rect_to_list(info.rcWork),
        "dpiX": dpi_x.value,
        "dpiY": dpi_y.value,
        "scalePercent": round(dpi_x.value * 100 / 96),
    }


def window_placement(hwnd_value):
    hwnd = wt.HWND(hwnd_value)
    placement = WINDOWPLACEMENT()
    placement.length = ctypes.sizeof(WINDOWPLACEMENT)
    user32.GetWindowPlacement(hwnd, ctypes.byref(placement))
    return {"showCmd": placement.showCmd,
            "rcNormalPosition": rect_to_list(placement.rcNormalPosition)}


def dpi_for_window(hwnd_value):
    try:
        return user32.GetDpiForWindow(wt.HWND(hwnd_value))
    except Exception:
        return None


def dom_geometry(hwnd_value=None):
    """Prefer HTTP POST samples; fall back to the QA title payload on the window."""
    try:
        with urllib.request.urlopen(DOM_URL, timeout=2) as response:
            data = json.loads(response.read().decode("utf-8"))
            if isinstance(data, dict) and data.get("innerWidth"):
                return data
    except Exception:
        pass
    if not hwnd_value:
        return {"error": "no DOM sample source"}
    title = window_text(wt.HWND(hwnd_value))
    marker = "QA_GEOM|"
    if marker not in title:
        return {"error": "title payload missing", "title": title}
    try:
        return json.loads(title.split(marker, 1)[1])
    except Exception as error:
        return {"error": str(error), "title": title[:200]}


def expected_canvas(client_width, client_height):
    zoom = min(client_width / 1600.0, client_height / 1000.0)
    width = 1600.0 * zoom
    height = 1000.0 * zoom
    return {
        "zoom": zoom,
        "scaledWidth": width,
        "scaledHeight": height,
        "gutterX": max(0.0, (client_width - width) / 2.0),
        "gutterY": max(0.0, (client_height - height) / 2.0),
    }


def grab_virtual_desktop():
    x = user32.GetSystemMetrics(SM_XVIRTUALSCREEN)
    y = user32.GetSystemMetrics(SM_YVIRTUALSCREEN)
    width = user32.GetSystemMetrics(SM_CXVIRTUALSCREEN)
    height = user32.GetSystemMetrics(SM_CYVIRTUALSCREEN)
    screen_dc = user32.GetDC(None)
    mem_dc = gdi32.CreateCompatibleDC(screen_dc)
    bitmap = gdi32.CreateCompatibleBitmap(screen_dc, width, height)
    previous = gdi32.SelectObject(mem_dc, bitmap)
    ok = bool(gdi32.BitBlt(mem_dc, 0, 0, width, height, screen_dc, x, y, SRCCOPY))
    info = BITMAPINFOHEADER()
    info.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    info.biWidth = width
    info.biHeight = -height
    info.biPlanes = 1
    info.biBitCount = 32
    buffer = ctypes.create_string_buffer(width * height * 4)
    rows = gdi32.GetDIBits(mem_dc, bitmap, 0, height,
                           ctypes.cast(buffer, ctypes.c_void_p),
                           ctypes.byref(info), DIB_RGB_COLORS)
    gdi32.SelectObject(mem_dc, previous)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(mem_dc)
    user32.ReleaseDC(None, screen_dc)
    if not ok or rows != height:
        raise OSError(ctypes.get_last_error(), "BitBlt/GetDIBits failed")
    return {"origin": [x, y], "size": [width, height], "data": buffer.raw}


def write_png(path, width, height, data):
    raw = bytearray()
    stride = width * 4
    for row_index in range(height):
        raw.append(0)
        row = data[row_index * stride:(row_index + 1) * stride]
        for i in range(0, len(row), 4):
            raw.extend((row[i + 2], row[i + 1], row[i], 255))

    def chunk(tag, payload):
        value = tag + payload
        return (struct.pack(">I", len(payload)) + value
                + struct.pack(">I", zlib.crc32(value) & 0xFFFFFFFF))

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(bytes(raw), 6))
           + chunk(b"IEND", b""))
    with open(path, "wb") as handle:
        handle.write(png)


def crop_png(path, desktop, box):
    x0, y0, x1, y1 = box
    origin_x, origin_y = desktop["origin"]
    width, height = desktop["size"]
    data = desktop["data"]
    stride = width * 4
    left = max(0, min(width, x0 - origin_x))
    top = max(0, min(height, y0 - origin_y))
    right = max(0, min(width, x1 - origin_x))
    bottom = max(0, min(height, y1 - origin_y))
    crop_w = max(1, right - left)
    crop_h = max(1, bottom - top)
    rows = []
    for row_index in range(top, bottom):
        start = row_index * stride + left * 4
        rows.append(data[start:start + crop_w * 4])
    write_png(path, crop_w, crop_h, b"".join(rows))
    return [crop_w, crop_h]


def topmost_overlaps(hwnd_value, frame):
    """Return visible windows that intersect the target visible frame."""
    overlaps = []

    @ENUMPROC
    def visit(other, _):
        if int(other) == hwnd_value or not user32.IsWindowVisible(other):
            return True
        rect = wt.RECT()
        user32.GetWindowRect(other, ctypes.byref(rect))
        other_rect = rect_to_list(rect)
        if (other_rect[0] < frame[2] and other_rect[2] > frame[0]
                and other_rect[1] < frame[3] and other_rect[3] > frame[1]):
            overlaps.append({"hwnd": int(other), "title": window_text(other),
                             "class": class_name(other), "rect": other_rect})
        return True

    user32.EnumWindows(visit, 0)
    return overlaps


def sample_state(hwnd_value, state, desktop_dir):
    hwnd = wt.HWND(hwnd_value)
    user32.SetForegroundWindow(hwnd)
    time.sleep(0.4)
    outer = os_outer(hwnd_value)
    frame = dwm_frame(hwnd_value)
    client_rect, client_size = client_screen_rect(hwnd_value)
    monitor = monitor_info(hwnd_value)
    placement = window_placement(hwnd_value)
    sample = {
        "state": state,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "hwnd": hwnd_value,
        "pid": None,
        "isZoomed": bool(user32.IsZoomed(hwnd)),
        "windowPlacement": placement,
        "osOuterRect": outer,
        "dwmVisibleFrame": frame,
        "clientScreenRect": client_rect,
        "clientSize": client_size,
        "monitor": monitor,
        "dpiForWindow": dpi_for_window(hwnd_value),
        "systemMetrics": {
            "monitorCount": user32.GetSystemMetrics(SM_CMONITORS),
            "virtualScreen": [user32.GetSystemMetrics(SM_XVIRTUALSCREEN),
                              user32.GetSystemMetrics(SM_YVIRTUALSCREEN),
                              user32.GetSystemMetrics(SM_CXVIRTUALSCREEN),
                              user32.GetSystemMetrics(SM_CYVIRTUALSCREEN)],
        },
        "foregroundHwnd": int(user32.GetForegroundWindow()),
    }
    sample["dom"] = dom_geometry(hwnd_value)
    if client_size:
        sample["expectedCanvas"] = expected_canvas(client_size[0], client_size[1])
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    sample["pid"] = pid.value

    desktop = grab_virtual_desktop()
    desktop_path = os.path.join(desktop_dir, f"window-geometry-{state}-virtual-desktop-bitblt.png")
    write_png(desktop_path, desktop["size"][0], desktop["size"][1], desktop["data"])
    box = frame or outer
    crop_path = os.path.join(desktop_dir, f"window-geometry-{state}-window-crop-bitblt.png")
    crop_size = crop_png(crop_path, desktop, box)
    sample["evidence"] = {
        "api": "GetDC(NULL)+CreateCompatibleDC+BitBlt(SRCCOPY)",
        "printWindowUsed": False,
        "virtualDesktopPng": desktop_path,
        "windowCropPng": crop_path,
        "windowCropSize": crop_size,
        "cropBox": box,
        "foregroundMatchesTarget": sample["foregroundHwnd"] == hwnd_value,
        "overlappingWindows": topmost_overlaps(hwnd_value, box),
    }

    with open(os.path.join(desktop_dir, f"window-geometry-{state}.json"), "w",
              encoding="utf-8") as handle:
        json.dump(sample, handle, ensure_ascii=False, indent=2)
    return sample


def main():
    hwnd_value = find_app()
    report = {"hwnd": hwnd_value, "samples": []}
    if not hwnd_value:
        report["error"] = "Micro-CT Workstation window not found"
    else:
        hwnd = wt.HWND(hwnd_value)
        report["samples"].append(sample_state(hwnd_value, "maximized", OUT))
        user32.ShowWindow(hwnd, SW_RESTORE)
        time.sleep(1.5)
        report["samples"].append(sample_state(hwnd_value, "restored", OUT))
        user32.ShowWindow(hwnd, SW_MAXIMIZE)
        time.sleep(1.5)
        report["samples"].append(sample_state(hwnd_value, "remaximized", OUT))

    with open(os.path.join(OUT, "window-geometry-summary.json"), "w",
              encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(os.path.join(OUT, "window-geometry-summary.json"))


if __name__ == "__main__":
    main()
