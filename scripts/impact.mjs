/**
 * Report the modules, dependants, hidden edges, and gates affected by a change.
 *
 * With no arguments the script reads staged, unstaged, deleted, renamed, and
 * untracked worktree paths. Pass `--base <git-ref>` to also include committed
 * changes after that ref. The output is JSON so both people and automation can
 * consume the same result without parsing prose.
 */

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { loadModuleMap, normalizePath, ownersForPath, repositoryRoot } from "./lib/module-map.mjs";

function runGit(argumentsList) {
  return execFileSync("git", argumentsList, {
    cwd: repositoryRoot,
    encoding: "utf8",
    maxBuffer: 8 * 1024 * 1024,
  });
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  if (index < 0) return null;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function worktreePaths() {
  const output = runGit(["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
  const records = output.split("\0").filter(Boolean);
  const paths = [];
  for (let index = 0; index < records.length; index += 1) {
    const record = records[index];
    const status = record.slice(0, 2);
    const firstPath = record.slice(3);
    if (status.includes("R") || status.includes("C")) {
      paths.push(records[index + 1] ?? firstPath);
      index += 1;
    } else {
      paths.push(firstPath);
    }
  }
  return paths.map(normalizePath);
}

const base = argumentValue("--base");
const changed = new Set(worktreePaths());
if (base) {
  for (const file of runGit(["diff", "--name-only", "-z", `${base}..HEAD`]).split("\0").filter(Boolean)) {
    changed.add(normalizePath(file));
  }
}

const { modules, edges } = await loadModuleMap();
const owningModules = new Set();
const unowned = [];
const removedUnowned = [];
for (const file of changed) {
  const owners = ownersForPath(modules, file);
  if (owners.length === 0) {
    // A removed path may legitimately disappear from the new ownership map
    // during a module move. Keep it visible for review without failing the
    // current-tree coverage gate.
    if (existsSync(path.join(repositoryRoot, file))) unowned.push(file);
    else removedUnowned.push(file);
  }
  for (const owner of owners) owningModules.add(owner.id);
}

// Edges point from consumer to provider. Walk them backwards from changed
// providers so the report includes every transitive consumer that may break.
const impactedModules = new Set(owningModules);
let discoveredMore = true;
while (discoveredMore) {
  discoveredMore = false;
  for (const edge of edges) {
    if (impactedModules.has(edge.to) && !impactedModules.has(edge.from)) {
      impactedModules.add(edge.from);
      discoveredMore = true;
    }
  }
}

const moduleById = new Map(modules.map((module) => [module.id, module]));
const requiredGates = new Set();
for (const id of impactedModules) {
  for (const gate of moduleById.get(id)?.gates ?? []) requiredGates.add(gate);
}
const relevantEdges = edges.filter(
  (edge) => edge.strength === "strong" && (impactedModules.has(edge.from) || impactedModules.has(edge.to)),
);

console.log(JSON.stringify({
  base,
  changed: [...changed].sort(),
  owningModules: [...owningModules].sort(),
  impactedModules: [...impactedModules].sort(),
  strongEdges: relevantEdges,
  requiredGates: [...requiredGates].sort(),
  unowned,
  removedUnowned,
}, null, 2));

if (unowned.length > 0) process.exitCode = 1;
