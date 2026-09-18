import react from "@vitejs/plugin-react";
import { qaDomGeometryPlugin } from "./qa-dom-geometry-plugin.mjs";

/** QA-only Vite config: identical app entry with a DOM geometry probe injected. */
export default {
  root: "E:/Main/OneDrive/LanZhouUniv/Class/mluti-energy imaging/CT/Micro-CT-App",
  plugins: react ? [react(), qaDomGeometryPlugin()] : [qaDomGeometryPlugin()],
  server: { port: 4173, strictPort: true, host: "127.0.0.1" },
};
