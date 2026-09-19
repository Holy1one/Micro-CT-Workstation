"""Final geometry probe: force the window onto the primary monitor, verify the
maximized/restored contract, and capture GDI BitBlt evidence for each state."""

import ctypes
import ctypes.wintypes as wt
import json
import os
import struct
import time
import urllib.request
import zlib

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "docs", "shots")
DOM_URL = "http://127.0.0.1:4173/__qa_dom_geometry__"
PRIMARY_WORK = (0, 0, 1920, 1032)

user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi", use_last_error=True)
shcore = ctypes.WinDLL("shcore", use_last_error=True)

SW_MAXIMIZE = 3
SW_RESTORE = 9
SWP_NOSIZE = 0x0001
SWP_NOZORDER = 0x0004
HWND_TOP = 0
SRCCOPY = 0x00CC0020
DIB_RGB_COLORS = 0
DWMWA_EXTENDED_FRAME_BOUNDS = 9
SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN = 76, 77, 78, 79
SM_CMONITORS = 80
MDT_EFFECTIVE_DPI = 0

for name, restype in [
    ("IsZoomed", wt.BOOL), ("IsWindowVisible", wt.BOOL), ("GetWindowRect", wt.BOOL),
    ("GetClientRect", wt.BOOL), ("ShowWindow", wt.BOOL), ("SetForegroundWindow", wt.BOOL),
    ("SetWindowPos", wt.BOOL), ("ClientToScreen", wt.BOOL), ("GetMonitorInfoW", wt.BOOL),
]:
    fn = getattr(user32, name)
    fn.restype = restype

user32.IsZoomed.argtypes = [wt.HWND]
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.GetClientRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.ClientToScreen.argtypes = [wt.HWND, ctypes.POINTER(wt.POINT)]
user32.GetWindowTextLengthW.argtypes = [wt.HWND]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.SetWindowPos.argtypes = [wt.HWND, wt.HWND, ctypes.c_int, ctypes.c_int,
                                ctypes.c_int, ctypes.c_int, wt.UINT]
user32.SetForegroundWindow.argtypes = [wt.HWND]
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
user32.PrintWindow.argtypes = [wt.HWND, wt.HDC, wt.UINT]
user32.PrintWindow.restype = wt.BOOL
user32.MonitorFromWindow.argtypes = [wt.HWND, wt.DWORD]
user32.MonitorFromWindow.restype = wt.HMONITOR
user32.GetMonitorInfoW.argtypes = [wt.HMONITOR, ctypes.c_void_p]
user32.GetSystemMetrics.argtypes = [ctypes.c_int]
user32.GetSystemMetrics.restype = ctypes.c_int
user32.GetDC.argtypes = [wt.HWND]
user32.GetDC.restype = wt.HDC
user32.ReleaseDC.argtypes = [wt.HWND, wt.HDC]
user32.GetDpiForWindow.argtypes = [wt.HWND]
user32.GetDpiForWindow.restype = ctypes.c_uint
user32.GetWindowPlacement.argtypes = [wt.HWND, ctypes.c_void_p]

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
                                    ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint)]


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
        if user32.IsWindowVisible(hwnd) and class_name(hwnd) == "Tauri Window" \
                and window_text(hwnd) == "Micro-CT Workstation":
            found.append(int(hwnd))
        return True

    user32.EnumWindows(visit, 0)
    return found[0] if found else None


def rect_list(rect):
    return [rect.left, rect.top, rect.right, rect.bottom]


def dom_geometry():
    try:
        with urllib.request.urlopen(DOM_URL, timeout=2) as response:
            data = json.loads(response.read().decode("utf-8"))
            if isinstance(data, dict) and data.get("innerWidth"):
                return data
    except Exception as error:
        return {"error": str(error)}
    return {"error": "no sample"}


DESIGN_WIDTH = 1920.0
DESIGN_HEIGHT = 1080.0
MIN_DESIGN_HEIGHT = DESIGN_HEIGHT * 0.9
MAX_DESIGN_HEIGHT = 1400.0


