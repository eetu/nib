// The structured-clone half of persistence: values localStorage can't hold. File System Access
// handles aren't serializable but ARE cloneable, and font bytes are far too big for a string
// store — both live here instead.
//
// One database, one version, every store created in the same upgrade: an object store missing
// from an upgrade is missing forever for that browser, so stores are declared together rather
// than added per feature.

const DB_NAME = "nib";
const DB_VERSION = 2;
const STORES = ["handles", "fonts"] as const;

export type IdbStore = (typeof STORES)[number];

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      for (const store of STORES)
        if (!req.result.objectStoreNames.contains(store)) req.result.createObjectStore(store);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

/** Read one value, or `null` when it's absent or IndexedDB is unavailable (private mode, SSR). */
export async function idbGet<T>(store: IdbStore, key: string): Promise<T | null> {
  try {
    const db = await openDb();
    return await new Promise<T | null>((resolve) => {
      const tx = db.transaction(store, "readonly");
      const req = tx.objectStore(store).get(key);
      req.onsuccess = () => resolve((req.result as T) ?? null);
      req.onerror = () => resolve(null);
    });
  } catch {
    return null;
  }
}

/** Write one value. Best-effort: a browser with IndexedDB blocked simply doesn't remember. */
export async function idbPut(store: IdbStore, key: string, value: unknown): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(store, "readwrite");
      tx.objectStore(store).put(value, key);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  } catch {
    // persistence is best-effort
  }
}

/** Delete one value. */
export async function idbDelete(store: IdbStore, key: string): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve) => {
      const tx = db.transaction(store, "readwrite");
      tx.objectStore(store).delete(key);
      tx.oncomplete = () => resolve();
      tx.onerror = () => resolve();
    });
  } catch {
    // ignore
  }
}
