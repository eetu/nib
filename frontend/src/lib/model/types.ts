// The nib document model — pure data, no Svelte, no DOM. Everything editable is
// normalized to absolute cubic-bezier anchor nodes so one uniform shape covers
// M/L/H/V/C/S/Q/T/A on import and serializes back to a compact `d`.

export type Point = { x: number; y: number };

/** A smooth node keeps its two handles collinear (mirror on drag); a corner
 *  node moves them independently. */
export type NodeType = "corner" | "smooth";

/** An anchor point plus its optional bezier control handles, stored in
 *  absolute document coordinates. A segment between two adjacent nodes is a
 *  straight line iff the outgoing handle of the first and the incoming handle
 *  of the second are both absent. */
export type PathNode = {
  point: Point;
  handleIn?: Point;
  handleOut?: Point;
  type: NodeType;
};

export type Subpath = {
  nodes: PathNode[];
  closed: boolean;
};

/** A single <path> element: its editable model plus what we need to write the
 *  edit back into the original SVG source without disturbing anything else. */
export type PathElement = {
  /** Stable id for selection — the element's `id` attr, else `path-<index>`. */
  id: string;
  /** 0-based position among <path> elements in document order; maps to the
   *  live DOM node when rendering. */
  index: number;
  /** The `d` attribute exactly as it appeared in the source. */
  originalD: string;
  subpaths: Subpath[];
  /** Set once the user changes the geometry — only edited paths get their `d`
   *  re-serialized on export; everything else is preserved verbatim. */
  edited: boolean;
  /** True for paths drawn in-app (the pen tool): not present in the source, so
   *  they're appended on export and rendered from the model, not the imported
   *  DOM. */
  added?: boolean;
  /** Presentation attributes. For added paths this is the whole style (edited
   *  directly). For imported paths it's the style parsed from the source (used
   *  to display + reset); edits go to `styleOverride`. */
  attributes?: Record<string, string>;
  /** Style edits to an imported path — merged over `attributes` and spliced
   *  into the source tag on export (keeps everything else byte-for-byte). */
  styleOverride?: Record<string, string>;
  /** The imported path's opening `<path …>` tag exactly as in the source —
   *  the anchor for surgical d/style rewrites on export. */
  originalTag?: string;
  /** Soft-deleted: kept in the array (so indices stay stable + undo restores
   *  it) but omitted from render, hit-testing, and export. */
  deleted?: boolean;
  /** The user renamed this path — write its `id` into the exported markup. */
  renamed?: boolean;
  /** Id of the group this path belongs to (absent = top level / ungrouped). */
  layer?: string;
  /** Per-path visibility toggle (hidden = dropped from render + display:none on export). */
  hidden?: boolean;
};

export type ViewBox = {
  minX: number;
  minY: number;
  width: number;
  height: number;
};

/** A named layer — a flat, ordered grouping over paths (z-order + show/hide + active-target
 *  for new shapes). Exports as a top-level `<g>`. Matches the Rust `Layer`. */
export type Layer = {
  id: string;
  name: string;
  visible: boolean;
};

export type GradientStop = { offset: number; color: string; opacity?: number };

/** A gradient paint, referenced by fill/stroke as `url(#id)` and injected into `<defs>` on
 *  export. Coords are objectBoundingBox fractions (0..1). Matches the Rust `Gradient`. */
export type Gradient = {
  id: string;
  kind: "linear" | "radial";
  stops: GradientStop[];
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  cx: number;
  cy: number;
  r: number;
};

export type SvgDocument = {
  /** Original SVG text, kept so unedited markup exports byte-for-byte. */
  source: string;
  viewBox: ViewBox;
  paths: PathElement[];
  /** Named layers in z-order (bottom → top); empty = no explicit layers. */
  layers?: Layer[];
  /** The layer new shapes are added to. */
  activeLayer?: string;
  /** Gradient paints, injected into a `<defs>` on export. */
  gradients?: Gradient[];
};

/** Addresses one anchor node inside the document — the unit of selection and
 *  the identity a drag operates on. */
export type NodeRef = {
  pathIndex: number;
  subpathIndex: number;
  nodeIndex: number;
};

export function nodeRefEquals(a: NodeRef | null, b: NodeRef | null): boolean {
  if (!a || !b) return a === b;
  return (
    a.pathIndex === b.pathIndex && a.subpathIndex === b.subpathIndex && a.nodeIndex === b.nodeIndex
  );
}

/** A parametric primitive — the payload of the core's addShape/setShape ops. Matches the
 *  Rust `ShapeSpec` (serde tag "shape"). Shapes are built into ordinary editable paths. */
export type ShapeSpec =
  | { shape: "ellipse"; cx: number; cy: number; rx: number; ry: number }
  | { shape: "rect"; x0: number; y0: number; x1: number; y1: number }
  | { shape: "line"; x0: number; y0: number; x1: number; y1: number }
  | { shape: "polygon"; cx: number; cy: number; r: number; sides: number; rotation: number }
  | {
      shape: "star";
      cx: number;
      cy: number;
      outer: number;
      inner: number;
      points: number;
      rotation: number;
    };
