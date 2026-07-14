import { Editor as WasmEditor } from "$lib/core";
import { subpathsBounds } from "$lib/model/geometry";
import type {
  Gradient,
  Layer,
  NodeRef,
  NodeType,
  PathElement,
  PathNode,
  Point,
  ShapeSpec,
  Subpath,
  SvgDocument,
} from "$lib/model/types";
import { debounce, loadState, saveState } from "$lib/persistence";

import { tools } from "./tool.svelte";

type Bounds = { minX: number; minY: number; maxX: number; maxY: number };

type Session = {
  doc: SvgDocument | null;
  selection: NodeRef | null;
  selectedPath: number | null;
  selectedPaths?: number[];
  dirty: boolean;
  fileName: string | null;
};

/** The shape of `WasmEditor.state()` — a full render snapshot pulled after each mutation. */
type CoreState = {
  document: SvgDocument | null;
  selection: NodeRef | null;
  selectedPath: number | null;
  canUndo: boolean;
  canRedo: boolean;
};

type Clipboard = { subpaths: Subpath[]; attributes: Record<string, string> };

const SESSION_KEY = "session";

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function offsetSubpaths(subpaths: Subpath[], dx: number, dy: number): void {
  for (const sp of subpaths) {
    for (const n of sp.nodes) {
      n.point = { x: n.point.x + dx, y: n.point.y + dy };
      if (n.handleIn) n.handleIn = { x: n.handleIn.x + dx, y: n.handleIn.y + dy };
      if (n.handleOut) n.handleOut = { x: n.handleOut.x + dx, y: n.handleOut.y + dy };
    }
  }
}

/**
 * The editor's Svelte-facing facade. The authoritative document, geometry, ops, and undo
 * history all live in the Rust/WASM `nib-core` Editor; this store is a thin reactive mirror:
 * every method translates a call into one or more core **ops**, applies them to the WASM
 * engine, then pulls a fresh render snapshot into `$state` so the canvas + panels re-render.
 *
 * The public surface is unchanged from the old pure-TS store, so tools and components did
 * not have to change — only the internals now delegate to the core.
 */
class DocumentStore {
  #wasm: WasmEditor | null = null;

  // Reactive mirror of the WASM state, refreshed by #sync() after each mutation.
  doc = $state<SvgDocument | null>(null);
  selection = $state<NodeRef | null>(null);
  selectedPath = $state<number | null>(null);
  /** Object selection as a set of path indices (client-side). 0 = nothing, 1 = a single
   *  shape (full transform box), >1 = a multi-selection (union box + align/distribute). */
  selectedPaths = $state<number[]>([]);
  /** Path currently in node-editing mode (Figma-style: entered by double-click). While null
   *  the select tool moves whole shapes; when set, that path's anchors/handles are editable.
   *  Client-only editing mode — not persisted (reload starts in object mode). */
  nodeEditIndex = $state<number | null>(null);
  #canUndo = $state(false);
  #canRedo = $state(false);
  /** Unsaved changes since the last load/save — owned here (not mirrored) so a rehydrated
   *  dirty session survives selection changes. */
  dirty = $state(false);
  fileName = $state<string | null>(null);

