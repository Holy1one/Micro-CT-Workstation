/** Load actual TS source and its local imports for offline policy tests. No production fallback is created. */
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import { transformWithEsbuild } from "vite";
const root = new URL("../", import.meta.url);
const require = createRequire(root);
const cache = new Map();
export async function sourceModuleUrl(file) {
  const url = file instanceof URL ? file : new URL(file, root);
  if (cache.has(url.href)) return cache.get(url.href);
  const source = await readFile(url, "utf8");
  const { code } = await transformWithEsbuild(source, fileURLToPath(url), { loader: url.pathname.endsWith("tsx") ? "tsx" : "ts", target: "es2022", format: "esm" });
  let output = code;
  const imports = [...code.matchAll(/^\s*import\s+(?:[^;"']*?\bfrom\s+)?["']([^"'\r\n]+)["']/gm)];
  for (const match of imports) {
    const specifier = match[1];
    let target;
    if (specifier.startsWith(".")) {
      const base = new URL(specifier, url);
      const resolved = [base, new URL(`${base.href}.ts`), new URL(`${base.href}.tsx`)].find(item => existsSync(fileURLToPath(item)));
      if (!resolved) throw new Error(`Unresolved source import ${specifier} in ${url.href}`);
      target = await sourceModuleUrl(resolved);
    } else target = pathToFileURL(require.resolve(specifier)).href;
    output = output.replaceAll(`"${specifier}"`, `"${target}"`).replaceAll(`'${specifier}'`, `'${target}'`);
  }
  const encoded = `data:text/javascript;base64,${Buffer.from(output).toString("base64")}`;
  cache.set(url.href, encoded);
  return encoded;
}
export async function loadSourceModule(file) { return import(await sourceModuleUrl(file)); }
