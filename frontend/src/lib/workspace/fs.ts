// Thin wrappers over the File System Access API, plus universal fallbacks.
// The picker/writable APIs are Chromium-only; callers gate on supportsFolders()
// and fall back to open-single-file / download where unavailable.

export type WorkspaceFile = {
  name: string;
  handle: FileSystemFileHandle;
};

export function supportsFolders(): boolean {
  return typeof window !== "undefined" && "showDirectoryPicker" in window;
}

export function supportsFilePicker(): boolean {
  return typeof window !== "undefined" && "showOpenFilePicker" in window;
}

export function supportsSaveFilePicker(): boolean {
  return typeof window !== "undefined" && "showSaveFilePicker" in window;
}

function isAbort(err: unknown): boolean {
  return err instanceof DOMException && err.name === "AbortError";
}

/** Prompt for a directory (read+write). Returns null if the user cancels. */
export async function pickDirectory(): Promise<FileSystemDirectoryHandle | null> {
  try {
    return await window.showDirectoryPicker({ mode: "readwrite" });
  } catch (err) {
    if (isAbort(err)) return null;
    throw err;
  }
}

/** List the `.svg` files directly inside a directory, sorted by name. */
export async function listSvgFiles(dir: FileSystemDirectoryHandle): Promise<WorkspaceFile[]> {
  const files: WorkspaceFile[] = [];
  for await (const entry of dir.values()) {
    if (entry.kind === "file" && entry.name.toLowerCase().endsWith(".svg")) {
      files.push({ name: entry.name, handle: entry });
    }
  }
  files.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
  return files;
}

/** Open a single `.svg` (fallback when there's no folder). Null if cancelled. */
export async function pickSvgFile(): Promise<WorkspaceFile | null> {
  try {
    const [handle] = await window.showOpenFilePicker({
      multiple: false,
      types: [{ description: "SVG", accept: { "image/svg+xml": [".svg"] } }],
    });
    return { name: handle.name, handle };
  } catch (err) {
    if (isAbort(err)) return null;
    throw err;
  }
}

/** Prompt for a save location + name (Save As). Returns null if cancelled. */
export async function pickSaveFile(suggestedName: string): Promise<FileSystemFileHandle | null> {
  const name = suggestedName.toLowerCase().endsWith(".svg")
    ? suggestedName
    : `${suggestedName}.svg`;
  try {
    return await window.showSaveFilePicker({
      suggestedName: name,
      types: [{ description: "SVG", accept: { "image/svg+xml": [".svg"] } }],
    });
  } catch (err) {
    if (isAbort(err)) return null;
    throw err;
  }
}

export async function readFile(handle: FileSystemFileHandle): Promise<string> {
  const file = await handle.getFile();
  return file.text();
}

export async function writeFile(handle: FileSystemFileHandle, contents: string): Promise<void> {
  const writable = await handle.createWritable();
  await writable.write(contents);
  await writable.close();
}

/** Universal export: trigger a browser download of the SVG. */
export function downloadSvg(name: string, contents: string): void {
  const blob = new Blob([contents], { type: "image/svg+xml" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name.toLowerCase().endsWith(".svg") ? name : `${name}.svg`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
