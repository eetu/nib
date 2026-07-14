import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";

import { MIN_SHAPE, snapPoint, snapRadius } from "./shape-util";
import type { Tool } from "./types";

const SIDES = 6;
const START = -Math.PI / 2; // a vertex points up

/** Draw a regular polygon: press at the centre, drag out to the radius. */
export const polygonTool: Tool = {
  id: "polygon",
  cursor: () => "crosshair",
  hover(docPoint) {
    const { point, snapped } = snapPoint(docPoint);
    interaction.snapPoint = snapped ? point : null;
    interaction.closing = false;
  },
  begin(ctx) {
    editor.ensureBlank();
    if (!editor.doc) return null;
    const c = snapPoint(ctx.docPoint).point;
    const ref = editor.beginShape({
      shape: "polygon",
      cx: c.x,
      cy: c.y,
      r: 0,
      sides: SIDES,
      rotation: START,
    });
    let r = 0;
    return {
      move(docPoint) {
        r = snapRadius(c, docPoint);
        editor.setShape(ref.pathIndex, ref.subpathIndex, {
          shape: "polygon",
          cx: c.x,
          cy: c.y,
          r,
          sides: SIDES,
          rotation: START,
        });
      },
      up() {
        if (r < MIN_SHAPE) editor.revert();
        else editor.commit();
      },
      cancel() {
        editor.revert();
      },
    };
  },
};
