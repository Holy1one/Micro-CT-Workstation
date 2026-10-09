/** Stage only application runtime resources from the single maintained asset directory. */
import { copyFileSync, mkdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { validateWindowsIcon } from "./windows-icon.mjs";

const root = resolve(import.meta.dirname, "../..");
validateWindowsIcon(
  readFileSync(join(root, "assets/app-icon.ico")),
  readFileSync(join(root, "assets/app-128x128@2x.png")),
);
const output = join(root, ".cache/frontend/public/assets");
mkdirSync(output, { recursive: true });
// Use the transparent 256px derivative in the UI; retain the supplied 4K source in assets.
for (const [source, name] of [
  ["app-logo-light-256.png", "micro-ct-logo.png"],
  ["app-logo-dark-256.png", "micro-ct-logo-dark.png"],
  ["inter-var.woff2", "inter-var.woff2"],
  ["jetbrains-mono-var.woff2", "jetbrains-mono-var.woff2"],
]) {
  copyFileSync(join(root, "assets", source), join(output, name));
}
