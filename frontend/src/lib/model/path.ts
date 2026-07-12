import { SVGPathData } from "svg-pathdata";

import { cubicAt, distance, handlesCollinear, lerp, splitCubic } from "./geometry";
import type { PathNode, Point, Subpath } from "./types";

// After this pipeline the command stream is absolute and contains only
// M / L / H / V / C / Z — every curve is a cubic, so one code path handles all.
function normalizedCommands(d: string) {
  return new SVGPathData(d).toAbs().normalizeST().qtToC().aToC().commands;
}

const EPS = 1e-4;

function pushLine(sp: Subpath, point: Point): void {
  // Straight segment: the previous node's handleOut and this node's handleIn
  // both stay absent.
  sp.nodes.push({ point, type: "corner" });
}

/** Parse a path `d` string into editable subpaths of cubic anchor nodes. */
export function parsePathD(d: string): Subpath[] {
  const commands = normalizedCommands(d);
  const subpaths: Subpath[] = [];
  let current: Subpath | null = null;
  let cur: Point = { x: 0, y: 0 };
  let start: Point = { x: 0, y: 0 };

  for (const c of commands) {
    switch (c.type) {
      case SVGPathData.MOVE_TO: {
        cur = { x: c.x, y: c.y };
        start = cur;
        current = { nodes: [{ point: cur, type: "corner" }], closed: false };
        subpaths.push(current);
        break;
      }
      case SVGPathData.LINE_TO: {
        cur = { x: c.x, y: c.y };
        if (current) pushLine(current, cur);
        break;
      }
      case SVGPathData.HORIZ_LINE_TO: {
        cur = { x: c.x, y: cur.y };
        if (current) pushLine(current, cur);
        break;
      }
      case SVGPathData.VERT_LINE_TO: {
        cur = { x: cur.x, y: c.y };
        if (current) pushLine(current, cur);
        break;
      }
      case SVGPathData.CURVE_TO: {
        if (current) {
          const prev = current.nodes[current.nodes.length - 1];
          prev.handleOut = { x: c.x1, y: c.y1 };
          current.nodes.push({
            point: { x: c.x, y: c.y },
            handleIn: { x: c.x2, y: c.y2 },
            type: "corner",
          });
        }
        cur = { x: c.x, y: c.y };
        break;
      }
      case SVGPathData.CLOSE_PATH: {
        if (current) current.closed = true;
        cur = start;
        break;
      }
      default:
        // Unreachable after normalization, but keep the walker total.
        break;
    }
  }

  for (const sp of subpaths) foldClosingNode(sp);
  for (const sp of subpaths) inferNodeTypes(sp);
  return subpaths;
}

/**
 * When a closed subpath's last node lands on its first node (a curve that ended
 * exactly at the start before Z), fold that trailing node's incoming handle
 * onto the first node and drop it — leaving a clean cyclic model where the
 * closing segment is last→first.
 */
function foldClosingNode(sp: Subpath): void {
  if (!sp.closed || sp.nodes.length < 2) return;
  const first = sp.nodes[0];
  const last = sp.nodes[sp.nodes.length - 1];
  if (distance(first.point, last.point) <= EPS) {
    if (last.handleIn) first.handleIn = last.handleIn;
    sp.nodes.pop();
  }
}

/**
 * Mark a subpath closed. Its nodes are kept as-is (the closing segment runs
 * from the last node back to the first, emitting a trailing Z); a last node
 * that already coincides with the first is folded away so there's no
 * zero-length seam. Merging a dragged endpoint onto the start (close-by-snap)
 * is the caller's job — see the document store's closeLoop.
 */
export function closeSubpath(sp: Subpath): void {
  if (sp.nodes.length < 2) return;
  sp.closed = true;
  foldClosingNode(sp);
}

function inferNodeTypes(sp: Subpath): void {
  for (const node of sp.nodes) {
    if (node.handleIn && node.handleOut) {
      node.type = handlesCollinear(node.handleIn, node.point, node.handleOut) ? "smooth" : "corner";
    }
  }
}

function fmt(v: number, precision: number): string {
  return String(Number(v.toFixed(precision)));
}

function pt(p: Point, precision: number): string {
  return `${fmt(p.x, precision)} ${fmt(p.y, precision)}`;
}

/** A segment a→b: a line iff both adjoining handles are absent, else a cubic
 *  (a missing control defaults to its own anchor, i.e. a "half-straight"). */
