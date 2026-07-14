import { cubicAt, distance, subpathsBounds } from "$lib/model/geometry";
import { nearestOnSubpath, segmentControlPoints } from "$lib/model/path";
import type { NodeRef, Point, Subpath } from "$lib/model/types";
import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import { handlePoints, padBounds, ROTATE_KNOB_PX, SELECT_PAD_PX } from "./transform";
import type { Hit } from "./types";

const ANCHOR_HIT_PX = 11;
const HANDLE_HIT_PX = 11;
const SEGMENT_HIT_PX = 8;
const FLATTEN_STEPS = 12;

function screenDist(docPt: Point, screen: Point): number {
  return distance(viewport.toScreen(docPt), screen);
}

/** Sample a subpath's outline into a polyline (open subpaths close implicitly for fill). */
function flattenSubpath(sp: Subpath): Point[] {
  const n = sp.nodes.length;
  if (n < 2) return [];
  const pts: Point[] = [];
  const segs = sp.closed ? n : n - 1;
  for (let i = 0; i < segs; i++) {
    const [p0, p1, p2, p3] = segmentControlPoints(sp, i);
    for (let s = 0; s < FLATTEN_STEPS; s++) pts.push(cubicAt(p0, p1, p2, p3, s / FLATTEN_STEPS));
  }
  return pts;
}

/** Is `pt` inside the path's filled area? Nonzero-winding ray cast over the flattened
 *  subpaths (matches SVG's default fill-rule), so clicking a shape's body selects it. */
function pointInPath(subpaths: Subpath[], pt: Point): boolean {
  let winding = 0;
  for (const sp of subpaths) {
    const poly = flattenSubpath(sp);
    const m = poly.length;
    if (m < 3) continue;
    for (let i = 0; i < m; i++) {
      const a = poly[i];
      const b = poly[(i + 1) % m];
      const c = (b.x - a.x) * (pt.y - a.y) - (b.y - a.y) * (pt.x - a.x);
      if (a.y <= pt.y) {
        if (b.y > pt.y && c > 0) winding++;
      } else if (b.y <= pt.y && c < 0) {
        winding--;
      }
    }
  }
  return winding !== 0;
}

/**
 * What sits under the pointer, in priority order: a handle of the selected
 * node, then any anchor, then the nearest path segment, else empty. Distances
 * are measured in screen pixels so hit radii stay constant at any zoom.
 */
export function hitTest(screen: Point): Hit {
  const doc = editor.doc;
  if (!doc) return { kind: "empty" };

  // 1+2. Handles + anchors are hit-testable only while node-editing: any non-select tool
  //       (add/delete-node, pen), or the select tool after a double-click enters node mode.
  //       Otherwise the select tool's drag always moves the whole shape — no ambiguity over
  //       whether a node or the shape moves, which matters most when zoomed out and anchors
  //       cluster. Transform handles (step 3) cover object-mode resize/rotate instead.
  const nodeEditable = tools.active !== "select" || editor.nodeEditIndex !== null;
  if (nodeEditable) {
    // Handles — only the selected node exposes its control handles.
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

    // Anchors — nearest within the hit radius. Checked before transform handles so a path's
    // own nodes are never shadowed (e.g. a circle's nodes sit on the bbox edge-midpoints).
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
  }

  // 3. Transform handles — only for an object (whole-path) selection, at
  //    corners/edges not occupied by a node.
  if (editor.objectSelected && editor.selectedPathIndex !== null) {
    const p = doc.paths[editor.selectedPathIndex];
    if (p && !p.deleted) {
      const raw = subpathsBounds(p.subpaths);
      if (raw) {
        const bb = padBounds(raw, viewport.toDocLength(SELECT_PAD_PX));
        // Rotate knob, above the box's top-centre.
        const top = viewport.toScreen({ x: (bb.minX + bb.maxX) / 2, y: bb.minY });
        if (distance({ x: top.x, y: top.y - ROTATE_KNOB_PX }, screen) <= HANDLE_HIT_PX) {
          return { kind: "rotate" };
        }
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

  // 5. Fill — clicking inside a filled path's body selects it (front-most first). Skips
  //    stroke-only paths (fill="none"); an absent fill counts as filled (SVG default).
  for (let pathIndex = doc.paths.length - 1; pathIndex >= 0; pathIndex--) {
    const p = doc.paths[pathIndex];
    if (p.deleted) continue;
    const fill = p.styleOverride?.fill ?? p.attributes?.fill;
    if (fill === "none") continue;
    if (pointInPath(p.subpaths, docPoint)) return { kind: "fill", pathIndex };
  }

  return { kind: "empty" };
}
