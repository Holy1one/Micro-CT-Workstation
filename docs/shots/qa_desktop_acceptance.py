"""Physical desktop acceptance for the Micro-CT Tauri window.

Uses Win32 foreground/mouse input for the native directory dialog and GDI
BitBlt for the final full virtual-desktop evidence. It does not use PrintWindow.
"""

import ctypes
import ctypes.wintypes as wt
import json
import os
import struct
import sys
import time
import zlib

ROOT = r"E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App"
OUT_DIR = os.path.join(ROOT, "docs", "shots")
SAFE_SELECTION = os.path.join(ROOT, "docs", "shots")
REPORT = os.path.join(OUT_DIR, "qa-desktop-acceptance.json")
CAPTURE = os.path.join(OUT_DIR, "tauri-native-desktop-bitblt.png")

user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)

SW_RESTORE = 9
SW_MAXIMIZE = 3
SRCCOPY = 0x00CC0020
DIB_RGB_COLORS = 0
SM_XVIRTUALSCREEN = 76
SM_YVIRTUALSCREEN = 77
SM_CXVIRTUALSCREEN = 78
SM_CYVIRTUALSCREEN = 79
SM_CMONITORS = 80
WM_CLOSE = 0x0010
VK_ESCAPE = 0x1B
VK_RETURN = 0x0D
KEYEVENTF_KEYUP = 0x0002
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004

ENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
user32.EnumWindows.argtypes = [ENUMPROC, wt.LPARAM]
user32.GetWindowTextLengthW.argtypes = [wt.HWND]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.IsWindowVisible.restype = wt.BOOL
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.GetWindowRect.restype = wt.BOOL
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
user32.SetForegroundWindow.argtypes = [wt.HWND]
user32.SetForegroundWindow.restype = wt.BOOL
user32.BringWindowToTop.argtypes = [wt.HWND]
user32.GetForegroundWindow.restype = wt.HWND
user32.GetSystemMetrics.argtypes = [ctypes.c_int]
user32.GetSystemMetrics.restype = ctypes.c_int
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
user32.mouse_event.argtypes = [wt.DWORD, wt.DWORD, wt.DWORD, wt.DWORD, wt.ULONG]
user32.keybd_event.argtypes = [wt.BYTE, wt.BYTE, wt.DWORD, wt.ULONG]
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.GetDC.argtypes = [wt.HWND]
user32.GetDC.restype = wt.HDC
user32.ReleaseDC.argtypes = [wt.HWND, wt.HDC]

gdi32.CreateCompatibleDC.argtypes = [wt.HDC]
gdi32.CreateCompatibleDC.restype = wt.HDC
gdi32.CreateCompatibleBitmap.argtypes = [wt.HDC, ctypes.c_int, ctypes.c_int]
gdi32.CreateCompatibleBitmap.restype = wt.HBITMAP
gdi32.SelectObject.argtypes = [wt.HDC, wt.HGDIOBJ]
gdi32.SelectObject.restype = wt.HGDIOBJ
gdi32.BitBlt.argtypes = [
    wt.HDC,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    wt.HDC,
    ctypes.c_int,
    ctypes.c_int,
    wt.DWORD,
]
gdi32.BitBlt.restype = wt.BOOL
gdi32.GetDIBits.argtypes = [
    wt.HDC,
    wt.HBITMAP,
    ctypes.c_uint,
    ctypes.c_uint,
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_uint,
]
gdi32.GetDIBits.restype = ctypes.c_int
gdi32.DeleteObject.argtypes = [wt.HGDIOBJ]
gdi32.DeleteDC.argtypes = [wt.HDC]


class BitmapInfoHeader(ctypes.Structure):
    _fields_ = [
        ("biSize", wt.DWORD),
        ("biWidth", ctypes.c_long),
        ("biHeight", ctypes.c_long),
        ("biPlanes", wt.WORD),
        ("biBitCount", wt.WORD),
        ("biCompression", wt.DWORD),
        ("biSizeImage", wt.DWORD),
        ("biXPelsPerMeter", ctypes.c_long),
        ("biYPelsPerMeter", ctypes.c_long),
        ("biClrUsed", wt.DWORD),
        ("biClrImportant", wt.DWORD),
    ]


def window_text(hwnd):
    length = user32.GetWindowTextLengthW(hwnd)
    value = ctypes.create_unicode_buffer(length + 1)
    user32.GetWindowTextW(hwnd, value, length + 1)
    return value.value


def class_name(hwnd):
    value = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, value, 256)
    return value.value


def window_rect(hwnd):
    rect = wt.RECT()
    if not user32.GetWindowRect(hwnd, ctypes.byref(rect)):
        return None
    return [rect.left, rect.top, rect.right, rect.bottom]


def visible_windows():
    result = []

    @ENUMPROC
    def visit(hwnd, _lparam):
        if user32.IsWindowVisible(hwnd):
            result.append(
                {
                    "hwnd": int(hwnd),
                    "title": window_text(hwnd),
                    "class": class_name(hwnd),
                    "rect": window_rect(hwnd),
                }
            )
        return True

    user32.EnumWindows(visit, 0)
    return result


def find_window(title):
    matches = [item for item in visible_windows() if item["title"] == title]
    return matches[0] if matches else None


def foreground():
    hwnd = user32.GetForegroundWindow()
    if not hwnd:
        return None
    return {
        "hwnd": int(hwnd),
        "title": window_text(hwnd),
        "class": class_name(hwnd),
        "rect": window_rect(hwnd),
    }


