import { Editor as WasmEditor } from "$lib/core";
import { ellipseSubpath } from "$lib/model/shapes";
import type {
  NodeRef,
  NodeType,
  PathElement,
  PathNode,
  Point,
  Subpath,
  SvgDocument,
} from "$lib/model/types";
import { debounce, loadState, saveState } from "$lib/persistence";

import { tools } from "./tool.svelte";

type Session = {
  doc: SvgDocument | null;
  selection: NodeRef | null;
  selectedPath: number | null;
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
        if (s.selection) this.#wasm.select(s.selection);
        else if (s.selectedPath != null) this.#wasm.selectPath(s.selectedPath);
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
  }

  /** Apply one op to the core (live edit, no commit). Returns whether it mutated. */
  #apply(op: unknown): boolean {
    return this.#wasm ? this.#wasm.applyOp(op) : false;
  }

  // --- derived selection state -------------------------------------------

  get selectedPathIndex(): number | null {
    return this.selection ? this.selection.pathIndex : this.selectedPath;
  }

  get selectedPathElement(): PathElement | null {
    const i = this.selectedPathIndex;
    return i !== null ? (this.doc?.paths[i] ?? null) : null;
  }

  get objectSelected(): boolean {
    return this.selection === null && this.selectedPath !== null;
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
    this.#wasm?.select(ref);
    this.#sync();
    this.#persist();
  }

  selectPath(pathIndex: number | null): void {
    this.#wasm?.selectPath(pathIndex ?? undefined);
    this.#sync();
    this.#persist();
  }

  deselect(): void {
    this.#wasm?.deselect();
    this.#sync();
    this.#persist();
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

  resizeEllipse(
    pathIndex: number,
    subpathIndex: number,
    center: Point,
    rx: number,
    ry: number,
  ): void {
    this.#apply({ type: "resizeEllipse", path: pathIndex, subpath: subpathIndex, center, rx, ry });
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

  beginEllipse(center: Point): NodeRef {
    this.ensureBlank();
    if (!this.doc || !this.#wasm) return { pathIndex: 0, subpathIndex: 0, nodeIndex: 0 };
    const pathIndex = this.doc.paths.length;
    const subpaths: Subpath[] = [ellipseSubpath(center.x, center.y, 0, 0)];
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
    this.#wasm.select({ pathIndex, subpathIndex: 0, nodeIndex: 0 });
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
    } else if (this.selectedPath !== null) {
      this.movePathBy(this.selectedPath, dx, dy);
      this.commit();
    }
  }
}

export const editor = new DocumentStore();