function segment(a: PathNode, b: PathNode, precision: number): string {
  if (!a.handleOut && !b.handleIn) {
    return `L ${pt(b.point, precision)}`;
  }
  const c1 = a.handleOut ?? a.point;
  const c2 = b.handleIn ?? b.point;
  return `C ${pt(c1, precision)} ${pt(c2, precision)} ${pt(b.point, precision)}`;
}

/** The four cubic control points of the segment leaving node `i`
 *  (wrapping to node 0 for the closing segment of a closed subpath). */
export function segmentControlPoints(sp: Subpath, i: number): [Point, Point, Point, Point] {
  const n = sp.nodes.length;
  const a = sp.nodes[i];
  const b = sp.nodes[(i + 1) % n];
  return [a.point, a.handleOut ?? a.point, b.handleIn ?? b.point, b.point];
}

export type SegmentHit = {
  segmentIndex: number;
  t: number;
  point: Point;
  distance: number;
};

/** Nearest point on a subpath's outline to `target`. A coarse per-segment sample
 *  picks the winning segment + rough parameter, then a local ternary refinement
 *  pins the true nearest point — so the reported distance is accurate at any
 *  zoom (a coarse-only scan can overshoot the hit threshold on a click that is
 *  genuinely on the curve). segmentIndex is the node the segment leaves. */
export function nearestOnSubpath(sp: Subpath, target: Point, samples = 32): SegmentHit | null {
  const n = sp.nodes.length;
  if (n < 2) return null;
  const lastSeg = sp.closed ? n - 1 : n - 2;
  let bestSeg = -1;
  let bestT = 0;
  let bestD = Infinity;
  for (let i = 0; i <= lastSeg; i++) {
    const [p0, p1, p2, p3] = segmentControlPoints(sp, i);
    for (let s = 0; s <= samples; s++) {
      const t = s / samples;
      const d = distance(cubicAt(p0, p1, p2, p3, t), target);
      if (d < bestD) {
        bestD = d;
        bestSeg = i;
        bestT = t;
      }
    }
  }
  if (bestSeg < 0) return null;

  const [q0, q1, q2, q3] = segmentControlPoints(sp, bestSeg);
  let lo = Math.max(0, bestT - 1 / samples);
  let hi = Math.min(1, bestT + 1 / samples);
  for (let iter = 0; iter < 24; iter++) {
    const m1 = lo + (hi - lo) / 3;
    const m2 = hi - (hi - lo) / 3;
    const d1 = distance(cubicAt(q0, q1, q2, q3, m1), target);
    const d2 = distance(cubicAt(q0, q1, q2, q3, m2), target);
    if (d1 < d2) hi = m2;
    else lo = m1;
  }
  const t = (lo + hi) / 2;
  const point = cubicAt(q0, q1, q2, q3, t);
  return { segmentIndex: bestSeg, t, point, distance: distance(point, target) };
}

/** Insert a node on the segment leaving node `i` at parameter `t`, preserving
 *  the curve's shape (de Casteljau split; a straight segment stays straight). */
export function insertNodeAt(sp: Subpath, i: number, t: number): number {
  const n = sp.nodes.length;
  const a = sp.nodes[i];
  const b = sp.nodes[(i + 1) % n];
  if (!a.handleOut && !b.handleIn) {
    const point = lerp(a.point, b.point, t);
    sp.nodes.splice(i + 1, 0, { point, type: "corner" });
    return i + 1;
  }
  const [p0, p1, p2, p3] = segmentControlPoints(sp, i);
  const { left, right, point } = splitCubic(p0, p1, p2, p3, t);
  if (a.handleOut) a.handleOut = left[1];
  if (b.handleIn) b.handleIn = right[2];
  const newNode: PathNode = {
    point,
    handleIn: left[2],
    handleOut: right[1],
    type: "smooth",
  };
  sp.nodes.splice(i + 1, 0, newNode);
  return i + 1;
}

/** Serialize subpaths back to a compact absolute `d` string. */
export function pathToD(subpaths: Subpath[], precision = 3): string {
  const parts: string[] = [];
  for (const sp of subpaths) {
    if (sp.nodes.length === 0) continue;
    const n = sp.nodes;
    parts.push(`M ${pt(n[0].point, precision)}`);
    for (let i = 1; i < n.length; i++) {
      parts.push(segment(n[i - 1], n[i], precision));
    }
    if (sp.closed && n.length >= 2) {
      const last = n[n.length - 1];
      const first = n[0];
      // Emit an explicit closing curve only when the seam actually curves;
      // a straight close is just Z (implicit line back to the start).
      if (last.handleOut || first.handleIn) {
        parts.push(segment(last, first, precision));
      }
      parts.push("Z");
    }
  }
  return parts.join(" ");
}
