import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";

import { sampleFillAt } from "./hit";
import type { Tool } from "./types";

/** Eyedropper: click to sample the colour of the shape under the cursor and apply it to the current
 *  selection (or the new-shape default when nothing is selected). Drops back to select after a
 *  pick, like Illustrator/Figma.
 *
 *  Which paint it fills is `tools.eyedropperTarget`, set by whichever button armed it — so the one
 *  beside `stroke` samples a stroke colour. The keyboard shortcut leaves it at `fill`, the answer
 *  to "eyedropper" with nothing else said. */
export const eyedropperTool: Tool = {
  id: "eyedropper",
  cursor: () => "crosshair",
  begin(ctx) {
    const color = sampleFillAt(ctx.docPoint);
    if (color) editor.applySampledPaint(color, tools.eyedropperTarget);
    tools.release(); // back to whatever was armed before — the pen, usually
    return null;
  },
};
