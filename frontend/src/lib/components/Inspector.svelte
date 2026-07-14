<script lang="ts">
  import AlignCenterHorizontal from "@lucide/svelte/icons/align-center-horizontal";
  import AlignCenterVertical from "@lucide/svelte/icons/align-center-vertical";
  import AlignEndHorizontal from "@lucide/svelte/icons/align-end-horizontal";
  import AlignEndVertical from "@lucide/svelte/icons/align-end-vertical";
  import AlignHorizontalDistributeCenter from "@lucide/svelte/icons/align-horizontal-distribute-center";
  import AlignStartHorizontal from "@lucide/svelte/icons/align-start-horizontal";
  import AlignStartVertical from "@lucide/svelte/icons/align-start-vertical";
  import AlignVerticalDistributeCenter from "@lucide/svelte/icons/align-vertical-distribute-center";
  import Trash2 from "@lucide/svelte/icons/trash-2";

  import { subpathsBounds } from "$lib/model/geometry";
  import type { NodeType } from "$lib/model/types";
  import { editor } from "$lib/stores/document.svelte";
  import { tools } from "$lib/stores/tool.svelte";
  import { scaleSubpaths } from "$lib/tools/transform";

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

  function onDash(e: Event) {
    // A dash pattern like "4 2"; blank clears it back to a solid stroke.
    const v = (e.currentTarget as HTMLInputElement).value.trim();
    setStyle("stroke-dasharray", v || null);
  }

  function onOpacityInput(e: Event) {
    opacityLive = Number((e.currentTarget as HTMLInputElement).value);
  }

  function onOpacityChange(e: Event) {
    const pct = Number((e.currentTarget as HTMLInputElement).value);
    opacityLive = null;
    setStyle("opacity", pct >= 100 ? null : String(round(pct / 100)));
  }

  // Evaluate a numeric field that may hold a simple arithmetic expression ("100+20",
  // "3*4"). No eval/Function — a plain number or one binary op of + - * /.
  function evalNum(raw: string): number | null {
    const s = raw.trim();
    if (/^[-+]?\d*\.?\d+$/.test(s)) return Number(s);
    const m = s.match(/^([-+]?\d*\.?\d+)\s*([-+*/])\s*([-+]?\d*\.?\d+)$/);
    if (!m) return null;
    const a = Number(m[1]);
    const b = Number(m[3]);
    const r =
      m[2] === "+" ? a + b : m[2] === "-" ? a - b : m[2] === "*" ? a * b : b !== 0 ? a / b : NaN;
    return Number.isFinite(r) ? r : null;
  }

  function setX(e: Event) {
    const v = evalNum((e.currentTarget as HTMLInputElement).value);
    if (sel && node && v !== null) editor.setNodePoint(sel, { x: v, y: node.point.y });
  }

  function setY(e: Event) {
    const v = evalNum((e.currentTarget as HTMLInputElement).value);
    if (sel && node && v !== null) editor.setNodePoint(sel, { x: node.point.x, y: v });
  }

  function setType(type: NodeType) {
    if (sel) editor.setNodeType(sel, type);
  }

  // The selected path's bounding box, for the numeric transform panel.
  const bounds = $derived(path ? subpathsBounds(path.subpaths) : null);

  // Edit a bbox field: x/y translate the whole path; w/h scale it about its top-left corner.
  function setBBox(axis: "x" | "y" | "w" | "h", e: Event) {
    const v = evalNum((e.currentTarget as HTMLInputElement).value);
    if (v === null || !bounds || !path || pathIndex === null) return;
    const anchor = { x: bounds.minX, y: bounds.minY };
    const w = bounds.maxX - bounds.minX;
    const h = bounds.maxY - bounds.minY;
    if (axis === "x") editor.movePathBy(pathIndex, v - bounds.minX, 0);
    else if (axis === "y") editor.movePathBy(pathIndex, 0, v - bounds.minY);
    else if (axis === "w" && w > 0)
      editor.setSubpaths(pathIndex, scaleSubpaths(path.subpaths, anchor, v / w, 1));
    else if (axis === "h" && h > 0)
      editor.setSubpaths(pathIndex, scaleSubpaths(path.subpaths, anchor, 1, v / h));
    else return;
    editor.commit();
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
  {#if editor.multiSelected}
    <section>
      <h2>arrange · {editor.selectedPaths.length}</h2>
      <div class="arrange">
        <button title="align left" onclick={() => editor.align("left")}>
          <AlignStartVertical size={16} />
        </button>
        <button title="align horizontal centres" onclick={() => editor.align("hcenter")}>
          <AlignCenterVertical size={16} />
        </button>
        <button title="align right" onclick={() => editor.align("right")}>
          <AlignEndVertical size={16} />
        </button>
        <button title="align top" onclick={() => editor.align("top")}>
          <AlignStartHorizontal size={16} />
        </button>
        <button title="align vertical centres" onclick={() => editor.align("vcenter")}>
          <AlignCenterHorizontal size={16} />
        </button>
        <button title="align bottom" onclick={() => editor.align("bottom")}>
          <AlignEndHorizontal size={16} />
        </button>
      </div>
      {#if editor.selectedPaths.length >= 3}
        <div class="arrange">
          <button title="distribute horizontally" onclick={() => editor.distribute("h")}>
            <AlignHorizontalDistributeCenter size={16} />
          </button>
          <button title="distribute vertically" onclick={() => editor.distribute("v")}>
            <AlignVerticalDistributeCenter size={16} />
          </button>
        </div>
      {/if}
    </section>
  {/if}

  {#if path || isCreateTool}
    <section>
      <h2>{path ? "style" : "new shape style"}</h2>
      {#snippet seg(label: string, key: string, options: string[], dflt: string)}
        <div class="segrow">
          <span class="seglbl">{label}</span>
          <div class="segbtns">
            {#each options as opt (opt)}
              <button
                class:active={(style[key] ?? dflt) === opt}
                onclick={() => setStyle(key, opt)}
              >
                {opt}
              </button>
            {/each}
          </div>
        </div>
      {/snippet}
      <ColorInput
        label="fill"
        value={style.fill ?? "none"}
        editable
        oninput={(v) => previewStyle("fill", v)}
        onchange={(v) => setStyle("fill", v)}
      />
      {@render seg("fill rule", "fill-rule", ["nonzero", "evenodd"], "nonzero")}
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
      {@render seg("cap", "stroke-linecap", ["butt", "round", "square"], "butt")}
      {@render seg("join", "stroke-linejoin", ["miter", "round", "bevel"], "miter")}
      <label class="row">
        dash <input
          class="dash"
          type="text"
          value={style["stroke-dasharray"] ?? ""}
          placeholder="none"
          onchange={onDash}
          spellcheck="false"
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
    </section>
  {/if}

  {#if path && bounds}
    <section>
      <h2>transform</h2>
      <div class="coords">
        <label
          >x <input
            type="text"
            value={round(bounds.minX)}
            onchange={(e) => setBBox("x", e)}
          /></label
        >
        <label
          >y <input
            type="text"
            value={round(bounds.minY)}
            onchange={(e) => setBBox("y", e)}
          /></label
        >
      </div>
      <div class="coords">
        <label
          >w <input
            type="text"
            value={round(bounds.maxX - bounds.minX)}
            onchange={(e) => setBBox("w", e)}
          /></label
        >
        <label
          >h <input
            type="text"
            value={round(bounds.maxY - bounds.minY)}
            onchange={(e) => setBBox("h", e)}
          /></label
        >
      </div>
      {#if editor.objectSelected}
        <p class="hint">double-click to edit nodes</p>
      {/if}
    </section>
  {/if}

  {#if node && sel}
    <section>
      <h2>node</h2>
      <div class="coords">
        <label>x <input type="text" value={round(node.point.x)} onchange={setX} /></label>
        <label>y <input type="text" value={round(node.point.y)} onchange={setY} /></label>
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
    </section>
  {/if}

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
                  class:active={editor.selectedPaths.includes(pi)}
                  onclick={(e) => (e.shiftKey ? editor.togglePath(pi) : editor.selectPath(pi))}
                  ondblclick={() => startRename(pi, p.id)}
                  title="click to select · shift-click to multi-select · double-click to rename"
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

  /* Segmented style controls (cap / join / fill-rule). */
  .segrow {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }

  .seglbl {
    width: 44px;
    color: var(--halo-text-muted);
  }

  .segbtns {
    display: flex;
    flex: 1;
    min-width: 0;
    gap: 4px;
  }

  .segbtns button {
    flex: 1;
    min-width: 0;
    padding: 3px 0;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
    font-size: 11px;
    text-transform: capitalize;
  }

  .segbtns button.active {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
    background: var(--halo-accent-soft);
  }

  .dash {
    flex: 1;
    min-width: 0;
    font-size: 12px;
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

  .hint {
    margin: 2px 0 0;
    font-size: 11px;
    color: var(--halo-text-muted);
  }

  /* align / distribute icon buttons */
  .arrange {
    display: flex;
    gap: 4px;
    margin-bottom: 6px;
  }

  .arrange button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 5px 0;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
  }

  .arrange button:hover {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
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
