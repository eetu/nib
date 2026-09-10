import type { Point } from "$lib/model/types";

/**
 * Transient, per-gesture UI feedback the overlay reads while a drag is in
 * flight — kept out of the document model (nothing here is undoable).
 */
class Interaction {
  /** Point the dragged node is currently snapping to, for the snap ring. */
  snapPoint = $state<Point | null>(null);
  /** True when the current snap would close a loop (shows the closing hint). */
  closing = $state(false);

  /** Space bar held → the canvas pans instead of editing. */
  spaceHeld = $state(false);

  /** The pen tool is mid-path (between the first anchor and finishing). */
  penDrawing = $state(false);
  /** Live pointer position while drawing, for the rubber-band to the cursor. */
  penCursor = $state<Point | null>(null);
  /** Open endpoint the pen would resume from if clicked (hover affordance). */
  resumePoint = $state<Point | null>(null);

  /** Active smart-guide lines while dragging (document coords): vertical guides at these x,
   *  horizontal guides at these y. Drawn full-canvas by the overlay. */
  guidesX = $state<number[]>([]);
  guidesY = $state<number[]>([]);

  /** Rubber-band marquee rectangle (document coords) while selecting over empty canvas. */
  marquee = $state<{ x0: number; y0: number; x1: number; y1: number } | null>(null);

  /**
   * A rotation in flight: the selection box as it stood when the drag began, the point it turns
   * about, and how far it has turned (radians, clockwise).
   *
   * The overlay draws *this* box, turned — because the alternative is what it did before: derive
   * an axis-aligned box from geometry that is mid-rotation, which makes the box breathe wider and
   * narrower while the handles sit still. A box that turns with the shape says what is happening;
   * a box that changes size says something else is.
   */
  rotation = $state<{
    bounds: { minX: number; minY: number; maxX: number; maxY: number };
    pivot: Point;
    angle: number;
  } | null>(null);

  clearDrag(): void {
    this.rotation = null;
    this.snapPoint = null;
    this.closing = false;
    this.resumePoint = null;
    this.guidesX = [];
    this.guidesY = [];
    this.marquee = null;
  }
}

export const interaction = new Interaction();
