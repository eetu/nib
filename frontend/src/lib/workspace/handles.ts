// File System Access handles can't go in localStorage (not serializable) but
// they ARE structured-cloneable, so we stash them in IndexedDB. This is what
// lets save-back survive HMR and page reloads: the handle is rehydrated and,
// as long as its permission is still granted, `savesInPlace` comes back.

const DB_NAME = "nib";
const STORE = "handles";

type PermissionMode = { mode?: "read" | "readwrite" };
type WithPermissions = {
  queryPermission?(desc?: PermissionMode): Promise<PermissionState>;
  requestPermission?(desc?: PermissionMode): Promise<PermissionState>;
};

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export async function saveHandle(key: string, handle: FileSystemHandle): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.objectStore(STORE).put(handle, key);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  } catch {
    // IndexedDB unavailable — persistence is best-effort
  }
}

export async function loadHandle<T extends FileSystemHandle>(key: string): Promise<T | null> {
  try {
    const db = await openDb();
    return await new Promise<T | null>((resolve) => {
      const tx = db.transaction(STORE, "readonly");
      const req = tx.objectStore(STORE).get(key);
      req.onsuccess = () => resolve((req.result as T) ?? null);
      req.onerror = () => resolve(null);
    });
  } catch {
    return null;
  }
}

export async function removeHandle(key: string): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.objectStore(STORE).delete(key);
      tx.oncomplete = () => resolve();
      tx.onerror = () => resolve();
    });
  } catch {
    // ignore
  }
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
