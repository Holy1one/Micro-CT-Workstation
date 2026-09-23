/** Build current frontend and embedded engine, then publish the Windows portable EXE.
 * Only successful Tauri release builds replace the delivery copy; no device is opened.
 */
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, resolve } from "node:path";

if (process.platform !== "win32") throw new Error("Portable release requires Windows");
if (process.env.CARGO_BUILD_TARGET) throw new Error("Use the native Windows host target for portable:build");
const root = resolve(import.meta.dirname, "..");
const target = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : join(root, "target");
// Tauri beforeBuildCommand rebuilds both the engine and frontend before embedding.
execFileSync(process.execPath, [join(root, "node_modules/@tauri-apps/cli/tauri.js"), "build", "--no-bundle"], {
  cwd: root, stdio: "inherit",
});
const source = join(target, "release/ct-workstation.exe");
const destinationDirectory = join(root, "portable-release");
const destination = join(destinationDirectory, "micro-ct-workstation-portable.exe");
const staged = `${destination}.tmp`;
const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
mkdirSync(destinationDirectory, { recursive: true });
copyFileSync(source, staged);
const digest = sha256(source);
if (sha256(staged) !== digest) throw new Error("Portable EXE copy verification failed");
renameSync(staged, destination);
const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
const manifest = {
  version: JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version,
  builtAt: new Date().toISOString(),
  sourceCommit: git("rev-parse", "HEAD"),
  sourceDirty: Boolean(git("status", "--porcelain", "--untracked-files=normal", "--", ".", ":(exclude)portable-release")),
  executable: "micro-ct-workstation-portable.exe",
  sha256: digest,
  frontendIndexSha256: sha256(join(root, "dist/client/index.html")),
  command: "npm.cmd run portable:build",
};
writeFileSync(join(destinationDirectory, "build-info.json"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Published ${destination}\nSHA256 ${digest}`);
