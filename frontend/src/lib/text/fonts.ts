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
// Only raw `.ttf`/`.otf`/`.ttc` faces work: a `.woff2` is compressed, and decompressing it would
// cost another dependency in the wasm for a format that only exists to save bytes over the wire.

import { Editor } from "$lib/core";
import type { TextInfo } from "$lib/model/types";
import { idbGet, idbPut } from "$lib/persistence/idb";

const STORE = "fonts";

/** Font bytes ready to shape with, plus which face of a collection to use. */
export type LoadedFont = {
  bytes: Uint8Array;
  faceIndex: number;
  /** Human name for the face, for the "outlined with …" confirmation. */
  label: string;
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
function faceOf(bytes: Uint8Array, postscriptName: string, label: string): LoadedFont {
  return { bytes, faceIndex: Editor.faceIndexFor(bytes, postscriptName), label };
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
    return faceOf(bytes, best.postscriptName, best.fullName);
  }
  return null;
}

/**
 * Ask the user for a font file. The input lives in the DOM (hidden) rather than being created per
 * call so the picker is addressable — the same element the e2e drives.
 */
export function pickFontFile(): Promise<LoadedFont | null> {
  return new Promise((resolve) => {
    const input = document.querySelector<HTMLInputElement>("input[data-font-picker]");
    if (!input) {
      resolve(null);
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
      const bytes = new Uint8Array(await file.arrayBuffer());
      // A picked file identifies itself — take face 0 unless the name says otherwise.
      resolve(faceOf(bytes, "", file.name.replace(/\.[^.]+$/, "")));
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
