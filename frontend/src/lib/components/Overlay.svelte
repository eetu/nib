<script lang="ts">
  import { pathToD } from "$lib/model/path";
  import { nodeRefEquals, type Subpath } from "$lib/model/types";
  import { editor } from "$lib/stores/document.svelte";
  import { interaction } from "$lib/stores/interaction.svelte";
  import { tools } from "$lib/stores/tool.svelte";
  import { viewport } from "$lib/stores/viewport.svelte";
  import { activePivot } from "$lib/tools/rotate";
  import {
    type BoxFrame,
    boxFrame,
    framedCorners,
    padBounds,
    SELECT_PAD_PX,
  } from "$lib/tools/transform";

  // The transform box of a selected non-shape element (text/image/use). Built by EditorCanvas,
  // which owns the DOM measurement: a label's box comes from its own untransformed bbox mapped
  // through its screen matrix, so it arrives already turned by whatever transform it carries.
  let { elementFrame = null }: { elementFrame?: BoxFrame | null } = $props();

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
    editor.objectSelected ? (doc?.paths[editor.selectedPathIndex ?? -1] ?? null) : null,
  );
  // The transform box: drawn for a single object selection *or* a multi-select group, from the
  // selection's own frame (bounds measured at its tilt).
  const shapeBox = $derived(
    (boxPath && !boxPath.deleted) || editor.multiSelected ? editor.selectionFrame : null,
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

  // Live-boolean operand outlines: when a boolean group is "active" (one of its operands is
  // selected or node-edited) show *all* its operands as faint outlines, so the editable source
  // shapes are visible behind the computed result (Pixelmator-style). Driven by the tree's
  // boolean groups (their operand uids), not the retired flat `layer` field.
  // Where the rotate tool would turn: the placed pivot, else the selection's centre.
  const pivot = $derived(tools.active === "rotate" ? (tools.pivot ?? activePivot()) : null);

  const activeOperandUids = $derived.by(() => {
    const editUid = editor.nodeEditIndex != null ? doc?.paths[editor.nodeEditIndex]?.uid : null;
    const selected = new Set(
      [...editor.selectedPaths.map((i) => doc?.paths[i]?.uid), editUid].filter(
        (u): u is string => !!u,
      ),
    );
    // Every operand of any group with a selected/node-edited member (built from an iterable, so
    // no post-construction mutation — the lint bars mutable Sets in reactive scope).
    return new Set(
      editor.booleanResults
        .filter((r) => r.operandUids.some((u) => selected.has(u)))
        .flatMap((r) => r.operandUids),
    );
  });
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
    {#if interaction.marquee}
      {@const a = viewport.toScreen({ x: interaction.marquee.x0, y: interaction.marquee.y0 })}
      {@const b = viewport.toScreen({ x: interaction.marquee.x1, y: interaction.marquee.y1 })}
      <rect
        class="marquee"
        x={Math.min(a.x, b.x)}
        y={Math.min(a.y, b.y)}
        width={Math.abs(b.x - a.x)}
        height={Math.abs(b.y - a.y)}
      />
    {/if}
    {#each doc.paths as p, pi (pi)}
      {#if p.uid && activeOperandUids.has(p.uid) && !p.deleted}
        <path class="operand-outline" d={pathToD(toScreenSubpaths(p.subpaths))} />
      {/if}
    {/each}
    {#if outlineD}
      <!-- selection centerline: light casing + accent core so it reads on any
           stroke colour (Pixelmator-style) -->
      <path class="sel-outline-casing" d={outlineD} />
      <path class="sel-outline" d={outlineD} />
    {/if}
    {#snippet transformBox(frame: BoxFrame)}
      <!-- box + rotate knob + 8 resize handles, from a frame of screen-space points — so an
           upright box and a turned one draw by the same code. Only the handle squares are
           re-oriented, since they're the one part drawn at a fixed pixel size. -->
      {@const spin = (frame.angle * 180) / Math.PI}
      <polygon class="sel-box" points={frame.corners.map((p) => `${p.x},${p.y}`).join(" ")} />
      <line
        class="rotate-stem"
        x1={frame.stem.x}
        y1={frame.stem.y}
        x2={frame.knob.x}
        y2={frame.knob.y}
      />
      <circle class="rotate-knob" cx={frame.knob.x} cy={frame.knob.y} r="4.5" />
      {#each frame.handles as h (h.handle)}
        <rect
          class="xf-handle"
          x={h.point.x - 4}
          y={h.point.y - 4}
          width="8"
          height="8"
          transform={frame.angle ? `rotate(${spin} ${h.point.x} ${h.point.y})` : null}
        />
      {/each}
    {/snippet}
    {#if shapeBox}
      <!-- The box for a shape selection — one shape or a multi-select group, which scale and
           rotate as one (Pixelmator-style). Bounds come measured in the selection's own tilted
           frame, so a shape turned 30° gets a box that hugs it rather than the axis-aligned bounds
           around it; rotating the padded corners back out of that frame puts them where the shape
           is. It follows a rotation *live* for free: the geometry and the angle turn together, so
           bounds measured in the turning frame keep their size. -->
      {@const bb = padBounds(shapeBox.bounds, viewport.toDocLength(SELECT_PAD_PX))}
      {@const c = framedCorners(bb, shapeBox.angle).map((p) => viewport.toScreen(p))}
      {@render transformBox(boxFrame(c[0], c[1], c[2], c[3]))}
    {/if}
    {#if elementFrame}
      <!-- transform box for a selected non-shape element (text/image/use) -->
      {@render transformBox(elementFrame)}
    {/if}
    {#if tools.active === "rotate" && pivot}
      <!-- The pivot the rotate tool turns about: a crosshair you can grab and drag, drawn last so
           it stays legible over the selection outline. -->
      {@const at = viewport.toScreen(pivot)}
      <circle class="pivot-halo" cx={at.x} cy={at.y} r="9" />
      <line class="pivot-cross" x1={at.x - 7} y1={at.y} x2={at.x + 7} y2={at.y} />
      <line class="pivot-cross" x1={at.x} y1={at.y - 7} x2={at.x} y2={at.y + 7} />
      <circle class="pivot-dot" cx={at.x} cy={at.y} r="2.5" />
    {/if}
    {#if interaction.loupe}
      <!-- The eyedropper's loupe: the colour this click would take, and the shape it comes from.
           A CARD, not a bare chip — a swatch of the sampled colour sits on whatever it sampled,
           so by definition it matches its own background and reads as a hole. On the panel's own
           surface it reads against anything.
           No magnifier: nib samples the MODEL, so zoomed pixels would show antialiased edges it
           can never return. The exact value and the named shape it came from is the better
           answer, and it's one only a vector editor can give. -->
      {@const at = viewport.toScreen(interaction.loupe.at)}
      {@const from =
        interaction.loupe.from.length > 16
          ? `${interaction.loupe.from.slice(0, 15)}…`
          : interaction.loupe.from}
      {@const w = 124}
      {@const h = 36}
      <!-- Up-right of the cursor, flipping at the edges so the readout never leaves the canvas. -->
      {@const lx = at.x + 16 + w > viewport.pxWidth ? at.x - 16 - w : at.x + 16}
      {@const ly = at.y - 14 - h < 0 ? at.y + 14 : at.y - 14 - h}
      <rect class="loupe-card" x={lx} y={ly} width={w} height={h} rx="6" />
      <rect
        class="loupe-swatch"
        x={lx + 7}
        y={ly + 7}
        width={22}
        height={22}
        rx="4"
        fill={interaction.loupe.color}
      />
      <text class="loupe-hex" x={lx + 37} y={ly + 16}>{interaction.loupe.color}</text>
      <text class="loupe-from" x={lx + 37} y={ly + 28}>{from}</text>
    {/if}
    {#if editor.multiSelected}
      <!-- outline every selected path (accent centerline) so it's clear which are in the
           multi-selection, plus the union transform box for the group. -->
      {#each editor.selectedPaths as pi (pi)}
        {@const mp = doc.paths[pi]}
        {#if mp && !mp.deleted}
          {@const md = pathToD(toScreenSubpaths(mp.subpaths))}
          <path class="sel-outline-casing" d={md} />
          <path class="sel-outline" d={md} />
        {/if}
      {/each}
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

  /* live-boolean operand outlines — faint dashed source shapes behind the computed result */
  .operand-outline {
    fill: none;
    stroke: var(--halo-accent);
    stroke-width: 1;
    stroke-dasharray: 3 3;
    opacity: 0.5;
  }

  /* Selection: a solid accent border. Dashed is reserved for *borrowed* things (a live-boolean
     operand), so the box that says "this is yours to edit" must not wear the same coat. No fill —
     the box sits over artwork, and tinting what you are trying to judge is worse than useless. */
  .sel-box {
    fill: none;
    stroke: var(--halo-accent);
    stroke-width: 1;
    opacity: 0.85;
  }

  /* resize handles on the bounding box */
  .xf-handle {
    fill: var(--halo-bg-main);
    stroke: var(--halo-accent);
    stroke-width: 1.5;
  }

  /* Marquee: marching ants over a barely-there wash — the family's cue for "what operations will
     apply to", distinct from both the solid selection border and the dashed borrowed outline. */
  .marquee {
    fill: var(--halo-accent-soft);
    stroke: var(--halo-text-main);
    stroke-width: 1;
    stroke-dasharray: 4 4;
    opacity: 0.6;
    animation: ants 0.6s linear infinite;
  }

  @keyframes ants {
    to {
      stroke-dashoffset: -8;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .marquee {
      animation: none;
    }
  }

  /* smart alignment guides while dragging */
  .guide {
    stroke: var(--halo-accent);
    stroke-width: 1;
    opacity: 0.9;
    pointer-events: none;
  }

  /* The eyedropper's loupe — a card, so an arbitrary sampled colour has something neutral to sit
     on. Floating the swatch directly over the drawing put it against its own colour. */
  .loupe-card {
    fill: var(--halo-bg-main);
    stroke: var(--halo-border);
    stroke-width: 1;
    filter: drop-shadow(0 2px 6px rgb(0 0 0 / 0.18));
  }

  .loupe-swatch {
    stroke: var(--halo-border);
    stroke-width: 1;
  }

  .loupe-hex {
    fill: var(--halo-text-main);
    font-family: var(--halo-font-mono, ui-monospace, monospace);
    font-size: 11px;
  }

  .loupe-from {
    fill: var(--halo-text-muted);
    font-size: 10px;
  }

  /* The rotate tool's pivot: a target you can see against artwork of any colour — a pale halo
     under an accent crosshair. */
  .pivot-halo {
    fill: var(--halo-bg-main);
    opacity: 0.75;
    stroke: var(--halo-accent);
    stroke-width: 1;
  }

  .pivot-cross {
    stroke: var(--halo-accent);
    stroke-width: 1.5;
  }

  .pivot-dot {
    fill: var(--halo-accent);
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

  /* The pen's rubber band: a live aid that exists only under the hand, not a state anything is
     in. Dashed reads fine here because it is gone the moment the pointer stops. */
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
