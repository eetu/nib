// Finding the font bytes a label asks for. The engine shapes glyphs (`nib-core`'s rustybuzz), but
// it can't *fetch* a font: WASM has no filesystem and no view of installed fonts. So resolution
// lives here, and it's the browser half of the same job fontdb does for the backend.
//
// Three sources, in order:
//   1. what nib already loaded for that family (memory, then IndexedDB — font files are megabytes,
//      far past localStorage, and re-asking on every conversion would be miserable),
//   2. the installed fonts, via the Local Font Access API — Chromium-only and permission-gated,
//      the same trade the folder mode already makes,
//   3. a font file the user picks, which is the path every other browser takes.
//
// `.ttf`, `.otf`, `.ttc` and `.woff2` all work. The last one is what a font *download* is — so it
// is exactly what someone has on disk when they go looking for a face — and the core decompresses
// it (Brotli plus reversing WOFF2's glyph-table transform).

import { Editor } from "$lib/core";
import type { TextInfo } from "$lib/model/types";
import { idbGet, idbPut } from "$lib/persistence/idb";

const STORE = "fonts";

/** Font bytes ready to shape with, plus which face of a collection to use. */
export type LoadedFont = {
  bytes: Uint8Array;
  faceIndex: number;
  /** Human name for the face, for the "outlined with …" notice. */
  label: string;
  /** The face's own family — compared against what the label asked for, to spot a substitution. */
  family: string;
};

/** Fonts already resolved this session, keyed by {@link fontKey}. */
const memory = new Map<string, LoadedFont>();

/** What generic CSS families mean in practice, most-likely first. The API only knows real faces. */
const GENERIC_FAMILIES: Record<string, string[]> = {
  "sans-serif": ["Helvetica Neue", "Helvetica", "Arial", "Inter", "Liberation Sans", "DejaVu Sans"],
  serif: ["Times New Roman", "Georgia", "Liberation Serif", "DejaVu Serif"],
  monospace: ["Menlo", "SF Mono", "Consolas", "Liberation Mono", "DejaVu Sans Mono"],
  cursive: ["Comic Sans MS", "Apple Chancery"],
  fantasy: ["Papyrus", "Impact"],
  "system-ui": ["Helvetica Neue", "Segoe UI", "Roboto", "Arial"],
};

