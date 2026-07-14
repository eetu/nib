<script lang="ts">
  import Trash2 from "@lucide/svelte/icons/trash-2";

  import type { NodeType } from "$lib/model/types";
  import { editor } from "$lib/stores/document.svelte";
  import { tools } from "$lib/stores/tool.svelte";

  import ColorInput from "./ColorInput.svelte";

  const doc = $derived(editor.doc);
  const sel = $derived(editor.selection);
  const node = $derived(editor.selectedNode);

  // Style target = the selected path (a selected node implies its path).
  const path = $derived(editor.selectedPathElement);
  const pathIndex = $derived(editor.selectedPathIndex);
  const isCreateTool = $derived(tools.active === "pen" || tools.active === "circle");

  // Effective style being edited: a selected path (drawn = attributes, imported
  // = attributes + override), else the new-shape defaults when a create tool is
  // active — so you can set stroke/fill *before* drawing.
  const style = $derived<Record<string, string>>(
    path
      ? { ...(path.attributes ?? {}), ...(path.styleOverride ?? {}) }
      : isCreateTool
        ? tools.newStyle
        : {},
  );
  const opacityPct = $derived(Math.round((Number(style.opacity ?? "1") || 1) * 100));

  let opacityLive = $state<number | null>(null);
  const opacityShown = $derived(opacityLive ?? opacityPct);

  function round(v: number): number {
    return Math.round(v * 100) / 100;
  }

  function setStyle(key: string, value: string | null) {
    if (path && pathIndex !== null) editor.setPathStyle(pathIndex, key, value);
    else if (isCreateTool) tools.setNewStyle(key, value);
  }

  // Live preview while the color picker is open — reflect the change on the shape without
  // committing an undo step per event; setStyle (on picker close) records the single step.
  function previewStyle(key: string, value: string | null) {
    if (path && pathIndex !== null) editor.previewPathStyle(pathIndex, key, value);
    else if (isCreateTool) tools.setNewStyle(key, value);
  }

  function setWidth(e: Event) {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(v) && v >= 0) setStyle("stroke-width", String(v));
  }

  function onOpacityInput(e: Event) {
    opacityLive = Number((e.currentTarget as HTMLInputElement).value);
  }

  function onOpacityChange(e: Event) {
    const pct = Number((e.currentTarget as HTMLInputElement).value);
    opacityLive = null;
    setStyle("opacity", pct >= 100 ? null : String(round(pct / 100)));
  }

  function setX(e: Event) {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    if (sel && node && Number.isFinite(v)) editor.setNodePoint(sel, { x: v, y: node.point.y });
  }

  function setY(e: Event) {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    if (sel && node && Number.isFinite(v)) editor.setNodePoint(sel, { x: node.point.x, y: v });
  }

  function setType(type: NodeType) {
    if (sel) editor.setNodeType(sel, type);
  }

  let renaming = $state<number | null>(null);
  let renameValue = $state("");

  function startRename(pi: number, current: string) {
    renaming = pi;
    renameValue = current;
  }

  function commitRename(pi: number) {
    if (renaming !== pi) return;
    editor.renamePath(pi, renameValue);
    renaming = null;
  }

  function autofocus(node: HTMLInputElement) {
    node.focus();
    node.select();
  }
</script>

