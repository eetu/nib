import type { Point, ViewBox } from "$lib/model/types";

const MIN_SCALE = 0.02;
const MAX_SCALE = 400;

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/**
 * Maps document coordinates to on-screen pixels: `screen = doc * scale + t`.
 * The artwork is drawn in a scaled SVG group; the editing overlay is drawn in
 * screen space (via toScreen) so handles stay a constant pixel size at any zoom.
 */
class Viewport {
  scale = $state(1);
  tx = $state(0);
  ty = $state(0);
  /** Canvas size in pixels, kept current by EditorCanvas. */
  pxWidth = $state(0);
  pxHeight = $state(0);

  toScreen(p: Point): Point {
    return { x: p.x * this.scale + this.tx, y: p.y * this.scale + this.ty };
  }

  toDoc(p: Point): Point {
    return { x: (p.x - this.tx) / this.scale, y: (p.y - this.ty) / this.scale };
  }

  /** Convert a pixel length to document units (snap thresholds, hit radii). */
  toDocLength(px: number): number {
    return px / this.scale;
  }

  setSize(pxW: number, pxH: number): void {
    this.pxWidth = pxW;
    this.pxHeight = pxH;
  }

  /** Center + scale a document viewBox to fill the given pixel area. */
  fit(vb: ViewBox, pxW: number, pxH: number, pad = 0.9): void {
    if (vb.width <= 0 || vb.height <= 0 || pxW <= 0 || pxH <= 0) return;
    this.scale = clamp(Math.min(pxW / vb.width, pxH / vb.height) * pad, MIN_SCALE, MAX_SCALE);
    this.tx = pxW / 2 - (vb.minX + vb.width / 2) * this.scale;
    this.ty = pxH / 2 - (vb.minY + vb.height / 2) * this.scale;
  }

  /** Fit a viewBox using the current canvas size. */
  fitDocument(vb: ViewBox): void {
    this.fit(vb, this.pxWidth, this.pxHeight);
  }

  /** Zoom by `factor`, keeping the document point under `screen` fixed. */
  zoomAt(screen: Point, factor: number): void {
    const next = clamp(this.scale * factor, MIN_SCALE, MAX_SCALE);
    const k = next / this.scale;
    this.tx = screen.x - (screen.x - this.tx) * k;
    this.ty = screen.y - (screen.y - this.ty) * k;
    this.scale = next;
  }

  panBy(dxPx: number, dyPx: number): void {
    this.tx += dxPx;
    this.ty += dyPx;
  }
}

export const viewport = new Viewport();
