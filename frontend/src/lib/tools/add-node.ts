import { editor } from "$lib/stores/document.svelte";

import type { Tool } from "./types";

/** Click a segment to insert an anchor there (curve shape preserved). */
export const addNodeTool: Tool = {
  id: "add-node",
  cursor(hit) {
    return hit.kind === "segment" ? "copy" : "default";
  },
  begin(ctx) {
    const h = ctx.hit;
    if (h.kind === "segment") {
      editor.insertNode(h.pathIndex, h.subpathIndex, h.segmentIndex, h.t);
    } else if (h.kind === "anchor") {
      editor.select(h.ref);
    }
    return null;
  },
};
