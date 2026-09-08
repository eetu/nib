// The Local Font Access API isn't in this TS lib version (Chromium-only, permission-gated), so
// declare the slice nib uses — the same pattern as `workspace/file-system-access.d.ts`.

type FontData = {
  /** Family name as the system reports it, e.g. `Helvetica Neue`. */
  readonly family: string;
  /** Face name within the family, e.g. `Bold Italic`. */
  readonly style: string;
  readonly fullName: string;
  /** Identifies the exact face — which matters when the file is a `.ttc` collection. */
  readonly postscriptName: string;
  /** The raw font file. For a collection this is the whole `.ttc`, not one face. */
  blob(): Promise<Blob>;
};

interface Window {
  queryLocalFonts?(options?: { postscriptNames?: string[] }): Promise<FontData[]>;
}
