import { parseSvg, serializeSvg } from "$lib/model/document";
import { distance, normalize } from "$lib/model/geometry";
import { closeSubpath, insertNodeAt } from "$lib/model/path";
import { ellipseNodes, ellipseSubpath } from "$lib/model/shapes";
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

import { History } from "./history.svelte";
import { tools } from "./tool.svelte";

type Snapshot = { paths: PathElement[]; selection: NodeRef | null; selectedPath: number | null };

/** The persisted editing session — survives HMR / reload via the persistence
 *  layer. The undo stack is intentionally not persisted (it resets on reload). */
type Session = {
  doc: SvgDocument | null;
  selection: NodeRef | null;
  selectedPath: number | null;
  dirty: boolean;
  fileName: string | null;
};

const SESSION_KEY = "session";

const BLANK_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">\n</svg>`;

// The model is pure JSON-safe data (numbers/strings/booleans, optional handles
// simply absent), so a JSON round-trip is a reliable deep clone — and unlike
// structuredClone it sees straight through Svelte's $state proxies.
function clonePaths(paths: PathElement[]): PathElement[] {
  return JSON.parse(JSON.stringify(paths)) as PathElement[];
}

function cloneSubpaths(subpaths: Subpath[]): Subpath[] {
  return JSON.parse(JSON.stringify(subpaths)) as Subpath[];
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

type Clipboard = { subpaths: Subpath[]; attributes: Record<string, string> };

/**
 * The central editor state: the parsed document, the current selection, and
 * the mutation surface the tools drive. Mutations change the model live (so a
 * drag updates the canvas continuously); the tool calls `commit()` once at the
 * end of a gesture to record a single undo step.
 */
class DocumentStore {
  doc = $state<SvgDocument | null>(null);
  selection = $state<NodeRef | null>(null);
  /** The path the inspector styles/targets. A selected node implies its path. */
  selectedPath = $state<number | null>(null);
  /** Unsaved changes since the last load/save — the workspace layer reads it. */
  dirty = $state(false);
  /** Display name of the loaded document (its file name, "pasted.svg", etc.). */
  fileName = $state<string | null>(null);

  #history = new History<Snapshot>();
  #persist = debounce(() => {
    saveState<Session>(SESSION_KEY, {
      doc: this.doc,
      selection: this.selection,
      selectedPath: this.selectedPath,
      dirty: this.dirty,
      fileName: this.fileName,
    });
  }, 300);

  constructor() {
    // Rehydrate the last session so a code reload / refresh keeps your work.
    const s = loadState<Session>(SESSION_KEY);
    if (s?.doc) {
      this.doc = s.doc;
      this.selection = s.selection;
      this.selectedPath = s.selectedPath ?? null;
      this.dirty = s.dirty;
      this.fileName = s.fileName;
      this.#history.reset(this.#snapshot());
    }
  }

  /** The path the inspector targets: the selected node's path (always fresh),
   *  else an explicit path selection (PATHS row / path-body click). */
  get selectedPathIndex(): number | null {
    return this.selection ? this.selection.pathIndex : this.selectedPath;
  }

  get selectedPathElement(): PathElement | null {
    const i = this.selectedPathIndex;
    return i !== null ? (this.doc?.paths[i] ?? null) : null;
  }

  /** A whole-path (object) selection with no node picked — the state that shows
   *  the transform box. A node selection is "node editing" (no box). */
  get objectSelected(): boolean {
    return this.selection === null && this.selectedPath !== null;
  }

  get canUndo(): boolean {
    return this.#history.canUndo;
  }
  get canRedo(): boolean {
    return this.#history.canRedo;
  }

  get hasDocument(): boolean {
    return this.doc !== null;
  }

  get selectedNode(): PathNode | null {
    return this.selection ? this.#nodeAt(this.selection) : null;
  }

  /** Replace the document from SVG source. Throws if the source won't parse. */
  load(source: string, name: string | null = null): void {
    this.doc = parseSvg(source);
    this.selection = null;
    this.selectedPath = null;
    this.dirty = false;
    this.fileName = name;
    this.#history.reset(this.#snapshot());
    this.#persist();
  }

  clear(): void {
    this.doc = null;
    this.selection = null;
    this.selectedPath = null;
    this.dirty = false;
    this.fileName = null;
    this.#history.reset({ paths: [], selection: null, selectedPath: null });
    this.#persist();
  }

  /** Create an empty document to draw on if none is loaded. No-op otherwise. */
  ensureBlank(): void {
    if (this.doc) return;
    this.load(BLANK_SVG, "drawing.svg");
  }

  /** Current document serialized back to SVG (unedited markup preserved). */
  toSvg(): string {
    return this.doc ? serializeSvg(this.doc) : "";
  }

  markSaved(): void {
    this.dirty = false;
    this.#persist();
  }

  /** Select a node (implies selecting its path). */
  select(ref: NodeRef | null): void {
    this.selection = ref;
    if (ref) this.selectedPath = ref.pathIndex;
    this.#persist();
  }

  /** Select a whole path with no node (e.g. clicking its body / a PATHS row). */
  selectPath(pathIndex: number | null): void {
    this.selectedPath = pathIndex;
    this.selection = null;
    this.#persist();
  }

  /** Clear both path and node selection. */
  deselect(): void {
    this.selection = null;
    this.selectedPath = null;
    this.#persist();
  }

  /** Set/clear one presentation attribute on any path. Drawn paths edit their
   *  own attributes; imported paths accumulate a styleOverride merged into the
   *  source tag on export. Commits. */
  setPathStyle(pathIndex: number, key: string, value: string | null): void {
    const p = this.doc?.paths[pathIndex];
    if (!p) return;
    if (p.added) {
      const attrs = { ...(p.attributes ?? {}) };
      if (value === null) delete attrs[key];
      else attrs[key] = value;
      p.attributes = attrs;
    } else {
      const over = { ...(p.styleOverride ?? {}) };
      if (value === null) delete over[key];
      else over[key] = value;
      p.styleOverride = over;
    }
    this.commit();
  }

  // --- gesture lifecycle -------------------------------------------------

  /** Record the live-edited state as one undo step. */
  commit(): void {
    if (!this.doc) return;
    this.dirty = true;
    this.#history.commit(this.#snapshot());
    this.#persist();
  }

  /** Abandon an in-flight gesture, restoring the last committed state. */
  revert(): void {
    const prev = this.#history.current();
    if (prev) this.#restore(prev);
  }

  undo(): void {
    const s = this.#history.undo();
    if (s) {
      this.#restore(s);
      this.dirty = true;
      this.#persist();
    }
  }

  redo(): void {
    const s = this.#history.redo();
    if (s) {
      this.#restore(s);
      this.dirty = true;
      this.#persist();
    }
  }

  // --- live mutations (tool drives these; commit at gesture end) ---------

  /** Replace a path's geometry (used by the scale transform). Live. */
  setSubpaths(pathIndex: number, subpaths: Subpath[]): void {
    const p = this.doc?.paths[pathIndex];
    if (!p) return;
    p.subpaths = subpaths;
    this.#markEdited(pathIndex);
  }

  /** Translate an entire path (all subpaths' nodes + handles) by a delta —
   *  moving the whole shape. Live (tool commits at gesture end). */
  movePathBy(pathIndex: number, dx: number, dy: number): void {
    const path = this.doc?.paths[pathIndex];
    if (!path) return;
    for (const sp of path.subpaths) {
      for (const node of sp.nodes) {
        node.point = { x: node.point.x + dx, y: node.point.y + dy };
        if (node.handleIn) node.handleIn = { x: node.handleIn.x + dx, y: node.handleIn.y + dy };
        if (node.handleOut) node.handleOut = { x: node.handleOut.x + dx, y: node.handleOut.y + dy };
      }
    }
    this.#markEdited(pathIndex);
  }

  // --- clipboard + nudge ------------------------------------------------

  #clipboard: Clipboard | null = null;

  get canPaste(): boolean {
    return this.#clipboard !== null;
  }

  /** Copy the selected path (its geometry + effective style) to the clipboard. */
  copySelected(): void {
    const p = this.selectedPathElement;
    if (!p) return;
    const attributes = p.added
      ? { ...(p.attributes ?? {}) }
      : { ...(p.attributes ?? {}), ...(p.styleOverride ?? {}) };
    this.#clipboard = { subpaths: cloneSubpaths(p.subpaths), attributes };
  }

  /** Paste the clipboard as a new drawn path, offset so it's visible. Commits. */
  paste(): void {
    if (!this.#clipboard || !this.doc) return;
    const subpaths = cloneSubpaths(this.#clipboard.subpaths);
    offsetSubpaths(subpaths, 10, 10);
    const pathIndex = this.doc.paths.length;
    this.doc.paths.push({
      id: crypto.randomUUID(),
      index: pathIndex,
      originalD: "",
      subpaths,
      edited: true,
      added: true,
      attributes: { ...this.#clipboard.attributes },
    });
    this.select({ pathIndex, subpathIndex: 0, nodeIndex: 0 });
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

  /** Nudge the selected node, or the whole selected path if only a path is
   *  selected. Commits. */
  nudge(dx: number, dy: number): void {
    if (this.selection) {
      const node = this.#nodeAt(this.selection);
      if (node) {
        this.moveNode(this.selection, { x: node.point.x + dx, y: node.point.y + dy });
        this.commit();
      }
    } else if (this.selectedPath !== null) {
      this.movePathBy(this.selectedPath, dx, dy);
      this.commit();
    }
  }

  // --- node / handle mutations (tool drives these; commit at gesture end) --

  /** Move an anchor, carrying its handles along by the same delta. */
  moveNode(ref: NodeRef, to: Point): void {
    const node = this.#nodeAt(ref);
    if (!node) return;
    const dx = to.x - node.point.x;
    const dy = to.y - node.point.y;
    node.point = { x: to.x, y: to.y };
    if (node.handleIn) node.handleIn = { x: node.handleIn.x + dx, y: node.handleIn.y + dy };
    if (node.handleOut) node.handleOut = { x: node.handleOut.x + dx, y: node.handleOut.y + dy };
    this.#markEdited(ref.pathIndex);
  }

  /** Move one control handle. Smooth nodes keep the opposite handle collinear
   *  (mirrored direction, its own length preserved). */
  moveHandle(ref: NodeRef, which: "in" | "out", to: Point): void {
    const node = this.#nodeAt(ref);
    if (!node) return;
    if (which === "out") node.handleOut = { x: to.x, y: to.y };
    else node.handleIn = { x: to.x, y: to.y };

    if (node.type === "smooth") {
      const opposite = node[which === "out" ? "handleIn" : "handleOut"];
      if (opposite) {
        const len = distance(node.point, opposite);
        const dir = normalize({
          x: node.point.x - to.x,
          y: node.point.y - to.y,
        });
        const mirrored = {
          x: node.point.x + dir.x * len,
          y: node.point.y + dir.y * len,
        };
        if (which === "out") node.handleIn = mirrored;
        else node.handleOut = mirrored;
      }
    }
    this.#markEdited(ref.pathIndex);
  }

  setNodeType(ref: NodeRef, type: NodeType): void {
    const node = this.#nodeAt(ref);
    if (!node) return;
    node.type = type;
    this.#markEdited(ref.pathIndex);
    this.commit();
  }

  /** Directly set an anchor's position (inspector numeric input). Commits. */
  setNodePoint(ref: NodeRef, to: Point): void {
    this.moveNode(ref, to);
    this.commit();
  }

  // --- structural mutations (single actions; each commits) ---------------

  insertNode(pathIndex: number, subpathIndex: number, segmentIndex: number, t: number): void {
    const sp = this.#subpathAt(pathIndex, subpathIndex);
    if (!sp) return;
    const newIndex = insertNodeAt(sp, segmentIndex, t);
    this.#markEdited(pathIndex);
    this.selection = { pathIndex, subpathIndex, nodeIndex: newIndex };
    this.commit();
  }

  deleteNode(ref: NodeRef): void {
    const sp = this.#subpathAt(ref.pathIndex, ref.subpathIndex);
    if (!sp || ref.nodeIndex < 0 || ref.nodeIndex >= sp.nodes.length) return;
    sp.nodes.splice(ref.nodeIndex, 1);
    // A subpath needs >= 2 nodes to draw; drop it otherwise.
    const path = this.doc?.paths[ref.pathIndex];
    if (path && sp.nodes.length < 2) {
      path.subpaths.splice(ref.subpathIndex, 1);
    }
    // A path with no subpaths left is empty — soft-delete it (drops from the
    // list/render/export; undo restores it).
    const emptied = !!path && path.subpaths.length === 0;
    if (path && emptied) path.deleted = true;
    this.selection = null;
    this.selectedPath = emptied ? null : ref.pathIndex; // keep the path selected unless it's gone
    this.#markEdited(ref.pathIndex);
    this.commit();
  }

  /** Close a subpath's loop by merging its endpoint onto its start (the
   *  close-by-snap gesture). Commits. */
  closeLoop(pathIndex: number, subpathIndex: number): void {
    const sp = this.#subpathAt(pathIndex, subpathIndex);
    if (!sp || sp.closed || sp.nodes.length < 2) return;
    // snap the last node exactly onto the first, then close (folds the seam)
    const first = sp.nodes[0];
    const last = sp.nodes[sp.nodes.length - 1];
    last.point = { ...first.point };
    if (last.handleOut) last.handleOut = { ...last.handleOut };
    closeSubpath(sp);
    this.#markEdited(pathIndex);
    this.selection = { pathIndex, subpathIndex, nodeIndex: 0 };
    this.commit();
  }

  // --- drawing (pen tool) ------------------------------------------------

  /** Start a new drawn path with a first anchor at `point`. Returns its ref.
   *  Not committed until the pen gesture releases. */
  beginPath(point: Point): NodeRef {
    this.ensureBlank();
    const doc = this.doc;
    if (!doc) return { pathIndex: 0, subpathIndex: 0, nodeIndex: 0 };
    const pathIndex = doc.paths.length;
    doc.paths.push({
      id: crypto.randomUUID(),
      index: pathIndex,
      originalD: "",
      subpaths: [{ nodes: [{ point: { x: point.x, y: point.y }, type: "corner" }], closed: false }],
      edited: true,
      added: true,
      attributes: { ...tools.newStyle },
    });
    const ref = { pathIndex, subpathIndex: 0, nodeIndex: 0 };
    this.selection = ref;
    return ref;
  }

  /** Append an anchor to a subpath being drawn. Returns its ref. */
  appendNode(pathIndex: number, subpathIndex: number, point: Point): NodeRef {
    const sp = this.#subpathAt(pathIndex, subpathIndex);
    if (!sp) return { pathIndex, subpathIndex, nodeIndex: 0 };
    sp.nodes.push({ point: { x: point.x, y: point.y }, type: "corner" });
    const ref = { pathIndex, subpathIndex, nodeIndex: sp.nodes.length - 1 };
    this.selection = ref;
    this.#markEdited(pathIndex);
    return ref;
  }

  /** Pen drag: shape the anchor into a smooth node with mirrored handles. */
  setPenHandles(ref: NodeRef, out: Point): void {
    const node = this.#nodeAt(ref);
    if (!node) return;
    node.handleOut = { x: out.x, y: out.y };
    node.handleIn = { x: 2 * node.point.x - out.x, y: 2 * node.point.y - out.y };
    node.type = "smooth";
    this.#markEdited(ref.pathIndex);
  }

  /** Start a circle/ellipse as a closed 4-node path centred at `center` (radius
   *  0). Sized live via resizeEllipse; committed on release. */
  beginEllipse(center: Point): NodeRef {
    this.ensureBlank();
    const doc = this.doc;
    if (!doc) return { pathIndex: 0, subpathIndex: 0, nodeIndex: 0 };
    const pathIndex = doc.paths.length;
    doc.paths.push({
      id: crypto.randomUUID(),
      index: pathIndex,
      originalD: "",
      subpaths: [ellipseSubpath(center.x, center.y, 0, 0)],
      edited: true,
      added: true,
      attributes: { ...tools.newStyle },
    });
    const ref = { pathIndex, subpathIndex: 0, nodeIndex: 0 };
    this.selection = ref;
    return ref;
  }

  /** Resize the ellipse being drawn (live during the drag). */
  resizeEllipse(
    pathIndex: number,
    subpathIndex: number,
    center: Point,
    rx: number,
    ry: number,
  ): void {
    const sp = this.#subpathAt(pathIndex, subpathIndex);
    if (!sp) return;
    sp.nodes = ellipseNodes(center.x, center.y, rx, ry);
    sp.closed = true;
    this.#markEdited(pathIndex);
  }

  /** Rename a path — updates its display id and (on export) its `id` attr. */
  renamePath(pathIndex: number, name: string): void {
    const p = this.doc?.paths[pathIndex];
    const trimmed = name.trim();
    if (!p || !trimmed) return;
    p.id = trimmed;
    p.renamed = true;
    this.commit();
  }

  /** Remove a whole path (soft delete — undoable). Commits. */
  deletePath(pathIndex: number): void {
    const p = this.doc?.paths[pathIndex];
    if (!p) return;
    p.deleted = true;
    if (this.selectedPathIndex === pathIndex) {
      this.selection = null;
      this.selectedPath = null;
    }
    this.commit();
  }

  /** Close a subpath (connect last→first) without moving any node — the pen's
   *  "click the start point to finish" gesture. Commits. */
  closePath(pathIndex: number, subpathIndex: number): void {
    const sp = this.#subpathAt(pathIndex, subpathIndex);
    if (!sp || sp.closed || sp.nodes.length < 2) return;
    closeSubpath(sp);
    this.#markEdited(pathIndex);
    this.commit();
  }

  // --- internals ---------------------------------------------------------

  #nodeAt(ref: NodeRef): PathNode | null {
    return this.doc?.paths[ref.pathIndex]?.subpaths[ref.subpathIndex]?.nodes[ref.nodeIndex] ?? null;
  }

  #subpathAt(pathIndex: number, subpathIndex: number): Subpath | null {
    return this.doc?.paths[pathIndex]?.subpaths[subpathIndex] ?? null;
  }

  #markEdited(pathIndex: number): void {
    const p = this.doc?.paths[pathIndex];
    if (p) p.edited = true;
  }

  #snapshot(): Snapshot {
    return {
      paths: this.doc ? clonePaths(this.doc.paths) : [],
      selection: this.selection ? { ...this.selection } : null,
      selectedPath: this.selectedPath,
    };
  }

  #restore(s: Snapshot): void {
    if (!this.doc) return;
    this.doc.paths = clonePaths(s.paths);
    this.selection = s.selection ? { ...s.selection } : null;
    this.selectedPath = s.selectedPath;
  }
}

export const editor = new DocumentStore();
