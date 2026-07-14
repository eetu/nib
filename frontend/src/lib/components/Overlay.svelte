<script lang="ts">
  import { subpathsBounds } from "$lib/model/geometry";
  import { pathToD } from "$lib/model/path";
  import { nodeRefEquals, type Subpath } from "$lib/model/types";
  import { editor } from "$lib/stores/document.svelte";
  import { interaction } from "$lib/stores/interaction.svelte";
  import { tools } from "$lib/stores/tool.svelte";
  import { viewport } from "$lib/stores/viewport.svelte";
  import { handlePoints, padBounds, ROTATE_KNOB_PX, SELECT_PAD_PX } from "$lib/tools/transform";

  const doc = $derived(editor.doc);
  // Anchors show only while node-editing — any non-select tool, or the select tool in
  // node-edit mode (double-click). Object-mode select shows the transform box instead, so
  // the canvas stays uncluttered and a drag unambiguously moves the whole shape.
  const nodeEditing = $derived(tools.active !== "select" || editor.nodeEditIndex !== null);
  const sel = $derived(editor.selection);
  const selNode = $derived(editor.selectedNode);
  const selPath = $derived(editor.selectedPathIndex);
  // The transform box + centerline show only for an object (whole-path)
  // selection — node editing stays clean (just anchors + handles).
  const boxPath = $derived(
    editor.objectSelected ? (doc?.paths[editor.selectedPath ?? -1] ?? null) : null,
  );

  // Project a path's geometry into screen space so its outline can be traced as
  // a selection centerline in the (screen-space) overlay.
  function toScreenSubpaths(subpaths: Subpath[]): Subpath[] {
    return subpaths.map((sp) => ({
      closed: sp.closed,
      nodes: sp.nodes.map((n) => ({
        type: n.type,
        point: viewport.toScreen(n.point),
        handleIn: n.handleIn ? viewport.toScreen(n.handleIn) : undefined,
        handleOut: n.handleOut ? viewport.toScreen(n.handleOut) : undefined,
      })),
    }));
  }

  const outlineD = $derived(
    boxPath && !boxPath.deleted ? pathToD(toScreenSubpaths(boxPath.subpaths)) : "",
  );
</script>

