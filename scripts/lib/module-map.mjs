/**
 * Shared, dependency-free helpers for the repository module map.
 *
 * The functions in this file deliberately use only Node.js built-ins so the
 * architecture checks can run immediately after a clean checkout. Paths are
 * normalized to forward slashes because the declarations must behave the same
 * on Windows development machines and in cross-platform CI.
 */

import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
export const repositoryRoot = path.resolve(scriptDirectory, "../..");

/** Convert a native path to the stable repository-relative representation. */
export function normalizePath(value) {
  return value.replaceAll("\\", "/").replace(/^\.\//, "");
}

/** Read and parse a UTF-8 JSON declaration relative to the repository root. */
export async function readRepositoryJson(relativePath) {
  const text = await readFile(path.join(repositoryRoot, relativePath), "utf8");
  return JSON.parse(text);
}

/**
 * Convert the small glob dialect used by modules.json into a regular expression.
 * `*` stays inside one directory and `**` may cross directory boundaries.
 */
export function globToRegExp(pattern) {
  const normalized = normalizePath(pattern);
  let expression = "^";
  for (let index = 0; index < normalized.length; index += 1) {
    const character = normalized[index];
    if (character === "*" && normalized[index + 1] === "*") {
      const followedBySlash = normalized[index + 2] === "/";
      expression += followedBySlash ? "(?:.*/)?" : ".*";
      index += followedBySlash ? 2 : 1;
    } else if (character === "*") {
      expression += "[^/]*";
    } else {
      expression += character.replace(/[|\\{}()[\]^$+?.]/g, "\\$&");
    }
  }
  return new RegExp(`${expression}$`);
}

/** Return true when one declared ownership pattern contains the path. */
export function patternMatches(pattern, relativePath) {
  return globToRegExp(pattern).test(normalizePath(relativePath));
}

/** Return every module that claims a repository-relative path. */
export function ownersForPath(modules, relativePath) {
  return modules.filter((module) =>
    module.owns.some((pattern) => patternMatches(pattern, relativePath)),
  );
}

const ignoredDirectoryNames = new Set([
  ".git",
  ".vite",
  "__pycache__",
  "binaries",
  "dist",
  "node_modules",
  "target",
  "tmp",
]);

/** Recursively list versionable files while skipping regenerated build trees. */
export async function listRepositoryFiles(directory = repositoryRoot) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (entry.isDirectory() && ignoredDirectoryNames.has(entry.name)) continue;
    const absolutePath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listRepositoryFiles(absolutePath)));
    } else if (entry.isFile()) {
      files.push(normalizePath(path.relative(repositoryRoot, absolutePath)));
    }
  }
  return files.sort();
}

/** Load both declarations and expose their arrays with a single call. */
export async function loadModuleMap() {
  const [moduleDocument, edgeDocument] = await Promise.all([
    readRepositoryJson("module-map/modules.json"),
    readRepositoryJson("module-map/edges.json"),
  ]);
  return { modules: moduleDocument.modules, edges: edgeDocument.edges };
}
