// File System Access handles can't go in localStorage (not serializable) but
// they ARE structured-cloneable, so we stash them in IndexedDB. This is what
// lets save-back survive HMR and page reloads: the handle is rehydrated and,
// as long as its permission is still granted, `savesInPlace` comes back.

import { idbDelete, idbGet, idbPut } from "$lib/persistence/idb";

const STORE = "handles";

type PermissionMode = { mode?: "read" | "readwrite" };
type WithPermissions = {
  queryPermission?(desc?: PermissionMode): Promise<PermissionState>;
  requestPermission?(desc?: PermissionMode): Promise<PermissionState>;
};

export async function saveHandle(key: string, handle: FileSystemHandle): Promise<void> {
  await idbPut(STORE, key, handle);
}

export async function loadHandle<T extends FileSystemHandle>(key: string): Promise<T | null> {
  return idbGet<T>(STORE, key);
}

export async function removeHandle(key: string): Promise<void> {
  await idbDelete(STORE, key);
}

/**
 * Read+write permission for a handle. `request` gates the interactive prompt
 * (only from a user gesture). Without it, a "prompt" state still counts as
 * restorable — the Save click will request then.
 */
export async function ensurePermission(
  handle: FileSystemHandle,
  request: boolean,
): Promise<boolean> {
  const h = handle as FileSystemHandle & WithPermissions;
  const state = (await h.queryPermission?.({ mode: "readwrite" })) ?? "granted";
  if (state === "granted") return true;
  if (state === "denied") return false;
  if (!request) return true; // "prompt" — restorable, will ask on save
  return (await h.requestPermission?.({ mode: "readwrite" })) === "granted";
}
