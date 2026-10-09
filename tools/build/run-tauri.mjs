/** Resolve the grouped desktop crate and npm hooks from every Tauri entry point. */
import { spawnSync } from "node:child_process";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const result = spawnSync(process.execPath, [
  join(root, "node_modules/@tauri-apps/cli/tauri.js"), ...process.argv.slice(2),
], {
  cwd: root,
  stdio: "inherit",
  env: {
    ...process.env,
    TAURI_APP_PATH: join(root, "backend/src-tauri"),
    TAURI_FRONTEND_PATH: root,
    ...(process.env.CARGO_TARGET_DIR
      ? { CARGO_TARGET_DIR: resolve(root, process.env.CARGO_TARGET_DIR) } : {}),
  },
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