  #clipboard: Clipboard | null = null;
  #persist = debounce(() => {
    saveState<Session>(SESSION_KEY, {
      doc: this.doc,
      selection: this.selection,
      selectedPath: this.selectedPath,
      selectedPaths: this.selectedPaths,
      dirty: this.dirty,
      fileName: this.fileName,
    });
  }, 300);

  /** Bring the WASM engine online and rehydrate the last session. Must run after the core
   *  module is initialised (see +layout). Safe to call more than once. */
  init(): void {
    if (this.#wasm) return;
    this.#wasm = new WasmEditor();
    const s = loadState<Session>(SESSION_KEY);
    if (s?.doc) {
      try {
        this.#wasm.setDocument(s.doc);
        // Node-edit mode isn't persisted, so restore any selection as an object selection
        // (transform box) rather than a dangling node with nowhere to edit.
        const restore = s.selectedPaths?.length
          ? s.selectedPaths
          : (s.selection?.pathIndex ?? s.selectedPath) != null
            ? [(s.selection?.pathIndex ?? s.selectedPath) as number]
            : [];
        this.selectedPaths = restore;
        if (restore.length) this.#wasm.selectPath(restore[0]);
        this.fileName = s.fileName;
        this.#sync();
        this.dirty = s.dirty; // after #sync (which doesn't touch dirty)
      } catch {
        // A corrupt / incompatible persisted session: start empty rather than crash.
        this.#wasm.clear();
        this.#sync();
      }
    }
  }

  /** Pull the render snapshot from the core into the reactive mirror (not `dirty`). */
  #sync(): void {
    if (!this.#wasm) return;
    const st = this.#wasm.state() as CoreState;
    this.doc = st.document ?? null;
    this.selection = st.selection ?? null;
    this.selectedPath = st.selectedPath ?? null;
    this.#canUndo = st.canUndo;
    this.#canRedo = st.canRedo;
    // Keep the client object-selection set consistent with the core's single selection when
    // it was set outside the multi-select facade (drawing + paste use the low-level select).
    // A live multi-selection (length > 1) is client-owned, so it's left intact.
    if (this.selection) {
      const p = this.selection.pathIndex;
      if (this.selectedPaths.length !== 1 || this.selectedPaths[0] !== p) this.selectedPaths = [p];
    } else if (this.selectedPaths.length <= 1) {
      const want = this.selectedPath != null ? [this.selectedPath] : [];
      if (this.selectedPaths[0] !== want[0]) this.selectedPaths = want;
    }
  }

  /** Apply one op to the core (live edit, no commit). Returns whether it mutated. */
  #apply(op: unknown): boolean {
    return this.#wasm ? this.#wasm.applyOp(op) : false;
  }

  // --- derived selection state -------------------------------------------

  get selectedPathIndex(): number | null {
    if (this.selection) return this.selection.pathIndex;
    return this.selectedPaths.length === 1 ? this.selectedPaths[0] : null;
  }

  get selectedPathElement(): PathElement | null {
    const i = this.selectedPathIndex;
    return i !== null ? (this.doc?.paths[i] ?? null) : null;
  }

  /** Exactly one whole shape selected → the full transform box (scale/rotate) shows. */
  get objectSelected(): boolean {
    return (
      this.selection === null && this.nodeEditIndex === null && this.selectedPaths.length === 1
    );
  }

  /** More than one shape selected → a union box + align/distribute, move-as-a-group. */
  get multiSelected(): boolean {
    return this.selection === null && this.nodeEditIndex === null && this.selectedPaths.length > 1;
  }

  /** Union bounding box of the current object selection (for the multi-select box + align). */
  get selectionBounds(): Bounds | null {
    if (!this.doc || this.selectedPaths.length === 0) return null;
    let box: Bounds | null = null;
    for (const i of this.selectedPaths) {
      const p = this.doc.paths[i];
      if (!p || p.deleted) continue;
      const b = subpathsBounds(p.subpaths);
      if (!b) continue;
      box = box
        ? {
            minX: Math.min(box.minX, b.minX),
            minY: Math.min(box.minY, b.minY),
            maxX: Math.max(box.maxX, b.maxX),
            maxY: Math.max(box.maxY, b.maxY),
          }
        : { ...b };
    }
    return box;
  }

  get selectedNode(): PathNode | null {
    return this.selection ? this.#nodeAt(this.selection) : null;
  }

  get canUndo(): boolean {
    return this.#canUndo;
  }
  get canRedo(): boolean {
    return this.#canRedo;
  }
  get hasDocument(): boolean {
    return this.doc !== null;
  }
  get canPaste(): boolean {
    return this.#clipboard !== null;
  }

  #nodeAt(ref: NodeRef): PathNode | null {
    return this.doc?.paths[ref.pathIndex]?.subpaths[ref.subpathIndex]?.nodes[ref.nodeIndex] ?? null;
  }

  // --- lifecycle ---------------------------------------------------------

  /** Replace the document from SVG source. Throws (leaving the doc untouched) if it won't
   *  parse — the SOURCE drawer relies on this fail-safe. */
  load(source: string, name: string | null = null): void {
    if (!this.#wasm) return;
    this.#wasm.load(source); // throws on bad markup, before mutating
    this.fileName = name;
    this.dirty = false;
    this.#sync();
    this.#persist();
  }

  clear(): void {
    this.#wasm?.clear();
    this.fileName = null;
    this.dirty = false;
    this.#sync();
    this.#persist();
  }

  /** Create an empty document to draw on if none is loaded. No-op otherwise. */
  ensureBlank(): void {
    if (this.doc) return;
    this.#wasm?.ensureBlank();
    this.#sync();
    this.#persist();
  }

  /** Start a fresh blank document unconditionally (New), replacing any current one. */
  newDocument(name = "untitled.svg"): void {
    this.#wasm?.clear();
    this.#wasm?.ensureBlank();
    this.selectedPaths = [];
    this.nodeEditIndex = null;
    this.fileName = name;
    this.dirty = false;
    this.#sync();
    this.#persist();
  }

  /** Current document serialized back to SVG (unedited markup preserved byte-for-byte). */
  toSvg(): string {
    return this.#wasm?.toSvg() ?? "";
  }

  markSaved(): void {
    this.dirty = false;
    this.#persist();
  }

  // --- selection ---------------------------------------------------------

  select(ref: NodeRef | null): void {
    // Selecting a node implies node-editing that path; clears any multi-selection.
    this.nodeEditIndex = ref ? ref.pathIndex : null;
    this.selectedPaths = ref ? [ref.pathIndex] : [];
    this.#wasm?.select(ref);
    this.#sync();
    this.#persist();
  }

  selectPath(pathIndex: number | null): void {
    this.nodeEditIndex = null; // object mode
    this.selectedPaths = pathIndex == null ? [] : [pathIndex];
    this.#wasm?.selectPath(pathIndex ?? undefined);
    this.#sync();
    this.#persist();
  }

  /** Toggle a path in/out of the object selection (shift-click). */
  togglePath(pathIndex: number): void {
    this.nodeEditIndex = null;
    this.selectedPaths = this.selectedPaths.includes(pathIndex)
      ? this.selectedPaths.filter((i) => i !== pathIndex)
      : [...this.selectedPaths, pathIndex];
    const primary = this.selectedPaths.at(-1) ?? null;
    this.#wasm?.selectPath(primary ?? undefined);
    this.#sync();
    this.#persist();
  }

  /** Replace the object selection with a set of paths (marquee). */
  setSelectedPaths(indices: number[]): void {
    this.nodeEditIndex = null;
    this.selectedPaths = [...indices];
    this.#wasm?.selectPath(indices[0] ?? undefined);
    this.#sync();
    this.#persist();
  }

  deselect(): void {
    this.nodeEditIndex = null;
    this.selectedPaths = [];
    this.#wasm?.deselect();
    this.#sync();
    this.#persist();
  }

  /** Enter node-editing mode for a path (double-click): select it as the object, then flag
   *  its nodes as editable so the overlay shows anchors and the select tool edits them. */
  enterNodeEdit(pathIndex: number): void {
    this.#wasm?.selectPath(pathIndex);
    this.selectedPaths = [pathIndex];
    this.nodeEditIndex = pathIndex;
    this.#sync();
    this.#persist();
  }

  /** Leave node-editing mode — clears the active node but keeps the path object-selected. */
  exitNodeEdit(): void {
    if (this.nodeEditIndex === null && this.selection === null) return;
    this.nodeEditIndex = null;
    this.#wasm?.select(null); // drop the node; selectPath is preserved
    this.#sync();
    this.#persist();
  }

  // --- multi-selection group operations ----------------------------------

  /** Live-move every selected path by (dx, dy) — the group drag. Commit at gesture end. */
  moveSelectedBy(dx: number, dy: number): void {
    for (const i of this.selectedPaths) this.#apply({ type: "movePathBy", path: i, dx, dy });
    this.#sync();
  }

  /** Align every selected shape's bbox to one edge/centre of the selection's union box. */
  align(edge: "left" | "hcenter" | "right" | "top" | "vcenter" | "bottom"): void {
    const doc = this.doc;
    const u = this.selectionBounds;
    if (!doc || !u || this.selectedPaths.length < 2) return;
    for (const i of this.selectedPaths) {
      const p = doc.paths[i];
      if (!p || p.deleted) continue;
      const b = subpathsBounds(p.subpaths);
      if (!b) continue;
      let dx = 0;
      let dy = 0;
      if (edge === "left") dx = u.minX - b.minX;
      else if (edge === "right") dx = u.maxX - b.maxX;
      else if (edge === "hcenter") dx = (u.minX + u.maxX) / 2 - (b.minX + b.maxX) / 2;
      else if (edge === "top") dy = u.minY - b.minY;
      else if (edge === "bottom") dy = u.maxY - b.maxY;
      else dy = (u.minY + u.maxY) / 2 - (b.minY + b.maxY) / 2;
      if (dx || dy) this.#apply({ type: "movePathBy", path: i, dx, dy });
    }
    this.commit();
  }

  /** Even out spacing between the selected shapes' centres along one axis (needs ≥3). */
  distribute(axis: "h" | "v"): void {
    const doc = this.doc;
    if (!doc) return;
    const items = this.selectedPaths
      .map((i) => ({ i, b: subpathsBounds(doc.paths[i]?.subpaths ?? []) }))
      .filter((x): x is { i: number; b: Bounds } => !!x.b && !doc.paths[x.i]?.deleted);
    if (items.length < 3) return;
    const mid = (b: Bounds) => (axis === "h" ? (b.minX + b.maxX) / 2 : (b.minY + b.maxY) / 2);
    items.sort((a, c) => mid(a.b) - mid(c.b));
    const first = mid(items[0].b);
    const step = (mid(items[items.length - 1].b) - first) / (items.length - 1);
    items.forEach((it, k) => {
      if (k === 0 || k === items.length - 1) return;
      const delta = first + step * k - mid(it.b);
      if (axis === "h") this.#apply({ type: "movePathBy", path: it.i, dx: delta, dy: 0 });
      else this.#apply({ type: "movePathBy", path: it.i, dx: 0, dy: delta });
    });
    this.commit();
  }

  /** Combine the selected paths with a boolean op — replaces them with one result path. */
  booleanOp(op: "union" | "intersect" | "subtract" | "exclude"): void {
    if (this.selectedPaths.length < 2) return;
    const id = crypto.randomUUID();
    if (this.#apply({ type: "booleanOp", op, paths: [...this.selectedPaths], id })) {
      this.commit();
      this.selectPath((this.doc?.paths.length ?? 1) - 1); // the appended result
    }
  }

  /** Soft-delete every selected path (soft-delete keeps indices stable, so no reindexing). */
  deleteSelectedPaths(): void {
    if (this.selectedPaths.length === 0) return;
    for (const i of this.selectedPaths) this.#apply({ type: "deletePath", path: i });
    this.#wasm?.deselect();
    this.selectedPaths = [];
    this.commit();
  }

  // --- layers ------------------------------------------------------------

  get layers(): Layer[] {
    return this.doc?.layers ?? [];
  }
  get activeLayer(): string | null {
    return this.doc?.activeLayer ?? null;
  }

  /** Create a layer (auto-id) and make it active. */
  addLayer(name: string): void {
    const id = crypto.randomUUID();
    if (this.#apply({ type: "addLayer", id, name })) this.commit();
  }
  renameLayer(id: string, name: string): void {
    if (this.#apply({ type: "renameLayer", id, name })) this.commit();
  }
  deleteLayer(id: string): void {
    if (this.#apply({ type: "deleteLayer", id })) this.commit();
  }
  setLayerVisible(id: string, visible: boolean): void {
    if (this.#apply({ type: "setLayerVisible", id, visible })) this.commit();
  }
  reorderLayer(id: string, to: number): void {
    if (this.#apply({ type: "reorderLayer", id, to })) this.commit();
  }
  setActiveLayer(id: string | null): void {
    if (this.#apply({ type: "setActiveLayer", id: id ?? undefined })) this.commit();
  }
  /** Assign a path (or the whole current object selection) to a layer (`null` = unassign). */
  setPathLayer(pathIndex: number, layer: string | null): void {
    if (this.#apply({ type: "setPathLayer", path: pathIndex, layer: layer ?? undefined }))
      this.commit();
  }

  /** Group the current object selection into a new named group (a `<g>`), pulled contiguous. */
  groupSelection(name: string): void {
    const sel = [...this.selectedPaths].sort((a, b) => a - b);
    if (sel.length === 0) return;
    const id = crypto.randomUUID();
    if (this.#apply({ type: "groupPaths", paths: sel, id, name })) {
      this.commit();
      const start = sel[0];
      this.selectedPaths = sel.map((_, k) => start + k); // the now-contiguous block
      this.#persist();
    }
  }

  /** Dissolve a group — its paths become top level (the geometry is untouched). */
  ungroup(layerId: string): void {
    this.deleteLayer(layerId);
  }

  /** Show/hide a single path. */
  setPathHidden(pathIndex: number, hidden: boolean): void {
    if (this.#apply({ type: "setPathHidden", path: pathIndex, hidden })) this.commit();
  }
  /** Move every selected path onto a layer (one undo step). */
  assignSelectionToLayer(layer: string | null): void {
    if (this.selectedPaths.length === 0) return;
    let changed = false;
    for (const i of this.selectedPaths)
      changed =
        this.#apply({ type: "setPathLayer", path: i, layer: layer ?? undefined }) || changed;
    if (changed) this.commit();
  }

  // --- gradients ---------------------------------------------------------

  get gradients(): Gradient[] {
    return this.doc?.gradients ?? [];
  }
  gradientById(id: string): Gradient | null {
    return this.doc?.gradients?.find((g) => g.id === id) ?? null;
  }
  /** Upsert a gradient def as one undo step. */
  setGradient(gradient: Gradient): void {
    if (this.#apply({ type: "setGradient", gradient })) this.commit();
  }
  /** Live-preview a gradient edit (e.g. a stop-colour drag) without committing. */
  previewGradient(gradient: Gradient): void {
    this.#apply({ type: "setGradient", gradient });
    this.#sync();
  }
  removeGradient(id: string): void {
    if (this.#apply({ type: "removeGradient", id })) this.commit();
  }

  // --- gesture lifecycle -------------------------------------------------

  /** Record the live-edited state as one undo step. */
  commit(): void {
    if (!this.#wasm || !this.doc) return;
    this.#wasm.commit();
    this.dirty = true;
    this.#sync();
    this.#persist();
  }

  /** Abandon an in-flight gesture, restoring the last committed state. */
  revert(): void {
    this.#wasm?.revert();
    this.#sync();
  }

  undo(): void {
    if (this.#wasm?.undo()) {
      this.dirty = true;
      this.#sync();
      this.#persist();
    }
  }

  redo(): void {
    if (this.#wasm?.redo()) {
      this.dirty = true;
      this.#sync();
      this.#persist();
    }
  }

  // --- style -------------------------------------------------------------

  setPathStyle(pathIndex: number, key: string, value: string | null): void {
    if (this.#apply({ type: "setStyle", path: pathIndex, key, value })) this.commit();
  }

  /** Live-preview a style change without committing — for a color-picker drag, so the shape
   *  updates as you pick. The interaction commits once (via setPathStyle) when it settles. */
  previewPathStyle(pathIndex: number, key: string, value: string | null): void {
    this.#apply({ type: "setStyle", path: pathIndex, key, value });
    this.#sync();
  }

  // --- live mutations (tool drives these; commit at gesture end) ---------

  setSubpaths(pathIndex: number, subpaths: Subpath[]): void {
    this.#apply({ type: "setSubpaths", path: pathIndex, subpaths });
    this.#sync();
  }

  movePathBy(pathIndex: number, dx: number, dy: number): void {
    this.#apply({ type: "movePathBy", path: pathIndex, dx, dy });
    this.#sync();
  }

  moveNode(ref: NodeRef, to: Point): void {
    this.#apply({ type: "moveNode", node: ref, to });
    this.#sync();
  }

  moveHandle(ref: NodeRef, which: "in" | "out", to: Point): void {
    this.#apply({ type: "moveHandle", node: ref, which, to });
    this.#sync();
  }

  setPenHandles(ref: NodeRef, out: Point): void {
    this.#apply({ type: "setPenHandles", node: ref, out });
    this.#sync();
  }

  reverseSubpath(pathIndex: number, subpathIndex: number): void {
    this.#apply({ type: "reverseSubpath", path: pathIndex, subpath: subpathIndex });
    this.#sync();
  }

  /** Rebuild a shape subpath from an updated spec (live during a create-tool drag). */
  setShape(pathIndex: number, subpathIndex: number, spec: ShapeSpec): void {
    this.#apply({ type: "setShape", path: pathIndex, subpath: subpathIndex, spec });
    this.#sync();
  }

  // --- committing single actions -----------------------------------------

  setNodeType(ref: NodeRef, type: NodeType): void {
    if (this.#apply({ type: "setNodeType", node: ref, nodeType: type })) this.commit();
  }

  setNodePoint(ref: NodeRef, to: Point): void {
    if (this.#apply({ type: "moveNode", node: ref, to })) this.commit();
  }

  insertNode(pathIndex: number, subpathIndex: number, segmentIndex: number, t: number): void {
    if (
      !this.#apply({
        type: "insertNode",
        path: pathIndex,
        subpath: subpathIndex,
        segment: segmentIndex,
        t,
      })
    )
      return;
    // insert_node_at inserts after the segment's start node → the new node is segment+1.
    this.#wasm?.select({ pathIndex, subpathIndex, nodeIndex: segmentIndex + 1 });
    this.commit();
  }

  deleteNode(ref: NodeRef): void {
    if (!this.#apply({ type: "deleteNode", node: ref })) return;
    this.#sync();
    const path = this.doc?.paths[ref.pathIndex];
    const emptied = !path || path.deleted;
    if (emptied) this.#wasm?.deselect();
    else this.#wasm?.selectPath(ref.pathIndex);
    this.commit();
  }

  closeLoop(pathIndex: number, subpathIndex: number): void {
    if (!this.#apply({ type: "closeLoop", path: pathIndex, subpath: subpathIndex })) return;
    this.#wasm?.select({ pathIndex, subpathIndex, nodeIndex: 0 });
    this.commit();
  }

  closePath(pathIndex: number, subpathIndex: number): void {
    if (this.#apply({ type: "closePath", path: pathIndex, subpath: subpathIndex })) this.commit();
  }

  renamePath(pathIndex: number, name: string): void {
    if (this.#apply({ type: "renamePath", path: pathIndex, name })) this.commit();
  }

  deletePath(pathIndex: number): void {
    const wasSelected = this.selectedPathIndex === pathIndex;
    if (!this.#apply({ type: "deletePath", path: pathIndex })) return;
    if (wasSelected) this.#wasm?.deselect();
    this.commit();
  }

  /** Move a path within the ordered list (drag-drop in PATHS) — later = drawn on top. Selects
   *  the moved path at its new index. */
  reorderPath(from: number, to: number): void {
    if (from === to || !this.#apply({ type: "reorderPath", from, to })) return;
    this.commit();
    const last = (this.doc?.paths.length ?? 1) - 1;
    this.selectPath(Math.max(0, Math.min(to, last)));
  }

  // --- drawing (pen / circle) --------------------------------------------

  beginPath(point: Point): NodeRef {
    this.ensureBlank();
    if (!this.doc || !this.#wasm) return { pathIndex: 0, subpathIndex: 0, nodeIndex: 0 };
    const pathIndex = this.doc.paths.length;
    const subpaths: Subpath[] = [
      { nodes: [{ point: { x: point.x, y: point.y }, type: "corner" }], closed: false },
    ];
    this.#apply({
      type: "addPath",
      id: crypto.randomUUID(),
      subpaths,
      attributes: { ...tools.newStyle },
    });
    const ref = { pathIndex, subpathIndex: 0, nodeIndex: 0 };
    this.#wasm.select(ref);
    this.#sync();
    return ref;
  }

  appendNode(pathIndex: number, subpathIndex: number, point: Point): NodeRef {
    const before = this.doc?.paths[pathIndex]?.subpaths[subpathIndex]?.nodes.length ?? 0;
    this.#apply({
      type: "appendNode",
      path: pathIndex,
      subpath: subpathIndex,
      point: { x: point.x, y: point.y },
    });
    const ref = { pathIndex, subpathIndex, nodeIndex: before };
    this.#wasm?.select(ref);
    this.#sync();
    return ref;
  }

  /** Start a new shape path from a parametric spec (create tools seed it degenerate, then
   *  resize live via setShape). Returns its first node's ref. */
  beginShape(spec: ShapeSpec): NodeRef {
    this.ensureBlank();
    if (!this.doc || !this.#wasm) return { pathIndex: 0, subpathIndex: 0, nodeIndex: 0 };
    const pathIndex = this.doc.paths.length;
    this.#apply({
      type: "addShape",
      id: crypto.randomUUID(),
      spec,
      attributes: { ...tools.newStyle },
    });
    const ref = { pathIndex, subpathIndex: 0, nodeIndex: 0 };
    this.#wasm.select(ref);
    this.#sync();
    return ref;
  }

  // --- clipboard + nudge -------------------------------------------------

  copySelected(): void {
    const p = this.selectedPathElement;
    if (!p) return;
    const attributes = p.added
      ? { ...(p.attributes ?? {}) }
      : { ...(p.attributes ?? {}), ...(p.styleOverride ?? {}) };
    this.#clipboard = { subpaths: clone(p.subpaths), attributes };
  }

  paste(): void {
    if (!this.#clipboard || !this.doc || !this.#wasm) return;
    const subpaths = clone(this.#clipboard.subpaths);
    offsetSubpaths(subpaths, 10, 10);
    const pathIndex = this.doc.paths.length;
    this.#apply({
      type: "addPath",
      id: crypto.randomUUID(),
      subpaths,
      attributes: { ...this.#clipboard.attributes },
    });
    this.#wasm.selectPath(pathIndex); // object-select the paste (transform box, not nodes)
    this.selectedPaths = [pathIndex];
    this.nodeEditIndex = null;
    this.commit();
  }

  duplicateSelected(): void {
    this.copySelected();
    this.paste();
  }

  cutSelected(): void {
    const i = this.selectedPathIndex;
    if (i === null) return;
    this.copySelected();
    this.deletePath(i);
  }

  nudge(dx: number, dy: number): void {
    if (this.selection) {
      const node = this.selectedNode;
      if (node) {
        this.moveNode(this.selection, { x: node.point.x + dx, y: node.point.y + dy });
        this.commit();
      }
    } else if (this.selectedPaths.length > 0) {
      this.moveSelectedBy(dx, dy);
      this.commit();
    }
  }
}

export const editor = new DocumentStore();
