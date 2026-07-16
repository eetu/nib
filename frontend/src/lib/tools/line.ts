import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";

import { MIN_SHAPE, snapPoint } from "./shape-util";
import type { Tool } from "./types";

/** Draw a straight line: press one end, drag to the other. An open 2-node path. */
export const lineTool: Tool = {
  id: "line",
  cursor: () => "crosshair",
  hover(docPoint) {
    const { point, snapped } = snapPoint(docPoint);
    interaction.snapPoint = snapped ? point : null;
    interaction.closing = false;
  },
  begin(ctx) {
    editor.ensureBlank();
    if (!editor.doc) return null;
    const a = snapPoint(ctx.docPoint).point;
    const ref = editor.beginShape({ shape: "line", x0: a.x, y0: a.y, x1: a.x, y1: a.y });
    let b = a;
    return {
      move(docPoint, event) {
        const s = snapPoint(docPoint);
        b = s.point;
        // Shift snaps the line's angle to 45° steps, keeping its length.
        if (event.shiftKey) {
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const len = Math.hypot(dx, dy);
          const step = Math.PI / 4;
          const ang = Math.round(Math.atan2(dy, dx) / step) * step;
          b = { x: a.x + Math.cos(ang) * len, y: a.y + Math.sin(ang) * len };
          interaction.snapPoint = null; // the constrained endpoint is off-snap
        } else {
          interaction.snapPoint = s.snapped ? b : null; // ring the snapped endpoint, like the circle tool
        }
        editor.setShape(ref.pathIndex, ref.subpathIndex, {
          shape: "line",
          x0: a.x,
          y0: a.y,
          x1: b.x,
          y1: b.y,
        });
      },
      up() {
        interaction.snapPoint = null;
        if (Math.hypot(b.x - a.x, b.y - a.y) < MIN_SHAPE) editor.revert();
        else editor.commit();
      },
      cancel() {
        interaction.snapPoint = null;
        editor.revert();
      },
    };
  },
};
