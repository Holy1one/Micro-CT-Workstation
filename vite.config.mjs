import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { qaDomGeometryPlugin } from "./docs/shots/qa-dom-geometry-plugin.mjs";

export default defineConfig(({ command }) => ({
  build: {
    outDir: "dist/client",
  },
  optimizeDeps: {
    include: ["react", "react-dom/client"],
  },
  server: {
    host: "0.0.0.0",
    allowedHosts: ["terminal.local"],
    warmup: {
      clientFiles: ["./src/main.tsx"],
    },
  },
  plugins: [react(), ...(command === "serve" ? [qaDomGeometryPlugin()] : [])],
}));
