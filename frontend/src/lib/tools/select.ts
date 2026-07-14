import { subpathsBounds } from "$lib/model/geometry";
import type { NodeRef, Point, Subpath } from "$lib/model/types";
import { collectAnchors, findSnap, isCloseLoop, snapToGrid } from "$lib/snap";
import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import { alignGuides } from "./guides";
import {
  type Bounds,
  boxCenter,
  handleAnchor,
  rotateSubpaths,
  scaleSubpaths,
  transformCursor,
  type TransformHandle,
} from "./transform";
import type { DragSession, Tool } from "./types";

/** Constrain `current` to a horizontal or vertical line from `start` (the
 *  dominant axis) — the shift-to-axis behaviour. */
function axisLock(start: Point, current: Point): Point {
  const dx = current.x - start.x;
  const dy = current.y - start.y;
  return Math.abs(dx) >= Math.abs(dy) ? { x: current.x, y: start.y } : { x: start.x, y: current.y };
}

/** Resolve where a dragged anchor should land: snap onto another anchor (and
 *  flag a close-loop), else grid, else the raw pointer. Publishes the snap
 *  indicator for the overlay as a side effect. */
function resolveTarget(docPoint: Point, dragged: NodeRef): { point: Point; closing: boolean } {
  const doc = editor.doc;
  interaction.clearDrag();
  if (!doc) return { point: docPoint, closing: false };

  if (tools.snapEnabled) {
    const threshold = viewport.toDocLength(tools.snapThresholdPx);
    const hit = findSnap(docPoint, collectAnchors(doc, dragged), threshold);
    if (hit) {
      const closing = isCloseLoop(dragged, hit.target, doc);
      interaction.snapPoint = hit.target.point;
      interaction.closing = closing;
      return { point: hit.target.point, closing };
    }
  }
  if (tools.gridEnabled) return { point: snapToGrid(docPoint, tools.gridSize), closing: false };
  return { point: docPoint, closing: false };
}

function anchorDrag(ref: NodeRef, start: Point): DragSession {
  let closeAt: { pathIndex: number; subpathIndex: number } | null = null;
  let moved = false;
  return {
    move(docPoint, event) {
      let target: Point;
      if (event.shiftKey) {
        // axis-lock takes precedence over snapping
        target = axisLock(start, docPoint);
        closeAt = null;
        interaction.clearDrag();
      } else {
        const r = resolveTarget(docPoint, ref);
        target = r.point;
        closeAt = r.closing ? { pathIndex: ref.pathIndex, subpathIndex: ref.subpathIndex } : null;
      }
      editor.moveNode(ref, target);
      moved = true;
    },
    up() {
      interaction.clearDrag();
      if (!moved) return; // a plain click just selects — no undo step
      if (closeAt) editor.closeLoop(closeAt.pathIndex, closeAt.subpathIndex);
      else editor.commit();
    },
    cancel() {
      interaction.clearDrag();
      if (moved) editor.revert();
    },
  };
}

