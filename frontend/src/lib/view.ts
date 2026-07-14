import { subpathsBounds } from "$lib/model/geometry";
import type { ViewBox } from "$lib/model/types";
import { editor } from "$lib/stores/document.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

/** Union bounds of every non-deleted path's geometry (nodes + handles), or null if empty. */
function contentBounds(): { minX: number; minY: number; maxX: number; maxY: number } | null {
  const doc = editor.doc;
  if (!doc) return null;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let any = false;
  for (const p of doc.paths) {
    if (p.deleted) continue;
    const b = subpathsBounds(p.subpaths);
    if (!b) continue;
    any = true;
    minX = Math.min(minX, b.minX);
    minY = Math.min(minY, b.minY);
    maxX = Math.max(maxX, b.maxX);
    maxY = Math.max(maxY, b.maxY);
  }
  return any && maxX > minX && maxY > minY ? { minX, minY, maxX, maxY } : null;
}

/**
 * Frame the actual drawing (union of all path bounds), not the static viewBox — so a drawing
 * placed outside the declared viewport still centers. Falls back to the viewBox when there's
 * no geometry yet. Shared by the "0" shortcut and the rail's fit button.
 */
export function fitToView(): void {
  const doc = editor.doc;
  if (!doc) return;
  const b = contentBounds();
  const vb = b
    ? { minX: b.minX, minY: b.minY, width: b.maxX - b.minX, height: b.maxY - b.minY }
    : doc.viewBox;
  viewport.fitDocument(vb);
}

/**
 * The box the initial (on-load / reload) fit frames: the declared artboard **united with any
 * content that spills outside it** — so a shape drawn beyond the viewBox is never cut off on
 * load (the symptom that made reloads show only content near the origin). Also what export
 * uses so other apps see the whole drawing.
 */
export function loadViewBox(): ViewBox {
  const doc = editor.doc;
  const vb = doc?.viewBox ?? { minX: 0, minY: 0, width: 100, height: 100 };
  const b = contentBounds();
  if (!b) return vb;
  const minX = Math.min(vb.minX, b.minX);
  const minY = Math.min(vb.minY, b.minY);
  const maxX = Math.max(vb.minX + vb.width, b.maxX);
  const maxY = Math.max(vb.minY + vb.height, b.maxY);
  return { minX, minY, width: maxX - minX, height: maxY - minY };
}
