// "Convert to outlines", end to end: find the font a label asks for, shape it in the core, and
// apply the one op that turns the `<text>` into a path. Lives here rather than in the document
// store because it's a *flow* — it can prompt, and it reports on the app's banners — while the
// store stays a thin facade over the engine.
//
// Destructive by design: the words stop being words. That's the point (a path can be node-edited,
// boolean'd, offset; a label can't), and undo puts them back.

import type { TextInfo } from "$lib/model/types";
import { editor } from "$lib/stores/document.svelte";
import { workspace } from "$lib/stores/workspace.svelte";

import {
  cachedFont,
  canReadInstalledFonts,
  findFont,
  type LoadedFont,
  type PickedFont,
  pickFontFile,
  rememberFont,
  warmFont,
} from "./fonts";

/** The family the document actually asked for first, quotes and generics included. */
function askedFamily(info: TextInfo): string {
  return (
    info.family
      .split(",")[0]
      ?.trim()
      .replace(/^["']|["']$/g, "") ?? ""
  );
}

const GENERIC = /^(sans-serif|serif|monospace|cursive|fantasy|system-ui|ui-[a-z-]+)$/i;

/** Families the label's own runs asked for that aren't the label's — a tspan with its own font. */
function foreignFamilies(info: TextInfo): string[] {
  const seen = new Set<string>();
  for (const run of info.runs ?? []) {
    const family = run.family.split(",")[0]?.trim() ?? "";
    if (!run.text.trim() || !family) continue;
    if (family.toLowerCase() === askedFamily(info).toLowerCase()) continue;
    seen.add(family);
  }
  return [...seen];
}

/**
 * Say so when the shaping didn't use the letterforms the document described — either because the
 * asked-for family isn't here, or because a `<tspan>` wanted a different one and a single font
 * shaped the whole label. The outlines are correct geometry either way; silence about it reads as
 * "nib mangled my text" the next time someone looks.
 */
function noticeSubstitution(info: TextInfo, font: LoadedFont): void {
  const asked = askedFamily(info);
  const substituted =
    !!asked && !GENERIC.test(asked) && font.family.toLowerCase() !== asked.toLowerCase();
  const foreign = foreignFamilies(info).filter((f) => !GENERIC.test(f));
  if (substituted && foreign.length)
    workspace.notice = `outlined with ${font.label} — “${asked}” isn't available here, and the runs asking for ${foreign.join(", ")} were shaped in it too`;
  else if (substituted)
    workspace.notice = `outlined with ${font.label} — “${asked}” isn't available here, so the letterforms differ from the label`;
  else if (foreign.length)
    workspace.notice = `outlined entirely in ${font.label} — one font shapes a label, so the runs asking for ${foreign.join(", ")} changed typeface`;
}

/**
 * Outline the `<text>` node `uid`. Returns whether it converted; failures land on the error banner
 * rather than throwing, since every caller is a click.
 *
 * `allowPrompt` gates asking the user for a font file — it needs a user gesture, so a batch pass
 * over many labels resolves silently and reports the ones it couldn't do.
 */
export async function outlineText(uid: string, allowPrompt = true): Promise<boolean> {
  const info = editor.textInfo(uid);
  if (!info || !info.text.trim()) {
    workspace.error = "nothing to outline — that isn't a label with words in it";
    return false;
  }

  // Everything up to opening the file dialog stays synchronous, because the dialog needs the
  // click's user activation and Safari only honours that inside the gesture's own task — one
  // awaited IndexedDB read and `input.click()` is ignored with no error, which looks exactly like
  // a dead button. So: the session cache is consulted synchronously (warmFontFor fills it when a
  // label is selected), and where there's no Local Font Access API to ask, the picker opens right
  // here, before anything is awaited.
  let font: LoadedFont | null = cachedFont(info);
  let pending: Promise<PickedFont> | null = null;
  if (!font && allowPrompt && !canReadInstalledFonts()) pending = pickFontFile(info);

  // With the API available, asking it first is worth the await: it answers without a dialog, and
  // Chromium — the only engine that has it — keeps activation across the call.
  if (!font && !pending) font = await findFont(info);
  if (!font && allowPrompt && !pending) pending = pickFontFile(info);

  if (pending) {
    workspace.error = null;
    const picked = await pending;
    if (picked && "error" in picked) {
      workspace.error = picked.error;
      return false;
    }
    font = picked?.font ?? null;
    if (font) await rememberFont(info, font);
  }
  if (!font) {
    workspace.error = `no font found for “${info.family}” — pick a font file to outline with`;
    return false;
  }

  if (!editor.outlineText(uid, font.bytes, font.faceIndex)) {
    workspace.error = `“${font.label}” didn't shape that label — try a different face`;
    return false;
  }
  noticeSubstitution(info, font);
  return true;
}

/** Whether `uid` names a label this can convert — what the UI gates its button on. */
export function canOutlineText(uid: string | null): boolean {
  return !!uid && !!editor.textInfo(uid);
}

/**
 * Pull the font this label would use into the session cache, so the convert click can find it
 * without awaiting — see the activation note in {@link outlineText}. Call it when a label is
 * selected; it prompts for nothing and is safe to fire and forget.
 */
export function warmFontFor(uid: string | null): void {
  if (!uid) return;
  const info = editor.textInfo(uid);
  if (info) void warmFont(info);
}

/**
 * Outline every label in the document — the pass that makes a drawing font-independent before it
 * leaves nib. Each family is asked for once (the resolver caches), and one cancelled prompt stops
 * the asking for the rest of the run rather than nagging per label.
 *
 * Each conversion is its own undo step, so a batch undoes one label at a time.
 */
export async function outlineAllText(): Promise<{ done: number; skipped: number }> {
  const labels = editor.textInfos();
  let done = 0;
  let skipped = 0;
  let allowPrompt = true;
  for (const label of labels) {
    if (await outlineText(label.uid, allowPrompt)) done++;
    else {
      skipped++;
      allowPrompt = false;
    }
  }
  if (skipped)
    workspace.error = `outlined ${done} of ${labels.length} labels — ${skipped} had no usable font`;
  return { done, skipped };
}