{#if doc}
  <g class="overlay">
    {#each interaction.guidesX as gx (gx)}
      <line
        class="guide"
        x1={viewport.toScreen({ x: gx, y: 0 }).x}
        y1={0}
        x2={viewport.toScreen({ x: gx, y: 0 }).x}
        y2={viewport.pxHeight}
      />
    {/each}
    {#each interaction.guidesY as gy (gy)}
      <line
        class="guide"
        x1={0}
        y1={viewport.toScreen({ x: 0, y: gy }).y}
        x2={viewport.pxWidth}
        y2={viewport.toScreen({ x: 0, y: gy }).y}
      />
    {/each}
    {#if outlineD}
      <!-- selection centerline: light casing + accent core so it reads on any
           stroke colour (Pixelmator-style) -->
      <path class="sel-outline-casing" d={outlineD} />
      <path class="sel-outline" d={outlineD} />
    {/if}
    {#if boxPath && !boxPath.deleted}
      {@const raw = subpathsBounds(boxPath.subpaths)}
      {#if raw}
        {@const bb = padBounds(raw, viewport.toDocLength(SELECT_PAD_PX))}
        {@const tl = viewport.toScreen({ x: bb.minX, y: bb.minY })}
        {@const br = viewport.toScreen({ x: bb.maxX, y: bb.maxY })}
        <rect class="sel-box" x={tl.x} y={tl.y} width={br.x - tl.x} height={br.y - tl.y} />
        {@const top = viewport.toScreen({ x: (bb.minX + bb.maxX) / 2, y: bb.minY })}
        <line class="rotate-stem" x1={top.x} y1={top.y} x2={top.x} y2={top.y - ROTATE_KNOB_PX} />
        <circle class="rotate-knob" cx={top.x} cy={top.y - ROTATE_KNOB_PX} r="4.5" />
        {#each handlePoints(bb) as h (h.handle)}
          {@const hp = viewport.toScreen(h.point)}
          <rect class="xf-handle" x={hp.x - 4} y={hp.y - 4} width="8" height="8" />
        {/each}
      {/if}
    {/if}
    {#if nodeEditing}
      {#each doc.paths as path, pi (pi)}
        {#if !path.deleted}
          {#each path.subpaths as sp, si (si)}
            {#each sp.nodes as node, ni (ni)}
              {@const s = viewport.toScreen(node.point)}
              {@const selected = nodeRefEquals(sel, {
                pathIndex: pi,
                subpathIndex: si,
                nodeIndex: ni,
              })}
              {#if node.type === "smooth"}
                <circle
                  class="anchor"
                  class:inpath={pi === selPath && !editor.objectSelected}
                  class:selected
                  cx={s.x}
                  cy={s.y}
                  r="4.5"
                />
              {:else}
                <rect
                  class="anchor"
                  class:inpath={pi === selPath && !editor.objectSelected}
                  class:selected
                  x={s.x - 4}
                  y={s.y - 4}
                  width="8"
                  height="8"
                />
              {/if}
            {/each}
          {/each}
        {/if}
      {/each}
    {/if}

    {#if sel && selNode}
      {@const p = viewport.toScreen(selNode.point)}
      <circle class="sel-ring" cx={p.x} cy={p.y} r="8" />
      {#if selNode.handleIn}
        {@const h = viewport.toScreen(selNode.handleIn)}
        <line class="handle-line" x1={p.x} y1={p.y} x2={h.x} y2={h.y} />
        <circle class="handle" cx={h.x} cy={h.y} r="4" />
      {/if}
      {#if selNode.handleOut}
        {@const h = viewport.toScreen(selNode.handleOut)}
        <line class="handle-line" x1={p.x} y1={p.y} x2={h.x} y2={h.y} />
        <circle class="handle" cx={h.x} cy={h.y} r="4" />
      {/if}
    {/if}

    {#if interaction.penDrawing && interaction.penCursor && selNode}
      {@const a = viewport.toScreen(selNode.point)}
      {@const b = viewport.toScreen(interaction.penCursor)}
      <line class="pen-rubber" x1={a.x} y1={a.y} x2={b.x} y2={b.y} />
    {/if}

    {#if interaction.resumePoint && !interaction.penDrawing}
      {@const s = viewport.toScreen(interaction.resumePoint)}
      <circle class="resume" cx={s.x} cy={s.y} r="9" />
    {/if}

    {#if interaction.snapPoint}
      {@const s = viewport.toScreen(interaction.snapPoint)}
      <circle
        class="snap"
        class:closing={interaction.closing}
        cx={s.x}
        cy={s.y}
        r={interaction.closing ? 11 : 8}
      />
    {/if}
  </g>
{/if}

<style>
  .overlay {
    pointer-events: none;
  }

  /* selection centerline: light casing + accent core (fill:none is critical —
     a closed path would otherwise render a black fill). */
  .sel-outline-casing {
    fill: none;
    stroke: #ffffff;
    stroke-width: 3;
    opacity: 0.55;
  }

  .sel-outline {
    fill: none;
    stroke: var(--halo-accent);
    stroke-width: 1.25;
  }

  /* selection bounding box around the selected path */
  .sel-box {
    fill: none;
    stroke: var(--halo-accent);
    stroke-width: 1;
    stroke-dasharray: 4 3;
    opacity: 0.7;
  }

  /* resize handles on the bounding box */
  .xf-handle {
    fill: var(--halo-bg-main);
    stroke: var(--halo-accent);
    stroke-width: 1.5;
  }

  /* smart alignment guides while dragging */
  .guide {
    stroke: var(--halo-accent);
    stroke-width: 1;
    opacity: 0.9;
    pointer-events: none;
  }

  /* rotate knob above the box top-centre */
  .rotate-stem {
    stroke: var(--halo-accent);
    stroke-width: 1;
    opacity: 0.7;
  }

  .rotate-knob {
    fill: var(--halo-bg-main);
    stroke: var(--halo-accent);
    stroke-width: 1.5;
  }

  .anchor {
    fill: var(--halo-bg-main);
    stroke: var(--halo-text-muted);
    stroke-width: 1.5;
  }

  /* anchors of the selected path get an accent outline (path selected) */
  .anchor.inpath {
    stroke: var(--halo-accent);
  }

  .anchor.selected {
    fill: var(--halo-accent);
    stroke: var(--halo-accent);
  }

  /* A ring around the selected node so it stays clearly visible amid the
     handle knobs. */
  .sel-ring {
    fill: none;
    stroke: var(--halo-accent);
    stroke-width: 1.5;
    opacity: 0.9;
  }

  .pen-rubber {
    stroke: var(--halo-accent);
    stroke-width: 1.5;
    stroke-dasharray: 4 3;
    opacity: 0.8;
  }

  .handle-line {
    stroke: var(--halo-accent);
    stroke-width: 1;
    opacity: 0.7;
  }

  .handle {
    fill: var(--halo-body);
    stroke: var(--halo-accent);
    stroke-width: 1.5;
  }

  .snap {
    fill: none;
    stroke: var(--halo-accent);
    stroke-width: 2;
  }

  /* "resume drawing from here" ring on an open endpoint the pen can pick up. */
  .resume {
    fill: var(--halo-accent-soft);
    stroke: var(--halo-accent);
    stroke-width: 2;
  }

  .snap.closing {
    fill: var(--halo-accent-soft);
    stroke-width: 2.5;
  }
</style>
