import { debounce, loadState, saveState } from "$lib/persistence";

export type ToolId =
  "select" | "pen" | "circle" | "rect" | "line" | "polygon" | "star" | "add-node" | "delete-node";

/** Style applied to the next drawn path/shape (editable up front via the
 *  "new shape style" panel; also the reset defaults). */
const DEFAULT_STYLE: Record<string, string> = {
  fill: "none",
  stroke: "currentColor",
  "stroke-width": "2",
  "stroke-linecap": "round",
  "stroke-linejoin": "round",
};

type Prefs = {
  snapEnabled: boolean;
  snapThresholdPx: number;
  gridEnabled: boolean;
  gridSize: number;
  guidesEnabled: boolean;
  newStyle: Record<string, string>;
  cornerRadius: number;
};

const PREFS_KEY = "prefs";

/** The active editing tool plus the snap/grid settings the tools consult and the
 *  style new shapes are created with. Persisted (they're preferences); the
 *  active tool resets to select. */
class ToolState {
  active = $state<ToolId>("select");

  snapEnabled = $state(true);
  /** Snap radius in screen pixels (converted to doc units via the viewport). */
  snapThresholdPx = $state(12);

  gridEnabled = $state(false);
  gridSize = $state(10);

  /** Smart alignment guides while dragging shapes (edges/centres snap to other shapes + the
   *  canvas). */
  guidesEnabled = $state(true);

  /** Presentation attributes stamped onto pen/circle paths at creation. */
  newStyle = $state<Record<string, string>>({ ...DEFAULT_STYLE });

  /** Corner radius (doc units) the rect tool draws with — 0 = sharp. Persisted like the style. */
  cornerRadius = $state(0);

  constructor() {
    const p = loadState<Prefs>(PREFS_KEY);
    if (p) {
      this.snapEnabled = p.snapEnabled;
      this.snapThresholdPx = p.snapThresholdPx;
      this.gridEnabled = p.gridEnabled;
      this.gridSize = p.gridSize;
      this.guidesEnabled = p.guidesEnabled ?? true;
      if (p.newStyle) this.newStyle = p.newStyle;
      this.cornerRadius = p.cornerRadius ?? 0;
    }
    const save = debounce((prefs: Prefs) => saveState<Prefs>(PREFS_KEY, prefs), 300);
    $effect.root(() => {
      $effect(() => {
        save({
          snapEnabled: this.snapEnabled,
          snapThresholdPx: this.snapThresholdPx,
          gridEnabled: this.gridEnabled,
          gridSize: this.gridSize,
          guidesEnabled: this.guidesEnabled,
          newStyle: this.newStyle,
          cornerRadius: this.cornerRadius,
        });
      });
    });
  }

  set(id: ToolId): void {
    this.active = id;
  }

  /** Set/clear one attribute of the new-shape style. */
  setNewStyle(key: string, value: string | null): void {
    const next = { ...this.newStyle };
    if (value === null) delete next[key];
    else next[key] = value;
    this.newStyle = next;
  }
}

export const tools = new ToolState();
