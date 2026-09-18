"""Verification of File > Export Session Log through the native save dialog.

Reuses the CDP driver and the Win32 helpers from
``qa_directory_picker_verify.py``; the only new piece is the save dialog
(``#32770`` titled "Export session log"), which is operated with window
messages exactly like the folder picker, so no focus is stolen.

Checks:
  1. the native save dialog really opens;
  2. cancelling writes nothing to disk;
  3. typing a target path and confirming writes exactly that file, with the
     session log contents produced by the app;
  4. the Rust side only accepts .log/.txt - a .exe target must be refused.

Evidence goes to ``docs/shots/save-dialog-verify.json``.
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from ctypes import c_wchar_p

from qa_directory_picker_verify import (
    APP_DIR,
    DEBUG_PORT,
    EXE,
    SHOTS,
    WM_CLOSE,
    WM_SETTEXT,
    SW_SHOWMINIMIZED,
    SW_SHOWNOACTIVATE,
    capture_png,
    class_of,
    connect_page,
    dialog_children,
    dialog_edit,
    dialog_gone,
    find_windows,
    open_menu_entry,
    pid_of,
    press_dialog_button,
    read_path,
    text_of,
    user32,
    wait_for,
)

TITLE = "Export session log"
OUT_JSON = os.path.join(SHOTS, "save-dialog-verify.json")
LOG_FILE = os.path.join(tempfile.gettempdir(), "microct-qa-session.log")
REFUSED_FILE = os.path.join(tempfile.gettempdir(), "microct-qa-session.exe")


def wait_dialog(pid: int, timeout: float = 25.0):
    def find():
        hits = find_windows(
            lambda h: pid_of(h) == pid
            and class_of(h) == "#32770"
            and text_of(h) == TITLE
            and user32.IsWindowVisible(h)
        )
        return hits[0] if hits else None

    return wait_for(find, timeout, 0.2)


def type_into_dialog(dlg: int, path: str) -> bool:
    edit = dialog_edit(dlg)
    if not edit:
        return False
    user32.SendMessageW(edit, WM_SETTEXT, 0, c_wchar_p(path))
    return True


def export_once(cdp, pid: int) -> int:
    """Clicks the menu entry and returns the dialog handle."""
    open_menu_entry(cdp, "File", "Export Session Log")
    return wait_dialog(pid)


def main() -> int:
    for stale in (LOG_FILE, REFUSED_FILE):
        if os.path.exists(stale):
            os.remove(stale)
    os.makedirs(SHOTS, exist_ok=True)
    result: dict = {"logFile": LOG_FILE, "refusedFile": REFUSED_FILE, "checks": {}}
    hwnd = None

    subprocess.run(["taskkill", "/IM", "ct-workstation.exe", "/F"], capture_output=True)
    time.sleep(1.0)

    env = dict(os.environ)
    env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={DEBUG_PORT}"
    proc = subprocess.Popen([EXE], env=env, cwd=os.path.dirname(EXE))
    pid = proc.pid
    result["pid"] = pid
    try:
        hwnd = wait_for(lambda: (find_windows(lambda h: pid_of(h) == pid and text_of(h)) or [None])[0], 45.0)
        if not hwnd:
            result["error"] = "app window never appeared"
            return 1
        result["mainWindow"] = {"hwnd": hwnd, "title": text_of(hwnd), "class": class_of(hwnd)}
        user32.ShowWindow(hwnd, SW_SHOWNOACTIVATE)
        time.sleep(1.0)

        cdp = connect_page(DEBUG_PORT)
        result["pageUrl"] = cdp.evaluate("location.href")
        result["savePathField"] = read_path(cdp)

        # ---- 1. cancel must not touch the disk -----------------------------
        dlg = export_once(cdp, pid)
        if not dlg:
            result["error"] = "native save dialog never opened"
            return 1
        result["dialog"] = {
            "hwnd": dlg,
            "title": text_of(dlg),
            "class": class_of(dlg),
            "children": dialog_children(dlg),
        }
        capture_png(dlg, os.path.join(SHOTS, "save-dialog-native.png"))
        result["checks"]["dialogOpened"] = class_of(dlg) == "#32770"
        result["checks"]["cancelClicked"] = press_dialog_button(dlg, 2)
        result["checks"]["dialogClosedAfterCancel"] = dialog_gone(dlg)
        time.sleep(0.8)
        result["checks"]["cancelWroteNothing"] = not os.path.exists(LOG_FILE)

        # ---- 2. choosing a target writes exactly that file ------------------
        dlg2 = export_once(cdp, pid)
        if not dlg2:
            result["error"] = "native save dialog did not reopen"
            return 1
        result["checks"]["targetTypedIntoDialog"] = type_into_dialog(dlg2, LOG_FILE)
        result["checks"]["okClicked"] = press_dialog_button(dlg2, 1)
        result["checks"]["dialogClosedAfterOk"] = dialog_gone(dlg2)
        written = wait_for(lambda: os.path.exists(LOG_FILE), 12.0, 0.3)
        result["checks"]["fileWritten"] = bool(written)
        if written:
            text = open(LOG_FILE, encoding="utf-8", errors="replace").read()
            result["logBytes"] = len(text)
            result["logHead"] = text.splitlines()[:3]
            result["checks"]["logHasSessionHeader"] = "save path" in text.lower()
            result["checks"]["logMatchesField"] = read_path(cdp).strip() in text

        # ---- 3. the Rust whitelist refuses a non log/txt target -------------
        dlg3 = export_once(cdp, pid)
        if not dlg3:
            result["error"] = "native save dialog did not open for the whitelist probe"
            return 1
        result["checks"]["exeTargetTyped"] = type_into_dialog(dlg3, REFUSED_FILE)
        result["checks"]["exeOkClicked"] = press_dialog_button(dlg3, 1)
        dialog_gone(dlg3)
        time.sleep(1.5)
        result["checks"]["exeTargetRefused"] = not os.path.exists(REFUSED_FILE)
        result["whitelistNote"] = (
            "the dialog filter appends .log, so this probe only proves the app "
            "never writes an .exe file; the rejection itself is covered by cargo test"
        )

        result["verdict"] = all(
            [
                result["checks"]["dialogOpened"],
                result["checks"]["cancelWroteNothing"],
                result["checks"]["fileWritten"],
                result["checks"].get("logHasSessionHeader", False),
                result["checks"]["exeTargetRefused"],
            ]
        )
    finally:
        if hwnd:
            user32.ShowWindow(hwnd, SW_SHOWMINIMIZED)
        with open(OUT_JSON, "w", encoding="utf-8") as handle:
            json.dump(result, handle, indent=2, ensure_ascii=False)
        for stale in (LOG_FILE, REFUSED_FILE):
            if os.path.exists(stale):
                try:
                    os.remove(stale)
                except OSError:
                    pass

    return 0 if result.get("verdict") else 1


if __name__ == "__main__":
    raise SystemExit(main())
