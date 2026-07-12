<script lang="ts">
  import { STYLE_KEYS } from "$lib/model/document";
  import { pathToD } from "$lib/model/path";
  import type { Point } from "$lib/model/types";
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
  const newPaths = $derived(editor.doc?.paths.filter((p) => p.added && !p.deleted) ?? []);

  function screenOf(e: PointerEvent | WheelEvent): Point {
    const r = svgEl.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  // Re-import the artwork whenever a new document is loaded (source changes),
  // then fit it to the viewport. Editing mutates the model in place, so this
  // does not re-run on drags.
  $effect(() => {
    const doc = editor.doc;
    if (!artworkGroup) return;
    // This <g> is left empty in the template — nib owns its children (the
    // imported foreign SVG), so Svelte never diffs against them and imperative
    // DOM here is safe. Same rationale for the setAttribute update below.
    // eslint-disable-next-line svelte/no-dom-manipulating
    artworkGroup.replaceChildren();
    livePaths = [];
    if (!doc) return;
    const parsed = new DOMParser().parseFromString(doc.source, "image/svg+xml");
    const src = parsed.querySelector("svg");
    if (!src) return;
    for (const child of Array.from(src.childNodes)) {
      // eslint-disable-next-line svelte/no-dom-manipulating
      artworkGroup.appendChild(document.importNode(child, true));
    }
    livePaths = Array.from(artworkGroup.querySelectorAll("path"));
    if (wrap) viewport.setSize(wrap.clientWidth, wrap.clientHeight);
    viewport.fitDocument(doc.viewBox);
  });

  // Keep the viewport's pixel size current (drives the fit-to-view button).
  $effect(() => {
    viewport.setSize(pxW, pxH);
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
      if (p.deleted) {
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

  function onWheel(e: WheelEvent) {
    if (!editor.doc) return;
    e.preventDefault();
    // Trackpad pinch arrives as ctrl+wheel; ⌘/ctrl+wheel is the mouse zoom.
    // A plain wheel / two-finger scroll pans (matching trackpad expectations).
    if (e.ctrlKey || e.metaKey) {
      viewport.zoomAt(screenOf(e), Math.exp(-e.deltaY * 0.0025));
    } else {
      viewport.panBy(-e.deltaX, -e.deltaY);
    }
  }
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
    onwheel={onWheel}
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
