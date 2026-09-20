# Module map

This directory is the machine-readable source of truth for repository ownership and hidden dependencies.

- `modules.json` assigns every maintained area to one module and records its risk, public surface, and verification gates.
- `edges.json` records dependencies that a compiler may not fully protect, such as Tauri command names, JSONL messages, serial protocols, and UI semantics.
- `directory-policy.json` freezes the approved immediate child directories at important functional boundaries. A new functional directory requires the request described in the applicable `AGENTS.md` before this declaration changes.
- `docs/architecture/module-graph.md` is generated from these files. Do not edit the generated graph by hand.

The map is navigation, not a substitute for source code or tests. Update the JSON files in the same change whenever a directory moves, a public contract changes, or a new hidden dependency is introduced.

Commands:

```powershell
npm.cmd run module-map:generate
npm.cmd run module-map:check
npm.cmd run impact
```

JSON is used instead of YAML so the scripts need no additional parser dependency. Because standard JSON cannot contain comments, each record contains explicit description, execution, and reason fields.
