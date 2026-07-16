<script lang="ts">
  import type { GradientStop } from "$lib/model/types";
  import { editor } from "$lib/stores/document.svelte";
  import { settings } from "$lib/stores/settings.svelte";

  import ColorInput from "./ColorInput.svelte";

  type Mode = "none" | "solid" | "linear" | "radial";
  // Gradients are an advanced paint; basic (touch-up) mode offers solid colour only. An
  // existing gradient still displays + edits — you just can't create one in basic.
  const modes = $derived<[Mode, string][]>(
    settings.uiLevel === "advanced"
      ? [
          ["none", "—"],
          ["solid", "solid"],
          ["linear", "linear"],
          ["radial", "radial"],
        ]
      : [
          ["none", "—"],
          ["solid", "solid"],
        ],
  );

  // A paint control: solid colour (ColorInput), or a linear/radial gradient whose def lives
  // in the document (referenced by `url(#id)`). Gradient edits go through the editor's
  // gradient ops; the paint value (fill/stroke) is set via the passed callbacks.
  let {
    label,
    value,
    setPaint,
    previewPaint,
  }: {
    label: string;
    value: string;
    setPaint: (v: string | null) => void;
    previewPaint: (v: string | null) => void;
  } = $props();

  const gradId = $derived(value.startsWith("url(#") ? value.slice(5, -1) : null);
  const grad = $derived(gradId ? editor.gradientById(gradId) : null);
  // A gradient referenced by the fill but defined in the imported source `<defs>` (not nib's
  // editable model) — resolved from the render tree so we show its actual stops, not the raw
  // `url(#id)` string. Read-only; switching mode adopts its stops into an editable gradient.
  const importedGrad = $derived(
    gradId && !grad ? (editor.importedGradients.get(gradId) ?? null) : null,
  );
  // Whichever gradient backs the current value (editable model first, else imported source).
  const anyGrad = $derived(grad ?? importedGrad);
  const mode = $derived<"none" | "solid" | "linear" | "radial">(
    anyGrad ? anyGrad.kind : value === "none" || value === "" ? "none" : "solid",
  );

  // A stop as a CSS gradient colour-stop, honouring per-stop opacity (a color→transparent fade
  // reads as solid otherwise). `color-mix` applies alpha to any colour format (hex/named/rgb).
  function stopCss(s: { color: string; offset: number; opacity?: number }): string {
    const c =
      s.opacity != null && s.opacity < 1
        ? `color-mix(in srgb, ${s.color} ${Math.round(s.opacity * 100)}%, transparent)`
        : s.color;
    return `${c} ${Math.round(s.offset * 100)}%`;
  }
  // CSS (+ SVG) gradients clamp out-of-order stops — a stop whose offset is less than the previous
  // one collapses onto it — so a mid gradient added out of order vanishes. Sort by offset for the
  // preview (the model keeps insertion order so a stop's index stays stable while dragging).
  const stopsCss = $derived(
    anyGrad
      ? [...anyGrad.stops]
          .sort((a, b) => a.offset - b.offset)
          .map(stopCss)
          .join(", ")
      : "",
  );
  const previewBg = $derived(
    anyGrad
      ? anyGrad.kind === "radial"
        ? `radial-gradient(circle, ${stopsCss})`
        : `linear-gradient(90deg, ${stopsCss})`
      : "",
  );

  function setMode(m: "none" | "solid" | "linear" | "radial") {
    if (m === "none") return setPaint("none");
    if (m === "solid") {
      return setPaint(anyGrad?.stops[0]?.color ?? (value.startsWith("#") ? value : "#000000"));
    }
    if (grad) return editor.setGradient({ ...grad, kind: m });
    const id = `grad-${crypto.randomUUID().slice(0, 8)}`;
    // Adopt an imported gradient's stops into an editable model gradient; else a fresh two-stop.
    const stops = importedGrad
      ? importedGrad.stops.map((s) => ({
          offset: s.offset,
          color: s.color,
          ...(s.opacity != null ? { opacity: s.opacity } : {}),
        }))
      : [
          { offset: 0, color: value.startsWith("#") ? value : "#4b7bec" },
          { offset: 1, color: "#ffffff" },
        ];
    editor.setGradient({
      id,
      kind: m,
      stops,
      x1: 0,
      y1: 0.5,
      x2: 1,
      y2: 0.5,
      cx: 0.5,
      cy: 0.5,
      r: 0.5,
    });
    setPaint(`url(#${id})`);
  }

  function updateStop(i: number, patch: Partial<GradientStop>, preview = false) {
    if (!grad) return;
    const next = { ...grad, stops: grad.stops.map((s, j) => (j === i ? { ...s, ...patch } : s)) };
    if (preview) editor.previewGradient(next);
    else editor.setGradient(next);
  }

  function addStop() {
    if (!grad) return;
    const stops = [...grad.stops, { offset: 0.5, color: grad.stops[0].color }];
    editor.setGradient({ ...grad, stops });
    selStop = stops.length - 1;
  }

  function removeStop(i: number) {
    if (!grad || grad.stops.length <= 2) return;
    editor.setGradient({ ...grad, stops: grad.stops.filter((_, j) => j !== i) });
  }

  // Draggable stop markers along the gradient bar.
  let barEl = $state<HTMLDivElement>();
  let dragStop = $state<number | null>(null);
  let selStop = $state(0);

  $effect(() => {
    if (grad && selStop >= grad.stops.length) selStop = grad.stops.length - 1;
  });

  function stopDown(i: number, e: PointerEvent) {
    selStop = i;
    dragStop = i;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }
  function stopMove(e: PointerEvent) {
    if (dragStop === null || !barEl) return;
    const r = barEl.getBoundingClientRect();
    const t = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
    updateStop(dragStop, { offset: Math.round(t * 100) / 100 }, true);
  }
  function stopUp() {
    if (dragStop === null) return;
    dragStop = null;
    if (grad) editor.setGradient(grad); // commit the previewed offset as one undo step
  }

  // Linear direction as an angle in 0–360° ↔ the objectBoundingBox vector, centred on 0.5,0.5.
  const angle = $derived(
    grad && grad.kind === "linear"
      ? (Math.round((Math.atan2(grad.y2 - grad.y1, grad.x2 - grad.x1) * 180) / Math.PI) + 360) % 360
      : 0,
  );
  function setAngle(deg: number, preview = false) {
    if (!grad) return;
    const t = (deg * Math.PI) / 180;
    const c = Math.cos(t) / 2;
    const s = Math.sin(t) / 2;
    const next = { ...grad, x1: 0.5 - c, y1: 0.5 - s, x2: 0.5 + c, y2: 0.5 + s };
    if (preview) editor.previewGradient(next);
    else editor.setGradient(next);
  }

  // Radial gradient centre / radius (objectBoundingBox fractions).
  function setRadial(key: "cx" | "cy" | "r", v: number) {
    if (!grad || !Number.isFinite(v)) return;
    editor.setGradient({ ...grad, [key]: v });
  }
