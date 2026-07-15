import { tightBounds } from "$lib/model/geometry";
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

function bboxIntersects(a: Bounds, b: Bounds): boolean {
  return a.minX <= b.maxX && a.maxX >= b.minX && a.minY <= b.maxY && a.maxY >= b.minY;
}

/** Move the whole object selection (one shape, or a multi-selection as a group). Shift
 *  axis-locks; otherwise smart guides align the selection's union box to other shapes + the
 *  canvas. A plain click (no move) on a member of a multi-selection reduces it to that one
 *  shape (Figma-style). */
function selectionDrag(start: Point, primary: number, wasMulti: boolean): DragSession {
  const doc = editor.doc;
  const base = editor.selectionBounds;
  const sel = new Set(editor.selectedPaths);
  const others: Bounds[] = [];
  doc?.paths.forEach((p, i) => {
    if (sel.has(i) || p.deleted) return;
    const b = tightBounds(p.subpaths);
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
      editor.moveSelectedBy(dx, dy);
      appliedX = tx;
      appliedY = ty;
      moved = true;
    },
    up() {
      interaction.clearDrag();
      if (moved) editor.commit();
      else if (wasMulti) editor.selectPath(primary);
    },
    cancel() {
      interaction.clearDrag();
      if (moved) editor.revert();
    },
  };
}

/** Rubber-band selection over empty canvas: every shape whose bbox intersects the box gets
 *  selected. A plain click (no drag) on empty clears the selection. */
function marqueeDrag(start: Point): DragSession {
  let moved = false;
  return {
    move(docPoint) {
      moved = true;
      interaction.marquee = { x0: start.x, y0: start.y, x1: docPoint.x, y1: docPoint.y };
    },
    up() {
      const m = interaction.marquee;
      interaction.marquee = null;
      const doc = editor.doc;
      if (!moved || !m || !doc) {
        editor.deselect();
        return;
      }
      const rect: Bounds = {
        minX: Math.min(m.x0, m.x1),
        minY: Math.min(m.y0, m.y1),
        maxX: Math.max(m.x0, m.x1),
        maxY: Math.max(m.y0, m.y1),
      };
      const hits: number[] = [];
      doc.paths.forEach((p, i) => {
        if (p.deleted) return;
        const b = tightBounds(p.subpaths);
        if (b && bboxIntersects(b, rect)) hits.push(i);
      });
      if (hits.length) editor.setSelectedPaths(hits);
      else editor.deselect();
    },
    cancel() {
      interaction.marquee = null;
    },
  };
}

/** Deep-clone the subpaths of every selected path — the reference geometry a transform drag
 *  scales/rotates from (stable across the whole gesture). One shape or a whole group; both
 *  transform about the union box, so a multi-selection scales/rotates as one (Pixelmator-style). */
function snapshotTargets(): { pi: number; ref: Subpath[] }[] {
  const doc = editor.doc;
  if (!doc) return [];
  return editor.selectedPaths
    .map((pi) => {
      const p = doc.paths[pi];
      return p && !p.deleted
        ? { pi, ref: JSON.parse(JSON.stringify(p.subpaths)) as Subpath[] }
        : null;
    })
    .filter((t): t is { pi: number; ref: Subpath[] } => t !== null);
}

/** Rotate the object selection (one shape or a multi-select group) by dragging the knob above
 *  the box. Rotation is about the union box centre, relative to the geometry at drag start;
 *  shift snaps to 15° steps. */
function rotateDrag(start: Point): DragSession {
  const targets = snapshotTargets();
  const bb = editor.selectionBounds;
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
      for (const t of targets) editor.setSubpaths(t.pi, rotateSubpaths(t.ref, center, delta));
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

/** Scale the object selection (one shape or a multi-select group) by dragging a bounding-box
 *  handle, about the opposite anchor of the union box; shift keeps the aspect ratio. */
function scaleDrag(handle: TransformHandle): DragSession {
  const targets = snapshotTargets();
  const bb = editor.selectionBounds;
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
      for (const t of targets) editor.setSubpaths(t.pi, scaleSubpaths(t.ref, g.anchor, sx, sy));
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
      return editor.selectedPaths.length ? scaleDrag(hit.handle) : null;
    }
    if (hit.kind === "rotate") {
      return editor.selectedPaths.length ? rotateDrag(ctx.docPoint) : null;
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
      const pi = hit.pathIndex;
      // Shift- or ⌘-click toggles a shape in/out of the multi-selection (no drag).
      if (ctx.event.shiftKey || ctx.event.metaKey) {
        editor.togglePath(pi);
        return null;
      }
      // Grabbing a member of a multi-selection drags the whole group; grabbing any other
      // shape object-selects it (the path you're node-editing keeps node mode).
      const inMulti = editor.multiSelected && editor.selectedPaths.includes(pi);
      if (!inMulti && editor.nodeEditIndex !== pi) editor.selectPath(pi);
      return selectionDrag(ctx.docPoint, pi, inMulti);
    }
    // Empty canvas → rubber-band marquee (a plain click clears the selection).
    return marqueeDrag(ctx.docPoint);
  },
};
