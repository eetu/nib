import { cubicAt, distance } from "$lib/model/geometry";
import { nearestOnSubpath, segmentControlPoints } from "$lib/model/path";
import type { NodeRef, Point, Subpath } from "$lib/model/types";
import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";

import {
  boxFrame,
  framedCorners,
  frameHit,
  HANDLE_HIT_PX,
  padBounds,
  SELECT_PAD_PX,
} from "./transform";
import type { Hit } from "./types";

const ANCHOR_HIT_PX = 11;
const SEGMENT_HIT_PX = 8;
const FLATTEN_STEPS = 12;

function screenDist(docPt: Point, screen: Point): number {
  return distance(viewport.toScreen(docPt), screen);
}

/** Sample a subpath's outline into a polyline (open subpaths close implicitly for fill). */
function flattenSubpath(sp: Subpath): Point[] {
  const n = sp.nodes.length;
  if (n < 2) return [];
  const pts: Point[] = [];
  const segs = sp.closed ? n : n - 1;
  for (let i = 0; i < segs; i++) {
    const [p0, p1, p2, p3] = segmentControlPoints(sp, i);
    for (let s = 0; s < FLATTEN_STEPS; s++) pts.push(cubicAt(p0, p1, p2, p3, s / FLATTEN_STEPS));
  }
  return pts;
}

/** Is `pt` inside the path's filled area? Nonzero-winding ray cast over the flattened
 *  subpaths (matches SVG's default fill-rule), so clicking a shape's body selects it. */
function pointInPath(subpaths: Subpath[], pt: Point): boolean {
  let winding = 0;
  for (const sp of subpaths) {
    const poly = flattenSubpath(sp);
    const m = poly.length;
    if (m < 3) continue;
    for (let i = 0; i < m; i++) {
      const a = poly[i];
      const b = poly[(i + 1) % m];
      const c = (b.x - a.x) * (pt.y - a.y) - (b.y - a.y) * (pt.x - a.x);
      if (a.y <= pt.y) {
        if (b.y > pt.y && c > 0) winding++;
      } else if (b.y <= pt.y && c < 0) {
        winding--;
      }
    }
  }
  return winding !== 0;
}

/**
 * What sits under the pointer, in priority order: a handle of the selected
 * node, then any anchor, then the nearest path segment, else empty. Distances
 * are measured in screen pixels so hit radii stay constant at any zoom.
 */
export function hitTest(screen: Point): Hit {
  const doc = editor.doc;
  if (!doc) return { kind: "empty" };

  // Component-definition shapes live inside `<defs>`: they paint only via `<use>` (never directly at
  // their def-space coords), so they must not be phantom click targets. Skip them in every scan.
  const defUids = editor.defPathUids;

  // 1+2. Handles + anchors are hit-testable only while node-editing: any non-select tool
  //       (add/delete-node, pen), or the select tool after a double-click enters node mode.
  //       Otherwise the select tool's drag always moves the whole shape — no ambiguity over
  //       whether a node or the shape moves, which matters most when zoomed out and anchors
  //       cluster. Transform handles (step 3) cover object-mode resize/rotate instead.
  const nodeEditable = tools.active !== "select" || editor.nodeEditIndex !== null;
  if (nodeEditable) {
    // Handles — only the selected node exposes its control handles.
    const sel = editor.selection;
    if (sel) {
      const node = doc.paths[sel.pathIndex]?.subpaths[sel.subpathIndex]?.nodes[sel.nodeIndex];
      if (node) {
        if (node.handleOut && screenDist(node.handleOut, screen) <= HANDLE_HIT_PX) {
          return { kind: "handle", ref: sel, which: "out" };
        }
        if (node.handleIn && screenDist(node.handleIn, screen) <= HANDLE_HIT_PX) {
          return { kind: "handle", ref: sel, which: "in" };
        }
      }
    }

    // Anchors — nearest within the hit radius. Checked before transform handles so a path's
    // own nodes are never shadowed (e.g. a circle's nodes sit on the bbox edge-midpoints).
    let bestAnchor: { ref: NodeRef; d: number } | null = null;
    doc.paths.forEach((path, pathIndex) => {
      if (path.deleted || path.locked || defUids.has(path.uid ?? "")) return;
      path.subpaths.forEach((sp, subpathIndex) => {
        sp.nodes.forEach((n, nodeIndex) => {
          const d = screenDist(n.point, screen);
          if (d <= ANCHOR_HIT_PX && (!bestAnchor || d < bestAnchor.d)) {
            bestAnchor = { ref: { pathIndex, subpathIndex, nodeIndex }, d };
          }
        });
      });
    });
    if (bestAnchor) return { kind: "anchor", ref: (bestAnchor as { ref: NodeRef }).ref };
  }

  // 3. Transform handles — for an object (whole-path) selection *or* a multi-select group,
  //    at corners/edges of the union box not occupied by a node. Both use selectionBounds so
  //    a group scales/rotates as one (Pixelmator-style).
  if (editor.objectSelected || editor.multiSelected) {
    const box = editor.selectionFrame;
    if (box) {
      // Built the same way the overlay builds it, and measured against the same points — so a
      // turned box grabs exactly where it looks, knob included.
      const bb = padBounds(box.bounds, viewport.toDocLength(SELECT_PAD_PX));
      const c = framedCorners(bb, box.angle).map((p) => viewport.toScreen(p));
      const hit = frameHit(boxFrame(c[0], c[1], c[2], c[3]), screen);
      if (hit?.t === "rotate") return { kind: "rotate" };
      if (hit) return { kind: "transform", handle: hit.handle };
    }
  }

  // 4. Segment — nearest outline point within the hit radius (for add-node).
  const docPoint = viewport.toDoc(screen);
  const threshDoc = viewport.toDocLength(SEGMENT_HIT_PX);
  let best: Hit | null = null;
  let bestD = Infinity;
  doc.paths.forEach((path, pathIndex) => {
    if (path.deleted || path.locked || defUids.has(path.uid ?? "")) return;
    path.subpaths.forEach((sp, subpathIndex) => {
      const hit = nearestOnSubpath(sp, docPoint);
      if (hit && hit.distance <= threshDoc && hit.distance < bestD) {
        bestD = hit.distance;
        best = {
          kind: "segment",
          pathIndex,
          subpathIndex,
          segmentIndex: hit.segmentIndex,
          t: hit.t,
          point: hit.point,
        };
      }
    });
  });
  if (best) return best;

  // 5. Fill — clicking inside a filled path's body selects it (front-most first). Skips
  //    stroke-only paths (fill="none"); an absent fill counts as filled (SVG default).
  for (let pathIndex = doc.paths.length - 1; pathIndex >= 0; pathIndex--) {
    const p = doc.paths[pathIndex];
    if (p.deleted || p.locked || defUids.has(p.uid ?? "")) continue;
    const fill = p.styleOverride?.fill ?? p.attributes?.fill;
    if (fill === "none") continue;
    if (pointInPath(p.subpaths, docPoint)) return { kind: "fill", pathIndex };
  }

  return { kind: "empty" };
}

