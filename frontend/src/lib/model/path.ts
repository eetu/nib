import { cubicAt, distance } from "./geometry";
import type { PathNode, Point, Subpath } from "./types";

// Client-side render + hit-test helpers over the document contract. The authoritative
// parse/serialize/edit logic lives in the Rust core (core/src/model/path.rs); these are the
// pure functions the canvas + tools need locally: serialize a subpath to a `d` string for
// rendering, and find the nearest point on an outline for hit-testing.

function fmt(v: number, precision: number): string {
  return String(Number(v.toFixed(precision)));
}

function pt(p: Point, precision: number): string {
  return `${fmt(p.x, precision)} ${fmt(p.y, precision)}`;
}

/** A segment a→b: a line iff both adjoining handles are absent, else a cubic (a missing
 *  control defaults to its own anchor, i.e. a "half-straight"). */
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

/** Nearest point on a subpath's outline to `target`. A coarse per-segment sample picks the
 *  winning segment + rough parameter, then a local ternary refinement pins the true nearest
 *  point — accurate at any zoom. segmentIndex is the node the segment leaves. */
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

/** Serialize subpaths to a compact absolute `d` string for rendering. Mirrors the core's
 *  serializer (core/src/model/path.rs) so on-canvas geometry matches the exported `d`. */
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
      if (last.handleOut || first.handleIn) {
        parts.push(segment(last, first, precision));
      }
      parts.push("Z");
    }
  }
  return parts.join(" ");
}
