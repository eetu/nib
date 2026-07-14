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
      move(docPoint) {
        b = snapPoint(docPoint).point;
        editor.setShape(ref.pathIndex, ref.subpathIndex, {
          shape: "line",
          x0: a.x,
          y0: a.y,
          x1: b.x,
          y1: b.y,
        });
        interaction.snapPoint = null;
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
