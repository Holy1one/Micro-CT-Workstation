/**
 * Vite configuration for the React workstation.
 * Production output is written to .cache/frontend/dist/client; temporary QA instrumentation
 * must not be imported from docs or evidence directories.
 */

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
const applicationVersion=JSON.parse(readFileSync(new URL('../package.json',import.meta.url),'utf8')).version;

export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  publicDir: fileURLToPath(new URL('../.cache/frontend/public', import.meta.url)),
  cacheDir: fileURLToPath(new URL('../.cache/frontend/vite', import.meta.url)),
  define: { 'import.meta.env.VITE_APP_VERSION': JSON.stringify(applicationVersion) },
  build: {
    outDir: fileURLToPath(new URL('../.cache/frontend/dist/client', import.meta.url)),
    emptyOutDir: true,
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
