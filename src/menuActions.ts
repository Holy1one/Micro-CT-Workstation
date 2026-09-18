/**
 * Top menu model for the workstation console.
 *
 * The entries are real commands: every id is handled in `App.tsx` and either
 * dispatches through the engine control chain or opens a desktop shell action
 * (native picker / Explorer / save dialog). Nothing here is decorative, and the
 * engine stays the single source of truth for state and safety.
 */

export type TopMenu = "File" | "Edit" | "Tools" | "Help";

export type MenuActionId =
  | "file.newTask"
  | "file.openImageFolder"
  | "file.openLastResult"
  | "file.exportLog"
  | "edit.undo"
  | "edit.redo"
  | "edit.resetParameters"
  | "edit.preferences"
  | "tools.runPreflight"
  | "tools.homeAllAxes"
  | "tools.restorePrevious"
  | "tools.deviceDiagnostics"
  | "help.userGuide"
  | "help.safetyNotes"
  | "help.about";

export interface MenuEntry {
  id: MenuActionId;
  label: string;
}

export interface MenuGroup {
  label: TopMenu;
  entries: readonly MenuEntry[];
}

export const MENU_GROUPS: readonly MenuGroup[] = [
  {
    label: "File",
    entries: [
      { id: "file.newTask", label: "New Scan Task…" },
      { id: "file.openImageFolder", label: "Open Image Folder…" },
      { id: "file.openLastResult", label: "Open Last Result…" },
      { id: "file.exportLog", label: "Export Session Log…" },
    ],
  },
  {
    label: "Edit",
    entries: [
      { id: "edit.undo", label: "Undo" },
      { id: "edit.redo", label: "Redo" },
      { id: "edit.resetParameters", label: "Reset Parameters" },
      { id: "edit.preferences", label: "Preferences…" },
    ],
  },
  {
    label: "Tools",
    entries: [
      { id: "tools.runPreflight", label: "Run Preflight" },
      { id: "tools.homeAllAxes", label: "Home All Axes" },
      { id: "tools.restorePrevious", label: "Restore Previous Scan" },
      { id: "tools.deviceDiagnostics", label: "Device Diagnostics…" },
    ],
  },
  {
    label: "Help",
    entries: [
      { id: "help.userGuide", label: "User Guide" },
      { id: "help.safetyNotes", label: "Safety Notes" },
      { id: "help.about", label: "About Micro-CT Workstation" },
    ],
  },
];

/** Why an entry is greyed out; `null` means the engine accepts it right now. */
export interface MenuEntryState {
  available: boolean;
  reason: string | null;
}

export type MenuAvailability = Record<MenuActionId, MenuEntryState>;

/** Parameter values restored by `Edit → Reset Parameters`. */
export const DEFAULT_SCAN_SETUP = {
  projectionCount: 120,
  exposureMs: 200,
  maxXraySec: 600,
} as const;
