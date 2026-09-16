<script lang="ts">
  import AlignCenterHorizontal from "@lucide/svelte/icons/align-center-horizontal";
  import AlignCenterVertical from "@lucide/svelte/icons/align-center-vertical";
  import AlignEndHorizontal from "@lucide/svelte/icons/align-end-horizontal";
  import AlignEndVertical from "@lucide/svelte/icons/align-end-vertical";
  import AlignHorizontalDistributeCenter from "@lucide/svelte/icons/align-horizontal-distribute-center";
  import AlignStartHorizontal from "@lucide/svelte/icons/align-start-horizontal";
  import AlignStartVertical from "@lucide/svelte/icons/align-start-vertical";
  import AlignVerticalDistributeCenter from "@lucide/svelte/icons/align-vertical-distribute-center";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import Copy from "@lucide/svelte/icons/copy";
  import FlipHorizontal2 from "@lucide/svelte/icons/flip-horizontal-2";
  import FlipVertical2 from "@lucide/svelte/icons/flip-vertical-2";
  import Group from "@lucide/svelte/icons/group";
  import PaintBucket from "@lucide/svelte/icons/paint-bucket";
  import Trash2 from "@lucide/svelte/icons/trash-2";

  import { tightBounds } from "$lib/model/geometry";
  import type { NodeType } from "$lib/model/types";
  import { editor } from "$lib/stores/document.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { tools } from "$lib/stores/tool.svelte";
  import { canReadInstalledFonts, installedFamilyNames } from "$lib/text/fonts";
  import { outlineText, warmFontFor } from "$lib/text/outline";
  import { activePivot } from "$lib/tools/rotate";
  import { boxCenter, scaleSubpaths } from "$lib/tools/transform";

  import ColorInput from "./ColorInput.svelte";
  import PaintInput from "./PaintInput.svelte";

  // Basic (touch-up) mode hides pro sections (arrange/align, path craft, booleans, skew,
  // grouping, gradients); advanced shows them. Keeps basic = tweak-and-save.
  const advanced = $derived(settings.uiLevel === "advanced");
  const doc = $derived(editor.doc);
  const sel = $derived(editor.selection);
  const node = $derived(editor.selectedNode);

  // Style target = the selected path (a selected node implies its path).
  const path = $derived(editor.selectedPathElement);
  const pathIndex = $derived(editor.selectedPathIndex);
  // `subject`, not `active`: with the eyedropper borrowed over the pen, this panel is still the
  // pen's — it's what you opened the eyedropper to change.
  const isCreateTool = $derived(
    ["pen", "circle", "rect", "line", "polygon", "star"].includes(tools.subject),
  );

  // A selected non-shape element (text/image/use) — its render node, edited generically by attr.
  const elementSel = $derived(editor.selectedElement);
  // Selecting a label pulls its stored font into memory, so the convert click can read it without
  // awaiting — Safari drops a file dialog opened after an await (see lib/text/outline.ts).
  $effect(() => {
    if (elementSel?.kind === "element" && elementSel.tag === "text") warmFontFor(elementSel.uid);
  });
  const elTag = $derived(elementSel?.kind === "element" ? elementSel.tag : "");
  const elAttr = (k: string) => (elementSel?.kind === "element" ? (elementSel.attrs[k] ?? "") : "");
  const elText = $derived(
    elementSel?.kind === "element"
      ? (elementSel.children.find((c) => c.kind === "text")?.text ?? "")
      : "",
  );
  function setElAttr(key: string, e: Event) {
    const v = (e.currentTarget as HTMLInputElement).value.trim();
    if (elementSel?.kind === "element") editor.setNodeAttr(elementSel.uid, key, v || null);
  }
  function setElText(e: Event) {
    if (elementSel?.kind === "element")
      editor.setNodeText(elementSel.uid, (e.currentTarget as HTMLInputElement).value);
  }

  /**
   * Families to suggest in the font field.
   *
   * A `font-family` is just text in the document — anything can be typed, and a name the machine
   * lacks still renders wherever the font exists. Chromium can *list* what's installed
   * (permission-gated), so where it will, the field offers those names; everywhere else the same
   * field takes typing, with the generic families as the floor.
   */
  const GENERIC_FAMILIES = ["sans-serif", "serif", "monospace"];
  let installedFamilies = $state<string[]>([]);
  let askedForFamilies = false;

  async function suggestFamilies(): Promise<void> {
    if (askedForFamilies || !canReadInstalledFonts()) return;
    askedForFamilies = true; // one prompt per session, whatever the answer
    installedFamilies = await installedFamilyNames();
  }

  const fontSuggestions = $derived([...GENERIC_FAMILIES, ...installedFamilies]);

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
  // Paint set to "none" makes its sub-controls no-ops — dim/disable them so they don't read as live.
  const fillNone = $derived((style.fill ?? "none") === "none");
  const strokeNone = $derived((style.stroke ?? "none") === "none");

  let opacityLive = $state<number | null>(null);

  // Copy-style gives a brief confirmation tint (it's otherwise a silent action), matching the top
  // bar's Copy-SVG feedback.
  let styleCopied = $state(false);
  function copyStyleWithFeedback() {
    editor.copyStyle();
    styleCopied = true;
    setTimeout(() => (styleCopied = false), 1200);
  }

  // When on, the boolean buttons build a *live* (non-destructive) boolean group — operands stay
  // editable and the result recomputes — instead of baking + deleting the inputs.
  let booleanLive = $state(false);
  function doBoolean(op: "union" | "subtract" | "intersect" | "exclude"): void {
    if (booleanLive) editor.makeBooleanGroup(op);
    else editor.booleanOp(op);
  }
  const opacityShown = $derived(opacityLive ?? opacityPct);

  let offsetDist = $state(4);

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

  // Corner radius the rect tool draws with (a tool preference, shown in "new shape style").
  function setCornerRadius(e: Event) {
    const v = evalNum((e.currentTarget as HTMLInputElement).value);
    if (v !== null && v >= 0) tools.cornerRadius = v;
  }

  // Drop shadow (the one authorable effect) — a filter def + the path's `filter` attr.
  const hasShadow = $derived((style.filter ?? "").includes("url("));
  function toggleShadow() {
    if (pathIndex === null) return;
    if (hasShadow) editor.clearDropShadow(pathIndex);
    else editor.dropShadow(pathIndex);
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

  // The document as a subject — what the panel shows when nothing is selected.
  const canvas = $derived(doc?.viewBox ?? { minX: 0, minY: 0, width: 0, height: 0 });
  const shapeCount = $derived(doc?.paths.filter((p) => !p.deleted).length ?? 0);

  function setCanvas(key: "minX" | "minY" | "width" | "height", e: Event) {
    const v = evalNum((e.currentTarget as HTMLInputElement).value);
    if (v === null) return;
    const next = { ...canvas, [key]: v };
    editor.setViewBox(next.minX, next.minY, next.width, next.height);
  }

  // The selected path's bounding box, for the numeric transform panel.
  //
  // Deliberately the **document-space** box, not the shape's own tilted one (`boxAngle`), even
  // once the canvas box is turned: X/Y here are the bbox corner, a place you can point at in the
  // document, whereas the same corner measured in a tilted frame is a coordinate in a space
  // nothing else in the panel uses. So the numbers and the field below scale along the same axes
  // they're quoted in. (Pixelmator's own-size-plus-angle panel is a further step, and would want
  // all four fields moved into the shape's frame together.)
  const bounds = $derived(path ? tightBounds(path.subpaths) : null);

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

  // Shear the selected path by a one-shot angle (deg) about the transform pivot (default box
  // centre); the input resets to 0 so each entry applies once.
  function skew(axis: "x" | "y", e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const deg = evalNum(input.value);
    input.value = "0";
    if (deg === null || deg === 0 || !path || pathIndex === null || !bounds) return;
    // The same point the rotate tool would turn about — the box's centre as drawn, so a typed
    // angle and a dragged one agree even when the box is turned.
    const center = activePivot() ?? boxCenter(bounds);
    const k = Math.tan((deg * Math.PI) / 180);
    // Through the semantic `affinePath` op, like rotate below — a skew used to be written as a
    // wholesale geometry replacement, so the intent never reached the core and MCP had no way to
    // shear at all.
    editor.skewPath(pathIndex, axis === "x" ? k : 0, axis === "y" ? k : 0, center);
  }

  // Rotate the selected path a one-shot angle (deg, clockwise) — the input resets to 0 so each
  // entry applies once. Routes through the semantic `rotatePath` op.
  //
  // About the rotate tool's pivot when one is placed, else the centre of the box as drawn: with the
  // pivot marker on screen, typing an angle has to mean the same thing as dragging one — and on a
  // turned selection the drawn centre isn't the centre of its document-axis bounds.
  function rotateBy(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const deg = evalNum(input.value);
    input.value = "0";
    if (deg !== null && deg !== 0 && pathIndex !== null)
      editor.rotatePath(pathIndex, deg, activePivot() ?? undefined);
  }
  function rotateQuick(deg: number) {
    if (pathIndex !== null) editor.rotatePath(pathIndex, deg, activePivot() ?? undefined);
  }

  // uid → doc.paths array index, so a component part maps to its editable path. LAYERS keeps its
  // own copy of this; it's three lines of derived state, not a module.
  const uidToIndex = $derived(
    new Map((doc?.paths ?? []).flatMap((p, i) => (p.uid ? [[p.uid, i] as const] : []))),
  );

  let compExpanded = $state<string[]>([]);
  function toggleComp(uid: string) {
    compExpanded = compExpanded.includes(uid)
      ? compExpanded.filter((x) => x !== uid)
      : [...compExpanded, uid];
  }

  function createComponentFromSelection() {
    const name = prompt("component name:", `component ${editor.components.length + 1}`);
    if (name?.trim()) editor.createComponentFromSelection(name.trim());
  }
  function renameComp(uid: string, current: string) {
    const name = prompt("rename component:", current);
    if (name?.trim() && name.trim() !== current) editor.renameComponent(uid, name.trim());
  }
  function deleteComp(uid: string, name: string, instances: number) {
    const note = instances ? ` and its ${instances} instance${instances === 1 ? "" : "s"}` : "";
    if (confirm(`delete component “${name}”${note}?`)) editor.deleteComponent(uid);
  }
</script>

<aside class="inspector">
  {#if editor.multiSelected && advanced}
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
      <label
        class="live-toggle"
        title="live = non-destructive: operands stay editable, result recomputes"
      >
        <input type="checkbox" bind:checked={booleanLive} /> live (non-destructive)
      </label>
      <div class="combine">
        <button title="unite" onclick={() => doBoolean("union")}>union</button>
        <button title="front minus back" onclick={() => doBoolean("subtract")}>subtract</button>
        <button title="intersection" onclick={() => doBoolean("intersect")}>intersect</button>
        <button title="exclude overlap" onclick={() => doBoolean("exclude")}>exclude</button>
      </div>
      <button
        class="combine-all"
        title="make compound path — one element, subpaths kept distinct"
        onclick={() => editor.combinePaths()}>compound path</button
      >
    </section>
  {/if}

  {#if elementSel && elementSel.kind === "element"}
    <section>
      <h2>{elTag}</h2>
      {#if elTag === "text"}
        <label class="row">
          <span class="rlbl">text</span>
          <input
            class="dash"
            type="text"
            aria-label="text content"
            value={elText}
            onchange={setElText}
            spellcheck="false"
          />
        </label>
      {/if}
      <div class="coords">
        <label
          >x <input type="text" value={elAttr("x")} onchange={(e) => setElAttr("x", e)} /></label
        >
        <label
          >y <input type="text" value={elAttr("y")} onchange={(e) => setElAttr("y", e)} /></label
        >
      </div>
      {#if elTag === "image" || elAttr("width") || elAttr("height")}
        <div class="coords">
          <label
            >w <input
              type="text"
              value={elAttr("width")}
              onchange={(e) => setElAttr("width", e)}
            /></label
          >
          <label
            >h <input
              type="text"
              value={elAttr("height")}
              onchange={(e) => setElAttr("height", e)}
            /></label
          >
        </div>
      {/if}
      {#if elTag === "text"}
        <label class="row">
          <span class="rlbl">font</span>
          <!-- Free text with suggestions, not a closed picker: the family is a name in the
               document, and a font this machine lacks is still the right answer for a file that
               will be opened somewhere else. -->
          <input
            class="dash"
            type="text"
            list="nib-font-families"
            placeholder="sans-serif"
            spellcheck="false"
            value={elAttr("font-family")}
            onfocus={suggestFamilies}
            onchange={(e) => setElAttr("font-family", e)}
          />
        </label>
        <datalist id="nib-font-families">
          {#each fontSuggestions as family (family)}
            <option value={family}></option>
          {/each}
        </datalist>
        <div class="coords">
          <label
            >size <input
              type="number"
              min="0"
              step="1"
              value={elAttr("font-size") || "16"}
              onchange={(e) => setElAttr("font-size", e)}
            /></label
          >
          <label
            >weight <input
              type="text"
              placeholder="normal"
              spellcheck="false"
              value={elAttr("font-weight")}
              onchange={(e) => setElAttr("font-weight", e)}
            /></label
          >
        </div>
        <!-- Slant is two-state, so it's a toggle rather than another text field — lit when on,
             which is what an accent border means everywhere else in this panel. -->
        <button
          class="ghost-btn slant-toggle"
          class:on={elAttr("font-style") === "italic"}
          title="italic"
          onclick={() =>
            elementSel?.kind === "element" &&
            editor.setNodeAttr(
              elementSel.uid,
              "font-style",
              elAttr("font-style") === "italic" ? null : "italic",
            )}
        >
          italic
        </button>
        <ColorInput
          label="fill"
          value={elAttr("fill") || "#000000"}
          editable
          oninput={(v) =>
            elementSel?.kind === "element" && editor.previewNodeAttr(elementSel.uid, "fill", v)}
          onchange={(v) =>
            elementSel?.kind === "element" && editor.setNodeAttr(elementSel.uid, "fill", v)}
        />
        <button
          class="detach-btn"
          title="turn the words into editable path geometry — node-editable, boolean-able, and it renders anywhere without the font (undo brings the text back)"
          onclick={() => elementSel?.kind === "element" && outlineText(elementSel.uid)}
        >
          convert to outlines
        </button>
      {/if}
      {#if elTag === "use"}
        <button
          class="detach-btn"
          title="bake this instance into independent, editable shapes (leaves the component + other instances)"
          onclick={() => elementSel?.kind === "element" && editor.detachInstance(elementSel.uid)}
        >
          detach instance
        </button>
        <p class="hint">instance of a component · move/resize is free</p>
      {:else}
        <p class="hint">{elTag} element · attributes edit in place</p>
      {/if}
    </section>
  {/if}

  {#if !path && !elementSel && !isCreateTool}
    <!-- Nothing selected. The honest subject then isn't "nothing" — it's the DOCUMENT, which has
         real properties worth showing. Deliberately not an auto-selection: selecting is something
         the user does, and a shape selected on their behalf is one Delete away from being lost.
         Deliberately not a hint telling them to select something either — that's furniture. -->
    <section>
      <h2>document</h2>
      {#if editor.hasDocument}
        <!-- Origin as well as size: without it the canvas can only be resized from its top-left,
             and cropping to something in the middle of the artwork is the usual reason to touch
             these numbers at all. -->
        <div class="coords">
          <label
            >x <input
              type="number"
              step="1"
              aria-label="canvas x"
              value={round(canvas.minX)}
              onchange={(e) => setCanvas("minX", e)}
            /></label
          >
          <label
            >y <input
              type="number"
              step="1"
              aria-label="canvas y"
              value={round(canvas.minY)}
              onchange={(e) => setCanvas("minY", e)}
            /></label
          >
        </div>
        <div class="coords">
          <label
            >w <input
              type="number"
              min="1"
              step="1"
              aria-label="canvas width"
              value={round(canvas.width)}
              onchange={(e) => setCanvas("width", e)}
            /></label
          >
          <label
            >h <input
              type="number"
              min="1"
              step="1"
              aria-label="canvas height"
              value={round(canvas.height)}
              onchange={(e) => setCanvas("height", e)}
            /></label
          >
        </div>
        <!-- Crops or pads, like every editor's canvas size. Content left outside is clipped on
             export, which is the only way the number can mean anything. -->
        <p class="note">{shapeCount} shape{shapeCount === 1 ? "" : "s"} · crops or pads</p>
        <button class="ghost-btn wide" onclick={() => editor.fitViewBoxToContent()}>
          fit canvas to artwork
        </button>
      {:else}
        <p class="empty">no document</p>
      {/if}
    </section>
  {/if}

  {#if path || isCreateTool}
    <section>
      <div class="lhead">
        <h2>{path ? "style" : "new shape style"}</h2>
        {#if path}
          <div class="lhead-actions">
            <button
              class="ghost-btn"
              class:ok={styleCopied}
              title={styleCopied ? "copied" : "copy style"}
              aria-label="copy style"
              onclick={copyStyleWithFeedback}><Copy size={13} /></button
            >
            <button
              class="ghost-btn"
              title="paste style"
              aria-label="paste style"
              disabled={!editor.canPasteStyle}
              onclick={() => editor.pasteStyle()}><PaintBucket size={13} /></button
            >
          </div>
        {/if}
      </div>
      {#snippet seg(label: string, key: string, options: string[], dflt: string, disabled: boolean)}
        <div class="segrow" class:dim={disabled}>
          <span class="seglbl">{label}</span>
          <div class="segbtns">
            {#each options as opt (opt)}
              <button
                class:active={(style[key] ?? dflt) === opt}
                {disabled}
                onclick={() => setStyle(key, opt)}
              >
                {opt}
              </button>
            {/each}
          </div>
        </div>
      {/snippet}
      <PaintInput
        label="fill"
        value={style.fill ?? "none"}
        setPaint={(v) => setStyle("fill", v)}
        previewPaint={(v) => previewStyle("fill", v)}
      />
      {@render seg("rule", "fill-rule", ["nonzero", "evenodd"], "nonzero", fillNone)}
      <PaintInput
        label="stroke"
        value={style.stroke ?? "none"}
        setPaint={(v) => setStyle("stroke", v)}
        previewPaint={(v) => previewStyle("stroke", v)}
      />
      <label class="row" class:dim={strokeNone}>
        <span class="rlbl">width</span>
        <input
          type="number"
          min="0"
          step="0.5"
          value={style["stroke-width"] ?? "1"}
          disabled={strokeNone}
          onchange={setWidth}
        />
      </label>
      {@render seg("cap", "stroke-linecap", ["butt", "round", "square"], "butt", strokeNone)}
      {@render seg("join", "stroke-linejoin", ["miter", "round", "bevel"], "miter", strokeNone)}
      <label class="row" class:dim={strokeNone}>
        <span class="rlbl">dash</span>
        <input
          class="dash"
          type="text"
          value={style["stroke-dasharray"] ?? ""}
          placeholder="none"
          disabled={strokeNone}
          onchange={onDash}
          spellcheck="false"
        />
      </label>
      <label class="row">
        <span class="rlbl">opacity</span>
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
      {#if !path && tools.active === "rect"}
        <label class="row">
          <span class="rlbl">corner°</span>
          <input
            type="number"
            min="0"
            step="1"
            value={tools.cornerRadius}
            oninput={setCornerRadius}
            aria-label="corner radius"
          />
        </label>
      {/if}
      {#if advanced && path}
        <button
          class="ghost-btn shadow-toggle"
          class:on={hasShadow}
          title="soft drop shadow (offset + blur)"
          onclick={toggleShadow}
        >
          {hasShadow ? "remove drop shadow" : "+ drop shadow"}
        </button>
      {/if}
    </section>
  {/if}

  {#if path && bounds}
    <section>
      <h2>transform</h2>
      <div class="pairrow">
        <span class="rlbl">pos</span>
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
      <div class="pairrow">
        <span class="rlbl">size</span>
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
      <div class="offsetrow">
        <span class="seglbl">rotate°</span>
        <input type="number" step="15" value="0" onchange={rotateBy} aria-label="rotate degrees" />
        <button
          class="ghost-btn"
          title="rotate 90° counter-clockwise"
          onclick={() => rotateQuick(-90)}>⟲</button
        >
        <button class="ghost-btn" title="rotate 90° clockwise" onclick={() => rotateQuick(90)}
          >⟳</button
        >
      </div>
      <div class="pathops">
        <button class="ghost-btn" title="flip horizontal (⇧H)" onclick={() => editor.flip("h")}>
          <FlipHorizontal2 size={14} /> flip h
        </button>
        <button class="ghost-btn" title="flip vertical (⇧V)" onclick={() => editor.flip("v")}>
          <FlipVertical2 size={14} /> flip v
        </button>
      </div>
      {#if advanced}
        <div class="pathops">
          <button class="ghost-btn" onclick={() => editor.simplifyPath()}>simplify</button>
          <button class="ghost-btn" onclick={() => editor.outlineStroke()}>outline stroke</button>
        </div>
        {#if path && path.subpaths.length > 1}
          <button
            class="combine-all"
            title="release compound — split subpaths into separate, individually styleable paths"
            onclick={() => editor.releaseCompound()}>release compound</button
          >
        {/if}
        <div class="offsetrow">
          <span class="seglbl">offset</span>
          <input type="number" step="1" bind:value={offsetDist} />
          <button class="ghost-btn" onclick={() => editor.offsetPath(offsetDist)}>apply</button>
        </div>
        <div class="pairrow">
          <span class="rlbl">skew°</span>
          <label>x <input type="number" step="1" value="0" onchange={(e) => skew("x", e)} /></label>
          <label>y <input type="number" step="1" value="0" onchange={(e) => skew("y", e)} /></label>
        </div>
      {/if}
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

  {#if advanced && (editor.components.length || editor.selectedPaths.length >= 1)}
    <section class="components">
      <div class="lhead">
        <h2>components</h2>
        {#if editor.selectedPaths.length >= 1}
          <button
            class="ghost-btn"
            title="create a reusable component from the selection"
            aria-label="create component from selection"
            onclick={createComponentFromSelection}
          >
            <Group size={13} /> create
          </button>
        {/if}
      </div>
      {#if editor.components.length}
        <ul class="complist">
          {#each editor.components as c (c.uid)}
            <li>
              <div class="comprow">
                <button
                  class="disclosure"
                  aria-label={compExpanded.includes(c.uid) ? "collapse parts" : "show parts"}
                  onclick={() => toggleComp(c.uid)}
                >
                  {#if compExpanded.includes(c.uid)}
                    <ChevronDown size={12} />
                  {:else}
                    <ChevronRight size={12} />
                  {/if}
                </button>
                <button
                  class="row-btn"
                  ondblclick={() => renameComp(c.uid, c.name)}
                  title="double-click to rename (updates every instance)"
                >
                  <span class="pid">{c.name}</span>
                  <span class="meta">{c.partUids.length} parts · {c.instanceCount}×</span>
                </button>
                <button
                  class="stamp"
                  title="stamp an instance"
                  onclick={() => editor.stampInstance(c.name)}>+ stamp</button
                >
                <button
                  class="del"
                  title="delete this component and all its instances"
                  aria-label="delete component"
                  onclick={() => deleteComp(c.uid, c.name, c.instanceCount)}
                >
                  <Trash2 size={12} />
                </button>
              </div>
              {#if compExpanded.includes(c.uid)}
                <ul class="partlist">
                  {#each c.partUids as pu (pu)}
                    {@const idx = uidToIndex.get(pu)}
                    {#if idx !== undefined}
                      <li>
                        <button
                          class="part-btn"
                          class:active={editor.selectedPathIndex === idx}
                          title="select this part — editing it (fill, geometry) updates every instance"
                          onclick={() => editor.selectPath(idx)}
                        >
                          {doc?.paths[idx]?.id ?? "part"}
                        </button>
                      </li>
                    {/if}
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty">select shapes, then “create”</p>
      {/if}
    </section>
  {/if}
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

  /* The shared STYLE-panel label column — keeps every control aligned at one x. Matches
     `.seglbl` + PaintInput's `.plabel`/`.slbl`. */
  .rlbl {
    width: 50px;
    flex: none;
    color: var(--halo-text-muted);
    white-space: nowrap;
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
    width: 50px;
    flex: none;
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

  .hint {
    margin: 2px 0 0;
    font-size: 11px;
    color: var(--halo-text-muted);
  }

  /* layers panel */
  .lhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .ghost-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
    font-size: 11px;
  }

  .ghost-btn:hover:not(:disabled) {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  /* brief confirmation tint after copy-style */
  .ghost-btn.ok {
    border-color: var(--halo-connected);
    color: var(--halo-connected);
  }

  .ghost-btn:disabled {
    opacity: 0.4;
  }

  .lhead-actions {
    display: flex;
    gap: 4px;
  }

  .pathops {
    display: flex;
    gap: 4px;
    margin-top: 6px;
  }

  .pathops .ghost-btn {
    flex: 1;
    justify-content: center;
  }

  .offsetrow {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }

  .offsetrow input {
    flex: 1;
    min-width: 0;
  }

  .offsetrow .ghost-btn {
    justify-content: center;
  }

  /* A paired row: a 50px leading label (.rlbl) + two axis-prefixed inputs sharing the rest, so
     pos / size / skew align to the same control column as the single-label rows. */
  .pairrow {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
  }

  .pairrow label {
    display: flex;
    flex: 1;
    min-width: 0;
    align-items: center;
    gap: 5px;
    color: var(--halo-text-muted);
  }

  .pairrow label input {
    width: 100%;
    min-width: 0;
  }

  /* a style row whose paint is "none" — its sub-controls are inert, so dim them */
  .dim {
    opacity: 0.45;
  }

  .complist {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
  }

  .comprow {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 1px 2px;
  }

  .comprow .row-btn {
    flex: 1;
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 6px;
    min-width: 0;
    padding: 3px 6px;
    border: none;
    border-radius: var(--halo-radius-pill);
    background: transparent;
    color: var(--halo-text-main);
    text-align: left;
    font-size: 12px;
  }

  .comprow .row-btn:hover {
    background: var(--halo-bg-main);
  }

  .comprow .meta {
    flex: none;
    font-size: 10px;
    color: var(--halo-text-muted);
  }

  .comprow .stamp {
    flex: none;
    padding: 2px 7px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
    font-size: 11px;
  }

  .comprow .stamp:hover {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .comprow .del {
    flex: none;
    display: flex;
    align-items: center;
    padding: 3px;
    border: none;
    border-radius: var(--halo-radius-pill);
    background: transparent;
    color: var(--halo-text-muted);
    cursor: pointer;
  }

  .comprow .del:hover {
    color: var(--halo-danger, #e5484d);
    background: var(--halo-bg-main);
  }

  .detach-btn {
    width: 100%;
    margin-top: 6px;
    padding: 4px 8px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
    font-size: 11px;
    cursor: pointer;
  }

  .detach-btn:hover {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .shadow-toggle {
    width: 100%;
    margin-top: 6px;
  }

  .wide {
    width: 100%;
    margin-top: 6px;
  }

  .note {
    margin: 6px 0 0;
    color: var(--halo-text-muted);
    font-size: 11px;
  }

  .shadow-toggle.on {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .slant-toggle {
    width: 100%;
    margin-top: 6px;
    font-style: italic;
  }

  .slant-toggle.on {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .comprow .disclosure {
    flex: none;
    display: flex;
    align-items: center;
    padding: 2px;
    border: none;
    background: transparent;
    color: var(--halo-text-muted);
  }

  .partlist {
    list-style: none;
    margin: 0 0 2px;
    padding: 0 0 0 18px;
  }

  .part-btn {
    width: 100%;
    padding: 2px 6px;
    border: none;
    border-radius: var(--halo-radius-pill);
    background: transparent;
    color: var(--halo-text-muted);
    text-align: left;
    font-size: 11px;
  }

  .part-btn:hover {
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
  }

  .part-btn.active {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  /* right-click context menu */

  /* live-boolean badge on a group header */

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

  /* boolean path ops (union / subtract / intersect / exclude) */
  .live-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 6px 0 4px;
    font-size: 11px;
    color: var(--halo-text-muted);
    cursor: pointer;
  }

  .combine {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
  }

  .combine button {
    padding: 4px 0;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
    font-size: 11px;
  }

  .combine button:hover {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .combine-all {
    width: 100%;
    margin-top: 4px;
    padding: 4px 0;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
    font-size: 11px;
  }

  .combine-all:hover {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }
</style>
