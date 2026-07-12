import type { NodeRef, Point } from "$lib/model/types";
import { collectAnchors, findSnap, snapToGrid } from "$lib/snap";
import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import type { DragSession, Tool } from "./types";

// The subpath currently being drawn — persists across clicks until finished.
let drawing: { pathIndex: number; subpathIndex: number } | null = null;

/** Finish the current path, leaving it as drawn. */
export function finishPen(): void {
  drawing = null;
  interaction.penDrawing = false;
  interaction.penCursor = null;
}

function penDrag(ref: NodeRef): DragSession {
  return {
    move(docPoint) {
      // Dragging the just-placed anchor shapes it into a smooth curve.
      editor.setPenHandles(ref, docPoint);
    },
    up() {
      editor.commit();
    },
    cancel() {
      editor.revert();
      finishPen();
    },
  };
}

/** Snap the placement point to an existing anchor (returning its ref, so a
 *  click on the start node can close the loop) or to the grid. */
function penPoint(docPoint: Point): { point: Point; snapRef: NodeRef | null } {
  const doc = editor.doc;
  if (!doc) return { point: docPoint, snapRef: null };
  if (tools.snapEnabled) {
    const threshold = viewport.toDocLength(tools.snapThresholdPx);
    const hit = findSnap(docPoint, collectAnchors(doc, editor.selection), threshold);
    if (hit) return { point: hit.target.point, snapRef: hit.target.ref };
  }
  if (tools.gridEnabled) return { point: snapToGrid(docPoint, tools.gridSize), snapRef: null };
  return { point: docPoint, snapRef: null };
}

export const penTool: Tool = {
  id: "pen",
  cursor: () => "crosshair",
  hover(docPoint) {
    // Rubber-band from the last-placed anchor to the cursor while drawing.
    if (interaction.penDrawing) interaction.penCursor = docPoint;
  },
  begin(ctx) {
    editor.ensureBlank();
    if (!editor.doc) return null;
    const { point, snapRef } = penPoint(ctx.docPoint);
    interaction.clearDrag();

    if (drawing) {
      const d = drawing;
      // Clicking the subpath's own start node closes the loop and finishes.
      const onStart =
        snapRef?.pathIndex === d.pathIndex &&
        snapRef?.subpathIndex === d.subpathIndex &&
        snapRef?.nodeIndex === 0;
      if (onStart) {
        editor.closePath(d.pathIndex, d.subpathIndex);
        finishPen();
        return null;
      }
      const ref = editor.appendNode(d.pathIndex, d.subpathIndex, point);
      return penDrag(ref);
    }

    const ref = editor.beginPath(point);
    drawing = { pathIndex: ref.pathIndex, subpathIndex: ref.subpathIndex };
    interaction.penDrawing = true;
    return penDrag(ref);
  },
};
