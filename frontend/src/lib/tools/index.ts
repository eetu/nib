import Circle from "@lucide/svelte/icons/circle";
import Eraser from "@lucide/svelte/icons/eraser";
import Hexagon from "@lucide/svelte/icons/hexagon";
import MousePointer2 from "@lucide/svelte/icons/mouse-pointer-2";
import PenTool from "@lucide/svelte/icons/pen-tool";
import Pipette from "@lucide/svelte/icons/pipette";
import Plus from "@lucide/svelte/icons/plus";
import Slash from "@lucide/svelte/icons/slash";
import Square from "@lucide/svelte/icons/square";
import Star from "@lucide/svelte/icons/star";
import Type from "@lucide/svelte/icons/type";
import type { Component } from "svelte";

import type { ToolId } from "$lib/stores/tool.svelte";

import { addNodeTool } from "./add-node";
import { circleTool } from "./circle";
import { deleteNodeTool } from "./delete-node";
import { eyedropperTool } from "./eyedropper";
import { lineTool } from "./line";
import { penTool } from "./pen";
import { polygonTool } from "./polygon";
import { rectTool } from "./rect";
import { selectTool } from "./select";
import { starTool } from "./star";
import { textTool } from "./text";
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
 *  more than one tool (so the rail stays scannable as shape primitives grow). `advanced`
 *  groups are hidden in the basic UI level (touch-up mode) — the engine still has them. */
export type ToolGroup = {
  name: string;
  flyout?: boolean;
  advanced?: boolean;
  tools: ToolDef[];
};

export const TOOL_GROUPS: ToolGroup[] = [
  {
    name: "select",
    tools: [
      {
        id: "select",
        tool: selectTool,
        label: "select & move",
        shortcut: "v",
        icon: MousePointer2,
      },
    ],
  },
  {
    name: "draw",
    tools: [{ id: "pen", tool: penTool, label: "pen", shortcut: "p", icon: PenTool }],
  },
  {
    name: "text",
    tools: [{ id: "text", tool: textTool, label: "text", shortcut: "t", icon: Type }],
  },
  {
    name: "eyedropper",
    tools: [
      {
        id: "eyedropper",
        tool: eyedropperTool,
        label: "eyedropper (sample colour)",
        shortcut: "i",
        icon: Pipette,
      },
    ],
  },
  {
    // Shape primitives — rect / line / polygon / star slot in here and the group becomes a
    // flyout automatically once there's more than one. Creating primitives is a from-scratch
    // (advanced) activity, so the group is hidden in basic (touch-up) mode.
    name: "shapes",
    flyout: true,
    advanced: true,
    tools: [
      { id: "circle", tool: circleTool, label: "circle", shortcut: "c", icon: Circle },
      { id: "rect", tool: rectTool, label: "rectangle", shortcut: "r", icon: Square },
      { id: "line", tool: lineTool, label: "line", shortcut: "l", icon: Slash },
      { id: "polygon", tool: polygonTool, label: "polygon", shortcut: "g", icon: Hexagon },
      { id: "star", tool: starTool, label: "star", shortcut: "s", icon: Star },
    ],
  },
  {
    name: "nodes",
    tools: [
      { id: "add-node", tool: addNodeTool, label: "add node", shortcut: "a", icon: Plus },
      {
        id: "delete-node",
        tool: deleteNodeTool,
        label: "delete node",
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

/** Tool ids that live in an `advanced` group — hidden (and their shortcuts inert) in the
 *  basic UI level, so a basic user never activates an off-screen tool. */
export const ADVANCED_TOOL_IDS: Set<ToolId> = new Set(
  TOOL_GROUPS.filter((g) => g.advanced).flatMap((g) => g.tools.map((t) => t.id)),
);

export { hitTest } from "./hit";
export { finishPen } from "./pen";
export type { DragSession, Hit, Tool, ToolContext } from "./types";
