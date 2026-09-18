/** QA-only Vite plugin (not part of the app build): serves real DOM geometry. */
let latest = null;

export function qaDomGeometryPlugin() {
  return {
    name: "qa-dom-geometry-probe",
    transformIndexHtml() {
      return [
        {
          tag: "script",
          injectTo: "body",
          children: `
(() => {
  const push = () => {
    try {
      const canvas = document.querySelector(".design-canvas");
      const shell = document.querySelector(".viewport-shell");
      const rect = (node) => {
        if (!node) return null;
        const r = node.getBoundingClientRect();
        return { x: r.x, y: r.y, width: r.width, height: r.height, top: r.top, left: r.left, right: r.right, bottom: r.bottom };
      };
      const payload = {
        sampleId: String(Date.now()),
        timestamp: new Date().toISOString(),
        innerWidth: window.innerWidth,
        innerHeight: window.innerHeight,
        devicePixelRatio: window.devicePixelRatio,
        visualViewport: window.visualViewport
          ? { width: window.visualViewport.width, height: window.visualViewport.height, scale: window.visualViewport.scale }
          : null,
        documentScroll: {
          scrollWidth: document.documentElement.scrollWidth,
          scrollHeight: document.documentElement.scrollHeight,
          clientWidth: document.documentElement.clientWidth,
          clientHeight: document.documentElement.clientHeight,
        },
        viewportShell: rect(shell),
        designCanvas: rect(canvas),
        designCanvasZoom: canvas ? getComputedStyle(canvas).zoom : null,
      };
      document.title = "QA_GEOM|" + JSON.stringify(payload);
      fetch("/__qa_dom_geometry__", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(payload) }).catch(() => {});
    } catch (error) {
      /* no-op */
    }
  };
  push();
  setInterval(push, 250);
  window.addEventListener("resize", push);
})();
          `.trim(),
        },
      ];
    },
    configureServer(server) {
      const handler = (req, res, next) => {
        if (!req.url || !req.url.startsWith("/__qa_dom_geometry__")) return next();
        if (req.method === "POST") {
          let body = "";
          req.on("data", (chunk) => { body += chunk; });
          req.on("end", () => {
            try { latest = JSON.parse(body); } catch { latest = body; }
            res.statusCode = 204;
            res.end();
          });
          return;
        }
        res.statusCode = 200;
        res.setHeader("Content-Type", "application/json");
        res.setHeader("Cache-Control", "no-store");
        res.end(JSON.stringify(latest ?? { error: "no DOM sample yet" }));
      };
      server.middlewares.stack.unshift({ route: "", handle: handler });
    },
  };
}

export default qaDomGeometryPlugin;
