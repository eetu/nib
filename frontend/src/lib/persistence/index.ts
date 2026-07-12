// A tiny state-persistence layer. State is written behind a swappable adapter:
// localStorage today, a backend later. Keeping the editor's state here means a
// code reload (Vite HMR) or a page refresh no longer wipes the loaded document,
// its edits, or the current selection — the stores rehydrate from here in their
// constructors, so a re-created singleton comes back with its state intact.

const NS = "nib:";

export interface PersistenceAdapter {
  get(key: string): string | null;
  set(key: string, value: string): void;
  remove(key: string): void;
}

class LocalStorageAdapter implements PersistenceAdapter {
  get(key: string): string | null {
    try {
      return localStorage.getItem(key);
    } catch {
      return null; // SSR / disabled storage — persistence is best-effort
    }
  }

  set(key: string, value: string): void {
    try {
      localStorage.setItem(key, value);
    } catch {
      // quota exceeded / private mode — drop silently
    }
  }

  remove(key: string): void {
    try {
      localStorage.removeItem(key);
    } catch {
      // ignore
    }
  }
}

let adapter: PersistenceAdapter = new LocalStorageAdapter();

/**
 * Swap the storage backend — e.g. an API-backed adapter once nib grows a
 * backend. A network adapter should keep a synchronous in-memory cache, seed it
 * before the app first reads (the stores hydrate eagerly), and write through to
 * the server in the background so `get`/`set` stay synchronous here.
 */
export function setPersistenceAdapter(next: PersistenceAdapter): void {
  adapter = next;
}

export function loadState<T>(key: string): T | null {
  const raw = adapter.get(NS + key);
  if (raw === null) return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

export function saveState<T>(key: string, value: T): void {
  adapter.set(NS + key, JSON.stringify(value));
}

export function removeState(key: string): void {
  adapter.remove(NS + key);
}

/** Coalesce rapid writes (a burst of edits) into a single persisted write. */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number,
): (...args: A) => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return (...args: A) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}