function handleDrag(ref: NodeRef, which: "in" | "out", anchor: Point): DragSession {
  let moved = false;
  return {
    move(docPoint, event) {
      let point = docPoint;
      if (event.shiftKey) point = axisLock(anchor, docPoint);
      else if (tools.gridEnabled) point = snapToGrid(docPoint, tools.gridSize);
      editor.moveHandle(ref, which, point);
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

// Snap distance (screen px) for smart guides — a bit tighter than anchor snapping.
const GUIDE_PX = 6;

/** Drag a path's body to move the whole shape (all nodes translate together). Shift locks
 *  the translation to one axis; otherwise smart guides align its edges/centre to other
 *  shapes + the canvas (when enabled). */
function pathDrag(pathIndex: number, start: Point): DragSession {
  const doc = editor.doc;
  const base = doc?.paths[pathIndex] ? subpathsBounds(doc.paths[pathIndex].subpaths) : null;
  // Other shapes' bounds, captured once, as smart-guide targets.
  const others: Bounds[] = [];
  doc?.paths.forEach((p, i) => {
    if (i === pathIndex || p.deleted) return;
    const b = subpathsBounds(p.subpaths);
    if (b) others.push(b);
  });
  let appliedX = 0;
  let appliedY = 0;
  let moved = false;
  return {
    move(docPoint, event) {
      let tx = docPoint.x - start.x;
      let ty = docPoint.y - start.y;
      if (event.shiftKey) {
        if (Math.abs(tx) >= Math.abs(ty)) ty = 0;
        else tx = 0;
        interaction.guidesX = [];
        interaction.guidesY = [];
      } else if (tools.guidesEnabled && base && doc) {
        const moving = {
          minX: base.minX + tx,
          minY: base.minY + ty,
          maxX: base.maxX + tx,
          maxY: base.maxY + ty,
        };
        const g = alignGuides(moving, others, doc.viewBox, viewport.toDocLength(GUIDE_PX));
        tx += g.dx;
        ty += g.dy;
        interaction.guidesX = g.gx;
        interaction.guidesY = g.gy;
      }
      const dx = tx - appliedX;
      const dy = ty - appliedY;
      if (dx === 0 && dy === 0) return;
      editor.movePathBy(pathIndex, dx, dy);
      appliedX = tx;
      appliedY = ty;
      moved = true;
    },
    up() {
      interaction.clearDrag();
      if (moved) editor.commit();
    },
    cancel() {
      interaction.clearDrag();
      if (moved) editor.revert();
    },
  };
}

/** Rotate the selected path by dragging the knob above the box. Rotation is about the box
 *  centre, relative to the geometry at drag start; shift snaps to 15° steps. */
function rotateDrag(pathIndex: number, start: Point): DragSession {
  const path = editor.doc?.paths[pathIndex];
  const ref = path ? (JSON.parse(JSON.stringify(path.subpaths)) as Subpath[]) : [];
  const bb = subpathsBounds(ref);
  const center = bb ? boxCenter(bb) : { x: 0, y: 0 };
  const startAngle = Math.atan2(start.y - center.y, start.x - center.x);
  let moved = false;
  return {
    move(cursor, event) {
      if (!bb) return;
      let delta = Math.atan2(cursor.y - center.y, cursor.x - center.x) - startAngle;
      if (event.shiftKey) {
        const step = Math.PI / 12; // 15°
        delta = Math.round(delta / step) * step;
      }
      editor.setSubpaths(pathIndex, rotateSubpaths(ref, center, delta));
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

/** Scale the selected path by dragging a bounding-box handle. Scaling is
 *  relative to the geometry at drag start; shift keeps the aspect ratio. */
function scaleDrag(pathIndex: number, handle: TransformHandle): DragSession {
  const path = editor.doc?.paths[pathIndex];
  const ref = path ? (JSON.parse(JSON.stringify(path.subpaths)) as Subpath[]) : [];
  const bb = subpathsBounds(ref);
  let moved = false;
  return {
    move(cursor, event) {
      if (!bb) return;
      const g = handleAnchor(handle, bb);
      let sx = 1;
      let sy = 1;
      if (g.sx) {
        const d = g.moving.x - g.anchor.x;
        if (d !== 0) sx = (cursor.x - g.anchor.x) / d;
      }
      if (g.sy) {
        const d = g.moving.y - g.anchor.y;
        if (d !== 0) sy = (cursor.y - g.anchor.y) / d;
      }
      if (event.shiftKey && g.sx && g.sy) {
        const m = Math.max(Math.abs(sx), Math.abs(sy));
        sx = sx < 0 ? -m : m;
        sy = sy < 0 ? -m : m;
      }
      editor.setSubpaths(pathIndex, scaleSubpaths(ref, g.anchor, sx, sy));
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

export const selectTool: Tool = {
  id: "select",
  cursor(hit) {
    if (hit.kind === "transform") return transformCursor(hit.handle);
    if (hit.kind === "rotate") return "grab";
    if (hit.kind === "handle" || hit.kind === "anchor") return "grab";
    if (hit.kind === "segment" || hit.kind === "fill") return "move";
    return "default";
  },
  begin(ctx) {
    const { hit } = ctx;
    if (hit.kind === "transform") {
      const pi = editor.selectedPathIndex;
      return pi !== null ? scaleDrag(pi, hit.handle) : null;
    }
    if (hit.kind === "rotate") {
      const pi = editor.selectedPathIndex;
      return pi !== null ? rotateDrag(pi, ctx.docPoint) : null;
    }
    if (hit.kind === "handle") {
      editor.select(hit.ref);
      const anchor = editor.selectedNode ? { ...editor.selectedNode.point } : ctx.docPoint;
      return handleDrag(hit.ref, hit.which, anchor);
    }
    if (hit.kind === "anchor") {
      editor.select(hit.ref);
      const start = editor.selectedNode ? { ...editor.selectedNode.point } : ctx.docPoint;
      return anchorDrag(hit.ref, start);
    }
    if (hit.kind === "segment" || hit.kind === "fill") {
      // Grab a path's body (its outline or its filled interior) to select + move it.
      editor.selectPath(hit.pathIndex);
      return pathDrag(hit.pathIndex, ctx.docPoint);
    }
    editor.deselect();
    return null;
  },
};