<aside class="inspector">
  <section>
    <h2>{path ? "style" : isCreateTool ? "new shape style" : "style"}</h2>
    {#if path || isCreateTool}
      <ColorInput
        label="fill"
        value={style.fill ?? "none"}
        editable
        oninput={(v) => previewStyle("fill", v)}
        onchange={(v) => setStyle("fill", v)}
      />
      <ColorInput
        label="stroke"
        value={style.stroke ?? "none"}
        editable
        oninput={(v) => previewStyle("stroke", v)}
        onchange={(v) => setStyle("stroke", v)}
      />
      <label class="row">
        width <input
          type="number"
          min="0"
          step="0.5"
          value={style["stroke-width"] ?? "1"}
          onchange={setWidth}
        />
      </label>
      <label class="row">
        opacity
        <input
          type="range"
          min="0"
          max="100"
          value={opacityShown}
          oninput={onOpacityInput}
          onchange={onOpacityChange}
        />
        <span class="pct">{opacityShown}%</span>
      </label>
    {:else}
      <p class="empty">no path selected</p>
    {/if}
  </section>

  <section>
    <h2>node</h2>
    {#if node && sel}
      <div class="coords">
        <label
          >x <input type="number" step="0.5" value={round(node.point.x)} onchange={setX} /></label
        >
        <label
          >y <input type="number" step="0.5" value={round(node.point.y)} onchange={setY} /></label
        >
      </div>
      <div class="typerow">
        <button class:active={node.type === "corner"} onclick={() => setType("corner")}
          >corner</button
        >
        <button class:active={node.type === "smooth"} onclick={() => setType("smooth")}
          >smooth</button
        >
      </div>
      <button class="delete" onclick={() => sel && editor.deleteNode(sel)}>
        <Trash2 size={15} /> delete node
      </button>
    {:else}
      <p class="empty">no node selected</p>
    {/if}
  </section>

  <section>
    <h2>snap</h2>
    <label class="row"
      ><input type="checkbox" bind:checked={tools.snapEnabled} /> snap to points</label
    >
    <label class="row sub">
      radius <input type="number" min="2" max="40" bind:value={tools.snapThresholdPx} /> px
    </label>
    <label class="row"
      ><input type="checkbox" bind:checked={tools.gridEnabled} /> snap to grid</label
    >
    <label class="row sub">
      size <input type="number" min="1" max="200" bind:value={tools.gridSize} />
    </label>
  </section>

  <section class="paths">
    <h2>paths</h2>
    {#if doc && doc.paths.some((p) => !p.deleted)}
      <ul>
        {#each doc.paths as p, pi (pi)}
          {#if !p.deleted}
            {@const nodes = p.subpaths.reduce((n, sp) => n + sp.nodes.length, 0)}
            {@const closed = p.subpaths.some((sp) => sp.closed)}
            <li>
              {#if renaming === pi}
                <input
                  class="rename"
                  bind:value={renameValue}
                  use:autofocus
                  onblur={() => commitRename(pi)}
                  onkeydown={(e) => {
                    if (e.key === "Enter") commitRename(pi);
                    else if (e.key === "Escape") renaming = null;
                  }}
                />
              {:else}
                <button
                  class="row-btn"
                  class:active={editor.selectedPathIndex === pi}
                  onclick={() => editor.selectPath(pi)}
                  ondblclick={() => startRename(pi, p.id)}
                  title="double-click to rename"
                >
                  <span class="pid">{p.id}</span>
                  <span class="meta">
                    {nodes} nodes{closed ? " · closed" : ""}{p.added
                      ? " · drawn"
                      : p.edited
                        ? " · edited"
                        : ""}
                  </span>
                </button>
              {/if}
              <button
                class="trash"
                title="delete path"
                aria-label="delete path"
                onclick={() => editor.deletePath(pi)}
              >
                <Trash2 size={14} />
              </button>
            </li>
          {/if}
        {/each}
      </ul>
    {:else}
      <p class="empty">no paths</p>
    {/if}
  </section>
</aside>

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    width: 232px;
    padding: 4px 0;
    background: var(--halo-bg-light);
    border-left: 1px solid var(--halo-border);
    overflow-y: auto;
  }

  section {
    padding: 12px 14px;
    border-bottom: 1px solid var(--halo-border);
  }

  section.paths {
    flex: 1;
    min-height: 0;
    border-bottom: none;
  }

  h2 {
    margin: 0 0 8px;
    font-family: var(--halo-font-heading);
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--halo-text-muted);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }

  .row.sub {
    padding-left: 20px;
    color: var(--halo-text-muted);
  }

  .row input[type="number"] {
    width: 56px;
  }

  .row input[type="range"] {
    flex: 1;
    min-width: 0;
  }

  .pct {
    width: 34px;
    text-align: right;
    color: var(--halo-text-muted);
    font-variant-numeric: tabular-nums;
  }

  .coords {
    display: flex;
    gap: 8px;
    margin-bottom: 8px;
  }

  .coords label {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--halo-text-muted);
  }

  .coords input {
    width: 100%;
  }

  .typerow {
    display: flex;
    gap: 4px;
    margin-bottom: 10px;
  }

  .typerow button {
    flex: 1;
    padding: 4px 0;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
  }

  .typerow button.active {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
    background: var(--halo-accent-soft);
  }

  .delete {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
  }

  .delete:hover {
    border-color: var(--halo-error);
    color: var(--halo-error);
  }

  .empty {
    margin: 0;
    color: var(--halo-text-muted);
    font-style: italic;
  }

  .paths ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .paths li {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .row-btn {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 6px 8px;
    border: none;
    border-radius: var(--halo-radius-pill);
    background: transparent;
    text-align: left;
    color: var(--halo-text-main);
  }

  .row-btn:hover {
    background: var(--halo-bg-main);
  }

  .row-btn.active {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  .rename {
    flex: 1;
    min-width: 0;
    margin: 2px 0;
    font-size: 12px;
  }

  .trash {
    flex: none;
    display: inline-flex;
    align-items: center;
    padding: 5px;
    border: none;
    border-radius: var(--halo-radius-pill);
    background: transparent;
    color: var(--halo-text-muted);
  }

  .trash:hover {
    color: var(--halo-error);
    background: var(--halo-bg-main);
  }

  .pid {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    font-size: 11px;
    color: var(--halo-text-muted);
  }
</style>
