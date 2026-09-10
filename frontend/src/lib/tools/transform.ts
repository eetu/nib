import { tightBounds } from "$lib/model/geometry";
import type { PathNode, Point, Subpath } from "$lib/model/types";

export type TransformHandle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
export type Bounds = { minX: number; minY: number; maxX: number; maxY: number };

/** How far (screen px) the selection box + handles sit outside the shape. */
export const SELECT_PAD_PX = 8;

/** Expand a bounding box outward by `pad` (document units). */
export function padBounds(bb: Bounds, pad: number): Bounds {
  return { minX: bb.minX - pad, minY: bb.minY - pad, maxX: bb.maxX + pad, maxY: bb.maxY + pad };
}

/** The 8 resize handles of a bounding box, in document coordinates. */
export function handlePoints(bb: Bounds): { handle: TransformHandle; point: Point }[] {
  const midX = (bb.minX + bb.maxX) / 2;
  const midY = (bb.minY + bb.maxY) / 2;
  return [
    { handle: "nw", point: { x: bb.minX, y: bb.minY } },
    { handle: "n", point: { x: midX, y: bb.minY } },
    { handle: "ne", point: { x: bb.maxX, y: bb.minY } },
    { handle: "e", point: { x: bb.maxX, y: midY } },
    { handle: "se", point: { x: bb.maxX, y: bb.maxY } },
    { handle: "s", point: { x: midX, y: bb.maxY } },
    { handle: "sw", point: { x: bb.minX, y: bb.maxY } },
    { handle: "w", point: { x: bb.minX, y: midY } },
  ];
}

/**
 * A selection box in **screen space, as points** — four corners and eight handles rather than an
 * axis-aligned rect.
 *
 * A turned box can't be an `x/y/width/height` rect plus a rotation without the hit-test and the
 * drawing each doing that arithmetic separately, which is how they drift apart. Points are the one
 * representation both can share: the overlay traces the corners, the hit-test measures against the
 * same handles, and neither knows or cares whether the box is upright, turned, or (under a
 * non-uniformly-scaled ancestor) sheared into a parallelogram.
 *
 * `angle` is the box's own tilt in radians, and is *only* for orienting things that are drawn at a
 * fixed pixel size — the handle squares and the resize cursors. Everything positional is already in
 * the points.
 */
export type BoxFrame = {
  /** nw, ne, se, sw — in that order, so the polygon traces the box. */
  corners: [Point, Point, Point, Point];
  handles: { handle: TransformHandle; point: Point }[];
  /** Where the rotate knob's stem leaves the box (the top edge's midpoint). */
  stem: Point;
  knob: Point;
  angle: number;
};

const mid = (a: Point, b: Point): Point => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });

/** Build a frame from a box's four screen-space corners (nw, ne, se, sw). */
export function boxFrame(nw: Point, ne: Point, se: Point, sw: Point): BoxFrame {
  const n = mid(nw, ne);
  const e = mid(ne, se);
  const s = mid(se, sw);
  const w = mid(sw, nw);
  // The knob rides the top edge's outward normal, so it stays off the box's own top however the
  // box is turned. Taken from the edge rather than from n - s, which says nothing for a box with
  // no height (a horizontal line, a single-line label scaled flat).
  const dx = ne.x - nw.x;
  const dy = ne.y - nw.y;
  const len = Math.hypot(dx, dy) || 1;
  let ux = dy / len;
  let uy = -dx / len;
  // ...and points *away* from the box: a mirrored transform flips which side that is.
  const c = mid(n, s);
  if (ux * (c.x - n.x) + uy * (c.y - n.y) > 0) {
    ux = -ux;
    uy = -uy;
  }
  return {
    corners: [nw, ne, se, sw],
    handles: [
      { handle: "nw", point: nw },
      { handle: "n", point: n },
      { handle: "ne", point: ne },
      { handle: "e", point: e },
      { handle: "se", point: se },
      { handle: "s", point: s },
      { handle: "sw", point: sw },
      { handle: "w", point: w },
    ],
    stem: n,
    knob: { x: n.x + ux * ROTATE_KNOB_PX, y: n.y + uy * ROTATE_KNOB_PX },
    angle: Math.atan2(dy, dx),
  };
}

/** The four corners (nw, ne, se, sw) of an axis-aligned box, optionally turned about a pivot. */
export function boundsCorners(
  bb: Bounds,
  spin?: { pivot: Point; angle: number } | null,
): [Point, Point, Point, Point] {
  const raw: [Point, Point, Point, Point] = [
    { x: bb.minX, y: bb.minY },
    { x: bb.maxX, y: bb.minY },
    { x: bb.maxX, y: bb.maxY },
    { x: bb.minX, y: bb.maxY },
  ];
  if (!spin) return raw;
  const cos = Math.cos(spin.angle);
  const sin = Math.sin(spin.angle);
  const at = (p: Point): Point => {
    const dx = p.x - spin.pivot.x;
    const dy = p.y - spin.pivot.y;
    return { x: spin.pivot.x + dx * cos - dy * sin, y: spin.pivot.y + dx * sin + dy * cos };
  };
  return [at(raw[0]), at(raw[1]), at(raw[2]), at(raw[3])];
}