</script>

<div class="paint">
  <div class="ptop">
    <span class="plabel">{label}</span>
    <div class="pmode">
      {#each modes as [m, lbl] (m)}
        <button class:active={mode === m} onclick={() => setMode(m)}>{lbl}</button>
      {/each}
    </div>
  </div>

  {#if mode === "solid" || mode === "none"}
    <ColorInput
      label=""
      {value}
      editable
      oninput={(v) => previewPaint(v)}
      onchange={(v) => setPaint(v)}
    />
  {:else if grad}
    <div class="bar" bind:this={barEl} style:background={previewBg}>
      {#each grad.stops as s, i (i)}
        <button
          class="marker"
          class:sel={selStop === i}
          style:left="{s.offset * 100}%"
          style:background={s.color}
          onpointerdown={(e) => stopDown(i, e)}
          onpointermove={stopMove}
          onpointerup={stopUp}
          aria-label="gradient stop {i + 1}"
        ></button>
      {/each}
    </div>
    <div class="stoprow">
      <ColorInput
        label=""
        value={grad.stops[selStop]?.color ?? "#000000"}
        editable
        oninput={(v) => updateStop(selStop, { color: v }, true)}
        onchange={(v) => updateStop(selStop, { color: v })}
      />
      <button
        class="rm"
        disabled={grad.stops.length <= 2}
        aria-label="remove stop"
        onclick={() => removeStop(selStop)}>×</button
      >
    </div>
    <div class="grow">
      <button class="addstop" onclick={addStop}>+ stop</button>
    </div>
    {#if grad.kind === "linear"}
      <label class="slider">
        <span class="slbl">angle</span>
        <input
          type="range"
          min="0"
          max="360"
          value={angle}
          oninput={(e) => setAngle(Number(e.currentTarget.value), true)}
          onchange={(e) => setAngle(Number(e.currentTarget.value))}
        />
        <span class="sval">{angle}°</span>
      </label>
    {/if}
    {#if grad.kind === "radial"}
      <div class="radial">
        <label
          >cx <input
            type="number"
            min="0"
            max="1"
            step="0.05"
            value={grad.cx}
            onchange={(e) => setRadial("cx", Number(e.currentTarget.value))}
          /></label
        >
        <label
          >cy <input
            type="number"
            min="0"
            max="1"
            step="0.05"
            value={grad.cy}
            onchange={(e) => setRadial("cy", Number(e.currentTarget.value))}
          /></label
        >
        <label
          >r <input
            type="number"
            min="0"
            max="2"
            step="0.05"
            value={grad.r}
            onchange={(e) => setRadial("r", Number(e.currentTarget.value))}
          /></label
        >
      </div>
    {/if}
  {:else if importedGrad}
    <!-- gradient defined in the imported source <defs>: show its actual stops (read-only);
         pick linear/radial above to adopt it into an editable gradient -->
    <div
      class="bar readonly"
      style:background={previewBg}
      title="imported gradient #{gradId}"
    ></div>
    <p class="imported-note">imported gradient · pick a mode above to make it editable</p>
  {/if}
</div>

<style>
  .paint {
    margin-bottom: 6px;
  }

  .ptop {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }

  .plabel {
    width: 50px;
    flex: none;
    color: var(--halo-text-muted);
  }

  .pmode {
    display: flex;
    flex: 1;
    min-width: 0;
    gap: 4px;
  }

  .pmode button {
    flex: 1;
    min-width: 0;
    padding: 3px 0;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
    font-size: 11px;
  }

  .pmode button.active {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
    background: var(--halo-accent-soft);
  }

  .bar {
    position: relative;
    height: 18px;
    margin: 2px 2px 12px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
  }

  .bar.readonly {
    margin-bottom: 4px;
    cursor: default;
  }

  .imported-note {
    margin: 0 2px 6px;
    font-size: 11px;
    font-style: italic;
    color: var(--halo-text-muted);
  }

  .marker {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 12px;
    padding: 0;
    transform: translate(-50%, -50%);
    border: 2px solid #fff;
    border-radius: 50%;
    box-shadow: 0 0 0 1px var(--halo-text-muted);
    cursor: ew-resize;
    touch-action: none;
  }

  .marker.sel {
    box-shadow: 0 0 0 2px var(--halo-accent);
  }

  .stoprow {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .stoprow :global(.field) {
    flex: 1;
    min-width: 0;
    margin-bottom: 6px;
  }

  .rm {
    width: 22px;
    height: 22px;
    flex: none;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
    line-height: 1;
  }

  .rm:disabled {
    opacity: 0.3;
  }

  .grow {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 2px 0 6px;
  }

  .addstop {
    padding: 3px 10px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
    font-size: 12px;
  }

  /* angle: a slider row aligned to the panel's label column (matches Inspector .row) */
  .slider {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }

  .slbl {
    width: 50px;
    flex: none;
    color: var(--halo-text-muted);
  }

  .slider input[type="range"] {
    flex: 1;
    min-width: 0;
  }

  .sval {
    width: 34px;
    flex: none;
    text-align: right;
    color: var(--halo-text-muted);
    font-variant-numeric: tabular-nums;
  }

  .radial {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }

  .radial label {
    display: flex;
    flex: 1;
    min-width: 0;
    align-items: center;
    gap: 4px;
    color: var(--halo-text-muted);
    font-size: 12px;
  }

  .radial input {
    width: 100%;
    min-width: 0;
  }
</style>
