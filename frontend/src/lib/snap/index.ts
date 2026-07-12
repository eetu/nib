import { distance } from "$lib/model/geometry";
import type { NodeRef, Point, SvgDocument } from "$lib/model/types";
import { nodeRefEquals } from "$lib/model/types";

/** A snappable anchor point plus the node it belongs to. */
export type SnapPoint = {
  point: Point;
  ref: NodeRef;
  /** True when this node is the first or last node of an *open* subpath — the
   *  candidates that matter for closing a loop. */
  endpoint: boolean;
};

export type SnapResult = {
  target: SnapPoint;
  distance: number;
};

/** Gather every anchor point in the document as a snap candidate, optionally
 *  excluding one node (the one being dragged). */
export function collectAnchors(doc: SvgDocument, exclude?: NodeRef | null): SnapPoint[] {
  const out: SnapPoint[] = [];
  doc.paths.forEach((path, pathIndex) => {
    if (path.deleted) return;
    path.subpaths.forEach((sp, subpathIndex) => {
      sp.nodes.forEach((node, nodeIndex) => {
        const ref: NodeRef = { pathIndex, subpathIndex, nodeIndex };
        if (exclude && nodeRefEquals(ref, exclude)) return;
        const endpoint = !sp.closed && (nodeIndex === 0 || nodeIndex === sp.nodes.length - 1);
        out.push({ point: { ...node.point }, ref, endpoint });
      });
    });
  });
  return out;
}

/** Nearest candidate within `threshold` (document units), or null. */
export function findSnap(
  from: Point,
  candidates: SnapPoint[],
  threshold: number,
): SnapResult | null {
  let best: SnapResult | null = null;
  for (const c of candidates) {
    const d = distance(from, c.point);
    if (d <= threshold && (!best || d < best.distance)) {
      best = { target: c, distance: d };
    }
  }
  return best;
}

/** Would dragging `dragged` (an endpoint of an open subpath) onto `target`
 *  close that subpath's loop? True when target is the *opposite* endpoint of
 *  the same open subpath. */
export function isCloseLoop(dragged: NodeRef, target: SnapPoint, doc: SvgDocument): boolean {
  if (!target.endpoint) return false;
  if (dragged.pathIndex !== target.ref.pathIndex) return false;
  if (dragged.subpathIndex !== target.ref.subpathIndex) return false;
  const sp = doc.paths[dragged.pathIndex]?.subpaths[dragged.subpathIndex];
  if (!sp || sp.closed || sp.nodes.length < 2) return false;
  const last = sp.nodes.length - 1;
  const draggedIsEnd = dragged.nodeIndex === 0 || dragged.nodeIndex === last;
  const targetIsOther = target.ref.nodeIndex !== dragged.nodeIndex;
  return draggedIsEnd && targetIsOther;
}

/** Snap a point to the nearest grid intersection. */
export function snapToGrid(p: Point, grid: number): Point {
  if (grid <= 0) return p;
  return { x: Math.round(p.x / grid) * grid, y: Math.round(p.y / grid) * grid };
}
