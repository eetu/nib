import type { PathNode, Subpath } from "./types";

// The magic constant for approximating a quarter circle with a cubic bezier.
const KAPPA = 0.5522847498307936;

/**
 * Four smooth cubic-bezier nodes (E, S, W, N) approximating an ellipse centred
 * at (cx, cy) with radii rx/ry. Returned as editable anchor nodes so a drawn
 * circle behaves like any other path — drag its nodes/handles to reshape it.
 */
export function ellipseNodes(cx: number, cy: number, rx: number, ry: number): PathNode[] {
  const kx = KAPPA * rx;
  const ky = KAPPA * ry;
  return [
    {
      point: { x: cx + rx, y: cy },
      handleIn: { x: cx + rx, y: cy - ky },
      handleOut: { x: cx + rx, y: cy + ky },
      type: "smooth",
    },
    {
      point: { x: cx, y: cy + ry },
      handleIn: { x: cx + kx, y: cy + ry },
      handleOut: { x: cx - kx, y: cy + ry },
      type: "smooth",
    },
    {
      point: { x: cx - rx, y: cy },
      handleIn: { x: cx - rx, y: cy + ky },
      handleOut: { x: cx - rx, y: cy - ky },
      type: "smooth",
    },
    {
      point: { x: cx, y: cy - ry },
      handleIn: { x: cx - kx, y: cy - ry },
      handleOut: { x: cx + kx, y: cy - ry },
      type: "smooth",
    },
  ];
}

export function ellipseSubpath(cx: number, cy: number, rx: number, ry: number): Subpath {
  return { nodes: ellipseNodes(cx, cy, rx, ry), closed: true };
}
