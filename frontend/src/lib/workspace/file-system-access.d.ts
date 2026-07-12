// The File System Access *picker* entry points aren't in this TypeScript lib
// version yet. The handle types (FileSystemDirectoryHandle / FileSystemFileHandle
// and their .values() / .getFile() / .createWritable()) already are, so we only
// augment Window with the two pickers we call.
export {};

type FilePickerAcceptType = {
  description?: string;
  accept: Record<string, string[]>;
};

declare global {
  interface Window {
    showDirectoryPicker(options?: {
      mode?: "read" | "readwrite";
    }): Promise<FileSystemDirectoryHandle>;
    showOpenFilePicker(options?: {
      multiple?: boolean;
      types?: FilePickerAcceptType[];
    }): Promise<FileSystemFileHandle[]>;
  }
}