def expected_canvas(width, height):
    """Mirror of src/canvas-layout.ts computeCanvasLayout() at the 1920x1080 baseline.

    Width leads: the design canvas always fills the client width and its height
    absorbs the remaining aspect ratio. Only extreme aspect ratios fall back to
    a symmetric contain gutter.
    """
    safe_width = max(0.0, float(width))
    safe_height = max(0.0, float(height))
    fill_zoom = safe_width / DESIGN_WIDTH if safe_width > 0 else 0.0
    fill_height = safe_height / fill_zoom if fill_zoom > 0 else 0.0
    can_fill = (fill_zoom > 0
                and MIN_DESIGN_HEIGHT <= fill_height <= MAX_DESIGN_HEIGHT)
    zoom = fill_zoom if can_fill else min(
        safe_width / DESIGN_WIDTH if safe_width > 0 else 0.0,
        safe_height / DESIGN_HEIGHT if safe_height > 0 else 0.0,
    )
    design_height = fill_height if can_fill else DESIGN_HEIGHT
    sw = DESIGN_WIDTH * zoom
    sh = design_height * zoom
    return {"mode": "fill" if can_fill else "contain",
            "designHeight": design_height,
            "zoom": zoom, "scaledWidth": sw, "scaledHeight": sh,
            "gutterX": max(0.0, (safe_width - sw) / 2.0),
            "gutterY": max(0.0, (safe_height - sh) / 2.0)}


PW_RENDERFULLCONTENT = 0x00000002
SW_SHOWNOACTIVATE = 4
SW_MINIMIZE = 6
# Full-virtual-desktop plates steal the foreground and interrupt the operator.
# Window-level capture is the default; set MICROCT_QA_DESKTOP_CAPTURE=1 for the
# legacy desktop plate when a compositing artefact has to be ruled out.
DESKTOP_CAPTURE = os.environ.get("MICROCT_QA_DESKTOP_CAPTURE", "") not in ("", "0", "false")


def grab_window(hwnd, width, height):
    """Capture the application window itself.

    No SetForegroundWindow, no GetDC(NULL) virtual-desktop plate: the window is
    asked to render into a memory DC. PW_RENDERFULLCONTENT is mandatory for the
    WebView2/Chromium surface -- PrintWindow(hwnd, dc, 0) only yields a flat
    dark plate (242 colours, 2 mid-row edges) instead of the real UI.
    """
    screen_dc = user32.GetDC(None)
    mem_dc = gdi32.CreateCompatibleDC(screen_dc)
    bitmap = gdi32.CreateCompatibleBitmap(screen_dc, width, height)
    previous = gdi32.SelectObject(mem_dc, bitmap)
    ok = bool(user32.PrintWindow(hwnd, mem_dc, PW_RENDERFULLCONTENT))
    info = BITMAPINFOHEADER()
    info.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    info.biWidth = width
    info.biHeight = -height
    info.biPlanes = 1
    info.biBitCount = 32
    buffer = ctypes.create_string_buffer(width * height * 4)
    rows = gdi32.GetDIBits(mem_dc, bitmap, 0, height, ctypes.cast(buffer, ctypes.c_void_p),
                           ctypes.byref(info), DIB_RGB_COLORS)
    gdi32.SelectObject(mem_dc, previous)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(mem_dc)
    user32.ReleaseDC(None, screen_dc)
    if not ok or rows != height:
        raise OSError(ctypes.get_last_error(), "PrintWindow failed")
    return buffer.raw


def score_pixels(data, width, height):
    """Tell a real render from a blank plate without opening the image."""
    colors = set()
    luma_sum = 0
    total = 0
    for off in range(0, len(data), 16):
        b, g, r = data[off], data[off + 1], data[off + 2]
        colors.add((r, g, b))
        luma_sum += (r * 299 + g * 587 + b * 114) // 1000
        total += 1
    row = height // 2
    base = row * width * 4
    edges = 0
    previous_pixel = None
    for x in range(0, width, 2):
        off = base + x * 4
        pixel = (data[off + 2], data[off + 1], data[off])
        if previous_pixel is not None and (
                abs(pixel[0] - previous_pixel[0]) + abs(pixel[1] - previous_pixel[1])
                + abs(pixel[2] - previous_pixel[2])) > 24:
            edges += 1
        previous_pixel = pixel
    return {
        "uniqueColors": len(colors),
        "meanLuma": round(luma_sum / total, 2) if total else None,
        "midRowEdges": edges,
        "verdict": "rendered" if len(colors) > 64 and edges > 8 else "blank-or-flat",
    }