/** The handle (or rotate knob) of `frame` under a screen point, if any. */
export function frameHit(
  frame: BoxFrame,
  screen: Point,
): { t: "rotate" } | { t: "scale"; handle: TransformHandle } | null {
  if (Math.hypot(screen.x - frame.knob.x, screen.y - frame.knob.y) <= HANDLE_HIT_PX)
    return { t: "rotate" };
  for (const { handle, point } of frame.handles)
    if (Math.hypot(screen.x - point.x, screen.y - point.y) <= HANDLE_HIT_PX)
      return { t: "scale", handle };
  return null;
}

/**
 * Is `p` inside the (convex) quad `corners`, within `pad` screen px of it?
 *
 * Signed cross products against each edge: all the same sign = inside. `pad` is applied per edge as
 * a distance, so the tolerance is even around a turned box rather than only on its axes.
 */
export function insideQuad(corners: readonly Point[], p: Point, pad = 0): boolean {
  let sign = 0;
  for (let i = 0; i < corners.length; i++) {
    const a = corners[i];
    const b = corners[(i + 1) % corners.length];
    const ex = b.x - a.x;
    const ey = b.y - a.y;
    const len = Math.hypot(ex, ey);
    if (len < 1e-9) continue; // degenerate edge — the neighbours still bound it
    // Distance from the edge line, positive on one side. `pad` lets the point sit just outside.
    const d = ((p.x - a.x) * ey - (p.y - a.y) * ex) / len;
    if (Math.abs(d) <= pad) continue; // within the grab tolerance of this edge either way
    const s = d > 0 ? 1 : -1;
    if (sign === 0) sign = s;
    else if (sign !== s) return false;
  }
  return true;
}

/** For a handle: the fixed anchor (opposite corner/edge), the moving point, and
 *  which axes it scales. Dragging `moving` toward/away from `anchor` scales. */
export function handleAnchor(
  handle: TransformHandle,
  bb: Bounds,
): { anchor: Point; moving: Point; sx: boolean; sy: boolean } {
  const west = handle.includes("w");
  const east = handle.includes("e");
  const north = handle.includes("n");
  const south = handle.includes("s");
  return {
    sx: west || east,
    sy: north || south,
    anchor: { x: east ? bb.minX : bb.maxX, y: south ? bb.minY : bb.maxY },
    moving: { x: east ? bb.maxX : bb.minX, y: south ? bb.maxY : bb.minY },
  };
}

/**
 * A shape's bounds measured in a frame tilted by `angle` — its *own* bounds, when the angle is the
 * one it was turned by (`PathElement.boxAngle`).
 *
 * The geometry is rotated back by `-angle` about the origin first, so the result is an
 * axis-aligned box *in that frame*; rotating its corners forward again (`framedCorners`) puts them
 * where the shape actually is. That round trip is what gives a turned shape a box that hugs it,
 * and handles that pull along its own edges rather than the document's.
 */
export function orientedBounds(subpaths: Subpath[], angle: number): Bounds | null {
  if (!angle) return tightBounds(subpaths);
  return tightBounds(rotateSubpaths(subpaths, ORIGIN, -angle));
}

const ORIGIN: Point = { x: 0, y: 0 };

/** A document point in a frame tilted by `angle` — where `orientedBounds` measures. */
export function toFrame(p: Point, angle: number): Point {
  if (!angle) return p;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  return { x: p.x * cos + p.y * sin, y: -p.x * sin + p.y * cos };
}

/** The inverse of `toFrame`: a framed point back in document space. */
export function fromFrame(p: Point, angle: number): Point {
  if (!angle) return p;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  return { x: p.x * cos - p.y * sin, y: p.x * sin + p.y * cos };
}

/** The corners (nw, ne, se, sw) of bounds measured in a frame tilted by `angle`, back in document
 *  space — the inverse of the rotation `orientedBounds` applied. */
export function framedCorners(bb: Bounds, angle: number): [Point, Point, Point, Point] {
  return boundsCorners(bb, angle ? { pivot: ORIGIN, angle } : null);
}

/** The centre of bounds measured in a tilted frame, back in document space — the pivot a rotation
 *  about "the box centre" actually turns about. */
export function framedCenter(bb: Bounds, angle: number): Point {
  const c = boxCenter(bb);
  if (!angle) return c;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  return { x: c.x * cos - c.y * sin, y: c.x * sin + c.y * cos };
}

/**
 * Scale a reference geometry about `anchor` by (sx, sy) **along the axes of a frame tilted by
 * `angle`** — so dragging the east handle of a box turned 30° widens the shape along its own
 * edge instead of stretching it across the document's x.
 */
