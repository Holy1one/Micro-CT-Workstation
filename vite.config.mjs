/**
 * Vite configuration for the React workstation.
 * Production output is written to dist/client; temporary QA instrumentation
 * must not be imported from docs or evidence directories.
 */

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
const applicationVersion=JSON.parse(readFileSync(new URL('./package.json',import.meta.url),'utf8')).version;

export default defineConfig({
  define: { 'import.meta.env.VITE_APP_VERSION': JSON.stringify(applicationVersion) },
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
  plugins: [react()],
});
