import type { NodeRef, Point } from "$lib/model/types";

/**
 * Transient, per-gesture UI feedback the overlay reads while a drag is in
 * flight — kept out of the document model (nothing here is undoable).
 */
class Interaction {
  /** Point the dragged node is currently snapping to, for the snap ring. */
  snapPoint = $state<Point | null>(null);
  /** True when the current snap would close a loop (shows the closing hint). */
  closing = $state(false);
  /** Node the pointer is hovering (highlight before you grab it). */
  hover = $state<NodeRef | null>(null);

  /** Space bar held → the canvas pans instead of editing. */
  spaceHeld = $state(false);

  /** The pen tool is mid-path (between the first anchor and finishing). */
  penDrawing = $state(false);
  /** Live pointer position while drawing, for the rubber-band to the cursor. */
  penCursor = $state<Point | null>(null);

  clearDrag(): void {
    this.snapPoint = null;
    this.closing = false;
  }
}

export const interaction = new Interaction();
