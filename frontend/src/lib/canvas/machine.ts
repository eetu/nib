import { assign, setup } from "xstate";

import type { Point } from "$lib/model/types";
import { editor } from "$lib/stores/document.svelte";
import { tools } from "$lib/stores/tool.svelte";
import { viewport } from "$lib/stores/viewport.svelte";
import { type DragSession, getTool, type Hit } from "$lib/tools";

// The canvas gesture lifecycle as an explicit statechart. It replaces the
// scattered drag/pan bookkeeping in EditorCanvas with named states:
//
//   idle ──DOWN(pan)────────────────▶ panning ──UP/CANCEL──▶ idle
//   idle ──DOWN──▶ (gesture) ──has session?──▶ dragging ──UP/CANCEL──▶ idle
//                              └──no session──▶ idle   (a click action ran)
//
// Tools stay the units of behaviour: a tool's begin() performs any click action
// and optionally returns a DragSession; the machine just owns *when* move/up/
// cancel are delivered to it, and drives panning. Hover + cursor live in the
// component (presentational, not part of a gesture).

export type CanvasContext = {
  session: DragSession | null;
  panFrom: Point | null;
};

export type CanvasEvent =
  | { type: "DOWN"; hit: Hit; docPoint: Point; event: PointerEvent; pan: boolean; screen: Point }
  | { type: "MOVE"; docPoint: Point; screen: Point; event: PointerEvent }
  | { type: "UP"; docPoint: Point }
  | { type: "CANCEL" };

export const canvasMachine = setup({
  types: {
    context: {} as CanvasContext,
    events: {} as CanvasEvent,
  },
  guards: {
    isPan: ({ event }) => event.type === "DOWN" && event.pan,
    hasSession: ({ context }) => context.session !== null,
  },
  actions: {
    startPan: assign({ panFrom: ({ event }) => (event.type === "DOWN" ? event.screen : null) }),
    trackPan: assign({ panFrom: ({ event }) => (event.type === "MOVE" ? event.screen : null) }),
    clearPan: assign({ panFrom: null }),
    doPan: ({ context, event }) => {
      if (event.type !== "MOVE" || !context.panFrom) return;
      viewport.panBy(event.screen.x - context.panFrom.x, event.screen.y - context.panFrom.y);
    },
    beginTool: assign({
      session: ({ event }) => {
        if (event.type !== "DOWN" || !editor.hasDocument) return null;
        return getTool(tools.active).begin({
          hit: event.hit,
          docPoint: event.docPoint,
          event: event.event,
        });
      },
    }),
    clearSession: assign({ session: null }),
    dragMove: ({ context, event }) => {
      if (event.type === "MOVE") context.session?.move(event.docPoint, event.event);
    },
    dragUp: ({ context, event }) => {
      if (event.type === "UP") context.session?.up(event.docPoint);
    },
    dragCancel: ({ context }) => context.session?.cancel(),
  },
}).createMachine({
  id: "canvas",
  context: { session: null, panFrom: null },
  initial: "idle",
  states: {
    idle: {
      on: {
        DOWN: [
          { guard: "isPan", target: "panning", actions: "startPan" },
          { target: "gesture", actions: "beginTool" },
        ],
      },
    },
    // Transient: a click either began a drag (→ dragging) or just ran an action.
    gesture: {
      always: [{ guard: "hasSession", target: "dragging" }, { target: "idle" }],
    },
    panning: {
      on: {
        MOVE: { actions: ["doPan", "trackPan"] },
        UP: { target: "idle", actions: "clearPan" },
        CANCEL: { target: "idle", actions: "clearPan" },
      },
    },
    dragging: {
      on: {
        MOVE: { actions: "dragMove" },
        UP: { target: "idle", actions: ["dragUp", "clearSession"] },
        CANCEL: { target: "idle", actions: ["dragCancel", "clearSession"] },
      },
    },
  },
});
