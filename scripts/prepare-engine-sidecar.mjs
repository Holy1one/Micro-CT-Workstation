import { copyFileSync, chmodSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";

execFileSync("cargo", ["build", "-p", "ct-engine", ...(release ? ["--release"] : [])], {
  cwd: root,
  stdio: "inherit",
});

const rustc = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
const triple = rustc.match(/^host:\s+(.+)$/m)?.[1]?.trim();
if (!triple) throw new Error("Unable to determine the Rust host target triple");

const executable = process.platform === "win32" ? "ct-engine.exe" : "ct-engine";
const source = join(root, "target", profile, executable);
const destinationDirectory = join(root, "src-tauri", "binaries");
const destination = join(destinationDirectory, `ct-engine-${triple}${process.platform === "win32" ? ".exe" : ""}`);
mkdirSync(destinationDirectory, { recursive: true });
copyFileSync(source, destination);
if (process.platform !== "win32") chmodSync(destination, 0o755);
console.log(`Prepared ${destination}`);
