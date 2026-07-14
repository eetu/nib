import { distance } from "$lib/model/geometry";
import type { NodeRef, Point } from "$lib/model/types";
import { collectAnchors, findSnap, snapToGrid } from "$lib/snap";
import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import type { DragSession, Tool } from "./types";

const ENDPOINT_HIT_PX = 11;

// The subpath currently being drawn — persists across clicks until finished.
let drawing: { pathIndex: number; subpathIndex: number } | null = null;

/** Finish the current path, leaving it as drawn. */
export function finishPen(): void {
  drawing = null;
  interaction.penDrawing = false;
  interaction.penCursor = null;
  interaction.resumePoint = null;
}

type EndpointHit = { pathIndex: number; subpathIndex: number; atHead: boolean; point: Point };

/** The open-subpath endpoint under `docPoint` (within the hit radius), if any —
 *  the anchor the pen would resume drawing from. Endpoints only, open subpaths
 *  only; a lone-node subpath counts as a tail. Independent of the snap toggle. */
function endpointAt(docPoint: Point): EndpointHit | null {
  const doc = editor.doc;
  if (!doc) return null;
  const threshold = viewport.toDocLength(ENDPOINT_HIT_PX);
  let best: EndpointHit | null = null;
  let bestD = Infinity;
  doc.paths.forEach((path, pathIndex) => {
    if (path.deleted) return;
    path.subpaths.forEach((sp, subpathIndex) => {
      if (sp.closed || sp.nodes.length === 0) return;
      const last = sp.nodes.length - 1;
      const ends =
        last === 0
          ? [{ atHead: false, i: 0 }]
          : [
              { atHead: true, i: 0 },
              { atHead: false, i: last },
            ];
      for (const e of ends) {
        const d = distance(sp.nodes[e.i].point, docPoint);
        if (d <= threshold && d < bestD) {
          bestD = d;
          best = { pathIndex, subpathIndex, atHead: e.atHead, point: { ...sp.nodes[e.i].point } };
        }
      }
    });
  });
  return best;
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

/** Resuming an endpoint: a plain click just picks the path up (no history — the
 *  next click appends), but dragging from it shapes that endpoint's out-handle
 *  into a smooth continuation, like the pen does for a freshly-placed node. */
function resumeDrag(ref: NodeRef): DragSession {
  let moved = false;
  return {
    move(docPoint) {
      editor.setPenHandles(ref, docPoint);
      moved = true;
    },
    up() {
      if (moved) editor.commit();
    },
    cancel() {
      if (moved) editor.revert();
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
  onDeactivate: finishPen,
  hover(docPoint) {
    if (interaction.penDrawing) {
      // Rubber-band from the last-placed anchor to the cursor while drawing.
      interaction.penCursor = docPoint;
      return;
    }
    // Idle: cue the open endpoint the pen would pick up if clicked here.
    interaction.resumePoint = endpointAt(docPoint)?.point ?? null;
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

    // Not drawing: clicking an existing open endpoint resumes that path rather
    // than starting a new one. Reverse the subpath if we grabbed its head, so
    // appends (and the close-on-start check) work off the tail as usual. This
    // is a pick-up only — the next click places the next node.
    const resume = endpointAt(ctx.docPoint);
    if (resume) {
      const { pathIndex, subpathIndex } = resume;
      if (resume.atHead) editor.reverseSubpath(pathIndex, subpathIndex);
      drawing = { pathIndex, subpathIndex };
      interaction.penDrawing = true;
      interaction.resumePoint = null;
      const nodes = editor.doc.paths[pathIndex]?.subpaths[subpathIndex]?.nodes.length ?? 1;
      const tail = { pathIndex, subpathIndex, nodeIndex: nodes - 1 };
      editor.select(tail);
      return resumeDrag(tail);
    }

    const ref = editor.beginPath(point);
    drawing = { pathIndex: ref.pathIndex, subpathIndex: ref.subpathIndex };
    interaction.penDrawing = true;
    return penDrag(ref);
  },
};
