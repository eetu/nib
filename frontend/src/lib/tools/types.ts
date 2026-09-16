import type { NodeRef, Point } from "$lib/model/types";
import type { ToolId } from "$lib/stores/tool.svelte";

import type { TransformHandle } from "./transform";

/**
 * Tools that work on the selection's BOX rather than on its nodes.
 *
 * While one is active, anchors are neither drawn nor hit-tested. That's not decluttering for its
 * own sake: a circle's four anchors sit exactly on its box's edge-midpoint handles, and anchors
 * are hit-tested first (so a path's own nodes are never shadowed by a transform handle). Without
 * this the tilt tool simply could not grab an ellipse — every edge handle would answer as an
 * anchor. It's the same rule the select tool already follows in object mode, named.
 */
export const TRANSFORM_TOOLS: ReadonlySet<ToolId> = new Set<ToolId>(["rotate", "distort"]);

/** What sits under the pointer at pointerdown, in priority order. */
export type Hit =
  | { kind: "handle"; ref: NodeRef; which: "in" | "out" }
  | { kind: "transform"; handle: TransformHandle }
  | { kind: "rotate" }
  | { kind: "anchor"; ref: NodeRef }
  | {
      kind: "segment";
      pathIndex: number;
      subpathIndex: number;
      segmentIndex: number;
      t: number;
      point: Point;
    }
  | { kind: "fill"; pathIndex: number }
  | { kind: "empty" };

export type ToolContext = {
  hit: Hit;
  /** Pointer position in document coordinates. */
  docPoint: Point;
  event: PointerEvent;
};

/** An in-flight drag. The canvas forwards pointer moves/up here until release. */
export type DragSession = {
  move(docPoint: Point, event: PointerEvent): void;
  up(docPoint: Point): void;
  cancel(): void;
};

export type Tool = {
  id: ToolId;
  /** CSS cursor for the current hit. */
  cursor(hit: Hit): string;
  /** Handle a pointerdown: perform any click action and optionally return a
   *  drag session to receive subsequent moves. */
  begin(ctx: ToolContext): DragSession | null;
  /** Called on pointer move when no drag is active — for live aids (snap
   *  indicator, rubber-band). `docPoint` is the pointer in document units. */
  hover?(docPoint: Point): void;
  /** Called when the user switches away from this tool — for cleanup (e.g. the pen
   *  finishing its in-progress path). Driven centrally when `tools.active` changes. */
  onDeactivate?(): void;
};
