import { subpathsBounds } from "$lib/model/geometry";
import { editor } from "$lib/stores/document.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

/**
 * Frame the actual drawing (union of all path bounds), not the static viewBox — so a drawing
 * placed outside the declared viewport still centers. Falls back to the viewBox when there's
 * no geometry yet. Shared by the "0" shortcut and the rail's fit button.
 */
export function fitToView(): void {
  const doc = editor.doc;
  if (!doc) return;
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
  const vb =
    any && maxX > minX && maxY > minY
      ? { minX, minY, width: maxX - minX, height: maxY - minY }
      : doc.viewBox;
  viewport.fitDocument(vb);
}
