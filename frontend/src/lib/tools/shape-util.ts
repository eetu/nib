import { distance } from "$lib/model/geometry";
import type { Point } from "$lib/model/types";
import { collectAnchors, findSnap, snapToGrid } from "$lib/snap";
import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

/** A drag shorter than this (doc units) is treated as a stray click, not a shape. */
export const MIN_SHAPE = 0.5;

/** True when the pointer event asks to momentarily bypass snapping — ⌘ (Mac) / Ctrl (Win) held,
 *  the cross-editor convention (Figma/Sketch/Affinity/XD). Suspends BOTH anchor + grid snapping for
 *  the duration of the drag; Shift (axis/angle constrain) and Alt (duplicate) stay untouched. */
export function snapBypassed(event?: PointerEvent | null): boolean {
  return !!event && (event.metaKey || event.ctrlKey);
}

/** Snap a placement point to an existing anchor (when snapping is on) or the grid. `bypass` (⌘/Ctrl
 *  held) returns the raw point. */
export function snapPoint(p: Point, bypass = false): { point: Point; snapped: boolean } {
  if (bypass) return { point: p, snapped: false };
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
export function snapRadius(center: Point, cursor: Point, bypass = false): number {
  const raw = distance(center, cursor);
  if (!bypass && tools.gridEnabled && tools.gridSize > 0) {
    return Math.round(raw / tools.gridSize) * tools.gridSize;
  }
  return raw;
}
