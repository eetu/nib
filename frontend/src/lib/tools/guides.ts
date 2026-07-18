import type { ViewBox } from "$lib/model/types";

import type { Bounds } from "./transform";

/** The (dx, dy) that snaps a moving box to the grid: per axis, the correction that lands the
 *  *nearest* of the box's min/mid/max edge on a grid line — so a shape clicks into grid alignment by
 *  whichever edge or centre is closest, and moves rigidly (the grab offset is preserved). */
export function gridSnapBox(box: Bounds, size: number): { dx: number; dy: number } {
  if (size <= 0) return { dx: 0, dy: 0 };
  const nearest = (a: number, b: number, c: number): number => {
    let best = 0;
    let min = Infinity;
    for (const v of [a, b, c]) {
      const corr = Math.round(v / size) * size - v;
      if (Math.abs(corr) < min) {
        min = Math.abs(corr);
        best = corr;
      }
    }
    return best;
  };
  const midX = (box.minX + box.maxX) / 2;
  const midY = (box.minY + box.maxY) / 2;
  return { dx: nearest(box.minX, midX, box.maxX), dy: nearest(box.minY, midY, box.maxY) };
}

// The alignment lines a bbox contributes on each axis: min edge · centre · max edge.
function xLines(b: Bounds): number[] {
  return [b.minX, (b.minX + b.maxX) / 2, b.maxX];
}
function yLines(b: Bounds): number[] {
  return [b.minY, (b.minY + b.maxY) / 2, b.maxY];
}

function uniq(values: number[]): number[] {
  return [...new Set(values.map((v) => Math.round(v * 100) / 100))];
}

/**
 * Smart-guide alignment for a moving bbox against other shapes + the canvas. Returns the snap
 * offset (dx, dy) that nudges the moving bbox onto the nearest edge/centre alignment within
 * `threshold` (doc units), plus the guide lines to show (doc x / y positions).
 */
export function alignGuides(
  moving: Bounds,
  others: Bounds[],
  vb: ViewBox,
  threshold: number,
): { dx: number; dy: number; gx: number[]; gy: number[] } {
  const canvas: Bounds = {
    minX: vb.minX,
    minY: vb.minY,
    maxX: vb.minX + vb.width,
    maxY: vb.minY + vb.height,
  };
  const targetsX = [...others, canvas].flatMap(xLines);
  const targetsY = [...others, canvas].flatMap(yLines);
  const mX = xLines(moving);
  const mY = yLines(moving);

  // Nearest alignment offset on one axis (0 if none within threshold).
  const snap = (movers: number[], targets: number[]): number => {
    let best = 0;
    let bestDist = threshold;
    for (const m of movers) {
      for (const t of targets) {
        const d = Math.abs(t - m);
        if (d < bestDist) {
          bestDist = d;
          best = t - m;
        }
      }
    }
    return best;
  };
  const dx = snap(mX, targetsX);
  const dy = snap(mY, targetsY);

  // Guide lines = the targets a (snapped) moving edge/centre now coincides with.
  const gx = uniq(targetsX.filter((t) => mX.some((m) => Math.abs(m + dx - t) < 0.01)));
  const gy = uniq(targetsY.filter((t) => mY.some((m) => Math.abs(m + dy - t) < 0.01)));
  return { dx, dy, gx, gy };
}
