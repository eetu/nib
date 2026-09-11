// Rotate about a pivot you can put anywhere — the one transform the unified select tool can't
// offer. Its box rotates about the box centre, and that covers most turning; what it can't do is
// swing a shape *around something else*: an arm about a shoulder, a hand about a wrist, a spoke
// about a hub. A pivot handle in the select tool would sit exactly where drag-to-move and
// double-click-to-node-edit already live, so this is a tool of its own instead — the way
// Illustrator has always done it.
//
// Two gestures, told apart by whether the pointer travelled: a click places the pivot, a drag
// turns the selection about it. Nothing else competes for the canvas while this tool is active.

import type { Point } from "$lib/model/types";
import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import { snapBypassed, snapshotTargets } from "./shape-util";
import { framedCenter, fromFrame, handlePoints, rotateSubpaths } from "./transform";
import type { DragSession, Tool } from "./types";

/** How close (in screen px) the pointer must be to grab the pivot rather than start a rotation. */
const PIVOT_GRAB_PX = 10;
/** Past this much travel a press is a rotation, not a click that places the pivot. */
const DRAG_SLOP_PX = 3;

/** Where the selection turns: the placed pivot, else the centre of the box **as drawn** — which
 *  for a turned selection is not the centre of its document-axis bounds. */
export function activePivot(): Point | null {
  if (tools.pivot) return tools.pivot;
  const box = editor.selectionFrame;
  return box ? framedCenter(box.bounds, box.angle) : null;
}

/** The points a dragged pivot snaps to: the selection box's corners, edge midpoints and centre —
 *  the placements you actually reach for ("turn about that corner"). Taken from the box as drawn,
 *  so on a turned selection the pivot snaps to the corners you can see. */
function pivotSnapPoints(): Point[] {
  const box = editor.selectionFrame;
  if (!box) return [];
  return [
    ...handlePoints(box.bounds).map((h) => fromFrame(h.point, box.angle)),
    framedCenter(box.bounds, box.angle),
  ];
}

/** Pull `p` onto a nearby box point, unless snapping is off or bypassed. */
function snapPivot(p: Point, event: PointerEvent): Point {
  if (!tools.snapEnabled || snapBypassed(event)) return p;
  const reach = viewport.toDocLength(tools.snapThresholdPx);
  let best: { point: Point; d: number } | null = null;
  for (const candidate of pivotSnapPoints()) {
    const d = Math.hypot(candidate.x - p.x, candidate.y - p.y);
    if (d <= reach && (!best || d < best.d)) best = { point: candidate, d };
  }
  return best ? best.point : p;
}

/** Drag the pivot itself to a new spot. No geometry moves; nothing to commit. */
function movePivotDrag(): DragSession {
  return {
    move(cursor, event) {
      tools.pivot = snapPivot(cursor, event);
    },
    up() {},
    cancel() {},
  };
}

/** Turn the selection about the pivot, relative to the geometry at drag start; shift snaps to 15°
 *  steps. A press that never travels far enough places the pivot instead — see `begin`. */
function rotateDrag(start: Point, pivot: Point): DragSession {
  const targets = snapshotTargets();
  const startAngle = Math.atan2(start.y - pivot.y, start.x - pivot.x);
  let moved = false;
  return {
    move(cursor, event) {
      let delta = Math.atan2(cursor.y - pivot.y, cursor.x - pivot.x) - startAngle;
      if (event.shiftKey) {
        const step = Math.PI / 12; // 15°
        delta = Math.round(delta / step) * step;
      }
      // Geometry and box tilt turn together, so the box swings about the pivot with the shape —
      // and stays turned afterwards, however far from the shape's centre the pivot sat.
      for (const t of targets) {
        editor.setSubpaths(t.pi, rotateSubpaths(t.ref, pivot, delta));
        editor.setBoxAngle(t.pi, t.angle + delta);
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

/**
 * A press that may become either gesture: it starts as a rotation, but if the pointer is released
 * within a few pixels it was a click, and a click places the pivot. Deciding at release (rather
 * than making the user aim at a handle) is what keeps "put the pivot there, now turn" fluid.
 */
function rotateOrPlace(start: Point, pivot: Point): DragSession {
  const rotation = rotateDrag(start, pivot);
  const from = viewport.toScreen(start);
  let travelled = false;
  return {
    move(cursor, event) {
      const at = viewport.toScreen(cursor);
      if (!travelled && Math.hypot(at.x - from.x, at.y - from.y) > DRAG_SLOP_PX) travelled = true;
      if (travelled) rotation.move(cursor, event);
    },
    up(cursor) {
      if (travelled) rotation.up(cursor);
      else tools.pivot = start; // a click, so this is where the pivot goes
    },
    cancel() {
      rotation.cancel();
    },
  };
}

/** Rotate about a movable pivot: click to place it, drag to turn. */
export const rotateTool: Tool = {
  id: "rotate",
  cursor: () => "crosshair",
  begin(ctx) {
    const pivot = activePivot();
    // Nothing selected: the tool has nothing to turn, and no centre to fall back on.
    if (!pivot || editor.selectedPaths.length === 0) return null;

    // Grabbing the pivot moves it; anywhere else starts the press that decides at release.
    const grab = viewport.toDocLength(PIVOT_GRAB_PX);
    if (Math.hypot(ctx.docPoint.x - pivot.x, ctx.docPoint.y - pivot.y) <= grab) {
      return movePivotDrag();
    }
    return rotateOrPlace(ctx.docPoint, pivot);
  },
  onDeactivate() {
    // The pivot belongs to a working session with this tool, not to the document.
    tools.pivot = null;
  },
};
