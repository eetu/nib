import { normalize } from "$lib/model/geometry";
import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";
import { tools } from "$lib/stores/tool.svelte";

import { MIN_SHAPE, snapBypassed, snapPoint, snapRadius } from "./shape-util";
import type { Tool } from "./types";

/** Draw a circle: press at the centre, drag out to the radius. The result is a closed 4-node
 *  bezier path, editable like any other. Hold ⌘/Ctrl to bypass centre + radius snapping. */
export const circleTool: Tool = {
  id: "circle",
  cursor: () => "crosshair",
  hover(docPoint) {
    const { point, snapped } = snapPoint(docPoint);
    interaction.snapPoint = snapped ? point : null;
    interaction.closing = false;
  },
  begin(ctx) {
    editor.ensureBlank();
    if (!editor.doc) return null;
    const c = snapPoint(ctx.docPoint, snapBypassed(ctx.event)).point;
    const ref = editor.beginShape({ shape: "ellipse", cx: c.x, cy: c.y, rx: 0, ry: 0 });
    let radius = 0;
    return {
      move(docPoint, event) {
        const bypass = snapBypassed(event);
        radius = snapRadius(c, docPoint, bypass);
        editor.setShape(ref.pathIndex, ref.subpathIndex, {
          shape: "ellipse",
          cx: c.x,
          cy: c.y,
          rx: radius,
          ry: radius,
        });
        if (!bypass && tools.gridEnabled && radius > 0) {
          const dir = normalize({ x: docPoint.x - c.x, y: docPoint.y - c.y });
          interaction.snapPoint = { x: c.x + dir.x * radius, y: c.y + dir.y * radius };
        } else {
          interaction.snapPoint = null;
        }
      },
      up() {
        interaction.snapPoint = null;
        if (radius < MIN_SHAPE) editor.revert();
        else editor.commit();
      },
      cancel() {
        interaction.snapPoint = null;
        editor.revert();
      },
    };
  },
};