def grab_desktop():
    x = user32.GetSystemMetrics(SM_XVIRTUALSCREEN)
    y = user32.GetSystemMetrics(SM_YVIRTUALSCREEN)
    w = user32.GetSystemMetrics(SM_CXVIRTUALSCREEN)
    h = user32.GetSystemMetrics(SM_CYVIRTUALSCREEN)
    screen_dc = user32.GetDC(None)
    mem_dc = gdi32.CreateCompatibleDC(screen_dc)
    bitmap = gdi32.CreateCompatibleBitmap(screen_dc, w, h)
    previous = gdi32.SelectObject(mem_dc, bitmap)
    ok = bool(gdi32.BitBlt(mem_dc, 0, 0, w, h, screen_dc, x, y, SRCCOPY))
    info = BITMAPINFOHEADER()
    info.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    info.biWidth = w
    info.biHeight = -h
    info.biPlanes = 1
    info.biBitCount = 32
    buffer = ctypes.create_string_buffer(w * h * 4)
    rows = gdi32.GetDIBits(mem_dc, bitmap, 0, h, ctypes.cast(buffer, ctypes.c_void_p),
                           ctypes.byref(info), DIB_RGB_COLORS)
    gdi32.SelectObject(mem_dc, previous)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(mem_dc)
    user32.ReleaseDC(None, screen_dc)
    if not ok or rows != h:
        raise OSError(ctypes.get_last_error(), "BitBlt failed")
    return {"origin": [x, y], "size": [w, h], "data": buffer.raw}


def write_png(path, width, height, data, stride_step=1):
    raw = bytearray()
    stride = width * 4
    for row_index in range(0, height, stride_step):
        raw.append(0)
        row = data[row_index * stride:(row_index + 1) * stride]
        for i in range(0, len(row), 4 * stride_step):
            raw.extend((row[i + 2], row[i + 1], row[i], 255))

    def chunk(tag, payload):
        value = tag + payload
        return (struct.pack(">I", len(payload)) + value
                + struct.pack(">I", zlib.crc32(value) & 0xFFFFFFFF))

    out_w = width // stride_step
    out_h = height // stride_step
    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", out_w, out_h, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(bytes(raw), 6))
           + chunk(b"IEND", b""))
    with open(path, "wb") as handle:
        handle.write(png)
    return [out_w, out_h]


def crop(path, desktop, box, stride_step=1):
    x0, y0, x1, y1 = box
    ox, oy = desktop["origin"]
    w, h = desktop["size"]
    stride = w * 4
    left = max(0, min(w, x0 - ox))
    top = max(0, min(h, y0 - oy))
    right = max(0, min(w, x1 - ox))
    bottom = max(0, min(h, y1 - oy))
    rows = []
    for row_index in range(top, bottom):
        start = row_index * stride + left * 4
        rows.append(desktop["data"][start:start + max(1, right - left) * 4])
    return write_png(path, right - left, bottom - top, b"".join(rows), stride_step=stride_step)


