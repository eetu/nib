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

  #activeHandle: FileSystemFileHandle | null = null;

  constructor() {
    // Rehydrate the file/folder handles (IndexedDB) so save-back + the folder
    // list survive HMR and reload. Async — Save flips back from Download to
    // Save once the handle resolves.
    void this.#hydrate();
  }

  async #hydrate(): Promise<void> {
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

  /** Clear the current error banner (dismiss). */
  dismissError(): void {
    this.error = null;
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
      editor.load(source, file.name);
      this.#clearHandle();
    } catch (e) {
      this.error = errMessage(e);
    }
  }

  /** Start a fresh blank document (New) + ready the pen to draw. Confirms first if there are
   *  unsaved changes. */
  newDocument(): void {
    if (editor.dirty && !confirm("Discard unsaved changes and start a new drawing?")) return;
    this.error = null;
    editor.newDocument();
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
      editor.markSaved();
    }
  }

  /** Load pasted/dropped SVG text (no backing handle → Save downloads). */
  importText(source: string, name = "untitled.svg"): void {
    this.error = null;
    try {
      editor.load(source, name);
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
      editor.markSaved();
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
      editor.load(source, name);
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
