import { editor } from "$lib/stores/document.svelte";

import type { Tool } from "./types";

/** Click a segment to insert an anchor there (curve shape preserved). */
export const addNodeTool: Tool = {
  id: "add-node",
  cursor(hit) {
    if (hit.kind === "segment") return "copy"; // insert an anchor here
    if (hit.kind === "anchor") return "pointer"; // clicking selects this node
    return "default";
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
