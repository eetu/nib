import { createActor } from "xstate";

import { type CanvasEvent, canvasMachine } from "$lib/canvas/machine";

/**
 * Runes wrapper around the canvas gesture statechart: mirrors the machine's
 * state into a reactive `mode` and forwards pointer events. EditorCanvas sends
 * DOWN/MOVE/UP/CANCEL; the machine orchestrates drag/pan (see canvas/machine).
 */
class CanvasInteraction {
  #actor = createActor(canvasMachine);
  mode = $state<"idle" | "panning" | "dragging">("idle");

  constructor() {
    this.#actor.subscribe((snap) => {
      const value = String(snap.value);
      if (value === "panning" || value === "dragging" || value === "idle") this.mode = value;
    });
    this.#actor.start();
  }

  get idle(): boolean {
    return this.mode === "idle";
  }
  get panning(): boolean {
    return this.mode === "panning";
  }
  get dragging(): boolean {
    return this.mode === "dragging";
  }

  send(event: CanvasEvent): void {
    this.#actor.send(event);
  }
}

export const canvas = new CanvasInteraction();
