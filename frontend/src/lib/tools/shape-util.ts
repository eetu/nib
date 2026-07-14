import { distance } from "$lib/model/geometry";
import type { Point } from "$lib/model/types";
import { collectAnchors, findSnap, snapToGrid } from "$lib/snap";
import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

/** A drag shorter than this (doc units) is treated as a stray click, not a shape. */
export const MIN_SHAPE = 0.5;

/** Snap a placement point to an existing anchor (when snapping is on) or the grid. */
export function snapPoint(p: Point): { point: Point; snapped: boolean } {
  const doc = editor.doc;
  if (doc && tools.snapEnabled) {
    const hit = findSnap(p, collectAnchors(doc, null), viewport.toDocLength(tools.snapThresholdPx));
    if (hit) return { point: hit.target.point, snapped: true };
  }
  if (tools.gridEnabled) return { point: snapToGrid(p, tools.gridSize), snapped: true };
  return { point: p, snapped: false };
}

/** Distance from centre to cursor, snapped to a grid multiple when the grid is on (so a
 *  radius-drag shape's cardinal points land on grid lines). */
export function snapRadius(center: Point, cursor: Point): number {
  const raw = distance(center, cursor);
  if (tools.gridEnabled && tools.gridSize > 0) {
    return Math.round(raw / tools.gridSize) * tools.gridSize;
  }
  return raw;
}
