import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";
import { tools } from "$lib/stores/tool.svelte";

import { MIN_SHAPE, snapBypassed, snapPoint } from "./shape-util";
import type { Tool } from "./types";

/** Draw a rectangle: press one corner, drag to the opposite one. A closed 4-corner path (or 8
 *  rounded ones when the tool's corner radius is set). */
export const rectTool: Tool = {
  id: "rect",
  cursor: () => "crosshair",
  hover(docPoint) {
    const { point, snapped } = snapPoint(docPoint);
    interaction.snapPoint = snapped ? point : null;
    interaction.closing = false;
  },
  begin(ctx) {
    editor.ensureBlank();
    if (!editor.doc) return null;
    const r = tools.cornerRadius;
    const a = snapPoint(ctx.docPoint, snapBypassed(ctx.event)).point;
    const ref = editor.beginShape({
      shape: "rect",
      x0: a.x,
      y0: a.y,
      x1: a.x,
      y1: a.y,
      rx: r,
      ry: r,
    });
    let b = a;
    return {
      move(docPoint, event) {
        const s = snapPoint(docPoint, snapBypassed(event));
        b = s.point;
        // Shift constrains to a square — the larger side sets both, keeping the drag direction.
        if (event.shiftKey) {
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const m = Math.max(Math.abs(dx), Math.abs(dy));
          b = { x: a.x + (dx < 0 ? -m : m), y: a.y + (dy < 0 ? -m : m) };
          interaction.snapPoint = null; // the squared corner is off-snap
        } else {
          interaction.snapPoint = s.snapped ? b : null; // ring the snapped corner, like the circle tool
        }
        editor.setShape(ref.pathIndex, ref.subpathIndex, {
          shape: "rect",
          x0: a.x,
          y0: a.y,
          x1: b.x,
          y1: b.y,
          rx: r,
          ry: r,
        });
      },
      up() {
        interaction.snapPoint = null;
        if (Math.abs(b.x - a.x) < MIN_SHAPE && Math.abs(b.y - a.y) < MIN_SHAPE) editor.revert();
        else editor.commit();
      },
      cancel() {
        interaction.snapPoint = null;
        editor.revert();
      },
    };
  },
};
