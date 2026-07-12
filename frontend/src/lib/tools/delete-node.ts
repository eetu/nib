import { editor } from "$lib/stores/document.svelte";

import type { Tool } from "./types";

/** Click an anchor to remove it (the neighbours rejoin). */
export const deleteNodeTool: Tool = {
  id: "delete-node",
  cursor(hit) {
    return hit.kind === "anchor" ? "pointer" : "default";
  },
  begin(ctx) {
    if (ctx.hit.kind === "anchor") editor.deleteNode(ctx.hit.ref);
    return null;
  },
};
