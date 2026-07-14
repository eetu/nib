import Circle from "@lucide/svelte/icons/circle";
import Eraser from "@lucide/svelte/icons/eraser";
import MousePointer2 from "@lucide/svelte/icons/mouse-pointer-2";
import PenTool from "@lucide/svelte/icons/pen-tool";
import Plus from "@lucide/svelte/icons/plus";
import type { Component } from "svelte";

import type { ToolId } from "$lib/stores/tool.svelte";

import { addNodeTool } from "./add-node";
import { circleTool } from "./circle";
import { deleteNodeTool } from "./delete-node";
import { penTool } from "./pen";
import { selectTool } from "./select";
import type { Tool } from "./types";

/** One tool's full definition — behavior + everything the rail/shortcuts need. This is the
 *  single place a tool is registered: add an entry here (and its `id` to `ToolId`) and it is
 *  wired everywhere (getTool, the rail button, its group/flyout, and its shortcut). */
export type ToolDef = {
  id: ToolId;
  tool: Tool;
  label: string;
  /** Single-key shortcut (lowercased), if any. */
  shortcut?: string;
  icon: Component;
};

/** A rail section. `flyout` groups collapse into one rail slot with a popup once they hold
 *  more than one tool (so the rail stays scannable as shape primitives grow). */
export type ToolGroup = {
  name: string;
  flyout?: boolean;
  tools: ToolDef[];
};

export const TOOL_GROUPS: ToolGroup[] = [
  {
    name: "select",
    tools: [
      {
        id: "select",
        tool: selectTool,
        label: "Select & move",
        shortcut: "v",
        icon: MousePointer2,
      },
    ],
  },
  {
    name: "draw",
    tools: [{ id: "pen", tool: penTool, label: "Draw path / pen", shortcut: "p", icon: PenTool }],
  },
  {
    // Shape primitives — rect / line / polygon / star slot in here and the group becomes a
    // flyout automatically once there's more than one.
    name: "shapes",
    flyout: true,
    tools: [{ id: "circle", tool: circleTool, label: "Circle", shortcut: "c", icon: Circle }],
  },
  {
    name: "nodes",
    tools: [
      { id: "add-node", tool: addNodeTool, label: "Add node", shortcut: "a", icon: Plus },
      {
        id: "delete-node",
        tool: deleteNodeTool,
        label: "Delete node",
        shortcut: "d",
        icon: Eraser,
      },
    ],
  },
];

const ALL: ToolDef[] = TOOL_GROUPS.flatMap((g) => g.tools);

const REGISTRY = Object.fromEntries(ALL.map((d) => [d.id, d.tool])) as Record<ToolId, Tool>;

export function getTool(id: ToolId): Tool {
  return REGISTRY[id];
}

/** Keyboard shortcut → tool id, derived from the definitions above. */
export const toolShortcuts: Record<string, ToolId> = {};
for (const d of ALL) {
  if (d.shortcut) toolShortcuts[d.shortcut] = d.id;
}

export { hitTest } from "./hit";
export { finishPen } from "./pen";
export type { DragSession, Hit, Tool, ToolContext } from "./types";
