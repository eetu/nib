import { askConfirm } from "$lib/confirm.svelte";
import { loadState, saveState } from "$lib/persistence";
import {
  downloadSvg,
  listSvgFiles,
  pickDirectory,
  pickSaveFile,
  pickSvgFile,
  readFile,
  supportsFilePicker,
  supportsFolders,
  supportsSaveFilePicker,
  type WorkspaceFile,
  writeFile,
} from "$lib/workspace/fs";
import { ensurePermission, loadHandle, removeHandle, saveHandle } from "$lib/workspace/handles";

import { editor } from "./document.svelte";
import { tools } from "./tool.svelte";

const ACTIVE_FILE = "activeFile";
const ACTIVE_DIR = "dir";
/** The document as it last stood on disk — the baseline Revert restores. */
const BASELINE_KEY = "baseline";

function errMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * The file side of the app: an opened folder's `.svg` list plus the currently
 * loaded file, and the save-back / export plumbing. Editing happens on one
 * active file at a time; `editor.dirty` tracks its unsaved state.
 */
class Workspace {
  readonly foldersSupported = supportsFolders();
  readonly filePickerSupported = supportsFilePicker();

  dirName = $state<string | null>(null);
  files = $state<WorkspaceFile[]>([]);
  /** True when Save writes back to a real file handle (vs. a download). */
  savesInPlace = $state(false);
  busy = $state(false);
  error = $state<string | null>(null);

  /**
   * The document as it was last read from — or written to — disk.
   *
   * Undo lives in memory, so without this a reload is the one gesture that makes every unsaved
   * change permanent: the working draft is restored, the history that could walk it back is not.
   * Keeping the on-disk text alongside the draft turns that into a question the user can answer.
   */
  savedSvg = $state<string | null>(null);
  /** Something worth saying that isn't a failure — e.g. which font a label was outlined with when
   *  it wasn't the one the document asked for. Same bar as `error`, different voice. */
  notice = $state<string | null>(null);

  #activeHandle: FileSystemFileHandle | null = null;

  constructor() {
    // Rehydrate the file/folder handles (IndexedDB) so save-back + the folder
    // list survive HMR and reload. Async — Save flips back from Download to
    // Save once the handle resolves.
    void this.#hydrate();
  }