/** Split a CSS `font-family` list into bare family names, quotes and spacing removed. */
function familyList(family: string): string[] {
  return family
    .split(",")
    .map((f) => f.trim().replace(/^["']|["']$/g, ""))
    .filter(Boolean);
}

/** Every family worth trying for a label, generics expanded to the faces they usually mean. */
function candidateFamilies(family: string): string[] {
  const out: string[] = [];
  for (const name of familyList(family)) {
    const generic = GENERIC_FAMILIES[name.toLowerCase()];
    if (generic) out.push(...generic);
    else out.push(name);
  }
  return out.length ? out : GENERIC_FAMILIES["sans-serif"];
}

/** The cache key for a face: the family as authored plus the weight/style that pick a face in it. */
export function fontKey(info: Pick<TextInfo, "family" | "weight" | "style">): string {
  return `${info.family}|${info.weight}|${info.style}`.toLowerCase();
}

/** Is this label asking for a bold face? (`bold`, or a numeric weight of 600+.) */
function wantsBold(weight: string): boolean {
  const n = Number.parseInt(weight, 10);
  return Number.isFinite(n) ? n >= 600 : weight.trim().toLowerCase() === "bold";
}

function wantsItalic(style: string): boolean {
  const s = style.trim().toLowerCase();
  return s === "italic" || s === "oblique";
}

/** How well a face's style name ("Bold Italic") answers what the label asked for; higher is better. */
function faceScore(face: FontData, info: TextInfo): number {
  const style = face.style.toLowerCase();
  const bold = /bold|black|heavy|semibold/.test(style);
  const italic = /italic|oblique/.test(style);
  let score = 0;
  score += bold === wantsBold(info.weight) ? 2 : 0;
  score += italic === wantsItalic(info.style) ? 2 : 0;
  // Prefer a plain face over a condensed/display cut when nothing else separates them.
  if (/regular|book|roman/.test(style)) score += 1;
  return score;
}

/** Wrap raw font-file bytes, resolving which face of a `.ttc` collection `postscriptName` names. */
function faceOf(
  bytes: Uint8Array,
  postscriptName: string,
  label: string,
  family: string,
): LoadedFont {
  return { bytes, faceIndex: Editor.faceIndexFor(bytes, postscriptName), label, family };
}

/** One face of a font file, as the core reads its name table (`Editor.facesIn`). */
export type FaceInfo = {
  index: number;
  family: string;
  style: string;
  weight: number;
  italic: boolean;
};

/** How well a *file's* face answers what the label asked for; higher is better. */
function fileFaceScore(face: FaceInfo, info: TextInfo): number {
  const bold = face.weight >= 600;
  let score = 0;
  score += bold === wantsBold(info.weight) ? 2 : 0;
  score += face.italic === wantsItalic(info.style) ? 2 : 0;
  // Closest weight breaks a tie between two faces that are both "bold enough".
  const asked = Number.parseInt(info.weight, 10);
  if (Number.isFinite(asked)) score += Math.max(0, 1 - Math.abs(face.weight - asked) / 900);
  return score;
}

/**
 * The face in a file that best answers `info` — slant and weight, in that order of stubbornness.
 * `null` for a file with no readable faces at all.
 *
 * This is why a collection needs choosing at all: `Helvetica.ttc` holds regular, bold and italic
 * with nothing in the *file* to say which the label wants, so taking the first would outline a bold
 * heading in regular — a wrong face that reads as a shaping bug.
 */
export function bestFace(faces: FaceInfo[], info: TextInfo): FaceInfo | null {
  if (!faces.length) return null;
  return faces.reduce((a, b) => (fileFaceScore(b, info) > fileFaceScore(a, info) ? b : a));
}

/** The installed face that best matches `info`, or `null` — no API, no permission, no match. */
async function fromInstalledFonts(info: TextInfo): Promise<LoadedFont | null> {
  if (typeof window === "undefined" || !window.queryLocalFonts) return null;
  let faces: FontData[];
  try {
    faces = await window.queryLocalFonts();
  } catch {
    return null; // permission denied / dismissed — fall through to picking a file
  }
  for (const family of candidateFamilies(info.family)) {
    const matches = faces.filter((f) => f.family.toLowerCase() === family.toLowerCase());
    if (!matches.length) continue;
    const best = matches.reduce((a, b) => (faceScore(b, info) > faceScore(a, info) ? b : a));
    const bytes = new Uint8Array(await (await best.blob()).arrayBuffer());
    return faceOf(bytes, best.postscriptName, best.fullName, best.family);
  }
  return null;
}

/** What the picker came back with: a usable face, a file that can't be one, or a cancelled dialog. */
export type PickedFont = { font: LoadedFont } | { error: string } | null;

/**
 * Ask the user for a font file. The input lives in the DOM (hidden) rather than being created per
 * call so the picker is addressable — the same element the e2e drives.
 *
 * A collection (`.ttc`) holds several faces and the file itself says nothing about which one the
 * label wants, so the weight and slant it asked for pick between them; picking blindly outlines a
 * bold label in regular and looks like a shaping bug rather than a wrong face.
 */
export function pickFontFile(info: TextInfo): Promise<PickedFont> {
  return new Promise((resolve) => {
    const input = document.querySelector<HTMLInputElement>("input[data-font-picker]");
    if (!input) {
      resolve({ error: "the font picker isn't available in this view" });
      return;
    }
    const done = async () => {
      input.removeEventListener("change", done);
      input.removeEventListener("cancel", cancelled);
      const file = input.files?.[0];
      input.value = ""; // so picking the same file twice still fires `change`
      if (!file) {
        resolve(null);
        return;
      }
      const picked = new Uint8Array(await file.arrayBuffer());
      // A `.woff2` is a compressed face. The core decodes it wherever fonts arrive, but decoding
      // here means the cache holds a face that's ready to shape rather than one that decompresses
      // again on every conversion.
      const bytes = Editor.decodeWoff2(picked) ?? picked;
      const faces = (Editor.facesIn(bytes) as FaceInfo[] | undefined) ?? [];
      const best = bestFace(faces, info);
      if (!best) {
        resolve({
          error: `“${file.name}” isn't a font nib can read — pick a .ttf, .otf, .ttc or .woff2 face`,
        });
        return;
      }
      const label = [best.family, best.style].filter(Boolean).join(" ") || file.name;
      resolve({
        font: { bytes, faceIndex: best.index, label, family: best.family },
      });
    };
    const cancelled = () => {
      input.removeEventListener("change", done);
      input.removeEventListener("cancel", cancelled);
      resolve(null);
    };
    input.addEventListener("change", done);
    input.addEventListener("cancel", cancelled);
    input.click();
  });
}

/** Remember a face for this family, in memory and across reloads. */
export async function rememberFont(info: TextInfo, font: LoadedFont): Promise<void> {
  memory.set(fontKey(info), font);
  await idbPut(STORE, fontKey(info), font);
}

/** Is the Local Font Access API available at all? (Chromium only, and permission-gated.) */
export function canReadInstalledFonts(): boolean {
  return typeof window !== "undefined" && !!window.queryLocalFonts;
}

/**
 * A face for `info` from this session's cache — **synchronously**, which is the point.
 *
 * Opening a file dialog requires the user's gesture to still be in force, and Safari only counts
 * that within the gesture's own task: one `await` of real I/O (an IndexedDB read, say) and
 * `input.click()` is silently ignored. So the click path has to be able to answer "do I already
 * have this font?" without awaiting anything, and {@link warmFont} is what keeps the answer useful.
 */
export function cachedFont(info: TextInfo): LoadedFont | null {
  return memory.get(fontKey(info)) ?? null;
}

/**
 * Pull a stored face for `info` into the session cache, so a later click can find it
 * synchronously. Called when a label is selected — ahead of the gesture, not during it.
 */
export async function warmFont(info: TextInfo): Promise<void> {
  const key = fontKey(info);
  if (memory.has(key)) return;
  const stored = await idbGet<LoadedFont>(STORE, key);
  if (stored) memory.set(key, stored);
}

/**
 * Font bytes for `info` without asking the user anything: the session cache, the stored one, then
 * the installed fonts. `null` means nib can't answer on its own — the caller offers
 * {@link pickFontFile}, which needs a user gesture anyway.
 */
export async function findFont(info: TextInfo): Promise<LoadedFont | null> {
  const key = fontKey(info);
  const cached = memory.get(key) ?? (await idbGet<LoadedFont>(STORE, key));
  if (cached) {
    memory.set(key, cached);
    return cached;
  }
  const installed = await fromInstalledFonts(info);
  if (installed) await rememberFont(info, installed);
  return installed;
}
