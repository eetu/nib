import type { Point, Subpath } from "./types";

export function add(a: Point, b: Point): Point {
  return { x: a.x + b.x, y: a.y + b.y };
}

/** Bounding box (doc units) of subpaths, from nodes + handles — a valid bound
 *  for a selection box (bezier curves stay within their control points). */
export function subpathsBounds(
  subpaths: Subpath[],
): { minX: number; minY: number; maxX: number; maxY: number } | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let any = false;
  for (const sp of subpaths) {
    for (const n of sp.nodes) {
      for (const p of [n.point, n.handleIn, n.handleOut]) {
        if (!p) continue;
        any = true;
        minX = Math.min(minX, p.x);
        minY = Math.min(minY, p.y);
        maxX = Math.max(maxX, p.x);
        maxY = Math.max(maxY, p.y);
      }
    }
  }
  return any ? { minX, minY, maxX, maxY } : null;
}

export function sub(a: Point, b: Point): Point {
  return { x: a.x - b.x, y: a.y - b.y };
}

export function scale(a: Point, k: number): Point {
  return { x: a.x * k, y: a.y * k };
}

export function lerp(a: Point, b: Point, t: number): Point {
  return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
}

export function distance(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

export function length(v: Point): number {
  return Math.hypot(v.x, v.y);
}

/** Unit vector in the direction of v (zero vector maps to {0,0}). */
export function normalize(v: Point): Point {
  const l = length(v);
  return l < 1e-9 ? { x: 0, y: 0 } : { x: v.x / l, y: v.y / l };
}

/** Are the incoming/outgoing handles of a node collinear through the point
 *  (i.e. the node reads as smooth)? Tolerant of handle length. */
export function handlesCollinear(
  handleIn: Point,
  point: Point,
  handleOut: Point,
  epsDeg = 3,
): boolean {
  const a = sub(point, handleIn); // direction into the point
  const b = sub(handleOut, point); // direction out of the point
  const la = length(a);
  const lb = length(b);
  if (la < 1e-6 || lb < 1e-6) return false;
  const cross = a.x * b.y - a.y * b.x;
  const sin = Math.abs(cross) / (la * lb);
  return sin <= Math.sin((epsDeg * Math.PI) / 180);
}

/**
 * Split a cubic bezier (p0..p3) at parameter t via de Casteljau, returning the
 * two sub-curves' control points. Used to insert a node on a segment without
 * changing the curve's shape.
 */
export function splitCubic(
  p0: Point,
  p1: Point,
  p2: Point,
  p3: Point,
  t: number,
): {
  left: [Point, Point, Point, Point];
  right: [Point, Point, Point, Point];
  point: Point;
} {
  const a = lerp(p0, p1, t);
  const b = lerp(p1, p2, t);
  const c = lerp(p2, p3, t);
  const d = lerp(a, b, t);
  const e = lerp(b, c, t);
  const f = lerp(d, e, t); // the point on the curve at t
  return {
    left: [p0, a, d, f],
    right: [f, e, c, p3],
    point: f,
  };
}

/** Evaluate a cubic bezier at t. */
export function cubicAt(p0: Point, p1: Point, p2: Point, p3: Point, t: number): Point {
  const u = 1 - t;
  const w0 = u * u * u;
  const w1 = 3 * u * u * t;
  const w2 = 3 * u * t * t;
  const w3 = t * t * t;
  return {
    x: w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
    y: w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
  };
}

/** Interior t-values (0,1) where a cubic's derivative is zero on one axis (its extrema). */
function axisExtrema(p0: number, p1: number, p2: number, p3: number): number[] {
  const a = p1 - p0;
  const b = p2 - p1;
  const c = p3 - p2;
  const qa = a - 2 * b + c;
  const qb = 2 * (b - a);
  const qc = a;
  const ts: number[] = [];
  const push = (t: number) => {
    if (t > 1e-6 && t < 1 - 1e-6) ts.push(t);
  };
  if (Math.abs(qa) < 1e-9) {
    if (Math.abs(qb) > 1e-9) push(-qc / qb);
  } else {
    const disc = qb * qb - 4 * qa * qc;
    if (disc >= 0) {
      const s = Math.sqrt(disc);
      push((-qb + s) / (2 * qa));
      push((-qb - s) / (2 * qa));
    }
  }
  return ts;
}

/** Tight bounding box of the *rendered* curves (anchors + bezier extrema), unlike
 *  {@link subpathsBounds} which uses the control-point hull — a long handle can balloon the
 *  hull far past the visible curve, so use this for the selection/transform box. */
export function tightBounds(
  subpaths: Subpath[],
): { minX: number; minY: number; maxX: number; maxY: number } | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let any = false;
  const acc = (x: number, y: number) => {
    any = true;
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
  };
  for (const sp of subpaths) {
    const nodes = sp.nodes;
    const n = nodes.length;
    if (n === 0) continue;
    for (const nd of nodes) acc(nd.point.x, nd.point.y);
    const segs = sp.closed ? n : n - 1;
    for (let i = 0; i < segs; i++) {
      const na = nodes[i];
      const nb = nodes[(i + 1) % n];
      const p0 = na.point;
      const p1 = na.handleOut ?? na.point;
      const p2 = nb.handleIn ?? nb.point;
      const p3 = nb.point;
      for (const t of axisExtrema(p0.x, p1.x, p2.x, p3.x)) {
        const p = cubicAt(p0, p1, p2, p3, t);
        acc(p.x, p.y);
      }
      for (const t of axisExtrema(p0.y, p1.y, p2.y, p3.y)) {
        const p = cubicAt(p0, p1, p2, p3, t);
        acc(p.x, p.y);
      }
    }
  }
  return any ? { minX, minY, maxX, maxY } : null;
}
