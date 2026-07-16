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
  // Live-boolean group results, keyed by the group node's uid: a `<g booleanOp>` in the render
  // tree paints this computed geometry instead of its operand children. Recomputed each core sync
  // (so it tracks operand drags), independent of the cached render tree.
  const booleanByUid = $derived(new Map(editor.booleanResults.map((r) => [r.uid, r] as const)));

  // The document renders declaratively from the tree (the root <svg>'s children), fetched once
  // per source change; editable shapes within pull live geometry from doc.paths by uid so edits
  // reflect reactively. This retires the old imperative import — z-order is now true document
  // order, and edited primitives no longer jump above their neighbours.
  let renderTree = $state<RenderNode[]>([]);
  let renderedSource: string | undefined;
  let renderedVersion = -1;
  let pendingFit = $state<ViewBox | null>(null);
  $effect(() => {
    const doc = editor.doc;
    const version = editor.treeVersion;
    // Re-fetch on a new source *or* a structural op (treeVersion bump); a plain re-render (a
    // geometry edit) leaves both unchanged, so the tree isn't re-marshalled per frame.
    if (doc?.source === renderedSource && version === renderedVersion) return;
    const fresh = doc?.source !== renderedSource;
    renderedSource = doc?.source;
    renderedVersion = version;
    if (!doc) {
      renderTree = [];
      pendingFit = null;
      return;
    }
    renderTree = editor.renderTree();
    // Frame the artboard + any content beyond it (only on a fresh load, not a structural edit).
    if (fresh) pendingFit = loadViewBox();
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
  // Attributes for an editable shape drawn as a `<path>`. A drawn (added) path's whole style lives
  // in `attributes` (edited live by the STYLE panel), so render that directly — the cached tree
  // node's attrs would be stale. An imported path keeps *all* the source element's attrs (class /
  // transform / clip-path / fill=url(#…) / …) minus geometry, then applies its `styleOverride` —
  // what keeps gradients, CSS classes, and transforms intact through the declarative render.
  function shapeAttrs(attrs: Record<string, string>, p: PathElement): Record<string, string> {
    if (p.added) return { ...(p.attributes ?? {}) };
    const out: Record<string, string> = {};
    for (const k in attrs) if (!GEOM_ATTRS.has(k)) out[k] = attrs[k];
    return { ...out, ...(p.styleOverride ?? {}) };
  }

  // Whether an editable shape node paints (skips deleted/hidden). Boolean-group operands never
  // reach here — a `<g booleanOp>` paints its computed result and doesn't recurse into children.
  function shapeVisible(p: PathElement): boolean {
    return !p.deleted && !p.hidden;
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

  // Non-shape element (text/image/use) selection + drag runs outside the (path-focused) gesture
  // machine, like pan/pinch. A drag translates the element's x/y live; commit at gesture end.
  let elDrag: { uid: string; x0: number; y0: number; start: Point; moved: boolean } | null = null;

  // The uid of a selectable opaque element (text/image/use) under the pointer, via the DOM node's
  // data-uid — used only when the model hit-test misses (shapes take priority).
  function elementHit(e: PointerEvent): { uid: string; el: Element } | null {
    const el = (e.target as Element | null)?.closest("[data-uid]") ?? null;
    if (!el) return null;
    const uid = (el as HTMLElement).dataset.uid;
    const tag = el.tagName.toLowerCase();
    if (uid && !pathByUid.has(uid) && (tag === "text" || tag === "image" || tag === "use"))
      return { uid, el };
    return null;
  }

  const round2 = (v: number) => Math.round(v * 100) / 100;

  // Begin dragging an element: capture its current x/y (from the DOM) + the start point.
  function startElDrag(uid: string, screen: Point): void {
    const el = svgEl.querySelector(`[data-uid="${CSS.escape(uid)}"]`);
    elDrag = {
      uid,
      x0: Number(el?.getAttribute("x") ?? "0") || 0,
      y0: Number(el?.getAttribute("y") ?? "0") || 0,
      start: viewport.toDoc(screen),
      moved: false,
    };
  }

  // Is a screen point inside the selected element's box (with a small grab tolerance)?
  function inElBox(p: Point): boolean {
    if (!elBox) return false;
    return (
      p.x >= elBox.x - 3 &&
      p.x <= elBox.x + elBox.w + 3 &&
      p.y >= elBox.y - 3 &&
      p.y <= elBox.y + elBox.h + 3
    );
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
    // Select tool, model hit missed → non-shape element (text/image/use). Once selected, dragging
    // anywhere in its box moves it (forgiving, like a transform-box body); else pick one by paint.
    if (!pan && tools.active === "select" && hit.kind === "empty") {
      if (editor.selectedElementUid && inElBox(screen)) {
        startElDrag(editor.selectedElementUid, screen);
        return;
      }
      const el = elementHit(e);
      if (el) {
        editor.selectElement(el.uid);
        startElDrag(el.uid, screen);
        return;
      }
    }
    if (!pan) editor.selectedElementUid = null; // any other click clears the element selection box
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
    if (elDrag) {
      const p = viewport.toDoc(screen);
      const dx = p.x - elDrag.start.x;
      const dy = p.y - elDrag.start.y;
      if (dx || dy) elDrag.moved = true;
      editor.previewNodeMove(elDrag.uid, round2(elDrag.x0 + dx), round2(elDrag.y0 + dy));
      return;
    }
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
    if (elDrag) {
      if (elDrag.moved) editor.commit(); // record the live-moved x/y as one undo step
      elDrag = null;
      return;
    }
    canvas.send({ type: "UP", docPoint: viewport.toDoc(screenOf(e)) });
  }

  // Screen-space bounding box of the selected non-shape element, measured from its rendered DOM
  // node (getBoundingClientRect handles font metrics / transforms the model can't know). Re-measured
  // when the selection, tree, or viewport changes; fed to the Overlay to draw its selection box.
  let elBox = $state<{ x: number; y: number; w: number; h: number } | null>(null);
  $effect(() => {
    const uid = editor.selectedElementUid;
    // deps: re-measure on tree edits + any viewport change
    void editor.treeVersion;
    void viewport.scale;
    void viewport.tx;
    void viewport.ty;
    void pxW;
    void pxH;
    if (!uid || !svgEl) {
      elBox = null;
      return;
    }
    const el = svgEl.querySelector(`[data-uid="${CSS.escape(uid)}"]`);
    if (!el) {
      elBox = null;
      return;
    }
    const r = el.getBoundingClientRect();
    const s = svgEl.getBoundingClientRect();
    elBox = { x: r.left - s.left, y: r.top - s.top, w: r.width, h: r.height };
  });

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
      {:else if n.hidden}
        <!-- hidden node + subtree: skipped -->
      {:else if n.booleanOp}
        <!-- live-boolean group: paint the computed result (operands stay editable but unpainted);
             recomputed each sync via booleanResults so it tracks operand drags -->
        {@const r = booleanByUid.get(n.uid)}
        {#if r}<path {...r.attributes} d={pathToD(r.subpaths)} />{/if}
      {:else}
        {@const p = pathByUid.get(n.uid)}
        {#if p}
          <!-- editable shape: drawn from the model (live geometry) in true z-order, keeping the
               source element's class/transform/fill=url(#…)/… so gradients + CSS survive -->
          {#if shapeVisible(p)}<path
              {...shapeAttrs(n.attrs, p)}
              d={pathToD(p.subpaths)}
              data-uid={n.uid}
            />{/if}
        {:else}
          <!-- opaque element (g / defs / text / image / …): rendered verbatim from the tree.
               Explicit SVG namespace — Svelte can't always infer it for a dynamic recursive tag,
               and gradient/defs elements in the wrong namespace silently stop functioning.
               data-uid links the DOM node back to its tree node for click-select + bbox measure. -->
          <svelte:element
            this={n.tag}
            xmlns="http://www.w3.org/2000/svg"
            {...n.attrs}
            data-uid={n.uid}
          >
            {#each n.children as c, i (i)}{@render renderNode(c)}{/each}
          </svelte:element>
        {/if}
      {/if}
    {/snippet}
    <g
      class="scene"
      transform={`translate(${viewport.tx} ${viewport.ty}) scale(${viewport.scale})`}
    >
      <!-- the whole document — imported, drawn, and baked booleans — rendered declaratively from
           the tree in true document order (one representation, one z-order) -->
      <g class="artwork">
        {#each renderTree as n, i (i)}{@render renderNode(n)}{/each}
      </g>
    </g>
    <Overlay elementBox={elBox} />
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