export function scaleSubpathsFramed(
  ref: Subpath[],
  anchor: Point,
  sx: number,
  sy: number,
  angle: number,
): Subpath[] {
  if (!angle) return scaleSubpaths(ref, anchor, sx, sy);
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const at = (p: Point): Point => {
    // Into the frame, scale, back out — all about the anchor, which therefore stays put.
    const dx = p.x - anchor.x;
    const dy = p.y - anchor.y;
    const fx = (dx * cos + dy * sin) * sx;
    const fy = (-dx * sin + dy * cos) * sy;
    return { x: anchor.x + fx * cos - fy * sin, y: anchor.y + fx * sin + fy * cos };
  };
  return mapSubpaths(ref, at);
}

/** Apply a point map to every anchor and handle, returning fresh subpaths. */
function mapSubpaths(ref: Subpath[], at: (p: Point) => Point): Subpath[] {
  return ref.map((sp) => ({
    closed: sp.closed,
    nodes: sp.nodes.map((n): PathNode => ({
      type: n.type,
      point: at(n.point),
      handleIn: n.handleIn ? at(n.handleIn) : undefined,
      handleOut: n.handleOut ? at(n.handleOut) : undefined,
    })),
  }));
}

/** Scale a reference geometry about an anchor by (sx, sy), returning fresh
 *  subpaths (does not mutate the reference). */
export function scaleSubpaths(ref: Subpath[], anchor: Point, sx: number, sy: number): Subpath[] {
  const at = (p: Point): Point => ({
    x: anchor.x + (p.x - anchor.x) * sx,
    y: anchor.y + (p.y - anchor.y) * sy,
  });
  return ref.map((sp) => ({
    closed: sp.closed,
    nodes: sp.nodes.map((n): PathNode => ({
      type: n.type,
      point: at(n.point),
      handleIn: n.handleIn ? at(n.handleIn) : undefined,
      handleOut: n.handleOut ? at(n.handleOut) : undefined,
    })),
  }));
}

/** Which way each handle pulls, as a screen-space angle in degrees (y down). */
const HANDLE_DEG: Record<TransformHandle, number> = {
  e: 0,
  se: 45,
  s: 90,
  sw: 135,
  w: 180,
  nw: 225,
  n: 270,
  ne: 315,
};

/**
 * The resize cursor for a handle on a box tilted by `angle` radians.
 *
 * There are only four double-arrow cursors, so this picks the one nearest the direction the handle
 * actually pulls — on a box turned 45° the "east" handle stretches down-right, and an `ew-resize`
 * arrow there points somewhere the drag won't go.
 */
export function transformCursor(handle: TransformHandle, angle = 0): string {
  const deg = HANDLE_DEG[handle] + (angle * 180) / Math.PI;
  // A resize axis is bidirectional, so only the direction mod 180° matters.
  const bucket = Math.round((((deg % 180) + 180) % 180) / 45) % 4;
  return ["ew-resize", "nwse-resize", "ns-resize", "nesw-resize"][bucket];
}

/** How far (screen px) the rotate knob sits above the box's top-centre. */
export const ROTATE_KNOB_PX = 22;

/** Hit radius (screen px) for grabbing a transform/resize handle or the rotate knob. Shared by
 *  the path hit-test and the element transform box so identical-looking handles grab identically. */
export const HANDLE_HIT_PX = 11;

/** The box centre — the rotation pivot. */
export function boxCenter(bb: Bounds): Point {
  return { x: (bb.minX + bb.maxX) / 2, y: (bb.minY + bb.maxY) / 2 };
}

/** Shear a reference geometry about `pivot` by factors (kx, ky) — kx = tan(skewX),
 *  ky = tan(skewY). Returns fresh subpaths (does not mutate the reference). */
export function shearSubpaths(ref: Subpath[], pivot: Point, kx: number, ky: number): Subpath[] {
  const at = (p: Point): Point => ({
    x: p.x + kx * (p.y - pivot.y),
    y: p.y + ky * (p.x - pivot.x),
  });
  return ref.map((sp) => ({
    closed: sp.closed,
    nodes: sp.nodes.map((n): PathNode => ({
      type: n.type,
      point: at(n.point),
      handleIn: n.handleIn ? at(n.handleIn) : undefined,
      handleOut: n.handleOut ? at(n.handleOut) : undefined,
    })),
  }));
}

/** Rotate a reference geometry about `pivot` by `angle` radians, returning fresh subpaths
 *  (does not mutate the reference). */
export function rotateSubpaths(ref: Subpath[], pivot: Point, angle: number): Subpath[] {
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const at = (p: Point): Point => {
    const dx = p.x - pivot.x;
    const dy = p.y - pivot.y;
    return { x: pivot.x + dx * cos - dy * sin, y: pivot.y + dx * sin + dy * cos };
  };
  return ref.map((sp) => ({
    closed: sp.closed,
    nodes: sp.nodes.map((n): PathNode => ({
      type: n.type,
      point: at(n.point),
      handleIn: n.handleIn ? at(n.handleIn) : undefined,
      handleOut: n.handleOut ? at(n.handleOut) : undefined,
    })),
  }));
}
