"""Headless layout audit of the fixed 16:9 workbench.

Runs against a plain static server (no desktop window, no focus, no Tauri):
a headless Edge instance is driven over CDP, the viewport is pinned, and every
panel is measured so bottom gutters and clipped panels can be read as numbers
instead of guessed from a screenshot.
"""

from __future__ import annotations

import json
import os
import time

from qa_directory_picker_verify import Cdp, SHOTS, connect_page

PORT = int(os.environ.get("MICROCT_QA_CDP_PORT", "9334"))
PAGE_URL = os.environ.get("MICROCT_QA_PAGE_URL", "http://localhost:5199/")
VIEWPORTS = [(1600, 900), (1920, 1080), (1280, 720)]
OUT_JSON = os.path.join(SHOTS, "layout-measure.json")

JS_MEASURE = """(() => {
  const round = n => Math.round(n * 100) / 100;
  const rect = el => {
    const r = el.getBoundingClientRect();
    return { x: round(r.x), y: round(r.y), w: round(r.width), h: round(r.height),
             bottom: round(r.bottom), right: round(r.right) };
  };
  const audit = el => ({
    cls: el.className,
    rect: rect(el),
    scrollH: el.scrollHeight, clientH: el.clientHeight,
    scrollW: el.scrollWidth, clientW: el.clientWidth,
    overflowY: el.scrollHeight - el.clientHeight,
    overflowX: el.scrollWidth - el.clientWidth,
  });
  const panels = Array.from(document.querySelectorAll('.panel')).map(el => {
    const head = el.querySelector('h2');
    const parent = el.parentElement;
    const info = audit(el);
    info.title = (head && head.textContent ? head.textContent : '').trim();
    info.parent = parent ? { cls: parent.className, rect: rect(parent) } : null;
    // distance between the panel bottom and the bottom of its column
    info.gutterBottom = info.parent ? round(info.parent.rect.bottom - info.rect.bottom) : null;
    return info;
  });
  const doc = document.documentElement;
  return {
    viewport: { w: window.innerWidth, h: window.innerHeight, dpr: window.devicePixelRatio },
    doc: { sw: doc.scrollWidth, sh: doc.scrollHeight, cw: doc.clientWidth, ch: doc.clientHeight },
    canvas: document.querySelector('.design-canvas') ? audit(document.querySelector('.design-canvas')) : null,
    columns: Array.from(document.querySelectorAll('aside, main, .column')).map(audit),
    panels,
  };
})()"""


def main() -> int:
    cdp = connect_page(PORT, timeout=30)
    cdp.call("Page.enable")
    report: dict = {"pageUrl": PAGE_URL, "viewports": {}}

    for width, height in VIEWPORTS:
        cdp.call(
            "Emulation.setDeviceMetricsOverride",
            {"width": width, "height": height, "deviceScaleFactor": 1, "mobile": False},
        )
        cdp.call("Page.navigate", {"url": PAGE_URL})
        deadline = time.time() + 20
        while time.time() < deadline:
            state = cdp.evaluate("document.readyState")
            if state == "complete":
                break
            time.sleep(0.4)
        time.sleep(1.2)
        report["viewports"][f"{width}x{height}"] = cdp.evaluate(JS_MEASURE)

    with open(OUT_JSON, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, ensure_ascii=False)

    for name, data in report["viewports"].items():
        doc = data["doc"]
        print(f"--- {name} doc {doc['sw']}x{doc['sh']} client {doc['cw']}x{doc['ch']}")
        for panel in data["panels"]:
            flag = "OVERFLOW" if panel["overflowY"] > 1 or panel["overflowX"] > 1 else ""
            print(
                f"  {panel['title'][:22]:<24} h={panel['rect']['h']:<7} "
                f"ovY={panel['overflowY']:<4} gutterBottom={panel['gutterBottom']} {flag}"
            )
    print(f"written: {OUT_JSON}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
