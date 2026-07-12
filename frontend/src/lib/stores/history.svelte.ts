/**
 * A generic snapshot-based undo/redo stack. Holds immutable state snapshots;
 * `canUndo`/`canRedo` are reactive so the UI can enable/disable its controls.
 * The document store owns an instance and decides what a snapshot is + how to
 * restore it (see document.svelte.ts).
 */
export class History<T> {
  #past = $state<T[]>([]);
  #future = $state<T[]>([]);
  #present: T | null = null;

  get canUndo(): boolean {
    return this.#past.length > 0;
  }

  get canRedo(): boolean {
    return this.#future.length > 0;
  }

  /** The current committed snapshot (used to revert an in-flight gesture). */
  current(): T | null {
    return this.#present;
  }

  /** Discard all history and seed the initial state. */
  reset(initial: T): void {
    this.#past = [];
    this.#future = [];
    this.#present = initial;
  }

  /** Record a new committed state; the old present becomes undoable. */
  commit(next: T): void {
    if (this.#present !== null) this.#past.push(this.#present);
    this.#present = next;
    this.#future = [];
  }

  undo(): T | null {
    if (this.#past.length === 0) return null;
    const prev = this.#past.pop() as T;
    if (this.#present !== null) this.#future.push(this.#present);
    this.#present = prev;
    return prev;
  }

  redo(): T | null {
    if (this.#future.length === 0) return null;
    const next = this.#future.pop() as T;
    if (this.#present !== null) this.#past.push(this.#present);
    this.#present = next;
    return next;
  }
}
