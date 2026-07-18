import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";

import type { Tool } from "./types";

/** Place a text label: click to drop a `<text>` element at the cursor, then edit its content, size,
 *  and fill in the Inspector's element section. Drops back to the select tool so the new label is
 *  immediately movable / editable. */
export const textTool: Tool = {
  id: "text",
  cursor: () => "text",
  begin(ctx) {
    editor.ensureBlank();
    if (!editor.doc) return null;
    editor.addText(ctx.docPoint.x, ctx.docPoint.y, "Text");
    tools.set("select");
    return null;
  },
};
