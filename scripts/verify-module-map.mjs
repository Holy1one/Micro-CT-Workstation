/**
 * Validate that the module map is internally consistent and covers the tree.
 *
 * The check rejects duplicate module identifiers, dangling dependency edges,
 * ownership patterns that match nothing, files claimed by multiple modules,
 * and files with no owner. Generated build trees are excluded by the shared
 * file walker because they are not design inputs.
 */

import { readdir } from "node:fs/promises";
import path from "node:path";
import {
  listRepositoryFiles,
  loadModuleMap,
  ownersForPath,
  patternMatches,
  readRepositoryJson,
  repositoryRoot,
} from "./lib/module-map.mjs";

const { modules, edges } = await loadModuleMap();
const directoryPolicy = await readRepositoryJson("module-map/directory-policy.json");
const files = await listRepositoryFiles();
const failures = [];
const moduleIds = new Set();

for (const module of modules) {
  if (moduleIds.has(module.id)) failures.push(`Duplicate module id: ${module.id}`);
  moduleIds.add(module.id);
  if (!module.owns.length) failures.push(`Module has no ownership patterns: ${module.id}`);
  for (const pattern of module.owns) {
    if (!files.some((file) => patternMatches(pattern, file))) {
      failures.push(`Ownership pattern matches no file: ${module.id} -> ${pattern}`);
    }
  }
}

for (const edge of edges) {
  if (!moduleIds.has(edge.from)) failures.push(`Unknown edge consumer: ${edge.from}`);
  if (!moduleIds.has(edge.to)) failures.push(`Unknown edge provider: ${edge.to}`);
  if (edge.from === edge.to) failures.push(`Self dependency is not useful: ${edge.from}`);
}

for (const file of files) {
  const owners = ownersForPath(modules, file);
  if (owners.length === 0) failures.push(`Unowned file: ${file}`);
  if (owners.length > 1) {
    failures.push(`Multiply owned file: ${file} -> ${owners.map(({ id }) => id).join(", ")}`);
  }
}

for (const parent of directoryPolicy.parents) {
  const parentPath = path.resolve(repositoryRoot, parent.path);
  const allowed = new Set(parent.allowedChildren);
  if (allowed.size !== parent.allowedChildren.length) {
    failures.push(`Duplicate allowed child in directory policy: ${parent.path}`);
  }
  const entries = await readdir(parentPath, { withFileTypes: true });
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    if (!allowed.has(entry.name)) {
      const relative = path.posix.join(parent.path === "." ? "" : parent.path, entry.name);
      failures.push(`Unapproved directory: ${relative}. Follow the NEW DIRECTORY REQUEST process before adding it.`);
    }
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exitCode = 1;
} else {
  console.log(`PASS module map covers ${files.length} files across ${modules.length} modules and ${directoryPolicy.parents.length} frozen directory boundaries`);
}
