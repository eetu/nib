import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";

import { sampleFillAt } from "./hit";
import type { Tool } from "./types";

/** Eyedropper: click to sample the colour of the shape under the cursor and apply it to the current
 *  selection's fill (or the new-shape default when nothing is selected). Drops back to select after
 *  a pick, like Illustrator/Figma. */
export const eyedropperTool: Tool = {
  id: "eyedropper",
  cursor: () => "crosshair",
  begin(ctx) {
    const color = sampleFillAt(ctx.docPoint);
    if (color) editor.applySampledFill(color);
    tools.set("select");
    return null;
  },
};
