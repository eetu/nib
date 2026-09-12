<script lang="ts">
  /**
   * LAYERS — the document's object tree, and the panel that *is* it.
   *
   * Every path/shape is a row and every `<g>` a collapsible group, nested to any depth and
   * reversed so top-of-stack reads first; z-order is document order. Rows rename in place, drag to
   * reorder and to move in and out of groups, toggle visibility and lock, and answer right-click
   * with the same menu their verbs live in.
   *
   * Lifted out of Inspector, which held both this and the style/arrange panels — two different
   * questions ("what exists?" and "what am I working on?") that the family layout puts on opposite
   * sides of the window. Moving it there is the next step; this seam is what lets it move.
   */
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import Eye from "@lucide/svelte/icons/eye";
  import EyeOff from "@lucide/svelte/icons/eye-off";
  import Group from "@lucide/svelte/icons/group";
  import Lock from "@lucide/svelte/icons/lock";
  import LockOpen from "@lucide/svelte/icons/lock-open";

  import { type MenuItem, openMenu } from "$lib/menu.svelte";
  import { tightBounds } from "$lib/model/geometry";
  import { pathToD } from "$lib/model/path";
  import type { PathElement, RenderNode } from "$lib/model/types";
  import { editor } from "$lib/stores/document.svelte";
  import { settings } from "$lib/stores/settings.svelte";

  const advanced = $derived(settings.uiLevel === "advanced");
  const doc = $derived(editor.doc);

  let collapsed = $state<string[]>([]);
  function toggleCollapse(id: string) {
    collapsed = collapsed.includes(id) ? collapsed.filter((x) => x !== id) : [...collapsed, id];
  }

  // --- nested object tree (E3): the panel renders the imported document structure from the core
  // render tree; drawn (added) content renders below via `drawnRows`. ---

  // Elements that aren't visual layers — hidden from the panel (+ their subtrees).
  const PANEL_SKIP = new Set(["defs", "style", "metadata", "title", "desc", "script"]);

  // The render tree, fetched from the core; guarded so it re-fetches only on a new source or a
  // structural op (treeVersion bump), not on every geometry edit / selection.
  let panelTree = $state<RenderNode[]>([]);
  let panelSrc: string | undefined;
  let panelVer = -1;
  $effect(() => {
    const src = doc?.source;
    const ver = editor.treeVersion;
    if (src === panelSrc && ver === panelVer) return;
    panelSrc = src;
    panelVer = ver;
    panelTree = doc ? editor.renderTree() : [];
  });

  // uid → doc.paths array index, so a tree shape row maps to its editable path.
  const uidToIndex = $derived(
    new Map((doc?.paths ?? []).flatMap((p, i) => (p.uid ? [[p.uid, i] as const] : []))),
  );

  // A tree element node is a *group* if it has visual element children (a `<g>`); a *shape* if it
  // maps to an editable path; else an opaque *leaf* (text / image / use).
  function treeKind(n: RenderNode): "group" | "shape" | "leaf" | "skip" {
    if (n.kind !== "element" || PANEL_SKIP.has(n.tag)) return "skip";
    const idx = uidToIndex.get(n.uid);
    if (idx !== undefined) return doc?.paths[idx]?.deleted ? "skip" : "shape";
    const hasKids = n.children.some((c) => c.kind === "element" && !PANEL_SKIP.has(c.tag));
    return hasKids ? "group" : "leaf";
  }
  function treeName(n: RenderNode): string {
    if (n.kind !== "element") return "";
    // A label's own words name it better than "text" or a generated id ever could — it's how
    // anyone refers to it out loud ("the title", "Rotate me"). An explicit id still wins, since
    // someone typed it on purpose.
    if (n.tag === "text" && !n.attrs.id) {
      const words = nodeText(n).trim();
      if (words) return words;
    }
    return n.attrs.id || n.tag;
  }

  /** The words inside an element, its `<tspan>`s included. */
  function nodeText(n: RenderNode): string {
    if (n.kind === "text") return n.text;
    return n.kind === "element" ? n.children.map(nodeText).join("") : "";
  }

  // Group context menu: reorder (z), toggle/flip the live-boolean op, ungroup. Works on any tree
  // group node (imported or drawn) — one representation.
  function openTreeGroupMenu(e: MouseEvent, n: RenderNode) {
    if (n.kind !== "element") return;
    const uid = n.uid;
    const items: MenuItem[] = [
      { label: "rename", run: () => startGroupRename(uid, treeName(n)) },
      { label: "bring to front", run: () => editor.reorderNodeExtreme(uid, true) },
      { label: "bring forward", run: () => editor.reorderNode(uid, true) },
      { label: "send backward", run: () => editor.reorderNode(uid, false) },
      { label: "send to back", run: () => editor.reorderNodeExtreme(uid, false) },
    ];
    // Live-boolean ops are a pro feature. In basic (touch-up) mode they stay listed but grey,
    // saying where they live — a verb that vanishes teaches that it doesn't exist.
    for (const op of ["union", "subtract", "intersect", "exclude"] as const) {
      items.push({
        label: op,
        hint: n.booleanOp === op ? "on" : advanced ? undefined : "advanced mode",
        disabled: !advanced,
        run: () => editor.setNodeBoolean(uid, op),
      });
    }
    if (n.booleanOp)
      items.push({
        label: "flatten (plain group)",
        disabled: !advanced,
        hint: advanced ? undefined : "advanced mode",
        run: () => editor.setNodeBoolean(uid, null),
      });
    items.push({ label: "ungroup", run: () => editor.ungroupNode(uid) });
    openMenu(e, treeName(n), items);
  }

  // Group the current selection into a nested `<g>` on the tree. Delegates to the facade so the
  // button and the ⌘G shortcut share one behaviour (naming, guards).
  function groupSelected() {
    editor.groupSelection();
  }

  function openPathMenu(e: MouseEvent, index: number, name: string) {
    const uid = doc?.paths[index]?.uid;
    openMenu(e, name, [
      { label: "rename", run: () => startRename(index, name) },
      { label: "duplicate", run: () => (editor.selectPath(index), editor.duplicateSelected()) },
      // Tree z-order needs a tree node; a drawn row that has none reorders by dragging instead,
      // and says so rather than hiding four verbs the row above it offers.
      {
        label: "bring to front",
        disabled: !uid,
        hint: uid ? undefined : "drag the row to reorder",
        run: () => uid && editor.reorderNodeExtreme(uid, true),
      },
      {
        label: "bring forward",
        disabled: !uid,
        hint: uid ? undefined : "drag the row to reorder",
        run: () => uid && editor.reorderNode(uid, true),
      },
      {
        label: "send backward",
        disabled: !uid,
        hint: uid ? undefined : "drag the row to reorder",
        run: () => uid && editor.reorderNode(uid, false),
      },
      {
        label: "send to back",
        disabled: !uid,
        hint: uid ? undefined : "drag the row to reorder",
        run: () => uid && editor.reorderNodeExtreme(uid, false),
      },
      { label: "delete", danger: true, run: () => editor.deletePath(index) },
    ]);
  }

  const BOOL_GLYPH: Record<string, string> = {
    union: "∪",
    subtract: "−",
    intersect: "∩",
    exclude: "⊕",
  };

  // A path's thumbnail fill/stroke: use its hex fill if any, else outline it in the accent.
  function thumbFill(p: PathElement): string {
    const f = p.attributes?.fill ?? p.styleOverride?.fill;
    return f && f.startsWith("#") ? f : "none";
  }
  function thumbStroke(p: PathElement): string {
    return thumbFill(p) === "none" ? "var(--halo-text-muted)" : "none";
  }

  let renaming = $state<number | null>(null);
  let renameValue = $state("");

  // Drag-drop reorder of the object tree by uid: reorder z (before/after) + move in/out of groups
  // (inside). The panel is reversed (top-of-stack first), so the visual drop maps to doc order:
  // dropping on a row's upper half = above in the panel = higher z = "after" in document order.
  let dragUid = $state<string | null>(null);
  let dropUid = $state<string | null>(null);
  let dropPos = $state<"before" | "after" | "inside">("before");

  function onRowDragStart(uid: string, e: DragEvent) {
    dragUid = uid;
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  function onRowDragOver(uid: string, isGroup: boolean, e: DragEvent) {
    if (!dragUid || dragUid === uid) return;
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const frac = (e.clientY - r.top) / r.height;
    // A group's middle band = drop inside; otherwise before/after (reversed → upper half = after).
    dropUid = uid;
    dropPos = isGroup && frac > 0.25 && frac < 0.75 ? "inside" : frac < 0.5 ? "after" : "before";
  }
  function onRowDrop(e: DragEvent) {
    e.preventDefault();
    if (dragUid && dropUid && dragUid !== dropUid) editor.moveTreeNode(dragUid, dropUid, dropPos);
    dragUid = null;
    dropUid = null;
  }
  function onRowDragEnd() {
    dragUid = null;
    dropUid = null;
  }

  function startRename(pi: number, current: string) {
    renaming = pi;
    renameValue = current;
  }

  function commitRename(pi: number) {
    if (renaming !== pi) return;
    editor.renamePath(pi, renameValue);
    renaming = null;
  }

  // Group rename edits the `<g>`'s `id` (its display name) via SetNodeAttr — keyed by uid, since a
  // group has no path index. Mirrors path rename (double-click / context menu).
  let renamingGroup = $state<string | null>(null);
  let groupRenameValue = $state("");
  function startGroupRename(uid: string, current: string) {
    renamingGroup = uid;
    groupRenameValue = current;
  }
  function commitGroupRename(uid: string) {
    if (renamingGroup !== uid) return;
    const name = groupRenameValue.trim();
    if (name) editor.setNodeAttr(uid, "id", name);
    renamingGroup = null;
  }

  function autofocus(node: HTMLInputElement) {
    node.focus();
    node.select();
  }
</script>

{#snippet pathRow(p: PathElement, index: number, nested: boolean, uid: string)}
  {@const b = tightBounds(p.subpaths)}
  <li
    class="pathrow"
    class:nested
    class:locked={p.locked}
    class:dropbefore={dropUid === uid && dropPos === "before"}
    class:dropafter={dropUid === uid && dropPos === "after"}
    draggable={renaming !== index}
    ondragstart={(e) => onRowDragStart(uid, e)}
    ondragover={(e) => onRowDragOver(uid, false, e)}
    ondragleave={() => (dropUid === uid ? (dropUid = null) : null)}
    ondrop={onRowDrop}
    ondragend={onRowDragEnd}
    oncontextmenu={(e) => openPathMenu(e, index, p.id)}
  >
    {#if b && b.maxX > b.minX && b.maxY > b.minY}
      {@const w = b.maxX - b.minX}
      {@const h = b.maxY - b.minY}
      {@const pad = Math.max(w, h) * 0.15}
      <svg
        class="thumb"
        viewBox="{b.minX - pad} {b.minY - pad} {w + pad * 2} {h + pad * 2}"
        preserveAspectRatio="xMidYMid meet"
        aria-hidden="true"
      >
        <path
          d={pathToD(p.subpaths)}
          fill={thumbFill(p)}
          stroke={thumbStroke(p)}
          stroke-width="1.5"
          vector-effect="non-scaling-stroke"
        />
      </svg>
    {:else}
      <span class="thumb empty"></span>
    {/if}
    {#if renaming === index}
      <input
        class="rename"
        bind:value={renameValue}
        use:autofocus
        onblur={() => commitRename(index)}
        onkeydown={(e) => {
          if (e.key === "Enter") commitRename(index);
          else if (e.key === "Escape") renaming = null;
        }}
      />
    {:else}
      <button
        class="row-btn"
        class:active={editor.selectedPaths.includes(index)}
        onclick={(e) =>
          e.shiftKey || e.metaKey ? editor.togglePath(index) : editor.selectPath(index)}
        ondblclick={() => startRename(index, p.id)}
        title="click to select · shift/⌘-click multi · double-click to rename · right-click for more"
      >
        <span class="pid">{p.id}</span>
      </button>
    {/if}
    <button
      class="eye"
      class:on={p.locked}
      title={p.locked ? "unlock" : "lock (not selectable on canvas)"}
      aria-label="toggle lock"
      onclick={() => editor.setPathLocked(index, !p.locked)}
    >
      {#if p.locked}<Lock size={13} />{:else}<LockOpen size={13} />{/if}
    </button>
    <button
      class="eye"
      title={p.hidden ? "show" : "hide"}
      aria-label="toggle visibility"
      onclick={() => editor.setPathHidden(index, !p.hidden)}
    >
      {#if p.hidden}<EyeOff size={13} />{:else}<Eye size={13} />{/if}
    </button>
  </li>
{/snippet}
{#snippet treeRow(n: RenderNode, depth: number)}
  {#if n.kind === "element"}
    {@const kind = treeKind(n)}
    {#if kind === "shape"}
      {@const idx = uidToIndex.get(n.uid)}
      {@const p = idx !== undefined ? doc?.paths[idx] : undefined}
      {#if p && idx !== undefined}
        {@render pathRow(p, idx, depth > 0, n.uid)}
      {/if}
    {:else if kind === "group"}
      <li
        class="grouphead"
        class:dropbefore={dropUid === n.uid && dropPos === "before"}
        class:dropafter={dropUid === n.uid && dropPos === "after"}
        class:dropinside={dropUid === n.uid && dropPos === "inside"}
        draggable="true"
        ondragstart={(e) => onRowDragStart(n.uid, e)}
        ondragover={(e) => onRowDragOver(n.uid, true, e)}
        ondragleave={() => (dropUid === n.uid ? (dropUid = null) : null)}
        ondrop={onRowDrop}
        ondragend={onRowDragEnd}
        oncontextmenu={(e) => openTreeGroupMenu(e, n)}
      >
        <button class="chev" aria-label="collapse group" onclick={() => toggleCollapse(n.uid)}>
          {#if collapsed.includes(n.uid)}<ChevronRight size={13} />{:else}<ChevronDown
              size={13}
            />{/if}
        </button>
        {#if renamingGroup === n.uid}
          <input
            class="rename"
            bind:value={groupRenameValue}
            use:autofocus
            onblur={() => commitGroupRename(n.uid)}
            onkeydown={(e) => {
              if (e.key === "Enter") commitGroupRename(n.uid);
              else if (e.key === "Escape") renamingGroup = null;
            }}
          />
        {:else}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span
            class="lname"
            ondblclick={() => startGroupRename(n.uid, treeName(n))}
            title="double-click to rename · right-click for group actions">{treeName(n)}</span
          >
        {/if}
        {#if n.booleanOp}
          <span class="bool-badge" title="live boolean: {n.booleanOp}"
            >{BOOL_GLYPH[n.booleanOp]}</span
          >
        {/if}
        <button
          class="eye"
          title={n.hidden ? "show group" : "hide group"}
          aria-label="toggle group visibility"
          onclick={() => editor.setNodeHidden(n.uid, !n.hidden)}
        >
          {#if n.hidden}<EyeOff size={13} />{:else}<Eye size={13} />{/if}
        </button>
      </li>
      {#if !collapsed.includes(n.uid)}
        {#each [...n.children].reverse() as c, i (i)}{@render treeRow(c, depth + 1)}{/each}
      {/if}
    {:else if kind === "leaf"}
      <li
        class="pathrow"
        class:nested={depth > 0}
        class:dropbefore={dropUid === n.uid && dropPos === "before"}
        class:dropafter={dropUid === n.uid && dropPos === "after"}
        draggable="true"
        ondragstart={(e) => onRowDragStart(n.uid, e)}
        ondragover={(e) => onRowDragOver(n.uid, false, e)}
        ondragleave={() => (dropUid === n.uid ? (dropUid = null) : null)}
        ondrop={onRowDrop}
        ondragend={onRowDragEnd}
      >
        <!-- A label has no geometry to draw a thumbnail from, so it gets the sign for text
               instead of a blank square: the slot still says what kind of thing the row is. -->
        {#if n.tag === "text"}
          <span class="thumb glyph" aria-hidden="true">T</span>
        {:else}
          <span class="thumb empty"></span>
        {/if}
        <button
          class="row-btn"
          class:active={editor.selectedElementUid === n.uid}
          onclick={() => editor.selectElement(n.uid)}
          title="click to select · edit its attributes in the panel"
        >
          <span class="pid">{treeName(n)}</span>
        </button>
        <button
          class="eye"
          title={n.hidden ? "show" : "hide"}
          aria-label="toggle visibility"
          onclick={() => editor.setNodeHidden(n.uid, !n.hidden)}
        >
          {#if n.hidden}<EyeOff size={13} />{:else}<Eye size={13} />{/if}
        </button>
      </li>
    {/if}
  {/if}
{/snippet}
<section class="layers">
  {#if editor.selectedPaths.length > 1 && advanced}
    <div class="lhead">
      <button
        class="ghost-btn"
        title="group selection"
        aria-label="group selection"
        onclick={groupSelected}
      >
        <Group size={13} /> group
      </button>
    </div>
  {/if}
  {#if panelTree.length}
    <ul class="layerlist">
      <!-- the whole document — imported + drawn, nested groups + booleans — as one tree;
             reversed so top-of-stack shows first (later in document order = drawn on top) -->
      {#each [...panelTree].reverse() as n, i (i)}{@render treeRow(n, 0)}{/each}
    </ul>
  {:else}
    <p class="empty">no shapes</p>
  {/if}
</section>

<style>
  .layerlist {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
  }

  .layerlist li {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 1px 2px;
    border-radius: var(--halo-radius-pill);
  }

  /* nested path rows sit under their group header */
  .layerlist li.nested {
    padding-left: 16px;
  }

  /* per-shape thumbnail on the left of a row */
  .thumb {
    width: 20px;
    height: 20px;
    flex: none;
  }

  .thumb.empty {
    display: inline-block;
  }

  /* drop indicators (panel is reversed, so doc-"after" = higher z = a line at the row's top). */
  .layerlist li.dropafter {
    box-shadow: inset 0 2px 0 var(--halo-accent);
  }

  .layerlist li.dropbefore {
    box-shadow: inset 0 -2px 0 var(--halo-accent);
  }

  .layerlist li.dropinside {
    box-shadow: inset 0 0 0 2px var(--halo-accent);
  }

  .layerlist .eye,
  .layerlist .chev {
    display: inline-flex;
    padding: 4px;
    border: none;
    background: transparent;
    color: var(--halo-text-muted);
  }

  .layerlist .eye:hover,
  .layerlist .chev:hover {
    color: var(--halo-accent);
  }

  .layerlist .eye.on {
    color: var(--halo-accent);
  }

  /* the text sign in a label row's thumbnail slot */
  .layerlist .thumb.glyph {
    display: grid;
    place-items: center;
    color: var(--halo-text-muted);
    font-family: var(--halo-font-heading);
    font-size: 13px;
    line-height: 1;
  }

  /* a locked row reads as inert (it isn't selectable on the canvas) */
  .pathrow.locked .row-btn,
  .pathrow.locked .thumb {
    opacity: 0.5;
  }

  .grouphead {
    font-family: var(--halo-font-heading);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .layerlist .lname {
    flex: 1;
    min-width: 0;
    padding: 4px 2px;
    border: none;
    background: transparent;
    color: var(--halo-text-main);
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pid {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Standing on its own in the navigator, it carries the padding the Inspector's `section`
     rhythm used to give it. */
  section.layers {
    display: flex;
    flex: 1;
    min-height: 0;
    flex-direction: column;
    padding: 10px 12px;
  }

  .bool-badge {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: var(--halo-radius);
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
    font-size: 11px;
    line-height: 1;
  }
</style>
