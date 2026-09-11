import { tightBounds } from "$lib/model/geometry";
import type { NodeRef, Point } from "$lib/model/types";
import { collectAnchors, findSnap, isCloseLoop, snapToGrid } from "$lib/snap";
import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import { alignGuides, gridSnapBox } from "./guides";
import { snapBypassed, snapshotTargets } from "./shape-util";
import {
  type Bounds,
  framedCenter,
  fromFrame,
  handleAnchor,
  rotateSubpaths,
  scaleSubpathsFramed,
  toFrame,
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
function resolveTarget(
  docPoint: Point,
  dragged: NodeRef,
  bypass = false,
): { point: Point; closing: boolean } {
  const doc = editor.doc;
  interaction.clearDrag();
  if (!doc || bypass) return { point: docPoint, closing: false };

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
        const r = resolveTarget(docPoint, ref, snapBypassed(event));
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
      else if (tools.gridEnabled && !snapBypassed(event))
        point = snapToGrid(docPoint, tools.gridSize);
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
      } else if (snapBypassed(event) || !base || !doc) {
        // ⌘/Ctrl held → move freely, no snapping.
        interaction.guidesX = [];
        interaction.guidesY = [];
      } else {
        const moving = {
          minX: base.minX + tx,
          minY: base.minY + ty,
          maxX: base.maxX + tx,
          maxY: base.maxY + ty,
        };
        if (tools.gridEnabled) {
          // Snap-to-grid: quantise the moving box — the nearest of its edges/centre per axis lands
          // on a grid line, and the whole selection translates rigidly (grab offset preserved).
          const g = gridSnapBox(moving, tools.gridSize);
          tx += g.dx;
          ty += g.dy;
          interaction.guidesX = [];
          interaction.guidesY = [];
        } else if (tools.guidesEnabled) {
          const g = alignGuides(moving, others, doc.viewBox, viewport.toDocLength(GUIDE_PX));
          tx += g.dx;
          ty += g.dy;
          interaction.guidesX = g.gx;
          interaction.guidesY = g.gy;
        } else {
          interaction.guidesX = [];
          interaction.guidesY = [];
        }
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
      // A no-move click on a member of an *ad-hoc* multi-selection reduces it to that shape
      // (Figma-style); a **group** selection stays whole (double-click drills in instead).
      else if (wasMulti && !editor.selectedGroupUid) editor.selectPath(primary);
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

/** Rotate the object selection (one shape or a multi-select group) by dragging the knob above
 *  the box. Rotation is about the union box centre, relative to the geometry at drag start;
 *  shift snaps to 15° steps. */
function rotateDrag(start: Point): DragSession {
  const targets = snapshotTargets();
  const box = editor.selectionFrame;
  // The centre of the box as drawn, which for a turned selection is not the centre of its
  // document-axis bounds.
  const center = box ? framedCenter(box.bounds, box.angle) : { x: 0, y: 0 };
  const startAngle = Math.atan2(start.y - center.y, start.x - center.x);
  let moved = false;
  return {
    move(cursor, event) {
      if (!box) return;
      let delta = Math.atan2(cursor.y - center.y, cursor.x - center.x) - startAngle;
      if (event.shiftKey) {
        const step = Math.PI / 12; // 15°
        delta = Math.round(delta / step) * step;
      }
      // Geometry and box tilt turn together, which is what makes the box follow the drag without
      // any separate "draw it turning" feedback: bounds measured in the turning frame keep their
      // size, so the box swings rather than breathing wide-to-tall.
      for (const t of targets) {
        editor.setSubpaths(t.pi, rotateSubpaths(t.ref, center, delta));
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

/** Scale the object selection (one shape or a multi-select group) by dragging a bounding-box
 *  handle, about the opposite anchor of the union box; shift keeps the aspect ratio. */
function scaleDrag(handle: TransformHandle): DragSession {
  const targets = snapshotTargets();
  const box = editor.selectionFrame;
  const angle = box?.angle ?? 0;
  let moved = false;
  return {
    move(cursor, event) {
      if (!box) return;
      // All of this happens **in the box's own frame**, where it's axis-aligned: the anchor and
      // moving points come straight off the framed bounds, and the cursor rotates in to meet them.
      // Otherwise the east handle of a turned box would stretch the shape across the document's x
      // while the handle itself travelled along the box's edge — the handle and the shape
      // disagreeing is exactly what a turned box must not do.
      const g = handleAnchor(handle, box.bounds);
      const cur = toFrame(cursor, angle);
      let sx = 1;
      let sy = 1;
      if (g.sx) {
        const d = g.moving.x - g.anchor.x;
        if (d !== 0) sx = (cur.x - g.anchor.x) / d;
      }
      if (g.sy) {
        const d = g.moving.y - g.anchor.y;
        if (d !== 0) sy = (cur.y - g.anchor.y) / d;
      }
      if (event.shiftKey && g.sx && g.sy) {
        const m = Math.max(Math.abs(sx), Math.abs(sy));
        sx = sx < 0 ? -m : m;
        sy = sy < 0 ? -m : m;
      }
      const anchor = fromFrame(g.anchor, angle);
      for (const t of targets)
        editor.setSubpaths(t.pi, scaleSubpathsFramed(t.ref, anchor, sx, sy, angle));
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
    if (hit.kind === "transform")
      return transformCursor(hit.handle, editor.selectionFrame?.angle ?? 0);
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
      // Grabbing a member of the current selection drags the whole thing; grabbing any other
      // shape group-selects it (its enclosing group as one unit, else the shape) — the path you're
      // node-editing keeps node mode. Double-click drills into a group to node-edit a shape.
      const inSel =
        editor.selectedPaths.includes(pi) && (editor.multiSelected || editor.objectSelected);
      if (!inSel && editor.nodeEditIndex !== pi) editor.selectGroup(pi);
      return selectionDrag(ctx.docPoint, pi, editor.selectedPaths.length > 1);
    }
    // Empty canvas → rubber-band marquee (a plain click clears the selection).
    return marqueeDrag(ctx.docPoint);
  },
};
