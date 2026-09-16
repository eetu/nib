// Tilt — lean a shape by dragging an edge of its box, the opposite edge pinned.
//
// This is the flat-perspective tool. It is deliberately NOT true perspective: a projective map
// sends a cubic bezier to a *rational* cubic, which SVG cannot express, so real vanishing-point
// perspective has to subdivide and approximate. An affine map sends a cubic to a cubic, so moving
// the anchors and their handles IS the transform — exact, reversible, and cheap. What you get is
// the parallel projection an isometric illustration is built from: parallel edges stay parallel.
//
// Why a tool of its own, like rotate-about-a-pivot: the select tool's eight handles already mean
// "scale", and its body already means "move". An edge that sheared instead of scaled would be the
// same handle answering two questions depending on a modifier nobody discovers.
//
// Only the four EDGE handles are live here. A corner drag can't specify an affine map — pinning
// the opposite corner leaves the two adjacent ones free, which is one equation and two unknowns —
// and the honest resolution of that is a general quad, which is the projective case this tool
// exists to stay out of. Scale and rotate stay in the select tool, which is where they read.

import type { Point } from "$lib/model/types";
import { editor } from "$lib/stores/document.svelte";

import { snapshotTargets } from "./shape-util";
import { distortSubpaths, toFrame, type TransformHandle } from "./transform";
import type { DragSession, Tool } from "./types";

type Bounds = { minX: number; minY: number; maxX: number; maxY: number };

/** The edges you can lean. Corners are inert — see the note at the top. */
const EDGES = new Set<TransformHandle>(["n", "e", "s", "w"]);

/**
 * The frame-space affine for dragging `handle` by `(du, dv)`, plus the pivot it turns about.
 *
 * One drag carries both components, which is what makes this read as tipping rather than as two
 * separate operations: the part of the motion running ALONG the edge shears (the top slides
 * sideways), and the part running ACROSS it scales (the shape foreshortens). Drag the top edge
 * right and down and the face tips away from you.
 *
 * `bb` is measured in the box's own frame, so this arithmetic is the same whether or not the
 * shape is already turned — the caller maps in and out.
 */
function edgeDistort(
  handle: TransformHandle,
  bb: { minX: number; minY: number; maxX: number; maxY: number },
  du: number,
  dv: number,
  shearOnly: boolean,
): { m: [number, number, number, number, number, number]; pivot: Point } | null {
  const w = bb.maxX - bb.minX;
  const h = bb.maxY - bb.minY;
  const midX = (bb.minX + bb.maxX) / 2;
  const midY = (bb.minY + bb.maxY) / 2;
  // A flat box has no thickness to lean across; dividing by it would send every point to infinity.
  if (h <= 1e-9 && (handle === "n" || handle === "s")) return null;
  if (w <= 1e-9 && (handle === "e" || handle === "w")) return null;

  switch (handle) {
    case "n": {
      const kx = -du / h;
      const sy = shearOnly ? 1 : (h - dv) / h;
      return { m: [1, 0, kx, sy, 0, 0], pivot: { x: midX, y: bb.maxY } };
    }
    case "s": {
      const kx = du / h;
      const sy = shearOnly ? 1 : (h + dv) / h;
      return { m: [1, 0, kx, sy, 0, 0], pivot: { x: midX, y: bb.minY } };
    }
    case "e": {
      const ky = dv / w;
      const sx = shearOnly ? 1 : (w + du) / w;
      return { m: [sx, ky, 0, 1, 0, 0], pivot: { x: bb.minX, y: midY } };
    }
    case "w": {
      const ky = -dv / w;
      const sx = shearOnly ? 1 : (w - du) / w;
      return { m: [sx, ky, 0, 1, 0, 0], pivot: { x: bb.maxX, y: midY } };
    }
    default:
      return null;
  }
}

/** Lean the selection by dragging one edge. Shift keeps it a pure shear (no foreshortening). */
function distortDrag(
  handle: TransformHandle,
  start: Point,
  angle: number,
  bb: Bounds,
): DragSession {
  const targets = snapshotTargets();
  const startFrame = toFrame(start, angle);
  let moved = false;
  return {
    move(cursor, event) {
      const at = toFrame(cursor, angle);
      const built = edgeDistort(
        handle,
        bb,
        at.x - startFrame.x,
        at.y - startFrame.y,
        event.shiftKey,
      );
      // A drag that would collapse the shape onto a line is simply not applied — the previous
      // frame stays on screen, so pulling back through it recovers rather than destroying.
      if (!built || built.m[0] * built.m[3] - built.m[1] * built.m[2] === 0) return;
      for (const t of targets) {
        editor.setSubpaths(t.pi, distortSubpaths(t.ref, angle, built.pivot, built.m));
      }
      moved = true;
    },
    up() {
      if (moved) editor.commit();
    },
    cancel() {
      if (moved) editor.revert();
    },
  };
}

/** Lean a selection: drag an edge of its box, the opposite edge pinned. */
export const distortTool: Tool = {
  id: "distort",
  cursor: () => "crosshair",
  begin(ctx) {
    const box = editor.selectionFrame;
    if (!box || editor.selectedPaths.length === 0) return null;
    // The shared hit-test already resolves the box's handles, padding and all, so a turned box
    // grabs exactly where it looks — and the tool inherits that rather than re-deriving it.
    if (ctx.hit.kind !== "transform" || !EDGES.has(ctx.hit.handle)) return null;

    // Ratios come from the UNPADDED box: the padding is a grab tolerance, and shearing by a
    // fraction of a box 8px bigger than the shape would lean it differently at every zoom.
    return distortDrag(ctx.hit.handle, ctx.docPoint, box.angle, box.bounds);
  },
};
