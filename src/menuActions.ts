/** Concrete workstation actions. Menus and panel controls reuse the same domain entries. */
import type { LineIconName } from "./icons";
export type TopMenu = "Task" | "Devices" | "View" | "Tools" | "Help";
export type MenuActionId =
  | "task.new" | "task.prepare" | "task.restore" | "task.abandon" | "task.openFolder" | "task.resetDraft" | "task.play" | "task.stop"
  | "devices.camera" | "devices.cameraSettings" | "devices.turntable" | "devices.turntableHome" | "devices.turntableBench" | "devices.xray"
  | "view.equipment" | "view.result" | "view.photo" | "view.live" | "view.images" | "view.reconstruction" | "view.logs"
  | "tools.deviceDiagnostics" | "tools.exportLog" | "tools.preferences"
  | "help.userGuide" | "help.safetyNotes" | "help.troubleshooting" | "help.checkUpdate" | "help.importUpdate" | "help.about";
export interface MenuEntry { id: MenuActionId; label: string; icon: LineIconName }
export interface MenuGroup { label: TopMenu; entries: readonly MenuEntry[] }
export const MENU_GROUPS: readonly MenuGroup[] = [
  { label: "Task", entries: [
    { id: "task.new", label: "New Scan Task…", icon: "task-new" },
    { id: "task.prepare", label: "Scan Preparation / Preflight…", icon: "shield-check" },
    { id: "task.play", label: "Start Scan", icon: "shield-check" },
    { id: "task.stop", label: "Stop", icon: "warning" },
    { id: "task.restore", label: "Restore Unfinished Scan…", icon: "history" },
    { id: "task.abandon", label: "Abandon Task / Retain Files…", icon: "warning" },
    { id: "task.openFolder", label: "Open Task Folder", icon: "folder-open" },
    { id: "task.resetDraft", label: "Reset Preparation Draft…", icon: "reset" },
  ] },
  { label: "Devices", entries: [
    { id: "devices.camera", label: "Camera Controls", icon: "camera" },
    { id: "devices.cameraSettings", label: "Camera Advanced Settings…", icon: "sliders" },
    { id: "devices.turntable", label: "Turntable Controls", icon: "crosshair" },
    { id: "devices.turntableHome", label: "Turntable HOME", icon: "crosshair" },
    { id: "devices.turntableBench", label: "Isolated Turntable Bench…", icon: "shield-check" },
    { id: "devices.xray", label: "X-ray Controls", icon: "radiation" },
  ] },
  { label: "View", entries: [
    { id: "view.equipment", label: "Equipment", icon: "cube" },
    { id: "view.result", label: "Reconstruction Result", icon: "cube" },
    { id: "view.photo", label: "Last Independent Photo", icon: "camera" },
    { id: "view.live", label: "Current Live View", icon: "camera" },
    { id: "view.images", label: "Captured Projections", icon: "image" },
    { id: "view.reconstruction", label: "Reconstruction Controls", icon: "cube" },
    { id: "view.logs", label: "Session Logs", icon: "terminal" },
  ] },
  { label: "Tools", entries: [
    { id: "tools.deviceDiagnostics", label: "Read-only Device Diagnostics…", icon: "pulse" },
    { id: "tools.exportLog", label: "Export Session Log…", icon: "export" },
    { id: "tools.preferences", label: "Interface Preferences…", icon: "sliders" },
  ] },
  { label: "Help", entries: [
    { id: "help.userGuide", label: "User Guide", icon: "book" },
    { id: "help.safetyNotes", label: "Safety Notes", icon: "warning" },
    { id: "help.troubleshooting", label: "Troubleshooting…", icon: "pulse" },
    { id: "help.checkUpdate", label: "Check Update…", icon: "history" },
    { id: "help.importUpdate", label: "Import Signed Offline Update…", icon: "folder-open" },
    { id: "help.about", label: "About Micro-CT Workstation", icon: "info" },
  ] },
];
export interface MenuEntryState { available: boolean; reason: string | null; label?: string }
export type MenuAvailability = Record<MenuActionId, MenuEntryState>;
/** Preparation defaults never change the camera shutter or physical geometry. */
export const DEFAULT_SCAN_SETUP = { projectionCount: 120, maxXraySec: 600 } as const;
