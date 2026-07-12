import type { ToolId } from "$lib/stores/tool.svelte";

import { addNodeTool } from "./add-node";
import { circleTool } from "./circle";
import { deleteNodeTool } from "./delete-node";
import { penTool } from "./pen";
import { selectTool } from "./select";
import type { Tool } from "./types";

const REGISTRY: Record<ToolId, Tool> = {
  select: selectTool,
  pen: penTool,
  circle: circleTool,
  "add-node": addNodeTool,
  "delete-node": deleteNodeTool,
};

export function getTool(id: ToolId): Tool {
  return REGISTRY[id];
}

/** Order + labels for the tool rail (icons are chosen in the component). */
export const TOOL_LIST: { id: ToolId; label: string }[] = [
  { id: "select", label: "Select & move" },
  { id: "pen", label: "Draw path (pen)" },
  { id: "circle", label: "Circle" },
  { id: "add-node", label: "Add node" },
  { id: "delete-node", label: "Delete node" },
];

export { hitTest } from "./hit";
export { finishPen } from "./pen";
export type { DragSession, Hit, Tool, ToolContext } from "./types";
