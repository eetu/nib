import { distance, subpathsBounds } from "$lib/model/geometry";
import { nearestOnSubpath } from "$lib/model/path";
import type { NodeRef, Point } from "$lib/model/types";
import { editor } from "$lib/stores/document.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import { handlePoints, padBounds, SELECT_PAD_PX } from "./transform";
import type { Hit } from "./types";

const ANCHOR_HIT_PX = 11;
const HANDLE_HIT_PX = 11;
const SEGMENT_HIT_PX = 8;

function screenDist(docPt: Point, screen: Point): number {
  return distance(viewport.toScreen(docPt), screen);
}

/**
 * What sits under the pointer, in priority order: a handle of the selected
 * node, then any anchor, then the nearest path segment, else empty. Distances
 * are measured in screen pixels so hit radii stay constant at any zoom.
 */
export function hitTest(screen: Point): Hit {
  const doc = editor.doc;
  if (!doc) return { kind: "empty" };

  // 1. Handles — only the selected node exposes its control handles.
  const sel = editor.selection;
  if (sel) {
    const node = doc.paths[sel.pathIndex]?.subpaths[sel.subpathIndex]?.nodes[sel.nodeIndex];
    if (node) {
      if (node.handleOut && screenDist(node.handleOut, screen) <= HANDLE_HIT_PX) {
        return { kind: "handle", ref: sel, which: "out" };
      }
      if (node.handleIn && screenDist(node.handleIn, screen) <= HANDLE_HIT_PX) {
        return { kind: "handle", ref: sel, which: "in" };
      }
    }
  }

  // 2. Anchors — nearest within the hit radius. Checked before transform
  //    handles so a path's own nodes are never shadowed (e.g. a circle's nodes
  //    sit on the bbox edge-midpoints).
  let bestAnchor: { ref: NodeRef; d: number } | null = null;
  doc.paths.forEach((path, pathIndex) => {
    if (path.deleted) return;
    path.subpaths.forEach((sp, subpathIndex) => {
      sp.nodes.forEach((n, nodeIndex) => {
        const d = screenDist(n.point, screen);
        if (d <= ANCHOR_HIT_PX && (!bestAnchor || d < bestAnchor.d)) {
          bestAnchor = { ref: { pathIndex, subpathIndex, nodeIndex }, d };
        }
      });
    });
  });
  if (bestAnchor) return { kind: "anchor", ref: (bestAnchor as { ref: NodeRef }).ref };

  // 3. Transform handles — only for an object (whole-path) selection, at
  //    corners/edges not occupied by a node.
  if (editor.objectSelected && editor.selectedPath !== null) {
    const p = doc.paths[editor.selectedPath];
    if (p && !p.deleted) {
      const raw = subpathsBounds(p.subpaths);
      if (raw) {
        const bb = padBounds(raw, viewport.toDocLength(SELECT_PAD_PX));
        for (const { handle, point } of handlePoints(bb)) {
          if (screenDist(point, screen) <= HANDLE_HIT_PX) return { kind: "transform", handle };
        }
      }
    }
  }

  // 4. Segment — nearest outline point within the hit radius (for add-node).
  const docPoint = viewport.toDoc(screen);
  const threshDoc = viewport.toDocLength(SEGMENT_HIT_PX);
  let best: Hit | null = null;
  let bestD = Infinity;
  doc.paths.forEach((path, pathIndex) => {
    if (path.deleted) return;
    path.subpaths.forEach((sp, subpathIndex) => {
      const hit = nearestOnSubpath(sp, docPoint);
      if (hit && hit.distance <= threshDoc && hit.distance < bestD) {
        bestD = hit.distance;
        best = {
          kind: "segment",
          pathIndex,
          subpathIndex,
          segmentIndex: hit.segmentIndex,
          t: hit.t,
          point: hit.point,
        };
      }
    });
  });
  if (best) return best;

  return { kind: "empty" };
}
