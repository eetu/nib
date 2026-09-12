import type { Point } from "$lib/model/types";
import { debounce, loadState, saveState } from "$lib/persistence";

export type ToolId =
  | "select"
  | "pen"
  | "circle"
  | "rect"
  | "line"
  | "polygon"
  | "star"
  | "text"
  | "eyedropper"
  | "rotate"
  | "add-node"
  | "delete-node";

/** Style applied to the next drawn path/shape (editable up front via the
 *  "new shape style" panel; also the reset defaults). */
/**
 * What a freshly drawn shape is painted with.
 *
 * The stroke used to be `currentColor`, which followed the UI theme — clever, and wrong for three
 * reasons that compounded. nib's job is producing a FILE: a path exported as
 * `stroke="currentColor"` renders a different colour in every context it lands in. The artwork is
 * judged against `settings.canvasBg`, which is deliberately orthogonal to the UI theme, so a
 * stroke that changed when you flipped the chrome to dark was answering the wrong question. And
 * it made the eyedropper useless — every shape you'd drawn carried the same deferred keyword, so
 * sampling any of them returned the same resolved grey, always.
 *
 * `currentColor` is still one keystroke away in the paint kind list, for when it is genuinely what
 * you want.
 */
const DEFAULT_STROKE = "#111111";
const DEFAULT_STYLE: Record<string, string> = {
  fill: "none",
  stroke: DEFAULT_STROKE,
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

  /**
   * Where the rotate tool turns the selection about, in document units. `null` = the selection's
   * own centre, which is what every rotation did before there was a tool to move it.
   *
   * It lives here rather than in the tool module because the Overlay draws it, and it is
   * deliberately *not* persisted: a pivot belongs to the shape you are working on right now, and
   * one restored from last week next to a different selection would rotate things into orbit.
   */
  pivot = $state<Point | null>(null);

  /**
   * Which paint the eyedropper fills in — set by whichever button armed it.
   *
   * The button beside `stroke` has to sample a stroke colour, or there are two eyedroppers doing
   * the same thing in different places. Not persisted: it belongs to the click that armed it, and
   * the keyboard shortcut deliberately resets it to `fill` — the answer to a bare "eyedropper".
   */
  eyedropperTarget = $state<"fill" | "stroke">("fill");

  constructor() {
    const p = loadState<Prefs>(PREFS_KEY);
    if (p) {
      this.snapEnabled = p.snapEnabled;
      this.snapThresholdPx = p.snapThresholdPx;
      this.gridEnabled = p.gridEnabled;
      this.gridSize = p.gridSize;
      this.guidesEnabled = p.guidesEnabled ?? true;
      if (p.newStyle) {
        // Anyone who used nib before this has `currentColor` saved as their stroke — the very
        // thing being fixed. Carry them over rather than leaving the bug persisted; a deliberate
        // `currentColor` is one pick away in the kind list.
        const saved = { ...p.newStyle };
        if (saved.stroke === "currentColor") saved.stroke = DEFAULT_STROKE;
        this.newStyle = saved;
      }
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

  /**
   * The tool a momentary one interrupted, restored when it lets go.
   *
   * Switching tools and *borrowing* one are different acts, and conflating them broke both ends:
   * arming the eyedropper to pick a stroke colour for the pen ended the pen's unfinished path
   * (a switch runs `onDeactivate`, and the pen's is "finish the path"), then dropped you on the
   * select tool afterwards. A borrow suspends the host instead, and hands it back.
   */
  host = $state<ToolId | null>(null);

  /**
   * The tool the UI should describe. A borrowed interlude is not a change of subject: while the
   * eyedropper is armed the rail still shows the pen lit and the style panel still edits the
   * pen's new-shape style — which is the whole reason you reached for the eyedropper.
   */
  get subject(): ToolId {
    return this.host ?? this.active;
  }

  /** Pick a tool deliberately — ends whatever the last one was doing. */
  set(id: ToolId): void {
    this.host = null;
    this.active = id;
  }

  /** Step onto a momentary tool, remembering the one to come back to. */
  borrow(id: ToolId): void {
    if (this.active !== id) this.host = this.active;
    this.active = id;
  }

  /** Hand the tool back to whoever lent it. */
  release(): void {
    const back = this.host ?? "select";
    this.host = null;
    this.active = back;
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
