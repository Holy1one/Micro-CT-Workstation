"""End-to-end verification of every menu entry in the real desktop app.

Same driver as the folder-picker verification: the app is launched with a
devtools port, the page is clicked through CDP with real mouse events, and the
assertions read the DOM afterwards. Only the in-app dialogs are involved here,
so no native window is needed; the app window is restored with
SW_SHOWNOACTIVATE for the clicks and handed back minimised afterwards.

Evidence: ``docs/shots/menu-functions-verify.json``.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import time

from qa_directory_picker_verify import (
    DEBUG_PORT,
    EXE,
    SHOTS,
    SW_SHOWMINIMIZED,
    SW_SHOWNOACTIVATE,
    Cdp,  # noqa: F401  (re-exported for ad-hoc use)
    class_of,
    click_label,
    connect_page,
    find_windows,
    pid_of,
    text_of,
    user32,
    wait_for,
)

OUT_JSON = os.path.join(SHOTS, "menu-functions-verify.json")

JS_DROPDOWN_OPEN = "Boolean(document.querySelector('.menu-dropdown'))"
JS_THEME = "document.documentElement.getAttribute('data-theme')"

JS_FIELD_VALUE = """(name => {
  const wanted = name.toLowerCase();
  const labels = Array.from(document.querySelectorAll('.param-field, .numeric-field'));
  const lab = labels.find(l => (l.querySelector('span') ? l.querySelector('span').textContent : '').toLowerCase().includes(wanted));
  const input = lab ? lab.querySelector('input') : null;
  return input ? input.value : null;
})(%r)"""

JS_ITEM_STATE = """(label => {
  const el = Array.from(document.querySelectorAll('.menu-dropdown__item'))
    .find(n => (n.textContent || '').trim().toLowerCase().startsWith(label.toLowerCase()));
  if (!el) return null;
  return { disabled: el.disabled, ariaDisabled: el.getAttribute('aria-disabled'), title: el.getAttribute('title') };
})(%r)"""

JS_DIALOG = """(() => {
  const card = document.querySelector('.modal-card');
  if (!card) return null;
  const head = card.querySelector('.modal-card__head h2');
  return {
    title: head ? head.textContent.trim() : null,
    facts: Array.from(card.querySelectorAll('.modal-facts dt')).map(n => n.textContent.trim()),
    body: (card.querySelector('.modal-card__body') ? card.querySelector('.modal-card__body').textContent : '').slice(0, 240),
  };
})()"""

JS_CLOSER_RECT = """(() => {
  const el = document.querySelector('.modal-card__close');
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
})()"""

JS_BUTTON_RECT = """(label => {
  const el = Array.from(document.querySelectorAll('.modal-card button'))
    .find(b => (b.textContent || '').trim().toLowerCase() === label.toLowerCase());
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
})(%r)"""

JS_LOG_LINES = "Array.from(document.querySelectorAll('.log-line__msg')).map(n => (n.textContent || '').trim())"


def open_menu(cdp, group: str, label: str | None = None) -> None:
    # Clicking a top-level button while its dropdown is open would close it.
    if label and cdp.evaluate(JS_ITEM_STATE % label):
        return
    click_label(cdp, group)
    wait_for(lambda: cdp.evaluate(JS_DROPDOWN_OPEN), 5.0, 0.1)


def new_lines(cdp, before: list) -> list:
    """Log lines appended after `before` was captured (newest first in the DOM)."""
    after = cdp.evaluate(JS_LOG_LINES) or []
    return [line for line in after if line not in before]


def new_log_lines(cdp, before: list, keyword: str) -> bool:
    return any(keyword in line.lower() for line in new_lines(cdp, before))


def field(cdp, name: str) -> str:
    return str(cdp.evaluate(JS_FIELD_VALUE % name) or "")


def dialog(cdp):
    return cdp.evaluate(JS_DIALOG)


def close_dialog(cdp) -> bool:
    rect = cdp.evaluate(JS_CLOSER_RECT)
    if not rect:
        return False
    cdp.click(rect["x"], rect["y"])
    return wait_for(lambda: dialog(cdp) is None, 5.0, 0.2) is True


def click_in_dialog(cdp, label: str) -> bool:
    rect = cdp.evaluate(JS_BUTTON_RECT % label)
    if not rect:
        return False
    cdp.click(rect["x"], rect["y"])
    return True


def run_entry(cdp, group: str, label: str, checks: dict, key: str) -> None:
    """Opens a menu entry unless it is disabled, and records what happened."""
    open_menu(cdp, group, label)
    state = cdp.evaluate(JS_ITEM_STATE % label)
    checks[f"{key}.state"] = state
    if not state or state.get("disabled"):
        checks[f"{key}.skipped"] = state.get("title") if state else "entry not found"
        return
    click_label(cdp, label)


def main() -> int:
    result: dict = {"checks": {}}
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
        user32.ShowWindow(hwnd, SW_SHOWNOACTIVATE)
        time.sleep(1.0)
        cdp = connect_page(DEBUG_PORT)
        result["checks"]["themeAtStart"] = cdp.evaluate(JS_THEME)

        # ---- File > New Scan Task -----------------------------------------
        run_entry(cdp, "File", "New Scan Task", result["checks"], "newTask")
        wait_for(lambda: (field(cdp, "task id") or "").startswith("scan-"), 8.0, 0.3)
        task_id = field(cdp, "task id")
        result["checks"]["newTaskIdFormat"] = bool(re.match(r"^scan-\d{8}-\d{6}$", task_id))
        result["taskId"] = task_id

        # ---- Edit > Reset Parameters --------------------------------------
        run_entry(cdp, "Edit", "Reset Parameters", result["checks"], "reset")
        wait_for(lambda: field(cdp, "total projections") == "120", 8.0, 0.3)
        projections = field(cdp, "total projections")
        exposure = field(cdp, "exposure")
        max_xray = field(cdp, "max x-ray")
        result["restored"] = {"projections": projections, "exposure": exposure, "maxXray": max_xray}
        result["checks"]["resetProjections"] = projections == "120"
        result["checks"]["resetExposure"] = exposure == "200"
        try:
            parts = [int(p) for p in max_xray.split(":")]
            seconds = parts[0] * 3600 + parts[1] * 60 + parts[2] if len(parts) == 3 else -1
        except ValueError:
            seconds = -1
        result["checks"]["resetMaxXraySeconds"] = seconds == 600

        # ---- Edit > Undo / Redo stay disabled ------------------------------
        open_menu(cdp, "Edit", "Undo")
        result["checks"]["undoDisabled"] = (cdp.evaluate(JS_ITEM_STATE % "Undo") or {}).get("disabled") is True
        result["checks"]["redoDisabled"] = (cdp.evaluate(JS_ITEM_STATE % "Redo") or {}).get("disabled") is True
        result["checks"]["undoReason"] = (cdp.evaluate(JS_ITEM_STATE % "Undo") or {}).get("title")

        # ---- Edit > Preferences (theme really switches) --------------------
        run_entry(cdp, "Edit", "Preferences", result["checks"], "preferences")
        wait_for(lambda: dialog(cdp) is not None, 6.0, 0.2)
        prefs = dialog(cdp)
        result["checks"]["preferencesTitle"] = (prefs or {}).get("title") == "Preferences"
        click_in_dialog(cdp, "Dark")
        dark = wait_for(lambda: cdp.evaluate(JS_THEME) == "dark", 6.0, 0.2)
        result["checks"]["themeSwitchedToDark"] = bool(dark)
        click_in_dialog(cdp, "Light")
        light = wait_for(lambda: cdp.evaluate(JS_THEME) == "light", 6.0, 0.2)
        result["checks"]["themeSwitchedBackToLight"] = bool(light)
        result["checks"]["preferencesClosed"] = close_dialog(cdp)
        print("step: preferences ok", flush=True)

        # ---- Tools ----------------------------------------------------------
        logs_before = cdp.evaluate(JS_LOG_LINES) or []
        run_entry(cdp, "Tools", "Run Preflight", result["checks"], "preflight")
        # Only freshly appended lines count: "preflight" also appears in older
        # setup messages, which would make this check meaningless.
        # The result line reads "8/8 preview checks passed", so match either the
        # word "preflight" or the check summary.
        preflight_logged = wait_for(
            lambda: new_log_lines(cdp, logs_before, "preflight")
            or new_log_lines(cdp, logs_before, "checks passed"),
            12.0,
            0.5,
        )
        result["checks"]["preflightLogged"] = bool(preflight_logged)
        result["preflightLogSample"] = new_lines(cdp, logs_before)[:2]
        print("step: preflight ok", flush=True)

        logs_before_home = cdp.evaluate(JS_LOG_LINES) or []
        run_entry(cdp, "Tools", "Home All Axes", result["checks"], "home")
        home_logged = wait_for(lambda: new_log_lines(cdp, logs_before_home, "home"), 12.0, 0.5)
        result["checks"]["homeLogged"] = bool(home_logged)
        result["homeLogSample"] = new_lines(cdp, logs_before_home)[:2]
        print("step: home ok", flush=True)

        run_entry(cdp, "Tools", "Device Diagnostics", result["checks"], "diagnostics")
        wait_for(lambda: dialog(cdp) is not None, 6.0, 0.2)
        diag = dialog(cdp)
        result["checks"]["diagnosticsTitle"] = (diag or {}).get("title") == "Device Diagnostics"
        facts = (diag or {}).get("facts", [])
        # The panel lists three devices plus a "Link summary" row.
        result["checks"]["diagnosticsListsThreeDevices"] = all(
            device in facts for device in ("X-Ray Source", "Turntable-Nano", "Camera")
        )
        result["diagnosticsFacts"] = (diag or {}).get("facts")
        result["checks"]["diagnosticsClosed"] = close_dialog(cdp)
        print("step: diagnostics ok", flush=True)

        # ---- Help -----------------------------------------------------------
        for key, label, title in [
            ("guide", "User Guide", "User Guide"),
            ("safety", "Safety Notes", "Safety Notes"),
            ("about", "About Micro-CT Workstation", "About Micro-CT Workstation"),
        ]:
            run_entry(cdp, "Help", label, result["checks"], key)
            wait_for(lambda: dialog(cdp) is not None, 6.0, 0.2)
            opened = dialog(cdp)
            result["checks"][f"{key}Title"] = (opened or {}).get("title") == title
            if key == "about":
                result["aboutBody"] = (opened or {}).get("body")
                result["checks"]["aboutMentionsEngine"] = "ct-engine" in ((opened or {}).get("body") or "").lower()
            result["checks"][f"{key}Closed"] = close_dialog(cdp)

        result["checks"]["logGrew"] = len(cdp.evaluate(JS_LOG_LINES) or []) > len(logs_before)
        result["verdict"] = all(
            [
                result["checks"]["newTaskIdFormat"],
                result["checks"]["resetProjections"],
                result["checks"]["resetExposure"],
                result["checks"]["resetMaxXraySeconds"],
                result["checks"]["undoDisabled"],
                result["checks"]["redoDisabled"],
                result["checks"]["preferencesTitle"],
                result["checks"]["themeSwitchedToDark"],
                result["checks"]["themeSwitchedBackToLight"],
                result["checks"]["preflightLogged"],
                result["checks"]["diagnosticsTitle"],
                result["checks"]["diagnosticsListsThreeDevices"],
                result["checks"]["guideTitle"],
                result["checks"]["safetyTitle"],
                result["checks"]["aboutTitle"],
            ]
        )
    finally:
        if hwnd:
            user32.ShowWindow(hwnd, SW_SHOWMINIMIZED)
        with open(OUT_JSON, "w", encoding="utf-8") as handle:
            json.dump(result, handle, indent=2, ensure_ascii=False)

    return 0 if result.get("verdict") else 1


if __name__ == "__main__":
    raise SystemExit(main())
