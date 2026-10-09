/** Desktop release operations; URLs, keys, files and device authority stay native. */
import { invoke } from '@tauri-apps/api/core';
import { isDesktopRuntime } from './desktopPaths';

export type UpdateChannel = 'stable' | 'candidate';
export interface UpdateState {
  updateId: string | null;
  phase: 'idle' | 'checking' | 'no-update' | 'available' | 'downloading' | 'verifying' | 'ready' | 'preparing' | 'applying' | 'cancelled' | 'failed';
  currentVersion: string;
  targetVersion: string | null;
  channel: UpdateChannel;
  kind: 'nsis' | 'msi' | 'development';
  source: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  verified: boolean;
  notice: string | null;
  error: string | null;
}
function desktop(): void { if (!isDesktopRuntime()) throw new Error('Software update requires the desktop application'); }
export async function getUpdateStatus(): Promise<UpdateState> { desktop(); return invoke('software_update_status'); }
export async function getApplicationVersion(): Promise<string> {
  if (!isDesktopRuntime()) return String(import.meta.env.VITE_APP_VERSION ?? 'Development preview');
  return (await getUpdateStatus()).currentVersion;
}
export async function checkSoftwareUpdate(channel: UpdateChannel): Promise<UpdateState> { desktop(); return invoke('software_update_check', { channel }); }
export async function cancelSoftwareUpdate(updateId: string): Promise<UpdateState> { desktop(); return invoke('software_update_cancel', { updateId }); }
export async function confirmSoftwareUpdate(updateId: string, expectedVersion: string): Promise<UpdateState> {
  desktop(); return invoke('software_update_confirm_apply', { updateId, expectedVersion });
}

export async function downloadSoftwareUpdate(updateId: string): Promise<UpdateState> { desktop(); return invoke('software_update_download', { updateId }); }
