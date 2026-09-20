/**
 * Static contract test that keeps every declared menu id connected to App.tsx.
 * Reading source text here is intentional: it catches decorative menu entries
 * without launching the browser or duplicating the React implementation.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const root = new URL("../", import.meta.url);
const read = (relative) => readFileSync(new URL(relative, root), "utf8");

const MENU_ID = /"(file\.[a-zA-Z]+|edit\.[a-zA-Z]+|tools\.[a-zA-Z]+|help\.[a-zA-Z]+)"/g;

test("every menu entry id is handled in App.tsx", () => {
  const model = read("src/menuActions.ts");
  const app = read("src/App.tsx");
  const ids = [...new Set([...model.matchAll(MENU_ID)].map((match) => match[1]))];

  assert.equal(ids.length, 15, "four menus, fifteen entries");
  for (const id of ids) {
    assert.match(app, new RegExp(`case "${id.replace(".", "\\.")}":`), `${id} needs a handler`);
  }
});

test("the four top menus keep their labels", () => {
  const model = read("src/menuActions.ts");
  for (const label of ["File", "Edit", "Tools", "Help"]) {
    assert.match(model, new RegExp(`label: "${label}"`), `${label} menu is missing`);
  }
});

test("menu entries are wired, never placeholders", () => {
  const model = read("src/menuActions.ts");
  const app = read("src/App.tsx");
  assert.doesNotMatch(model, /TODO|FIXME|placeholder|reserved/i);
  assert.match(app, /availability=\{availability\}/);
  assert.match(app, /onAction=\{\(id\) => void handleMenuAction\(id\)\}/);
  assert.match(app, /syncRevision=\{setupSync\}/);
});

test("desktop shell actions exist on both sides of the IPC boundary", () => {
  const paths = read("src/platform/desktopPaths.ts");
  const main = read("src-tauri/src/main.rs");
  const capability = read("src-tauri/capabilities/default.json");

  assert.match(paths, /export async function revealDirectory\(/);
  assert.match(paths, /export async function exportSessionLog\(/);
  assert.match(paths, /invoke<string>\("open_directory_in_shell"/);
  assert.match(paths, /invoke<string>\("export_session_log"/);

  assert.match(main, /fn open_directory_in_shell\(/);
  assert.match(main, /fn export_session_log\(/);
  assert.match(main, /open_directory_in_shell,\s*\n\s*export_session_log/);
  assert.match(capability, /"dialog:allow-save"/);
});

test("File > Open Image Folder refills the Save Path draft after picking", () => {
  const app = read("src/App.tsx");
  const start = app.indexOf('case "file.openImageFolder":');
  const end = app.indexOf('case "file.openLastResult":');
  assert.ok(start > 0 && end > start, "both menu branches must exist");
  const branch = app.slice(start, end);

  assert.match(branch, /chooseImageDirectory\(/);
  assert.match(branch, /setup: \{ savePath: selected \}/);
  // Without this flag the draft input keeps the previous text, so the field
  // disagrees with the folder the picker returned.
  assert.match(branch, /setupPendingSync\.current = true/);
  assert.match(branch, /revealDirectory\(selected\)/);
});

test("the save dialog stays the only write path and log targets are whitelisted", () => {
  const main = read("src-tauri/src/main.rs");
  const capability = read("src-tauri/capabilities/default.json");

  assert.match(main, /if extension != "log" && extension != "txt"/);
  assert.doesNotMatch(capability, /fs:|shell:/);
});
