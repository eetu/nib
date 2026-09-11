<script lang="ts">
  import { type MenuItem, openMenu } from "$lib/menu.svelte";
  import { pathToD } from "$lib/model/path";
  import type { PathElement, Point, RenderNode, ViewBox } from "$lib/model/types";
  import { canvas } from "$lib/stores/canvas.svelte";
  import { editor } from "$lib/stores/document.svelte";
  import { interaction } from "$lib/stores/interaction.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { tools } from "$lib/stores/tool.svelte";
  import { viewport } from "$lib/stores/viewport.svelte";
  import { outlineText } from "$lib/text/outline";
  import { getTool, type Hit, hitTest } from "$lib/tools";
  import {
    type BoxFrame,
    boxFrame,
    frameHit,
    handleAnchor,
    insideQuad,
    SELECT_PAD_PX,
    transformCursor,
    type TransformHandle,
  } from "$lib/tools/transform";
  import { fitToView, loadViewBox } from "$lib/view";

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

  // A pivot is placed against one selection — keep it when that selection merely moves, drop it
  // when the selection itself changes, so the next shape doesn't inherit a pivot from the last.
  $effect(() => {
    void editor.selectedPaths;
    void editor.selectedElementUid;
    tools.pivot = null;
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

  // Space to pan. (Escape is owned by the +page handler — it cancels an in-flight gesture, else
  // steps out of node-edit / deselects — so one Esc does exactly one thing, and it's gated on
  // open modals, which a raw canvas listener wouldn't be.)
  $effect(() => {
    function typing(): boolean {
      const el = document.activeElement;
      return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA");
    }
    function down(e: KeyboardEvent) {
      if (e.code === "Space" && !typing()) {
        interaction.spaceHeld = true;
        e.preventDefault();
      } else if (e.key === "Escape" && elXf) {
        // Cancel an in-flight element transform — it runs outside the gesture machine, so +page's
        // machine-cancel never reaches it. (+page's Escape still deselects: a coherent cancel.)
        cancelElXf();
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

  /** Abandon an in-flight element transform, reverting its live preview. Used when a second
   *  pointer / Escape / pointer-cancel interrupts it — so it doesn't commit a stray jump. */
  function cancelElXf(): void {
    if (!elXf) return;
    if (elXf.moved) editor.revert();
    elXf = null;
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

  // --- non-shape element (text/image/use) selection + transform ----------
  // These run outside the (path-focused) gesture machine, like pan/pinch. The element's geometry
  // isn't in the model, so its box is *measured* from the rendered DOM (getBoundingClientRect,
  // which accounts for font metrics + transforms). Move/resize/rotate compose an SVG `transform`
  // matrix on the node (a plain move with no existing transform edits x/y instead, keeping markup
  // + the inspector clean). The inspector still edits authored x/y/width/height/font-size.

  // The selected element's box, measured from the DOM — the model can't know font metrics, so
  // this is the only place its size comes from.
  //
  // Measured as the element's **own** untransformed box (`getBBox`) plus the matrix that maps its
  // user space to the screen (`getScreenCTM`, which includes the element's own `transform`), not as
  // a `getBoundingClientRect`. Those differ exactly when it matters: the client rect is the
  // axis-aligned box *around* a turned label, so a box drawn from it snaps upright the moment a
  // rotation is committed, and it can't say which way the label's own edges run. The bbox and the
  // matrix keep the box on the object — turned when the label is turned, and its handles on the
  // label's own axes, which is what makes a resize after a rotation mean anything.
  //
  // `local` rides along because the gestures need it: scale and rotate are composed in this same
  // space (see `startElXf`), where the box is axis-aligned and the arithmetic is the easy kind.
  type ElBox = {
    /** The padded box in the element's own user space. */
    local: { x: number; y: number; w: number; h: number };
    /** Own space → screen (svg-relative), as it stood when measured. */
    toScreen: DOMMatrix;
    frame: BoxFrame;
  };
  let elBox = $state<ElBox | null>(null);
  $effect(() => {
    const uid = editor.selectedElementUid;
    void editor.treeVersion; // re-measure on tree edits + any viewport change
    void viewport.scale;
    void viewport.tx;
    void viewport.ty;
    void pxW;
    void pxH;
    const el = uid && svgEl ? svgEl.querySelector(`[data-uid="${CSS.escape(uid)}"]`) : null;
    elBox = el ? measureElBox(el as SVGGraphicsElement) : null;
  });

  function measureElBox(el: SVGGraphicsElement): ElBox | null {
    let raw: DOMRect;
    try {
      raw = el.getBBox(); // throws in Firefox for anything with no rendered geometry
    } catch {
      return null;
    }
    const ctm = el.getScreenCTM?.();
    if (!ctm) return null;
    const r = svgEl.getBoundingClientRect();
    // Screen matrix, offset to svg-relative coordinates (the overlay's space), and detached: a
    // live SVGMatrix view would drift under us mid-gesture.
    const m = new DOMMatrix([ctm.a, ctm.b, ctm.c, ctm.d, ctm.e - r.left, ctm.f - r.top]);
    // The pad is a screen distance, so it converts per axis — under a `scale(8, 2)` ancestor one
    // user unit is worth eight screen px across and two down.
    const padX = SELECT_PAD_PX / (Math.hypot(m.a, m.b) || 1);
    const padY = SELECT_PAD_PX / (Math.hypot(m.c, m.d) || 1);
    const local = {
      x: raw.x - padX,
      y: raw.y - padY,
      w: raw.width + padX * 2,
      h: raw.height + padY * 2,
    };
    const at = (x: number, y: number): Point => {
      const q = new DOMPoint(x, y).matrixTransform(m);
      return { x: q.x, y: q.y };
    };
    return {
      local,
      toScreen: m,
      frame: boxFrame(
        at(local.x, local.y),
        at(local.x + local.w, local.y),
        at(local.x + local.w, local.y + local.h),
        at(local.x, local.y + local.h),
      ),
    };
  }

  // An element gesture works in one of two spaces, and which one is not a detail:
  //
  // **Move** works in the element's PARENT space. Both a composed translate and the `x`/`y` we
  // write are interpreted there, so a document-space delta is only correct for an element sitting
  // directly in the artwork root — nested under a `<g transform="scale(8)">` it moved eight times
  // too far. Dragging means "follow the cursor", which is a screen direction, so this is right.
  //
  // **Scale and rotate** work in the element's OWN space — the one its `getBBox` lives in, where
  // the box is axis-aligned. Composed on that side (`m0 · L`, not `L · m0`) the handles pull along
  // the *object's* axes, so dragging the east edge of a box turned 30° widens the label along its
  // own baseline rather than shearing it out across the parent's x. Once the box is drawn turned,
  // anything else visibly disagrees with the handle under the cursor.
  //
  // Both mappers are captured once per gesture: the parent's transform can't change mid-drag, and
  // the element's own must not — every frame composes onto the gesture's start, never onto the
  // previous frame's result.
  type ElXfBase = { uid: string; moved: boolean };
  type ElXf =
    | (ElXfBase & {
        mode: "moveXy";
        toParent: (p: Point) => Point;
        x0: number;
        y0: number;
        start: Point;
      })
    | (ElXfBase & { mode: "move"; toParent: (p: Point) => Point; m0: DOMMatrix; start: Point })
    | (ElXfBase & {
        mode: "scale";
        toOwn: (p: Point) => Point;
        m0: DOMMatrix;
        anchor: Point;
        startPt: Point;
        axisX: boolean;
        axisY: boolean;
        corner: boolean;
      })
    | (ElXfBase & {
        mode: "rotate";
        toOwn: (p: Point) => Point;
        m0: DOMMatrix;
        center: Point;
        startAngle: number;
      });
  let elXf: ElXf | null = null;

  /** Map screen (svg-relative) points into `el`'s parent coordinate space. Falls back to document
   *  space when the element has no measurable parent CTM (detached / display:none). */
  function parentMapper(el: SVGGraphicsElement | null): (p: Point) => Point {
    const parent = el?.parentNode as SVGGraphicsElement | null;
    return ctmMapper(parent?.getScreenCTM?.() ?? null);
  }

  /** Map screen (svg-relative) points into `el`'s own user space — where its `getBBox` lives, and
   *  the space scale + rotate are composed in. */
  function ownMapper(el: SVGGraphicsElement | null): (p: Point) => Point {
    return ctmMapper(el?.getScreenCTM?.() ?? null);
  }

  function ctmMapper(ctm: DOMMatrix | null): (p: Point) => Point {
    if (!ctm) return (p) => viewport.toDoc(p);
    const inv = ctm.inverse();
    const r = svgEl.getBoundingClientRect();
    return (p) => {
      const q = new DOMPoint(p.x + r.left, p.y + r.top).matrixTransform(inv);
      return { x: q.x, y: q.y };
    };
  }

  /** A detached copy of `m` (identity when absent) — never a live SVGMatrix view of an element. */
  function snapshotMatrix(m: DOMMatrix | undefined): DOMMatrix {
    return m ? new DOMMatrix([m.a, m.b, m.c, m.d, m.e, m.f]) : new DOMMatrix();
  }

  const round2 = (v: number) => Math.round(v * 100) / 100;
  const safeDiv = (n: number, d: number) => (Math.abs(d) < 1e-6 ? 1 : n / d);
  const matrixStr = (m: DOMMatrix) => `matrix(${m.a} ${m.b} ${m.c} ${m.d} ${m.e} ${m.f})`;

  // Element types that are objects in their own right — the ones a click can select.
  const SELECTABLE_TAGS = new Set(["text", "image", "use"]);

  // The uid of a selectable opaque element (text/image/use) under the pointer, via the DOM node's
  // data-uid — used only when the model hit-test misses (shapes take priority).
  //
  // It climbs rather than taking the nearest `data-uid`: every rendered node carries one, so a
  // click on a `<tspan>` inside a label lands on the tspan's uid, which is not an object anyone
  // selects. The label is the object; the run inside it is part of that label.
  function elementHit(e: PointerEvent): { uid: string; el: Element } | null {
    let el = (e.target as Element | null)?.closest("[data-uid]") ?? null;
    while (el) {
      const uid = (el as HTMLElement).dataset.uid;
      const tag = el.tagName.toLowerCase();
      if (uid && !pathByUid.has(uid) && SELECTABLE_TAGS.has(tag)) return { uid, el };
      el = el.parentElement?.closest("[data-uid]") ?? null;
    }
    return null;
  }

  // A transform handle (or rotate knob) of the selected element's box under `screen`, if any —
  // measured against the very points the overlay drew, so a turned box grabs where it looks.
  function elHandleHit(
    screen: Point,
  ): { t: "rotate" } | { t: "scale"; handle: TransformHandle } | null {
    return elBox ? frameHit(elBox.frame, screen) : null;
  }

  // Is a screen point inside the selected element's box (with a small grab tolerance)? A quad
  // test, not a rect one — a turned box must be grabbable where it actually is, and its
  // axis-aligned bounds would also claim the empty corners well outside it.
  function inElBox(p: Point): boolean {
    return !!elBox && insideQuad(elBox.frame.corners, p, 3);
  }

  // --- inline text editing -------------------------------------------------
  // Double-clicking a <text> opens an input laid over it at the label's own rendered size, so
  // editing reads as editing the label itself. Enter/blur commits (one undo step via setNodeText),
  // Escape abandons. Only offered for a flat label: a <text> with element children (tspans) carries
  // structure a single string would silently flatten, so those stay Inspector-only.
  //
  // Size is matched, colour deliberately isn't — a white label would vanish against the input's
  // own background, so the caret and text stay in the UI's foreground colour.
  type TextEdit = {
    uid: string;
    value: string;
    x: number;
    y: number;
    w: number;
    h: number;
    fontPx: number;
  };
  let textEdit = $state<TextEdit | null>(null);
  let textInput = $state<HTMLInputElement | null>(null);

  function startTextEdit(e: MouseEvent): boolean {
    // Resolved through the selection, not `e.target`: pointerdown captures the pointer on the
    // <svg>, which retargets the compatibility mouse events, so the double-click never reports
    // the <text> itself. The first click of the double-click already selected it.
    const uid = editor.selectedElementUid;
    if (!uid || !inElBox(screenOf(e))) return false;
    const el = svgEl.querySelector(`[data-uid="${CSS.escape(uid)}"]`) as SVGGraphicsElement | null;
    if (!el || el.tagName.toLowerCase() !== "text" || el.children.length > 0) return false;

    const r = el.getBoundingClientRect();
    const s = svgEl.getBoundingClientRect();
    // The element's font-size is in its own user units; the CTM scale converts it to screen px.
    const ctm = el.getScreenCTM();
    const localFont = parseFloat(getComputedStyle(el).fontSize) || 16;
    const fontPx = localFont * (ctm ? Math.hypot(ctm.a, ctm.b) : 1);
    editor.selectElement(uid);
    textEdit = {
      uid,
      value: el.textContent ?? "",
      // A little breathing room so the caret and a descender aren't clipped by the box.
      x: r.left - s.left - 2,
      y: r.top - s.top - 2,
      w: Math.max(r.width, fontPx * 2) + fontPx,
      h: r.height + 4,
      fontPx: Math.max(8, Math.min(fontPx, r.height || fontPx)),
    };
    return true;
  }

  function commitTextEdit(): void {
    const t = textEdit;
    textEdit = null;
    if (t) editor.setNodeText(t.uid, t.value);
  }

  function onTextEditKey(e: KeyboardEvent): void {
    // Stop the canvas/page shortcuts from seeing ordinary typing (Delete would remove the label).
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      commitTextEdit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      textEdit = null;
    }
  }

  // Focus + select-all on open, so typing replaces the label the way a rename field does.
  $effect(() => {
    if (textEdit && textInput) textInput.select();
  });

  // Begin a move / resize / rotate gesture on the element `uid`.
  function startElXf(
    uid: string,
    kind: { t: "move" } | { t: "scale"; handle: TransformHandle } | { t: "rotate" },
    screen: Point,
  ): void {
    const el = svgEl.querySelector(`[data-uid="${CSS.escape(uid)}"]`) as SVGGraphicsElement | null;
    // A *copy* of the element's start matrix, deliberately: `consolidate().matrix` is live in some
    // engines (Firefox), so keeping the reference would make every pointermove compose onto the
    // previous frame's result instead of the gesture's start — the element runs away from the
    // cursor, accelerating (move/rotate visibly, scale subtly).
    const m0 = snapshotMatrix(el?.transform.baseVal.consolidate()?.matrix);
    if (kind.t === "move") {
      const toParent = parentMapper(el);
      const start = toParent(screen);
      // No existing transform → move via x/y (clean markup); else compose a translate.
      if (el && !el.getAttribute("transform"))
        elXf = {
          uid,
          toParent,
          mode: "moveXy",
          x0: Number(el.getAttribute("x") ?? "0") || 0,
          y0: Number(el.getAttribute("y") ?? "0") || 0,
          start,
          moved: false,
        };
      else elXf = { uid, toParent, mode: "move", m0, start, moved: false };
      return;
    }
    if (!elBox) return;
    // Scale + rotate run in the element's own space, so the grabbed anchor comes straight off the
    // local box the handles were drawn from — no mapping, and it stays exactly under the handle
    // however the element or its ancestors are transformed.
    const l = elBox.local;
    const bb = { minX: l.x, minY: l.y, maxX: l.x + l.w, maxY: l.y + l.h };
    const toOwn = ownMapper(el);
    const start = toOwn(screen);
    if (kind.t === "rotate") {
      const center = { x: l.x + l.w / 2, y: l.y + l.h / 2 };
      elXf = {
        uid,
        toOwn,
        mode: "rotate",
        m0,
        center,
        startAngle: Math.atan2(start.y - center.y, start.x - center.x),
        moved: false,
      };
    } else {
      const { anchor, moving, sx, sy } = handleAnchor(kind.handle, bb);
      elXf = {
        uid,
        toOwn,
        mode: "scale",
        m0,
        anchor,
        startPt: moving,
        axisX: sx,
        axisY: sy,
        corner: sx && sy,
        moved: false,
      };
    }
  }

  // Apply the live transform for the in-flight element gesture at the current pointer.
  function moveElXf(screen: Point, shift: boolean): void {
    if (!elXf) return;
    const cur =
      elXf.mode === "scale" || elXf.mode === "rotate" ? elXf.toOwn(screen) : elXf.toParent(screen);
    elXf.moved = true;
    if (elXf.mode === "moveXy") {
      editor.previewNodeMove(
        elXf.uid,
        round2(elXf.x0 + cur.x - elXf.start.x),
        round2(elXf.y0 + cur.y - elXf.start.y),
      );
      return;
    }
    let next: DOMMatrix;
    if (elXf.mode === "move") {
      next = new DOMMatrix()
        .translate(cur.x - elXf.start.x, cur.y - elXf.start.y)
        .multiply(elXf.m0);
    } else if (elXf.mode === "scale") {
      let sx = elXf.axisX ? safeDiv(cur.x - elXf.anchor.x, elXf.startPt.x - elXf.anchor.x) : 1;
      let sy = elXf.axisY ? safeDiv(cur.y - elXf.anchor.y, elXf.startPt.y - elXf.anchor.y) : 1;
      if (shift && elXf.corner) {
        const s = Math.max(Math.abs(sx), Math.abs(sy));
        sx = (sx < 0 ? -1 : 1) * s;
        sy = (sy < 0 ? -1 : 1) * s;
      }
      // Composed on the element's own side (m0 · S), so the box scales along its own axes.
      const a = elXf.anchor;
      next = elXf.m0.translate(a.x, a.y).scale(sx, sy).translate(-a.x, -a.y);
    } else {
      const c = elXf.center;
      let deg = ((Math.atan2(cur.y - c.y, cur.x - c.x) - elXf.startAngle) * 180) / Math.PI;
      if (shift) deg = Math.round(deg / 15) * 15;
      // Likewise m0 · R: the box turns about its own centre, and the measured frame follows it
      // because it's built from this very matrix — no separate "draw it turning" feedback needed.
      next = elXf.m0.translate(c.x, c.y).rotate(deg).translate(-c.x, -c.y);
    }
    editor.previewNodeAttr(elXf.uid, "transform", matrixStr(next));
  }

  /**
   * Right-click on the canvas: the verbs for whatever is under the pointer.
   *
   * A drawing surface is where people reach for a context menu most, and this is the one surface
   * that used to answer with the browser's. It selects what it is about to act on first, so the
   * menu and the highlight always agree about the subject.
   */
  function onCanvasContextMenu(e: MouseEvent) {
    if (!editor.doc) return;
    const screen = screenOf(e);
    const hit = hitTest(screen);
    const element = elementHit(e as unknown as PointerEvent);

    // An anchor (or one of its handles) is a thing with verbs of its own — the shape it belongs to
    // is not what a right-click on a node is asking about.
    if (hit.kind === "anchor" || hit.kind === "handle") {
      const ref = hit.ref;
      const node =
        editor.doc.paths[ref.pathIndex]?.subpaths[ref.subpathIndex]?.nodes[ref.nodeIndex];
      const name = editor.doc.paths[ref.pathIndex]?.id ?? "path";
      editor.select(ref);
      openMenu(e, `node ${ref.nodeIndex + 1} · ${name}`, [
        {
          label: "smooth",
          hint: node?.type === "smooth" ? "on" : "handles stay collinear",
          disabled: node?.type === "smooth",
          run: () => editor.setNodeType(ref, "smooth"),
        },
        {
          label: "corner",
          hint: node?.type === "corner" ? "on" : "handles move apart",
          disabled: node?.type === "corner",
          run: () => editor.setNodeType(ref, "corner"),
        },
        { label: "delete node", danger: true, hint: "⌫", run: () => editor.deleteNode(ref) },
      ]);
      return;
    }

    if (hit.kind === "fill" || hit.kind === "segment") {
      const index = hit.kind === "fill" ? hit.pathIndex : hit.pathIndex;
      if (!editor.selectedPaths.includes(index)) editor.selectPath(index);
      const path = editor.doc.paths[index];
      const uid = path?.uid;
      const items: MenuItem[] = [
        { label: "duplicate", hint: "⌘D", run: () => editor.duplicateSelected() },
        { label: "copy style", run: () => editor.copyStyle() },
        {
          label: "paste style",
          disabled: !editor.canPasteStyle,
          hint: editor.canPasteStyle ? undefined : "copy a style first",
          run: () => editor.pasteStyle(),
        },
        {
          label: "bring to front",
          hint: "⌘⇧]",
          disabled: !uid,
          run: () => uid && editor.reorderNodeExtreme(uid, true),
        },
        {
          label: "send to back",
          hint: "⌘⇧[",
          disabled: !uid,
          run: () => uid && editor.reorderNodeExtreme(uid, false),
        },
        {
          label: path?.locked ? "unlock" : "lock",
          run: () => editor.setPathLocked(index, !path?.locked),
        },
        { label: "delete", danger: true, hint: "⌫", run: () => editor.deletePath(index) },
      ];
      openMenu(e, path?.id ?? "shape", items);
      return;
    }

    if (element) {
      editor.selectElement(element.uid);
      const label = editor.textInfo(element.uid);
      openMenu(e, label?.name || label?.text || element.el.tagName.toLowerCase(), [
        {
          label: "convert to outlines",
          disabled: !label,
          hint: label ? undefined : "only a text label outlines",
          run: () => void outlineText(element.uid),
        },
        { label: "hide", run: () => editor.setNodeHidden(element.uid, true) },
      ]);
      return;
    }

    // Empty canvas: the verbs that belong to the document rather than a shape.
    openMenu(e, "canvas", [
      { label: "paste", hint: "⌘V", disabled: !editor.canPaste, run: () => editor.paste() },
      { label: "select all", hint: "⌘A", run: () => editor.selectAll() },
      { label: "fit to view", hint: "0", run: () => fitToView() },
    ]);
  }

  function onPointerDown(e: PointerEvent) {
    if (!editor.doc) return;
    setPointer(e.pointerId, screenOf(e));
    // A second finger starts a pinch — abort any single-pointer gesture first.
    if (pointers.length >= 2) {
      svgEl.setPointerCapture(e.pointerId);
      cancelDrag();
      cancelElXf(); // and revert an in-flight element transform, else it commits a jump on release
      if (pointers.length === 2) pinch = pinchState();
      return;
    }
    const pan = e.button === 1 || interaction.spaceHeld;
    if (!pan && e.button !== 0) return;
    svgEl.setPointerCapture(e.pointerId);
    const screen = screenOf(e);
    const hit: Hit = pan ? { kind: "empty" } : hitTest(screen);
    // Select tool + a non-shape element (text/image/use): its transform box handles take priority
    // (even over a shape underneath), then — where the model hit missed — dragging in the box moves
    // it, or a fresh element under the pointer gets picked + move-dragged.
    if (!pan && tools.active === "select") {
      if (editor.selectedElementUid) {
        const h = elHandleHit(screen);
        if (h) {
          startElXf(editor.selectedElementUid, h, screen);
          return;
        }
      }
      if (hit.kind === "empty") {
        if (editor.selectedElementUid && inElBox(screen)) {
          startElXf(editor.selectedElementUid, { t: "move" }, screen);
          return;
        }
        const el = elementHit(e);
        if (el) {
          editor.selectElement(el.uid);
          startElXf(el.uid, { t: "move" }, screen);
          return;
        }
      }
    }
    if (!pan) editor.selectedElementUid = null; // any other click clears the element selection box
    canvas.send({ type: "DOWN", hit, docPoint: viewport.toDoc(screen), event: e, pan, screen });
  }

  function onPointerMove(e: PointerEvent) {
    if (pointers.some((q) => q.id === e.pointerId)) setPointer(e.pointerId, screenOf(e));
    if (pinch && pointers.length >= 2) {
      markWheeling(); // a pinch moves the viewport too, and never fires a wheel event
      const next = pinchState();
      if (pinch.dist > 0) viewport.zoomAt(pinch.mid, next.dist / pinch.dist);
      viewport.panBy(next.mid.x - pinch.mid.x, next.mid.y - pinch.mid.y);
      pinch = next;
      return;
    }
    const screen = screenOf(e);
    if (elXf) {
      moveElXf(screen, e.shiftKey);
      return;
    }
    if (!canvas.idle) {
      canvas.send({ type: "MOVE", docPoint: viewport.toDoc(screen), screen, event: e });
      return;
    }
    // idle → hover feedback (not part of a gesture). A selected element's transform box gets the
    // directional resize / rotate / move cursors (its handles aren't in the model hit-test).
    if (tools.active === "select" && editor.selectedElementUid) {
      const h = elHandleHit(screen);
      if (h) {
        hoverCursor = h.t === "rotate" ? "grab" : transformCursor(h.handle, elBox?.frame.angle);
        return;
      }
      if (inElBox(screen)) {
        hoverCursor = "move";
        return;
      }
    }
    activeTool.hover?.(viewport.toDoc(screen));
    if (editor.doc) hoverCursor = activeTool.cursor(hitTest(screen));
  }

  function onPointerUp(e: PointerEvent) {
    pointers = pointers.filter((q) => q.id !== e.pointerId);
    if (pointers.length === 0) pinch = null; // no fingers left → never leave a stale pinch
    if (svgEl.hasPointerCapture(e.pointerId)) svgEl.releasePointerCapture(e.pointerId);
    if (pinch) {
      if (pointers.length < 2) pinch = null; // pinch owned this gesture
      return;
    }
    if (elXf) {
      if (elXf.moved) editor.commit(); // record the live move/resize/rotate as one undo step
      elXf = null;
      return;
    }
    canvas.send({ type: "UP", docPoint: viewport.toDoc(screenOf(e)) });
  }

  // A cancelled pointer (OS gesture takeover, lost capture) must not commit — revert an in-flight
  // element transform / machine gesture and drop the pointer so a stale id can't wedge a phantom
  // pinch. (pointerup commits; pointercancel abandons — hence a separate handler.)
  function onPointerCancel(e: PointerEvent) {
    pointers = pointers.filter((q) => q.id !== e.pointerId);
    if (pointers.length < 2) pinch = null;
    if (svgEl.hasPointerCapture(e.pointerId)) svgEl.releasePointerCapture(e.pointerId);
    if (elXf) cancelElXf();
    else if (!canvas.idle) cancelDrag();
  }

  // Double-click a shape (select tool) to enter node-editing mode — Figma-style. Object
  // mode moves the whole shape on drag; nodes only become editable after entering here.
  function onDblClick(e: MouseEvent) {
    if (!editor.doc || tools.active !== "select") return;
    // A `<text>` label has no anchors to node-edit, so double-click means "edit the words" —
    // the same gesture every other editor uses, instead of a trip to the Inspector.
    if (startTextEdit(e)) return;
    const hit = hitTest(screenOf(e));
    // Already node-editing and the double-click landed on an anchor → toggle it between corner and
    // smooth (Pixelmator-style): smooth synthesizes tangent handles, corner straightens back to a
    // hard point. Object-mode double-clicks fall through to entering node mode below.
    if (editor.nodeEditIndex !== null && hit.kind === "anchor") {
      const { pathIndex, subpathIndex, nodeIndex } = hit.ref;
      const node = editor.doc.paths[pathIndex]?.subpaths[subpathIndex]?.nodes[nodeIndex];
      if (node) editor.setNodeType(hit.ref, node.type === "smooth" ? "corner" : "smooth");
      return;
    }
    if (hit.kind === "fill" || hit.kind === "segment") editor.enterNodeEdit(hit.pathIndex);
  }

  // Zoom responsiveness knobs (bump for snappier zoom). WHEEL_ZOOM_SENS scales
  // the ctrl/⌘+wheel step; PINCH_GAIN (>1) makes a Safari trackpad pinch zoom
  // faster than the raw finger spread.
  const WHEEL_ZOOM_SENS = 0.01;
  const PINCH_GAIN = 1.8;

  // Wheel zoom/pan never reaches the gesture machine (it's pure viewport), so it reports itself:
  // a wheel event marks the canvas as interacting until the stream stops. See `interacting`.
  const WHEEL_IDLE_MS = 180;
  let wheeling = $state(false);
  let wheelIdle: ReturnType<typeof setTimeout> | undefined;
  function markWheeling(): void {
    wheeling = true;
    clearTimeout(wheelIdle);
    wheelIdle = setTimeout(() => (wheeling = false), WHEEL_IDLE_MS);
  }

  /**
   * A gesture is moving the view or a shape right now — so the canvas may trade fidelity for
   * frames. It buys a lot: WebKit sizes a filter's surface from the object in *device* space, so a
   * drop shadow on a shape zoomed to 400x becomes a several-hundred-megapixel blur that it
   * re-rasterizes every frame, and panning collapses to ~4fps (measured; Chromium clips the same
   * filter to the viewport and stays at 60). Dropping filters for the duration of the gesture
   * brings that back to 40fps, and the `will-change` hint the rest of the way to 60. The accurate
   * frame paints as soon as the gesture ends.
   */
  const interacting = $derived(canvas.panning || canvas.dragging || wheeling);

  function onWheel(e: WheelEvent) {
    if (!editor.doc) return;
    e.preventDefault();
    markWheeling();
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
      markWheeling();
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
    onpointercancel={onPointerCancel}
    oncontextmenu={onCanvasContextMenu}
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
        <!-- stops sorted by offset: SVG (like CSS) clamps out-of-order stops, so a mid stop added
             out of order would otherwise vanish. Model keeps insertion order (stable drag index). -->
        {#each editor.doc.gradients as g (g.id)}
          {@const stops = [...g.stops].sort((a, b) => a.offset - b.offset)}
          {#if g.kind === "radial"}
            <radialGradient id={g.id} cx={g.cx} cy={g.cy} r={g.r}>
              {#each stops as s, i (i)}
                <stop offset={s.offset} stop-color={s.color} stop-opacity={s.opacity ?? 1} />
              {/each}
            </radialGradient>
          {:else}
            <linearGradient id={g.id} x1={g.x1} y1={g.y1} x2={g.x2} y2={g.y2}>
              {#each stops as s, i (i)}
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
      class:interacting
      transform={`translate(${viewport.tx} ${viewport.ty}) scale(${viewport.scale})`}
    >
      <!-- the whole document — imported, drawn, and baked booleans — rendered declaratively from
           the tree in true document order (one representation, one z-order) -->
      <g class="artwork">
        {#each renderTree as n, i (i)}{@render renderNode(n)}{/each}
      </g>
    </g>
    <Overlay elementFrame={elBox?.frame ?? null} />
  </svg>

  {#if textEdit}
    <!-- svelte-ignore a11y_autofocus -->
    <input
      bind:this={textInput}
      class="text-edit"
      autofocus
      spellcheck="false"
      aria-label="Edit text"
      style:left="{textEdit.x}px"
      style:top="{textEdit.y}px"
      style:width="{textEdit.w}px"
      style:height="{textEdit.h}px"
      style:font-size="{textEdit.fontPx}px"
      bind:value={textEdit.value}
      onkeydown={onTextEditKey}
      onblur={commitTextEdit}
    />
  {/if}
</div>

<style>
  .canvas-wrap {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }

  /* The inline label editor, laid over the <text> it edits. Deliberately plain — an accent ring
     and the page background, so it reads as the label becoming editable rather than a dialog. */
  .text-edit {
    position: absolute;
    z-index: 5;
    box-sizing: border-box;
    padding: 0 3px;
    border: 1px solid var(--halo-accent);
    border-radius: 3px;
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
    font: inherit;
    line-height: 1;
    outline: none;
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

  /* Fidelity yields to frames while a gesture runs — see `interacting`. A filter is the one
     document feature whose cost grows with the square of the zoom in WebKit, so it goes quiet for
     the duration; `will-change` then lets the compositor keep up. Both end with the gesture, and
     the next frame is the accurate one. */
  .scene.interacting {
    will-change: transform;
  }

  .scene.interacting :global(*) {
    filter: none;
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