def sample(hwnd_value, label):
    hwnd = wt.HWND(hwnd_value)
    # A minimized window reports a 0x0 client area and cannot be captured, so it
    # has to be shown first. SW_SHOWNOACTIVATE makes it visible without stealing
    # the operator's focus; the original show command is restored after capture.
    was_iconic = bool(user32.IsIconic(hwnd))
    if was_iconic:
        user32.ShowWindow(hwnd, SW_SHOWNOACTIVATE)
        time.sleep(1.2)
    if DESKTOP_CAPTURE:
        # Desktop plates need the window on top; window-level capture does not.
        user32.SetForegroundWindow(hwnd)
        time.sleep(0.5)
    outer = wt.RECT()
    client = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(outer))
    user32.GetClientRect(hwnd, ctypes.byref(client))
    top_left = wt.POINT(client.left, client.top)
    user32.ClientToScreen(hwnd, ctypes.byref(top_left))
    monitor = user32.MonitorFromWindow(hwnd, 2)
    info = MONITORINFO()
    info.cbSize = ctypes.sizeof(MONITORINFO)
    user32.GetMonitorInfoW(monitor, ctypes.byref(info))
    dpi_x = ctypes.c_uint()
    dpi_y = ctypes.c_uint()
    shcore.GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, ctypes.byref(dpi_x), ctypes.byref(dpi_y))
    dpi = user32.GetDpiForWindow(hwnd)
    placement = WINDOWPLACEMENT()
    placement.length = ctypes.sizeof(WINDOWPLACEMENT)
    user32.GetWindowPlacement(hwnd, ctypes.byref(placement))
    dwm = wt.RECT()
    dwm_result = dwmapi.DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS,
                                              ctypes.byref(dwm), ctypes.sizeof(dwm))
    client_w = client.right - client.left
    client_h = client.bottom - client.top
    dom = dom_geometry()
    data = {
        "label": label,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "hwnd": hwnd_value,
        "isZoomed": bool(user32.IsZoomed(hwnd)),
        "showCmd": placement.showCmd,
        "rcNormalPosition": rect_list(placement.rcNormalPosition),
        "osOuterRect": rect_list(outer),
        "osOuterSize": [outer.right - outer.left, outer.bottom - outer.top],
        "dwmVisibleFrame": rect_list(dwm) if dwm_result == 0 else None,
        "clientScreenRect": [top_left.x, top_left.y, top_left.x + client_w, top_left.y + client_h],
        "clientSize": [client_w, client_h],
        "dpiForWindow": dpi,
        "monitor": {
            "rcMonitor": rect_list(info.rcMonitor),
            "rcWork": rect_list(info.rcWork),
            "primary": bool(info.dwFlags & 1),
            "dpiX": dpi_x.value,
            "scalePercent": round(dpi_x.value * 100 / 96),
        },
        "dom": dom,
        "expectedCanvas": expected_canvas(client_w, client_h) if client_w else None,
        "monitorCount": user32.GetSystemMetrics(SM_CMONITORS),
        "virtualScreen": [user32.GetSystemMetrics(SM_XVIRTUALSCREEN),
                          user32.GetSystemMetrics(SM_YVIRTUALSCREEN),
                          user32.GetSystemMetrics(SM_CXVIRTUALSCREEN),
                          user32.GetSystemMetrics(SM_CYVIRTUALSCREEN)],
        "foregroundHwnd": int(user32.GetForegroundWindow()),
    }
    evidence = {
        "api": "PrintWindow(hwnd, PW_RENDERFULLCONTENT)",
        "printWindowUsed": True,
        "foregroundSteal": DESKTOP_CAPTURE,
        "foregroundMatchesTarget": data["foregroundHwnd"] == hwnd_value,
    }
    if client_w and client_h:
        frame = grab_window(hwnd, client_w, client_h)
        window_png = os.path.join(OUT, f"window-geometry-{label}-window-printwindow.png")
        evidence["windowPng"] = window_png
        evidence["windowPngSize"] = write_png(window_png, client_w, client_h, frame)
        evidence["renderStats"] = score_pixels(frame, client_w, client_h)
    if DESKTOP_CAPTURE:
        desktop = grab_desktop()
        virtual_png = os.path.join(OUT, f"window-geometry-{label}-virtual-desktop-bitblt.png")
        write_png(virtual_png, desktop["size"][0], desktop["size"][1], desktop["data"])
        box = data["dwmVisibleFrame"] or data["osOuterRect"]
        crop_png = os.path.join(OUT, f"window-geometry-{label}-window-crop-bitblt.png")
        evidence.update({
            "api": "PrintWindow(PW_RENDERFULLCONTENT) + GetDC(NULL) BitBlt plate",
            "virtualDesktopPng": virtual_png,
            "windowCropPng": crop_png,
            "windowCropSize": crop(crop_png, desktop, box),
            "cropBox": box,
        })
    data["evidence"] = evidence
    if was_iconic:
        # Hand the window back exactly as the operator left it.
        user32.ShowWindow(hwnd, SW_MINIMIZE)
        time.sleep(0.5)
    return data


def main():
    hwnd_value = find_app()
    report = {"hwnd": hwnd_value, "primaryWorkArea": list(PRIMARY_WORK), "samples": []}
    if not hwnd_value:
        report["error"] = "window not found"
    else:
        hwnd = wt.HWND(hwnd_value)
        # Force the window onto the primary monitor without changing its size.
        user32.SetWindowPos(hwnd, wt.HWND(HWND_TOP), PRIMARY_WORK[0] + 40, PRIMARY_WORK[1] + 40,
                            0, 0, SWP_NOSIZE | SWP_NOZORDER)
        time.sleep(1.5)
        report["samples"].append(sample(hwnd_value, "startup-observed"))
        user32.ShowWindow(hwnd, SW_MAXIMIZE)
        time.sleep(2.0)
        report["samples"].append(sample(hwnd_value, "requested-maximized"))
        user32.ShowWindow(hwnd, SW_RESTORE)
        time.sleep(2.0)
        report["samples"].append(sample(hwnd_value, "requested-restored"))
        user32.ShowWindow(hwnd, SW_MAXIMIZE)
        time.sleep(2.0)
        report["samples"].append(sample(hwnd_value, "remaximized"))
    with open(os.path.join(OUT, "window-geometry-summary.json"), "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(os.path.join(OUT, "window-geometry-summary.json"))


if __name__ == "__main__":
    main()
