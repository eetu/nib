<script lang="ts">
  import { STYLE_KEYS } from "$lib/model/document";
  import { pathToD } from "$lib/model/path";
  import type { Point, ViewBox } from "$lib/model/types";
  import { canvas } from "$lib/stores/canvas.svelte";
  import { editor } from "$lib/stores/document.svelte";
  import { interaction } from "$lib/stores/interaction.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { tools } from "$lib/stores/tool.svelte";
  import { viewport } from "$lib/stores/viewport.svelte";
  import { getTool, type Hit, hitTest } from "$lib/tools";

  import Overlay from "./Overlay.svelte";

  let wrap: HTMLDivElement;
  let svgEl: SVGSVGElement;
  let artworkGroup: SVGGElement;
  // Reactive so the d-update effect below re-runs after a (re-)import — matters
  // when the document is already present at mount (rehydrated from persistence),
  // where the import may land after the update effect's first run.
  let livePaths = $state<SVGPathElement[]>([]);
  let pxW = $state(0);
  let pxH = $state(0);
  let hoverCursor = $state("default");

  const activeTool = $derived(getTool(tools.active));
  const cursor = $derived(
    canvas.panning || canvas.dragging || interaction.spaceHeld ? "grabbing" : hoverCursor,
  );
  // Ids of hidden layers — their paths drop from render (+ export stamps display:none).
  const hiddenLayers = $derived(
    new Set((editor.doc?.layers ?? []).filter((l) => !l.visible).map((l) => l.id)),
  );
  // Drawn paths to render: skip hidden layers, ordered by layer z-order (unassigned last).
  const newPaths = $derived.by(() => {
    const doc = editor.doc;
    if (!doc) return [];
    const added = doc.paths.filter(
      (p) => p.added && !p.deleted && !(p.layer && hiddenLayers.has(p.layer)),
    );
    if (!doc.layers?.length) return added;
    const order = new Map(doc.layers.map((l, i) => [l.id, i]));
    return [...added].sort(
      (a, b) =>
        (a.layer != null ? (order.get(a.layer) ?? Infinity) : Infinity) -
        (b.layer != null ? (order.get(b.layer) ?? Infinity) : Infinity),
    );
  });

  // WebKit-only trackpad gesture event (Safari); not in the standard DOM lib.
  type GestureLike = Event & { scale: number; clientX: number; clientY: number };

  function screenOf(e: { clientX: number; clientY: number }): Point {
    const r = svgEl.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  // Re-import the artwork only when the document *source* changes (a new load) —
  // guarded so a spurious re-run (e.g. a viewport change) neither re-imports nor
  // re-fits. Editing mutates the model in place (applied to the live elements by
  // the effect below), so `source` is stable across edits. A fresh import
  // requests a one-shot fit via `pendingFit`.
  let importedSource: string | undefined;
  let pendingFit = $state<ViewBox | null>(null);
  $effect(() => {
    const doc = editor.doc;
    if (!artworkGroup) return;
    if (doc?.source === importedSource) return;
    importedSource = doc?.source;
    // This <g> is left empty in the template — nib owns its children (the
    // imported foreign SVG), so Svelte never diffs against them and imperative
    // DOM here is safe. Same rationale for the setAttribute update below.
    // eslint-disable-next-line svelte/no-dom-manipulating
    artworkGroup.replaceChildren();
    livePaths = [];
    if (!doc) {
      pendingFit = null;
      return;
    }
    const parsed = new DOMParser().parseFromString(doc.source, "image/svg+xml");
    const src = parsed.querySelector("svg");
    if (!src) return;
    for (const child of Array.from(src.childNodes)) {
      // eslint-disable-next-line svelte/no-dom-manipulating
      artworkGroup.appendChild(document.importNode(child, true));
    }
    livePaths = Array.from(artworkGroup.querySelectorAll("path"));
    pendingFit = doc.viewBox;
  });

  // Keep the viewport's pixel size current, and fit a freshly-loaded document
  // once the canvas has real pixels. Depends only on the size + the pending-fit
  // request, so zoom/pan (which change scale/tx/ty, not these) never re-fit.
  $effect(() => {
    viewport.setSize(pxW, pxH);
    if (pendingFit && pxW > 0 && pxH > 0) {
      viewport.fitDocument(pendingFit);
      pendingFit = null;
    }
  });

  // Reflect model edits into the live (imported) <path> elements: geometry, and
  // effective style (parsed attributes + any override). Applying the full
  // effective style every run means undo restores the original look too. Drawn
  // paths have no live element (rendered declaratively) and are skipped.
  $effect(() => {
    const doc = editor.doc;
    if (!doc) return;
    for (const p of doc.paths) {
      const el = livePaths[p.index];
      if (!el) continue;
      if (p.deleted || (p.layer && hiddenLayers.has(p.layer))) {
        el.setAttribute("display", "none");
        continue;
      }
      el.removeAttribute("display");
      el.setAttribute("d", p.edited ? pathToD(p.subpaths) : p.originalD);
      const eff = { ...(p.attributes ?? {}), ...(p.styleOverride ?? {}) };
      for (const key of STYLE_KEYS) {
        const v = eff[key];
        if (v == null) el.removeAttribute(key);
        else el.setAttribute(key, v);
      }
    }
  });

  // Draw imported paths in the model's order so the PATHS list (drag-drop reorder) controls
  // their z-order. appendChild moves each imported <path> to the end in array order; non-path
  // siblings (e.g. a background <rect>) keep their place, so reordered paths render above them.
  // Gated on the order signature so it only touches the DOM when the draw order actually
  // changes (reordering the DOM on every selection would reset the browser's dblclick count).
  let lastOrder = "";
  $effect(() => {
    const doc = editor.doc;
    if (!doc || !artworkGroup) return;
    const imported = doc.paths.filter((p) => !p.added);
    const order = imported.map((p) => p.index).join(",");
    if (order === lastOrder) return;
    lastOrder = order;
    for (const p of imported) {
      if (p.deleted) continue;
      const el = livePaths[p.index];
      // eslint-disable-next-line svelte/no-dom-manipulating
      if (el && el.parentNode === artworkGroup) artworkGroup.appendChild(el);
    }
  });

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
    <g
      class="scene"
      transform={`translate(${viewport.tx} ${viewport.ty}) scale(${viewport.scale})`}
    >
      <!-- imported artwork: nib fills this imperatively (see the effect) -->
      <g bind:this={artworkGroup} class="artwork"></g>
      <!-- pen-drawn paths: Svelte-managed, rendered straight from the model -->
      <g class="drawn">
        {#each newPaths as p (p.id)}
          <path {...p.attributes ?? {}} d={pathToD(p.subpaths)} />
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
