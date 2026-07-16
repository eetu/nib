import { Editor as WasmEditor } from "$lib/core";
import { STYLE_KEYS } from "$lib/model/document";
import { tightBounds } from "$lib/model/geometry";
import type {
  BooleanResult,
  Gradient,
  GradientStop,
  ImportedGradient,
  NodeRef,
  NodeType,
  PathElement,
  PathNode,
  Point,
  RenderNode,
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
  /** The structural tree, persisted separately from `doc` (it's `serde(skip)` on the doc to
   *  stay off the per-frame state payload) so structural edits survive a session reload. */
  tree?: unknown;
};

/** The shape of `WasmEditor.state()` — a full render snapshot pulled after each mutation. */
type CoreState = {
  document: SvgDocument | null;
  selection: NodeRef | null;
  selectedPath: number | null;
  canUndo: boolean;
  canRedo: boolean;
  /** Computed geometry of each live-boolean group (derived, not part of the doc). */
  booleanResults: BooleanResult[];
};

type Clipboard = { subpaths: Subpath[]; attributes: Record<string, string>; name: string };

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
  /** A non-shape element (text/image/use/…) selected by its tree `uid` — the object-selection for
   *  elements that aren't editable paths. Orthogonal to path selection (setting one clears the
   *  other). Client-only. */
  selectedElementUid = $state<string | null>(null);
  /** When set, the current `selectedPaths` are a **group** selected as one unit (Figma-style:
   *  clicking a grouped shape selects the whole group; double-click drills in). Distinguishes a
   *  group from an ad-hoc multi-selection so clicking a member doesn't reduce it. Client-only. */
  selectedGroupUid = $state<string | null>(null);
  /** Computed render geometry of each live-boolean group (mirrored from the core snapshot). */
  booleanResults = $state<BooleanResult[]>([]);
  /** Bumped by structural tree ops (hide/group/ungroup/reorder) — the canvas keys its cached
   *  `renderTree()` on it, since those change the tree without changing `source`. */
  treeVersion = $state(0);
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
      tree: this.#treeJson(),
    });
  }, 300);

  /** The structural tree serialized for persistence (`null` if none / on error). */
  #treeJson(): unknown {
    try {
      return this.#wasm?.treeJson() ?? null;
    } catch {
      return null;
    }
  }

  /** Bring the WASM engine online and rehydrate the last session. Must run after the core
   *  module is initialised (see +layout). Safe to call more than once. */
  init(): void {
    if (this.#wasm) return;
    this.#wasm = new WasmEditor();
    const s = loadState<Session>(SESSION_KEY);
    if (s?.doc) {
      try {
        this.#wasm.setDocument(s.doc);
        // Restore persisted structural edits (group/hide/reorder) onto the tree — otherwise
        // setDocument rebuilds the tree from source, losing them.
        if (s.tree) {
          try {
            this.#wasm.setTree(s.tree);
          } catch {
            /* stale/incompatible persisted tree — keep the source-rebuilt one */
          }
        }
        // Reconcile drawn paths into the tree — a no-op for a current session, but migrates one
        // persisted before drawn content lived in the tree (its added paths had no tree node).
        this.#wasm.syncDrawn();
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
    this.booleanResults = st.booleanResults ?? [];
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

  /** A friendly, unique path id/name — `base`, else `base 2`, `base 3`, … (drawn paths get a
   *  readable label in the layers list instead of a uuid; only written to export if renamed). */
  #freshId(base: string): string {
    const ids = new Set(this.doc?.paths.map((p) => p.id) ?? []);
    if (!ids.has(base)) return base;
    let n = 2;
    while (ids.has(`${base} ${n}`)) n++;
    return `${base} ${n}`;
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
      const b = tightBounds(p.subpaths);
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

  /** A normalized copy — every element regenerated canonically + editable shapes as `<path>` (no
   *  verbatim spans / primitives). For downstream tools that want plain paths. */
  toSvgNormalized(): string {
    return this.#wasm?.toSvgNormalized() ?? "";
  }

  /** The canvas render tree (root `<svg>`'s children) — the canvas draws it declaratively,
   *  pulling live geometry for editable shapes from `doc.paths` by uid. Re-fetched on source
   *  change + whenever `treeVersion` bumps (a structural op mutated the tree). */
  renderTree(): RenderNode[] {
    return (this.#wasm?.renderTree() as RenderNode[]) ?? [];
  }

  /** Gradients defined in the imported source `<defs>` — parsed from the render tree, keyed by
   *  id. These are NOT nib's editable `doc.gradients` (which it injects on export); they render
   *  verbatim from the tree, so a `url(#id)` fill can display its actual stops even though the
   *  gradient isn't in the model. Read-only for now (editing needs defs modeling — E5).
   *  Recomputed only on a source change or structural op (its reactive deps). */
  importedGradients = $derived.by(() => {
    void this.treeVersion; // deps: re-parse when the tree changes
    void this.doc?.source;
    const num = (s: string | undefined, dflt: number): number => {
      if (s == null || s === "") return dflt;
      const t = s.trim();
      return t.endsWith("%") ? Number(t.slice(0, -1)) / 100 : Number(t);
    };
    // Read a presentation value from the `stop-*` attr or an inline `style` fallback.
    const prop = (attrs: Record<string, string>, key: string): string | undefined =>
      attrs[key] ?? attrs.style?.match(new RegExp(`${key}:\\s*([^;]+)`))?.[1]?.trim();
    const stopOf = (attrs: Record<string, string>): GradientStop => {
      const op = prop(attrs, "stop-opacity");
      return {
        offset: num(attrs.offset, 0),
        color: prop(attrs, "stop-color") ?? "#000000",
        // Carry opacity when explicit (a color→transparent fade) so the preview + adopt keep it.
        ...(op != null && op !== "" ? { opacity: Number(op) } : {}),
      };
    };
    const entries: [string, ImportedGradient][] = [];
    const walk = (nodes: RenderNode[]): void => {
      for (const n of nodes) {
        if (n.kind !== "element") continue;
        if ((n.tag === "linearGradient" || n.tag === "radialGradient") && n.attrs.id) {
          const a = n.attrs;
          const stops = n.children
            .filter((c) => c.kind === "element" && c.tag === "stop")
            .map((c) => stopOf((c as { attrs: Record<string, string> }).attrs));
          // Adoptable only if it fits nib's model: objectBoundingBox units, no gradientTransform /
          // non-pad spread / focal point. Others still show read-only (their coords need defs work).
          const editable =
            (!a.gradientUnits || a.gradientUnits === "objectBoundingBox") &&
            !a.gradientTransform &&
            (!a.spreadMethod || a.spreadMethod === "pad") &&
            !a.fx &&
            !a.fy &&
            !a.fr;
          if (stops.length)
            entries.push([
              a.id,
              {
                kind: n.tag === "radialGradient" ? "radial" : "linear",
                stops,
                editable,
                x1: num(a.x1, 0),
                y1: num(a.y1, 0),
                x2: num(a.x2, 1),
                y2: num(a.y2, 0),
                cx: num(a.cx, 0.5),
                cy: num(a.cy, 0.5),
                r: num(a.r, 0.5),
              },
            ]);
        }
        walk(n.children);
      }
    };
    if (this.doc) walk(this.renderTree());
    return new Map(entries);
  });

  /** Show/hide any node in the document tree by its stable uid — a group, opaque element, or
   *  shape, at any depth (structural op). */
  setNodeHidden(uid: string, hidden: boolean): void {
    if (this.#apply({ type: "setNodeHidden", uid, hidden })) {
      this.commit();
      this.treeVersion++;
    }
  }

  /** Wrap tree nodes (`uids`, sharing one parent) in a new nested `<g id="name">`. Returns the
   *  new group's uid (or `null` if nothing was grouped). */
  groupNodes(uids: string[], name: string): string | null {
    if (uids.length === 0) return null;
    const uid = crypto.randomUUID();
    if (this.#apply({ type: "groupNodes", uids, uid, name })) {
      this.commit();
      this.treeVersion++;
      return uid;
    }
    return null;
  }

  /** Group the current multi-selection into a fresh nested `<g>` (⌘G and the Inspector button both
   *  route here, so their naming can't drift) — "group N" by the count of existing groups. No-op
   *  for a selection smaller than two. */
  groupSelection(): void {
    const uids = this.selectedPaths
      .map((i) => this.doc?.paths[i]?.uid)
      .filter((u): u is string => !!u);
    if (uids.length < 2) return;
    this.groupNodes(uids, `group ${this.#countGroups(this.renderTree()) + 1}`);
  }

  /** Ungroup the actively-selected group (⌘⇧G) — dissolve it back into its parent. No-op unless a
   *  group is selected. */
  ungroupSelection(): void {
    if (this.selectedGroupUid) this.ungroupNode(this.selectedGroupUid);
  }

  #countGroups(nodes: RenderNode[]): number {
    let c = 0;
    for (const n of nodes) {
      if (n.kind === "element") {
        if (n.tag === "g") c++;
        c += this.#countGroups(n.children);
      }
    }
    return c;
  }

  /** Set (`op`) or clear (`null`) the live-boolean op on a group node (by uid): turn a plain `<g>`
   *  into a live boolean, flip the operation, or flatten it back to a plain group. */
  setNodeBoolean(uid: string, op: "union" | "subtract" | "intersect" | "exclude" | null): void {
    if (this.#apply({ type: "setNodeBoolean", uid, op: op ?? undefined })) {
      this.commit();
      this.treeVersion++;
    }
  }

  /** Dissolve a group node (by uid) in the tree, splicing its children into the parent. */
  ungroupNode(uid: string): void {
    if (this.#apply({ type: "ungroupNode", uid })) {
      this.commit();
      this.treeVersion++;
    }
  }

  /** Move a tree node one slot within its parent — `forward` = higher z (later), else lower. */
  reorderNode(uid: string, forward: boolean): void {
    if (this.#apply({ type: "reorderNode", uid, forward })) {
      this.commit();
      this.treeVersion++;
    }
  }

  /** Move a tree node relative to another (drag-drop): `before`/`after` = sibling of `refUid`
   *  (reparenting across levels), `inside` = into that group. */
  moveTreeNode(uid: string, refUid: string, position: "before" | "after" | "inside"): void {
    if (uid === refUid) return;
    if (this.#apply({ type: "moveTreeNode", uid, refUid, position })) {
      this.commit();
      this.treeVersion++;
    }
  }

  /** Set (or remove, `null`) one attribute on any tree node by uid — the generic editor for
   *  non-shape elements (text/image/use). One undo step; refreshes the render tree. */
  setNodeAttr(uid: string, key: string, value: string | null): void {
    if (this.#apply({ type: "setNodeAttr", uid, key, value: value ?? undefined })) {
      this.commit();
      this.treeVersion++;
    }
  }

  /** Live-preview a node attribute change (a drag / colour pick) without committing an undo step;
   *  a following `setNodeAttr` (on settle) records the single step. */
  previewNodeAttr(uid: string, key: string, value: string | null): void {
    this.#apply({ type: "setNodeAttr", uid, key, value: value ?? undefined });
    this.treeVersion++;
    this.#sync();
  }

  /** Live-move an element to (x, y) doc coords during a drag (both attrs, one refresh); no commit
   *  — the caller commits once at gesture end. */
  previewNodeMove(uid: string, x: number, y: number): void {
    const a = this.#apply({ type: "setNodeAttr", uid, key: "x", value: String(x) });
    const b = this.#apply({ type: "setNodeAttr", uid, key: "y", value: String(y) });
    if (a || b) {
      this.treeVersion++;
      this.#sync();
    }
  }

  /** Replace a text element's content string (editing a `<text>` label). One undo step. */
  setNodeText(uid: string, text: string): void {
    if (this.#apply({ type: "setNodeText", uid, text })) {
      this.commit();
      this.treeVersion++;
    }
  }

  markSaved(): void {
    this.dirty = false;
    this.#persist();
  }

  // --- selection ---------------------------------------------------------

  /** The selected non-shape element's render node (found by uid in the render tree), or null.
   *  Recomputed when the selection or tree changes. */
  selectedElement = $derived.by((): RenderNode | null => {
    const uid = this.selectedElementUid;
    void this.treeVersion;
    void this.doc?.source;
    if (!uid || !this.doc) return null;
    const find = (nodes: RenderNode[]): RenderNode | null => {
      for (const n of nodes) {
        if (n.kind !== "element") continue;
        if (n.uid === uid) return n;
        const hit = find(n.children);
        if (hit) return hit;
      }
      return null;
    };
    return find(this.renderTree());
  });

  /** Object-select a non-shape element by its tree uid (text/image/use). Clears path selection —
   *  the two are mutually exclusive. `null` deselects. */
  selectElement(uid: string | null): void {
    this.nodeEditIndex = null;
    this.selectedPaths = [];
    this.selectedGroupUid = null;
    this.#wasm?.deselect();
    this.selectedElementUid = uid;
    this.#sync();
    this.#persist();
  }

  select(ref: NodeRef | null): void {
    // Selecting a node implies node-editing that path; clears any multi-selection.
    this.selectedElementUid = null;
    this.selectedGroupUid = null;
    this.nodeEditIndex = ref ? ref.pathIndex : null;
    this.selectedPaths = ref ? [ref.pathIndex] : [];
    this.#wasm?.select(ref);
    this.#sync();
    this.#persist();
  }

  /** The editable-shape path indices under the top-level group that contains the node `uid` (i.e.
   *  the outermost `<g>` that is a direct child of the root), plus that group's uid — or `null` if
   *  the node isn't inside a group (it's a direct child of the root). */
  #groupMembers(uid: string): { groupUid: string; indices: number[] } | null {
    const uidToIndex = new Map((this.doc?.paths ?? []).map((p, i) => [p.uid, i] as const));
    const contains = (n: RenderNode, target: string): boolean =>
      n.kind === "element" && (n.uid === target || n.children.some((c) => contains(c, target)));
    for (const top of this.renderTree()) {
      if (top.kind !== "element" || !contains(top, uid)) continue;
      if (top.uid === uid) return null; // a direct child of the root → not grouped
      const indices: number[] = [];
      const walk = (n: RenderNode) => {
        if (n.kind !== "element") return;
        const i = uidToIndex.get(n.uid);
        if (i !== undefined && !this.doc?.paths[i]?.deleted) indices.push(i);
        n.children.forEach(walk);
      };
      walk(top);
      return indices.length ? { groupUid: top.uid, indices } : null;
    }
    return null;
  }

  /** Select the shape at `pathIndex` group-aware (Figma-style): if it's inside a group, select the
   *  whole group as one unit; otherwise just that shape. Double-click drills in (node editing). */
  selectGroup(pathIndex: number): void {
    const uid = this.doc?.paths[pathIndex]?.uid;
    const group = uid ? this.#groupMembers(uid) : null;
    if (group) {
      this.setSelectedPaths(group.indices); // clears selectedGroupUid…
      this.selectedGroupUid = group.groupUid; // …then mark this a group selection
    } else {
      this.selectPath(pathIndex);
    }
  }

  selectPath(pathIndex: number | null): void {
    this.selectedElementUid = null;
    this.selectedGroupUid = null;
    this.nodeEditIndex = null; // object mode
    this.selectedPaths = pathIndex == null ? [] : [pathIndex];
    this.#wasm?.selectPath(pathIndex ?? undefined);
    this.#sync();
    this.#persist();
  }

  /** Toggle a path in/out of the object selection (shift-click). */
  togglePath(pathIndex: number): void {
    this.selectedElementUid = null;
    this.selectedGroupUid = null;
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
    this.selectedElementUid = null;
    this.selectedGroupUid = null;
    this.nodeEditIndex = null;
    this.selectedPaths = [...indices];
    this.#wasm?.selectPath(indices[0] ?? undefined);
    this.#sync();
    this.#persist();
  }

  deselect(): void {
    this.selectedElementUid = null;
    this.selectedGroupUid = null;
    this.nodeEditIndex = null;
    this.selectedPaths = [];
    this.#wasm?.deselect();
    this.#sync();
    this.#persist();
  }

  /** Enter node-editing mode for a path (double-click): select it as the object, then flag
   *  its nodes as editable so the overlay shows anchors and the select tool edits them. */
  enterNodeEdit(pathIndex: number): void {
    this.selectedElementUid = null;
    this.selectedGroupUid = null;
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
      const b = tightBounds(p.subpaths);
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
      .map((i) => ({ i, b: tightBounds(doc.paths[i]?.subpaths ?? []) }))
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

  /** Simplify the selected path — thin its nodes (RDP) at ~1% of its size. */
  simplifyPath(): void {
    const i = this.selectedPathIndex;
    const p = i !== null ? this.doc?.paths[i] : null;
    if (i === null || !p) return;
    const b = tightBounds(p.subpaths);
    const tol = b ? Math.max(b.maxX - b.minX, b.maxY - b.minY, 1) * 0.01 : 1;
    if (this.#apply({ type: "simplifyPath", path: i, tolerance: tol })) this.commit();
  }

  /** Offset the selected path's outline by `distance` (outward if positive), adding a new path. */
  offsetPath(distance: number): void {
    const i = this.selectedPathIndex;
    if (i === null || !Number.isFinite(distance) || distance === 0) return;
    const id = this.#freshId("offset");
    if (this.#apply({ type: "offsetPath", path: i, distance, id })) {
      this.commit();
      this.treeVersion++;
      this.selectPath((this.doc?.paths.length ?? 1) - 1);
    }
  }

  /** Expand the selected path's stroke into a filled outline shape. */
  outlineStroke(): void {
    const i = this.selectedPathIndex;
    const p = i !== null ? this.doc?.paths[i] : null;
    if (i === null || !p) return;
    const eff = { ...(p.attributes ?? {}), ...(p.styleOverride ?? {}) };
    const width = Number(eff["stroke-width"] ?? "1") || 1;
    const id = this.#freshId("outline");
    if (this.#apply({ type: "outlineStroke", path: i, width, id })) {
      this.commit();
      this.treeVersion++;
      this.selectPath((this.doc?.paths.length ?? 1) - 1);
    }
  }

  /** Combine the selected paths with a boolean op — replaces them with one result path. */
  booleanOp(op: "union" | "intersect" | "subtract" | "exclude"): void {
    if (this.selectedPaths.length < 2) return;
    const id = this.#freshId(op);
    if (this.#apply({ type: "booleanOp", op, paths: [...this.selectedPaths], id })) {
      this.commit();
      this.treeVersion++;
      this.selectPath((this.doc?.paths.length ?? 1) - 1); // the appended result
    }
  }

  /** Merge the selected paths into one compound path (subpaths kept distinct — no geometry
   *  merge), e.g. a line + a detached dome as a single element. */
  combinePaths(): void {
    if (this.selectedPaths.length < 2) return;
    const id = this.#freshId("compound");
    if (this.#apply({ type: "combinePaths", paths: [...this.selectedPaths], id })) {
      this.commit();
      this.treeVersion++;
      this.selectPath((this.doc?.paths.length ?? 1) - 1);
    }
  }

  /** Release a compound path: split its subpaths back into independent, individually
   *  styleable paths (the inverse of `combinePaths`). Selects the freed pieces. */
  releaseCompound(): void {
    const i = this.selectedPathIndex;
    const p = i !== null ? (this.doc?.paths[i] ?? null) : null;
    if (i === null || !p || p.subpaths.length < 2) return;
    const n = p.subpaths.length;
    const ids = Array.from({ length: n }, (_, k) => this.#freshId(`${p.id} ${k + 1}`));
    if (this.#apply({ type: "releaseCompound", path: i, ids })) {
      this.commit();
      this.treeVersion++;
      const len = this.doc?.paths.length ?? 0;
      this.setSelectedPaths(Array.from({ length: n }, (_, k) => len - n + k));
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

  // --- groups + live booleans -------------------------------------------

  /** Wrap the current object selection into a new **live boolean** group (non-destructive:
   *  the members stay editable operands; the group renders the computed boolean, recomputed as
   *  they change). Needs ≥2 selected paths. */
  makeBooleanGroup(op: "union" | "subtract" | "intersect" | "exclude"): void {
    const uids = [...this.selectedPaths]
      .sort((a, b) => a - b)
      .map((i) => this.doc?.paths[i]?.uid)
      .filter((u): u is string => !!u);
    if (uids.length < 2) return;
    const uid = crypto.randomUUID();
    // Group + mark boolean as ONE undo step (apply both, commit once).
    if (this.#apply({ type: "groupNodes", uids, uid, name: this.#freshId(op) })) {
      this.#apply({ type: "setNodeBoolean", uid, op });
      this.commit();
      this.treeVersion++;
    }
  }

  /** Show/hide a single path. */
  setPathHidden(pathIndex: number, hidden: boolean): void {
    if (this.#apply({ type: "setPathHidden", path: pathIndex, hidden })) this.commit();
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

  // --- copy / paste style (paint) ----------------------------------------

  #styleClipboard = $state<{ style: Record<string, string>; gradients: Gradient[] } | null>(null);

  get canPasteStyle(): boolean {
    return this.#styleClipboard !== null;
  }

  /** Copy the selected path's effective paint/style (+ any referenced gradient defs). */
  copyStyle(): void {
    const p = this.selectedPathElement;
    if (!p) return;
    const eff = { ...(p.attributes ?? {}), ...(p.styleOverride ?? {}) };
    const style: Record<string, string> = {};
    for (const k of STYLE_KEYS) if (eff[k] != null) style[k] = eff[k];
    const gradients: Gradient[] = [];
    for (const v of Object.values(style)) {
      const id = v.startsWith("url(#") ? v.slice(5, -1) : null;
      const g = id ? this.gradientById(id) : null;
      if (g && !gradients.some((x) => x.id === g.id)) gradients.push(clone(g));
    }
    this.#styleClipboard = { style, gradients };
  }

  /** Apply the copied style to every selected path (one undo step); upserts its gradients. */
  pasteStyle(): void {
    const clip = this.#styleClipboard;
    if (!clip || this.selectedPaths.length === 0) return;
    let changed = false;
    for (const g of clip.gradients)
      changed = this.#apply({ type: "setGradient", gradient: g }) || changed;
    for (const i of this.selectedPaths)
      for (const [key, value] of Object.entries(clip.style))
        changed = this.#apply({ type: "setStyle", path: i, key, value }) || changed;
    if (changed) this.commit();
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
    this.treeVersion++; // the restored tree may differ (a mid-gesture add) → re-fetch
  }

  undo(): void {
    if (this.#wasm?.undo()) {
      this.dirty = true;
      this.#sync();
      this.treeVersion++; // undo can add/remove nodes or change structure → re-fetch the tree
      this.#persist();
    }
  }

  redo(): void {
    if (this.#wasm?.redo()) {
      this.dirty = true;
      this.#sync();
      this.treeVersion++;
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
      id: this.#freshId("path"),
      subpaths,
      attributes: { ...tools.newStyle },
    });
    const ref = { pathIndex, subpathIndex: 0, nodeIndex: 0 };
    this.#wasm.select(ref);
    this.#sync();
    this.treeVersion++; // a new drawn path node → the canvas/panel must re-fetch the render tree
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
      id: this.#freshId(spec.shape === "rect" ? "rectangle" : spec.shape),
      spec,
      attributes: { ...tools.newStyle },
    });
    const ref = { pathIndex, subpathIndex: 0, nodeIndex: 0 };
    this.#wasm.select(ref);
    this.#sync();
    this.treeVersion++; // a new drawn shape node → re-fetch the render tree
    return ref;
  }

  // --- clipboard + nudge -------------------------------------------------

  copySelected(): void {
    const p = this.selectedPathElement;
    if (!p) return;
    const attributes = p.added
      ? { ...(p.attributes ?? {}) }
      : { ...(p.attributes ?? {}), ...(p.styleOverride ?? {}) };
    this.#clipboard = { subpaths: clone(p.subpaths), attributes, name: p.id };
  }

  paste(): void {
    if (!this.#clipboard || !this.doc || !this.#wasm) return;
    const subpaths = clone(this.#clipboard.subpaths);
    offsetSubpaths(subpaths, 10, 10);
    const pathIndex = this.doc.paths.length;
    this.#apply({
      type: "addPath",
      id: this.#freshId(`${this.#clipboard.name} copy`),
      subpaths,
      attributes: { ...this.#clipboard.attributes },
    });
    this.#wasm.selectPath(pathIndex); // object-select the paste (transform box, not nodes)
    this.selectedPaths = [pathIndex];
    this.nodeEditIndex = null;
    this.commit();
    this.treeVersion++; // pasted path node → re-fetch the render tree
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
