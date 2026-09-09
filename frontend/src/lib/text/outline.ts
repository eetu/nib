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

import { findFont, type LoadedFont, pickFontFile, rememberFont } from "./fonts";

/** The family the document actually asked for first, quotes and generics included. */
function askedFamily(info: TextInfo): string {
  return (
    info.family
      .split(",")[0]
      ?.trim()
      .replace(/^["']|["']$/g, "") ?? ""
  );
}

/**
 * Say so when the face that did the shaping isn't the family the label named. The outlines are
 * correct geometry either way, but they are no longer the letterforms the document described, and
 * silence there reads as "nib mangled my text" the next time someone looks.
 */
function noticeSubstitution(info: TextInfo, font: LoadedFont): void {
  const asked = askedFamily(info);
  const generic = /^(sans-serif|serif|monospace|cursive|fantasy|system-ui|ui-[a-z-]+)$/i.test(
    asked,
  );
  const same = !!font.family && font.family.toLowerCase() === asked.toLowerCase();
  if (!asked || generic || same) return;
  workspace.notice = `outlined with ${font.label} — “${asked}” isn't available here, so the letterforms differ from the label`;
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
  if (!info) {
    workspace.error = "that label is built from tspans — outline it one run at a time";
    return false;
  }
  if (!info.text.trim()) {
    workspace.error = "nothing to outline — the label is empty";
    return false;
  }

  let font: LoadedFont | null = await findFont(info);
  if (!font && allowPrompt) {
    workspace.error = null;
    const picked = await pickFontFile(info);
    if (picked && "error" in picked) {
      workspace.error = picked.error;
      return false;
    }
    font = picked?.font ?? null;
    if (font) await rememberFont(info, font);
  }
  if (!font) {
    workspace.error = `no font found for “${info.family}” — pick a .ttf/.otf file to outline with`;
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