def focus(hwnd):
    user32.ShowWindow(hwnd, SW_RESTORE)
    user32.ShowWindow(hwnd, SW_MAXIMIZE)
    user32.BringWindowToTop(hwnd)
    return bool(user32.SetForegroundWindow(hwnd))


def click(x, y):
    if not user32.SetCursorPos(int(x), int(y)):
        raise OSError(ctypes.get_last_error(), "SetCursorPos failed")
    time.sleep(0.2)
    user32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    user32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)


def press(vk):
    user32.keybd_event(vk, 0, 0, 0)
    user32.keybd_event(vk, 0, KEYEVENTF_KEYUP, 0)


def wait_foreground_change(original_hwnd, timeout=8.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        item = foreground()
        if item and item["hwnd"] != original_hwnd:
            return item
        time.sleep(0.1)
    return foreground()


def open_dialog(app):
    hwnd_value = app["hwnd"]
    hwnd = wt.HWND(hwnd_value)
    if not focus(hwnd):
        return None, "SetForegroundWindow returned false"
    time.sleep(1.0)
    rect = window_rect(hwnd)
    if not rect:
        return None, "application window rectangle unavailable"
    left, top, right, bottom = rect
    width = right - left
    height = bottom - top
    # Fixed 1600x1000 design canvas scales uniformly. The folder button is near
    # design coordinate (320, 222), inside the left scan panel.
    x = left + width * (320.0 / 1600.0)
    y = top + height * (222.0 / 1000.0)
    click(x, y)
    dialog = wait_foreground_change(hwnd_value)
    if not dialog or dialog["hwnd"] == hwnd_value:
        return None, "foreground did not change after physical folder-button click"
    return dialog, None


def write_png(path, width, height, data):
    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)
        row = data[y * stride : (y + 1) * stride]
        for i in range(0, len(row), 4):
            raw.extend((row[i + 2], row[i + 1], row[i], 255))

    def chunk(tag, payload):
        value = tag + payload
        return (
            struct.pack(">I", len(payload))
            + value
            + struct.pack(">I", zlib.crc32(value) & 0xFFFFFFFF)
        )

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 6))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as handle:
        handle.write(png)


def capture_virtual_desktop(path):
    x = user32.GetSystemMetrics(SM_XVIRTUALSCREEN)
    y = user32.GetSystemMetrics(SM_YVIRTUALSCREEN)
    width = user32.GetSystemMetrics(SM_CXVIRTUALSCREEN)
    height = user32.GetSystemMetrics(SM_CYVIRTUALSCREEN)
    screen_dc = user32.GetDC(None)
    mem_dc = gdi32.CreateCompatibleDC(screen_dc)
    bitmap = gdi32.CreateCompatibleBitmap(screen_dc, width, height)
    previous = gdi32.SelectObject(mem_dc, bitmap)
    ok = bool(gdi32.BitBlt(mem_dc, 0, 0, width, height, screen_dc, x, y, SRCCOPY))
    info = BitmapInfoHeader()
    info.biSize = ctypes.sizeof(BitmapInfoHeader)
    info.biWidth = width
    info.biHeight = -height
    info.biPlanes = 1
    info.biBitCount = 32
    buffer = ctypes.create_string_buffer(width * height * 4)
    rows = gdi32.GetDIBits(
        mem_dc,
        bitmap,
        0,
        height,
        ctypes.cast(buffer, ctypes.c_void_p),
        ctypes.byref(info),
        DIB_RGB_COLORS,
    )
    gdi32.SelectObject(mem_dc, previous)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(mem_dc)
    user32.ReleaseDC(None, screen_dc)
    if not ok or rows != height:
        raise OSError(ctypes.get_last_error(), "GDI BitBlt/GetDIBits failed")
    write_png(path, width, height, buffer.raw)
    return {"path": path, "origin": [x, y], "size": [width, height], "bitblt": ok}


def main():
    report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "safeSelection": SAFE_SELECTION,
        "monitorCount": user32.GetSystemMetrics(SM_CMONITORS),
        "dialog": {},
    }
    app = find_window("Micro-CT Workstation")
    report["appBefore"] = app
    if not app:
        report["error"] = "Micro-CT Workstation window not found"
    else:
        first, error = open_dialog(app)
        report["dialog"]["firstOpen"] = first
        report["dialog"]["firstOpenError"] = error
        if first and first["hwnd"] != app["hwnd"]:
            press(VK_ESCAPE)
            time.sleep(1.0)
            report["dialog"]["cancelReturnedToApp"] = (
                foreground() or {}
            ).get("hwnd") == app["hwnd"]
        else:
            report["dialog"]["cancelReturnedToApp"] = False

        second, error = open_dialog(app)
        report["dialog"]["secondOpen"] = second
        report["dialog"]["secondOpenError"] = error
        if second and second["hwnd"] != app["hwnd"]:
            # The real native picker starts in the current Save Path. The app
            # resolves that path to an existing Windows image directory. To avoid
            # navigating or modifying personal folders, cancel this second dialog;
            # record selection as unverified rather than faking a safe choice.
            report["dialog"]["selection"] = "UNVERIFIED: picker opened, but no safe physical navigation was attempted"
            press(VK_ESCAPE)
            time.sleep(1.0)
        else:
            report["dialog"]["selection"] = "UNVERIFIED: picker did not open"

        focus(wt.HWND(app["hwnd"]))
        time.sleep(1.0)
        report["foregroundBeforeCapture"] = foreground()
        report["desktopCapture"] = capture_virtual_desktop(CAPTURE)
        report["appAfter"] = find_window("Micro-CT Workstation")

    with open(REPORT, "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=2)
    print(REPORT)
    return 0 if app else 2


if __name__ == "__main__":
    sys.exit(main())
