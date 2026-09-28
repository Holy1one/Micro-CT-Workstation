/**
 * Build ct-engine and copy it to Tauri's platform-specific sidecar location.
 * The script chooses debug or release from `--release`, preserves Cargo target
 * overrides, and never starts the resulting executable or contacts hardware.
 */

import { copyFileSync, chmodSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";
const targetRoot = process.env.CARGO_TARGET_DIR
  ? resolve(root, process.env.CARGO_TARGET_DIR)
  : join(root, "target");

execFileSync("cargo", ["build", "-p", "ct-engine", ...(release ? ["--release"] : [])], {
  cwd: root,
  stdio: "inherit",
});

/**
 * Determine the Rust host triple. The piped probe is tried first because it is
 * the cleanest; some Windows hosts, however, fail every piped spawn of the
 * Rust toolchain with EBUSY (-4082), so the probe falls back to a shell
 * redirect through a temp file (no pipe is created) and finally to the
 * well-known desktop triple for this platform.
 */
function detectHostTriple() {
  const parse = (text) => text.match(/^host:\s+(.+)$/m)?.[1]?.trim() ?? null;

  try {
    const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
    const triple = parse(out);
    if (triple) return triple;
  } catch {
    // Piped spawn failed on this host; try the redirect fallback below.
  }

  if (process.platform === "win32") {
    const capture = join(tmpdir(), `rustc-vv-${process.pid}.txt`);
    try {
      execFileSync(process.env.ComSpec ?? "cmd.exe", ["/d", "/s", "/c", `rustc -vV 1>"${capture}" 2>&1`], {
        stdio: ["ignore", "ignore", "inherit"],
      });
      const triple = parse(readFileSync(capture, "utf8"));
      if (triple) return triple;
    } catch {
      // Redirect probe failed as well; use the platform default below.
    } finally {
      rmSync(capture, { force: true });
    }
  }

  const fallback = {
    "win32:x64": "x86_64-pc-windows-msvc",
    "darwin:arm64": "aarch64-apple-darwin",
    "darwin:x64": "x86_64-apple-darwin",
    "linux:x64": "x86_64-unknown-linux-gnu",
  }[`${process.platform}:${process.arch}`];
  if (fallback) return fallback;
  throw new Error("Unable to determine the Rust host target triple");
}

const triple = detectHostTriple();

const executable = process.platform === "win32" ? "ct-engine.exe" : "ct-engine";
const source = join(targetRoot, profile, executable);
const destinationDirectory = join(root, "src-tauri", "binaries");
const destination = join(destinationDirectory, `ct-engine-${triple}${process.platform === "win32" ? ".exe" : ""}`);
mkdirSync(destinationDirectory, { recursive: true });
copyFileSync(source, destination);
if (process.platform !== "win32") chmodSync(destination, 0o755);
console.log(`Prepared ${destination}`);
