import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";

import { MIN_SHAPE, snapPoint, snapRadius } from "./shape-util";
import type { Tool } from "./types";

const POINTS = 5;
const INNER_RATIO = 0.5;
const START = -Math.PI / 2; // a point points up

/** Draw a star: press at the centre, drag out to the outer radius (inner radius follows). */
export const starTool: Tool = {
  id: "star",
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
      shape: "star",
      cx: c.x,
      cy: c.y,
      outer: 0,
      inner: 0,
      points: POINTS,
      rotation: START,
    });
    let r = 0;
    return {
      move(docPoint) {
        r = snapRadius(c, docPoint);
        editor.setShape(ref.pathIndex, ref.subpathIndex, {
          shape: "star",
          cx: c.x,
          cy: c.y,
          outer: r,
          inner: r * INNER_RATIO,
          points: POINTS,
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
