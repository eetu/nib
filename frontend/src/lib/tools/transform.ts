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

export function transformCursor(handle: TransformHandle): string {
  if (handle === "nw" || handle === "se") return "nwse-resize";
  if (handle === "ne" || handle === "sw") return "nesw-resize";
  if (handle === "n" || handle === "s") return "ns-resize";
  return "ew-resize";
}