/** "rgb(59, 130, 246)" → "#3b82f6". `null` for anything that isn't an rgb triple. */
function rgbToHex(css: string): string | null {
  const m = css.match(/rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i);
  if (!m) return null;
  const hex = m
    .slice(1, 4)
    .map((n) => Math.round(Number(n)).toString(16).padStart(2, "0"))
    .join("");
  return `#${hex}`;
}

/**
 * A sampled paint, resolved to an actual colour.
 *
 * An eyedropper's job is "give me *that* colour", so handing back a paint that only *refers* to one
 * defeats it. `currentColor` is the case that bites: it's the default stroke every pen-drawn path
 * carries, so sampling your own strokes used to fill the field with the literal word — a value that
 * looks like whatever the theme's text colour is and changes when the theme does.
 *
 * `currentColor` resolves to what it actually renders as, read off the artwork it inherits from; a
 * gradient resolves to its first stop, which is the honest single answer to "what colour is that";
 * a plain colour (hex, or a named one like `red`) is already an answer.
 */
function resolveSampled(paint: string): string | null {
  const v = paint.trim();
  if (!v || v === "none") return null;
  if (v.startsWith("url(")) {
    const id = v.slice(v.indexOf("#") + 1, v.lastIndexOf(")"));
    const stop =
      editor.gradientById(id)?.stops[0]?.color ?? editor.importedGradients.get(id)?.stops[0]?.color;
    return stop ?? null;
  }
  if (v === "currentColor" || v === "inherit") {
    const el = typeof document === "undefined" ? null : document.querySelector("svg.canvas");
    const computed = el ? getComputedStyle(el).color : "";
    return rgbToHex(computed);
  }
  return v;
}

/** Distance from `docPoint` to a path's outline, in document units. `Infinity` if it has none. */
function distanceToOutline(subpaths: Subpath[], docPoint: Point): number {
  let best = Infinity;
  for (const sp of subpaths) {
    const hit = nearestOnSubpath(sp, docPoint);
    if (hit && hit.distance < best) best = hit.distance;
  }
  return best;
}

/**
 * The colour under `docPoint` for the eyedropper — what is actually *rendered* there, taken from
 * the front-most shape that paints the point. Includes locked shapes (sampling is read-only).
 * `null` = nothing there.
 *
 * The subtlety is unfilled shapes. `pointInPath` is a winding test on geometry and knows nothing
 * about paint, so a `fill="none"` shape "contains" every point inside its outline even though you
 * can see straight through it. A picture frame drawn as an unfilled rect and brought to the front
 * therefore answered for the entire scene: every click landed on the frame, fell past its `none`
 * fill, and returned its stroke — the same colour everywhere, whatever you pointed at.
 *
 * So a shape only answers where it paints: its fill anywhere inside, its stroke only within the
 * stroke's own width of the outline. Otherwise the point is see-through here and the shape below
 * gets asked.
 */
export function sampleAt(docPoint: Point): { color: string; from: string } | null {
  const doc = editor.doc;
  if (!doc) return null;
  const defUids = editor.defPathUids;
  // A couple of screen pixels of forgiveness, so a thin stroke is still pickable when zoomed out.
  const slack = viewport.toDocLength(2);
  for (let i = doc.paths.length - 1; i >= 0; i--) {
    const p = doc.paths[i];
    if (p.deleted || defUids.has(p.uid ?? "")) continue;
    const style = (k: string) => p.styleOverride?.[k] ?? p.attributes?.[k];
    const from = p.id || `#${i}`;
    const got = (c: string | null) => (c ? { color: c, from } : null);
    const fill = style("fill") ?? "#000000";
    if (fill !== "none" && pointInPath(p.subpaths, docPoint)) return got(resolveSampled(fill));
    const stroke = style("stroke");
    if (stroke && stroke !== "none") {
      const width = Number(style("stroke-width") ?? "1");
      const reach = (Number.isFinite(width) ? width : 1) / 2 + slack;
      if (distanceToOutline(p.subpaths, docPoint) <= reach) return got(resolveSampled(stroke));
    }
    // see-through here; keep looking at the shape below.
  }
  return null;
}
