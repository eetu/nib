import { distance, normalize } from "$lib/model/geometry";
import type { Point } from "$lib/model/types";
import { collectAnchors, findSnap, snapToGrid } from "$lib/snap";
import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import type { Tool } from "./types";

// A drag shorter than this (doc units) is treated as a stray click, not a shape.
const MIN_RADIUS = 0.5;

/** Snap the centre to an existing anchor or the grid. `snapped` says whether a
 *  snap actually applied (drives the visual aid). */
function resolveCenter(p: Point): { point: Point; snapped: boolean } {
  const doc = editor.doc;
  if (doc && tools.snapEnabled) {
    const hit = findSnap(p, collectAnchors(doc, null), viewport.toDocLength(tools.snapThresholdPx));
    if (hit) return { point: hit.target.point, snapped: true };
  }
  if (tools.gridEnabled) return { point: snapToGrid(p, tools.gridSize), snapped: true };
  return { point: p, snapped: false };
}

/** Radius from the centre to the cursor, snapped to a grid multiple when the
 *  grid is on (so the circle's cardinal points land on grid lines). */
function resolveRadius(center: Point, cursor: Point): number {
  const raw = distance(center, cursor);
  if (tools.gridEnabled && tools.gridSize > 0) {
    return Math.round(raw / tools.gridSize) * tools.gridSize;
  }
  return raw;
}

/** Draw a circle: press at the centre, drag out to the radius. The result is a
 *  closed 4-node bezier path, editable like any other. */
export const circleTool: Tool = {
  id: "circle",
  cursor: () => "crosshair",
  hover(docPoint) {
    // Show where the centre will land (grid/anchor snap) before pressing.
    const { point, snapped } = resolveCenter(docPoint);
    interaction.snapPoint = snapped ? point : null;
    interaction.closing = false;
  },
  begin(ctx) {
    editor.ensureBlank();
    if (!editor.doc) return null;
    const center = resolveCenter(ctx.docPoint).point;
    const ref = editor.beginEllipse(center);
    let radius = 0;
    return {
      move(docPoint) {
        radius = resolveRadius(center, docPoint);
        editor.resizeEllipse(ref.pathIndex, ref.subpathIndex, center, radius, radius);
        // Mark where the edge snapped to, in the drag direction.
        if (tools.gridEnabled && radius > 0) {
          const dir = normalize({ x: docPoint.x - center.x, y: docPoint.y - center.y });
          interaction.snapPoint = { x: center.x + dir.x * radius, y: center.y + dir.y * radius };
        } else {
          interaction.snapPoint = null;
        }
      },
      up() {
        interaction.snapPoint = null;
        if (radius < MIN_RADIUS) editor.revert();
        else editor.commit();
      },
      cancel() {
        interaction.snapPoint = null;
        editor.revert();
      },
    };
  },
};
