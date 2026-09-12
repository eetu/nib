import { editor } from "$lib/stores/document.svelte";
import { interaction } from "$lib/stores/interaction.svelte";
import { tools } from "$lib/stores/tool.svelte";

import { sampleAt } from "./hit";
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
    const got = sampleAt(ctx.docPoint);
    if (got) editor.applySampledPaint(got.color, tools.eyedropperTarget);
    interaction.loupe = null;
    tools.release(); // back to whatever was armed before — the pen, usually
    return null;
  },
  /**
   * The loupe: what this click would take, shown before it's taken.
   *
   * A magnifier over *pixels* is the usual shape of this, and it would be the wrong one here —
   * nib samples the MODEL, so a zoomed view of antialiased edges would show colours it can never
   * return. What it can say instead is better: the exact colour, and which named shape it comes
   * from. That turns a guess over a busy drawing into a read.
   */
  hover(docPoint) {
    const got = sampleAt(docPoint);
    interaction.loupe = got ? { at: docPoint, ...got } : null;
  },
  onDeactivate() {
    interaction.loupe = null;
  },
};