  async #hydrate(): Promise<void> {
    this.savedSvg = loadState<string | null>(BASELINE_KEY) ?? null;
    const file = await loadHandle<FileSystemFileHandle>(ACTIVE_FILE);
    if (file && (await ensurePermission(file, false))) {
      this.#activeHandle = file;
      this.savesInPlace = true;
    }
    const dir = await loadHandle<FileSystemDirectoryHandle>(ACTIVE_DIR);
    if (dir && (await ensurePermission(dir, false))) {
      try {
        this.files = await listSvgFiles(dir);
        this.dirName = dir.name;
      } catch {
        // folder gone / no access — leave the list empty
      }
    }
  }

  /** Is there a saved state to go back to, and would going back change anything? */
  get canRevert(): boolean {
    return this.savedSvg !== null && editor.dirty;
  }

  /** Remember what's on disk now, and survive a reload knowing it. */
  #setBaseline(svg: string | null): void {
    this.savedSvg = svg;
    saveState(BASELINE_KEY, svg);
  }

  /**
   * Throw away the edits made since the last save and go back to the file on disk.
   *
   * The one action here undo can't take back, so it's the one that asks first — and it says how
   * much is at stake rather than "are you sure?".
   */
  async revert(): Promise<void> {
    const baseline = this.savedSvg;
    if (baseline === null) return;
    const ok = await askConfirm({
      title: "revert to the saved file?",
      body: "every change since the last save is discarded — undo can't bring them back.",
      confirmLabel: "revert",
      danger: true,
    });
    if (!ok) return;
    this.error = null;
    try {
      editor.load(baseline, editor.fileName);
      editor.markSaved();
      this.notice = `reverted to ${editor.fileName ?? "the saved file"}`;
    } catch (e) {
      this.error = errMessage(e);
    }
  }

  /** Clear the current error banner (dismiss). */
  dismissError(): void {
    this.error = null;
  }

  /** Clear the current notice (dismiss). */
  dismissNotice(): void {
    this.notice = null;
  }

  /** Pick a folder and list its SVGs (Chromium only). */
  async openFolder(): Promise<void> {
    if (!this.foldersSupported) return;
    this.error = null;
    try {
      const dir = await pickDirectory();
      if (!dir) return;
      this.busy = true;
      this.files = await listSvgFiles(dir);
      this.dirName = dir.name;
      void saveHandle(ACTIVE_DIR, dir);
      if (this.files.length === 0) this.error = "no .svg files in that folder";
    } catch (e) {
      this.error = errMessage(e);
    } finally {
      this.busy = false;
    }
  }

  /** Load a file from the opened folder into the editor. */
  async openFile(file: WorkspaceFile): Promise<void> {
    await this.#loadFrom(file.handle, file.name, true);
  }

  /** Fallback: open a single file directly (no folder). */
  async openSingleFile(): Promise<void> {
    if (!this.filePickerSupported) return;
    this.error = null;
    try {
      const file = await pickSvgFile();
      if (!file) return;
      await this.#loadFrom(file.handle, file.name, true);
    } catch (e) {
      // A non-abort picker error (e.g. SecurityError) would otherwise escape as an unhandled
      // rejection via the un-awaited onclick — surface it instead.
      this.error = errMessage(e);
    }
  }

  /** Load a plain File (input picker or drag-drop) — works in every browser.
   *  No handle, so Save downloads. */
  async importFile(file: File): Promise<void> {
    this.error = null;
    try {
      const source = await file.text();
      editor.importDocument(source, file.name);
      this.#setBaseline(source);
      this.#clearHandle();
    } catch (e) {
      this.error = errMessage(e);
    }
  }

  /** Start a fresh blank document (New) + ready the pen to draw. Confirms first if there are
   *  unsaved changes. */
  async newDocument(): Promise<void> {
    if (editor.dirty) {
      const ok = await askConfirm({
        title: "start a new drawing?",
        body: "the unsaved changes in this one are discarded.",
        confirmLabel: "discard",
        danger: true,
      });
      if (!ok) return;
    }
    this.error = null;
    editor.newDocument();
    this.#setBaseline(null);
    this.#clearHandle();
    tools.set("pen");
  }

  /** Save the current document to a new file (Save As): a save picker (Chromium, adopts the
   *  handle so later Saves write back), else a named download. */
  async saveAs(): Promise<void> {
    if (!editor.hasDocument) return;
    const svg = editor.toSvg();
    const name = editor.fileName ?? "untitled.svg";
    this.error = null;
    if (supportsSaveFilePicker()) {
      this.busy = true;
      try {
        const handle = await pickSaveFile(name);
        if (!handle) return;
        await writeFile(handle, svg);
        this.#activeHandle = handle;
        this.savesInPlace = true;
        editor.fileName = handle.name;
        void saveHandle(ACTIVE_FILE, handle);
        this.#setBaseline(svg);
        editor.markSaved();
      } catch (e) {
        this.error = errMessage(e);
      } finally {
        this.busy = false;
      }
    } else {
      const input = prompt("Save as (filename):", name);
      if (input == null) return;
      downloadSvg(input, svg);
      this.#setBaseline(svg);
      editor.markSaved();
    }
  }

  /** Load pasted/dropped SVG text (no backing handle → Save downloads). */
  importText(source: string, name = "untitled.svg"): void {
    this.error = null;
    try {
      editor.importDocument(source, name);
      this.#setBaseline(source);
      this.#clearHandle();
    } catch (e) {
      this.error = errMessage(e);
    }
  }

  /** Save the current document: write back to its handle, else download it. */
  async save(): Promise<void> {
    if (!editor.hasDocument) return;
    const svg = editor.toSvg();
    this.busy = true;
    this.error = null;
    try {
      if (this.#activeHandle && (await ensurePermission(this.#activeHandle, true))) {
        await writeFile(this.#activeHandle, svg);
      } else {
        downloadSvg(editor.fileName ?? "nib.svg", svg);
      }
      this.#setBaseline(svg);
      editor.markSaved();
      this.notice = `saved ${editor.fileName ?? "nib.svg"}`;
    } catch (e) {
      this.error = errMessage(e);
    } finally {
      this.busy = false;
    }
  }

  async #loadFrom(
    handle: FileSystemFileHandle,
    name: string,
    savesInPlace: boolean,
  ): Promise<void> {
    this.busy = true;
    this.error = null;
    try {
      const source = await readFile(handle);
      editor.importDocument(source, name);
      this.#setBaseline(source);
      this.#activeHandle = handle;
      this.savesInPlace = savesInPlace;
      void saveHandle(ACTIVE_FILE, handle);
    } catch (e) {
      this.error = errMessage(e);
    } finally {
      this.busy = false;
    }
  }

  #clearHandle(): void {
    this.#activeHandle = null;
    this.savesInPlace = false;
    void removeHandle(ACTIVE_FILE);
  }
}

export const workspace = new Workspace();
