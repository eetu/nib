<script lang="ts">
  import { pathToD } from "$lib/model/path";
  import type { PathElement, Point, RenderNode, ViewBox } from "$lib/model/types";
  import { canvas } from "$lib/stores/canvas.svelte";
  import { editor } from "$lib/stores/document.svelte";
  import { interaction } from "$lib/stores/interaction.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { tools } from "$lib/stores/tool.svelte";
  import { viewport } from "$lib/stores/viewport.svelte";
  import { getTool, type Hit, hitTest } from "$lib/tools";
  import { loadViewBox } from "$lib/view";

  import Overlay from "./Overlay.svelte";

  let wrap: HTMLDivElement;
  let svgEl: SVGSVGElement;
  let pxW = $state(0);
  let pxH = $state(0);
  let hoverCursor = $state("default");

  const activeTool = $derived(getTool(tools.active));
  const cursor = $derived(
    canvas.panning || canvas.dragging || interaction.spaceHeld ? "grabbing" : hoverCursor,
  );
  // Ids of hidden layers — their paths drop from render.
  const hiddenLayers = $derived(
    new Set((editor.doc?.layers ?? []).filter((l) => !l.visible).map((l) => l.id)),
  );
  // Live boolean groups: members are operands → the computed result paints instead.
  const booleanLayers = $derived(editor.booleanLayerIds);

  // The document renders declaratively from the tree (the root <svg>'s children), fetched once
  // per source change; editable shapes within pull live geometry from doc.paths by uid so edits
  // reflect reactively. This retires the old imperative import — z-order is now true document
  // order, and edited primitives no longer jump above their neighbours.
  let renderTree = $state<RenderNode[]>([]);
  let renderedSource: string | undefined;
  let pendingFit = $state<ViewBox | null>(null);
  $effect(() => {
    const doc = editor.doc;
    if (doc?.source === renderedSource) return;
    renderedSource = doc?.source;
    if (!doc) {
      renderTree = [];
      pendingFit = null;
      return;
    }
    renderTree = editor.renderTree();
    // Frame the artboard + any content beyond it, so a reload never lands off-screen.
    pendingFit = loadViewBox();
  });

  // Keep the viewport's pixel size current, and fit a freshly-loaded document once the canvas
  // has real pixels (zoom/pan change scale/tx/ty, not these, so they never re-fit).
  $effect(() => {
    viewport.setSize(pxW, pxH);
    if (pendingFit && pxW > 0 && pxH > 0) {
      viewport.fitDocument(pendingFit);
      pendingFit = null;
    }
  });

  // Live geometry for editable shapes, keyed by their stable uid (edited or not).
  const pathByUid = $derived(
    new Map(
      (editor.doc?.paths ?? []).filter((p) => p.uid).map((p) => [p.uid as string, p] as const),
    ),
  );

  // Drawn (added) paths render on top of the imported tree (they have no source node).
  const addedPaths = $derived(
    editor.doc?.paths.filter(
      (p) =>
        p.added &&
        !p.deleted &&
        !p.hidden &&
        !(p.layer && hiddenLayers.has(p.layer)) &&
        !(p.layer && booleanLayers.has(p.layer)),
    ) ?? [],
  );

  // A path's effective style (drawn: attributes; imported: attributes ⊕ styleOverride).
  function eff(p: { attributes?: Record<string, string>; styleOverride?: Record<string, string> }) {
    return { ...(p.attributes ?? {}), ...(p.styleOverride ?? {}) };
  }

  // Geometry attributes replaced by the path's `d` (dropped when drawing a primitive as a path).
  const GEOM_ATTRS = new Set([
    "x",
    "y",
    "width",
    "height",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "x1",
    "y1",
    "x2",
    "y2",
    "points",
    "d",
  ]);
  // Attributes for an editable shape drawn as a `<path>`: keep *all* the source element's attrs
  // (class / transform / clip-path / fill=url(#…) / …) minus the geometry ones, then apply any
  // style edits. Using the node's full attrs (not just the model's parsed style) is what keeps
  // gradients, CSS classes, and transforms intact through the declarative render.
  function shapeAttrs(attrs: Record<string, string>, p: PathElement): Record<string, string> {
    const out: Record<string, string> = {};
    for (const k in attrs) if (!GEOM_ATTRS.has(k)) out[k] = attrs[k];
    return { ...out, ...(p.styleOverride ?? {}) };
  }

  // Whether an editable shape node paints (skips deleted/hidden/hidden-layer/boolean-operand).
  function shapeVisible(p: PathElement): boolean {
    return (
      !p.deleted &&
      !p.hidden &&
      !(p.layer && hiddenLayers.has(p.layer)) &&
      !(p.layer && booleanLayers.has(p.layer))
    );
  }

  // WebKit-only trackpad gesture event (Safari); not in the standard DOM lib.
  type GestureLike = Event & { scale: number; clientX: number; clientY: number };

  function screenOf(e: { clientX: number; clientY: number }): Point {
    const r = svgEl.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  // Space to pan, Escape to cancel a drag.
  $effect(() => {
    function typing(): boolean {
      const el = document.activeElement;
      return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA");
    }
    function down(e: KeyboardEvent) {
      if (e.code === "Space" && !typing()) {
        interaction.spaceHeld = true;
        e.preventDefault();
      } else if (e.key === "Escape") {
        cancelDrag();
      }
    }
    function up(e: KeyboardEvent) {
      if (e.code === "Space") interaction.spaceHeld = false;
    }
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  });

  function cancelDrag(): void {
    canvas.send({ type: "CANCEL" });
  }

  // Multi-touch pinch: track active pointers (screen coords, keyed by id); with
  // two down, the gesture is a pinch-zoom (+ pan of the midpoint), not an edit.
  let pointers: { id: number; p: Point }[] = [];
  let pinch: { dist: number; mid: Point } | null = null;

  function setPointer(id: number, p: Point): void {
    const existing = pointers.find((q) => q.id === id);
    if (existing) existing.p = p;
    else pointers.push({ id, p });
  }

  function pinchState(): { dist: number; mid: Point } {
    const [a, b] = pointers;
    return {
      dist: Math.hypot(b.p.x - a.p.x, b.p.y - a.p.y),
      mid: { x: (a.p.x + b.p.x) / 2, y: (a.p.y + b.p.y) / 2 },
    };
  }

  function onPointerDown(e: PointerEvent) {
    if (!editor.doc) return;
    setPointer(e.pointerId, screenOf(e));
    // A second finger starts a pinch — abort any single-pointer gesture first.
    if (pointers.length >= 2) {
      svgEl.setPointerCapture(e.pointerId);
      cancelDrag();
      if (pointers.length === 2) pinch = pinchState();
      return;
    }
    const pan = e.button === 1 || interaction.spaceHeld;
    if (!pan && e.button !== 0) return;
    svgEl.setPointerCapture(e.pointerId);
    const screen = screenOf(e);
    const hit: Hit = pan ? { kind: "empty" } : hitTest(screen);
    canvas.send({ type: "DOWN", hit, docPoint: viewport.toDoc(screen), event: e, pan, screen });
  }

  function onPointerMove(e: PointerEvent) {
    if (pointers.some((q) => q.id === e.pointerId)) setPointer(e.pointerId, screenOf(e));
    if (pinch && pointers.length >= 2) {
      const next = pinchState();
      if (pinch.dist > 0) viewport.zoomAt(pinch.mid, next.dist / pinch.dist);
      viewport.panBy(next.mid.x - pinch.mid.x, next.mid.y - pinch.mid.y);
      pinch = next;
      return;
    }
    const screen = screenOf(e);
    if (!canvas.idle) {
      canvas.send({ type: "MOVE", docPoint: viewport.toDoc(screen), screen, event: e });
      return;
    }
    // idle → hover feedback (not part of a gesture)
    activeTool.hover?.(viewport.toDoc(screen));
    if (editor.doc) hoverCursor = activeTool.cursor(hitTest(screen));
  }

  function onPointerUp(e: PointerEvent) {
    pointers = pointers.filter((q) => q.id !== e.pointerId);
    if (svgEl.hasPointerCapture(e.pointerId)) svgEl.releasePointerCapture(e.pointerId);
    if (pinch) {
      if (pointers.length < 2) pinch = null; // pinch owned this gesture
      return;
    }
    canvas.send({ type: "UP", docPoint: viewport.toDoc(screenOf(e)) });
  }

  // Double-click a shape (select tool) to enter node-editing mode — Figma-style. Object
  // mode moves the whole shape on drag; nodes only become editable after entering here.
  function onDblClick(e: MouseEvent) {
    if (!editor.doc || tools.active !== "select") return;
    const hit = hitTest(screenOf(e));
    if (hit.kind === "fill" || hit.kind === "segment") editor.enterNodeEdit(hit.pathIndex);
  }

  // Zoom responsiveness knobs (bump for snappier zoom). WHEEL_ZOOM_SENS scales
  // the ctrl/⌘+wheel step; PINCH_GAIN (>1) makes a Safari trackpad pinch zoom
  // faster than the raw finger spread.
  const WHEEL_ZOOM_SENS = 0.01;
  const PINCH_GAIN = 1.8;

  function onWheel(e: WheelEvent) {
    if (!editor.doc) return;
    e.preventDefault();
    // Chromium/Firefox deliver a trackpad pinch as ctrl+wheel; ⌘/ctrl+wheel is
    // the mouse zoom. A plain wheel / two-finger scroll pans.
    if (e.ctrlKey || e.metaKey) {
      // A Chromium pinch sends small deltaY per event (needs the higher
      // sensitivity to feel responsive); clamp so one big mouse notch can't
      // over-zoom.
      const dz = Math.max(-50, Math.min(50, e.deltaY));
      viewport.zoomAt(screenOf(e), Math.exp(-dz * WHEEL_ZOOM_SENS));
    } else {
      viewport.panBy(-e.deltaX, -e.deltaY);
    }
  }

  // Safari delivers a trackpad pinch as WebKit gesture events instead of a
  // ctrl+wheel (see onWheel). `scale` is cumulative since gesturestart, so zoom
  // by the step ratio at the cursor. Bound via a spread on the <svg> so it goes
  // through Svelte's event system (which flushes the viewport change to the
  // DOM) — a raw addEventListener would leave it stale, and a manual flushSync
  // re-runs the fit-on-load effect and snaps the zoom back.
  let gestureLast = 1;
  const gestureHandlers = {
    ongesturestart: (e: Event) => {
      e.preventDefault();
      gestureLast = (e as GestureLike).scale || 1;
    },
    ongesturechange: (e: Event) => {
      e.preventDefault();
      const g = e as GestureLike;
      if (gestureLast > 0 && g.scale > 0)
        viewport.zoomAt(screenOf(g), (g.scale / gestureLast) ** PINCH_GAIN);
      gestureLast = g.scale;
    },
    ongestureend: (e: Event) => e.preventDefault(),
  };
</script>

<div
  class="canvas-wrap"
  data-bg={settings.canvasBg}
  bind:this={wrap}
  bind:clientWidth={pxW}
  bind:clientHeight={pxH}
>
  <svg
    bind:this={svgEl}
    class="canvas"
    role="application"
    aria-label="SVG path editor canvas"
    style:cursor
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    ondblclick={onDblClick}
    onwheel={onWheel}
    {...gestureHandlers}
  >
    {#if tools.gridEnabled && tools.gridSize * viewport.scale >= 5}
      {@const step = tools.gridSize * viewport.scale}
      <defs>
        <pattern
          id="nib-grid"
          width={step}
          height={step}
          patternUnits="userSpaceOnUse"
          x={viewport.tx}
          y={viewport.ty}
        >
          <path class="grid-line" d={`M ${step} 0 L 0 0 0 ${step}`} />
        </pattern>
      </defs>
      <rect class="grid" width="100%" height="100%" fill="url(#nib-grid)" />
    {/if}
    {#if editor.doc?.gradients?.length}
      <defs>
        {#each editor.doc.gradients as g (g.id)}
          {#if g.kind === "radial"}
            <radialGradient id={g.id} cx={g.cx} cy={g.cy} r={g.r}>
              {#each g.stops as s, i (i)}
                <stop offset={s.offset} stop-color={s.color} stop-opacity={s.opacity ?? 1} />
              {/each}
            </radialGradient>
          {:else}
            <linearGradient id={g.id} x1={g.x1} y1={g.y1} x2={g.x2} y2={g.y2}>
              {#each g.stops as s, i (i)}
                <stop offset={s.offset} stop-color={s.color} stop-opacity={s.opacity ?? 1} />
              {/each}
            </linearGradient>
          {/if}
        {/each}
      </defs>
    {/if}
    {#snippet renderNode(n: RenderNode)}
      {#if n.kind === "text"}
        {n.text}
      {:else}
        {@const p = pathByUid.get(n.uid)}
        {#if p}
          <!-- editable shape: drawn from the model (live geometry) in true z-order, keeping the
               source element's class/transform/fill=url(#…)/… so gradients + CSS survive -->
          {#if shapeVisible(p)}<path {...shapeAttrs(n.attrs, p)} d={pathToD(p.subpaths)} />{/if}
        {:else}
          <!-- opaque element (g / defs / text / image / …): rendered verbatim from the tree.
               Explicit SVG namespace — Svelte can't always infer it for a dynamic recursive tag,
               and gradient/defs elements in the wrong namespace silently stop functioning. -->
          <svelte:element this={n.tag} xmlns="http://www.w3.org/2000/svg" {...n.attrs}>
            {#each n.children as c, i (i)}{@render renderNode(c)}{/each}
          </svelte:element>
        {/if}
      {/if}
    {/snippet}
    <g
      class="scene"
      transform={`translate(${viewport.tx} ${viewport.ty}) scale(${viewport.scale})`}
    >
      <!-- imported document: rendered declaratively from the tree in document order -->
      <g class="artwork">
        {#each renderTree as n, i (i)}{@render renderNode(n)}{/each}
      </g>
      <!-- in-app drawn paths: Svelte-managed, straight from the model (on top) -->
      <g class="drawn">
        {#each addedPaths as p (p.id)}
          <path {...eff(p)} d={pathToD(p.subpaths)} />
        {/each}
      </g>
      <!-- live boolean groups: the computed result (operands render as overlay outlines) -->
      <g class="booleans">
        {#each editor.booleanResults as r (r.layer)}
          <path {...r.attributes} d={pathToD(r.subpaths)} />
        {/each}
      </g>
    </g>
    <Overlay />
  </svg>
</div>

<style>
  .canvas-wrap {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }

  /* Backdrop the artwork previews against (settings.canvasBg). "checker" is the
     transparency grid; light/dark are absolute surfaces, independent of the UI
     theme, so an SVG can be checked on either. */
  .canvas-wrap[data-bg="checker"] {
    background: repeating-conic-gradient(var(--halo-bg-light) 0% 25%, transparent 0% 50%) 50% / 20px
      20px;
    background-color: var(--halo-bg-main);
  }

  .canvas-wrap[data-bg="light"] {
    background: #ffffff;
  }

  .canvas-wrap[data-bg="dark"] {
    background: #14161a;
  }

  .canvas {
    display: block;
    width: 100%;
    height: 100%;
    touch-action: none;
  }

  /* Drawn paths use stroke: currentColor; render them in theme text colour. */
  .scene {
    color: var(--halo-text-main);
  }

  .grid {
    pointer-events: none;
  }

  .grid-line {
    fill: none;
    stroke: var(--halo-border);
    stroke-width: 1;
  }
</style>
