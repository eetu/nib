<script lang="ts">
  import type { GradientStop } from "$lib/model/types";
  import { editor } from "$lib/stores/document.svelte";

  import ColorInput from "./ColorInput.svelte";

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
  const mode = $derived<"none" | "solid" | "linear" | "radial">(
    grad ? (grad.kind as "linear" | "radial") : value === "none" || value === "" ? "none" : "solid",
  );

  const stopsCss = $derived(
    grad ? grad.stops.map((s) => `${s.color} ${Math.round(s.offset * 100)}%`).join(", ") : "",
  );
  const previewBg = $derived(
    grad
      ? grad.kind === "radial"
        ? `radial-gradient(circle, ${stopsCss})`
        : `linear-gradient(90deg, ${stopsCss})`
      : "",
  );

  function setMode(m: "none" | "solid" | "linear" | "radial") {
    if (m === "none") return setPaint("none");
    if (m === "solid") {
      return setPaint(grad?.stops[0]?.color ?? (value.startsWith("#") ? value : "#000000"));
    }
    if (grad) return editor.setGradient({ ...grad, kind: m });
    const base = value.startsWith("#") ? value : "#4b7bec";
    const id = `grad-${crypto.randomUUID().slice(0, 8)}`;
    editor.setGradient({
      id,
      kind: m,
      stops: [
        { offset: 0, color: base },
        { offset: 1, color: "#ffffff" },
      ],
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
    const last = grad.stops[grad.stops.length - 1];
    editor.setGradient({ ...grad, stops: [...grad.stops, { offset: 1, color: last.color }] });
  }

  function removeStop(i: number) {
    if (!grad || grad.stops.length <= 2) return;
    editor.setGradient({ ...grad, stops: grad.stops.filter((_, j) => j !== i) });
  }

  // Linear direction as an angle (deg) ↔ the objectBoundingBox vector, centred on 0.5,0.5.
  const angle = $derived(
    grad && grad.kind === "linear"
      ? Math.round((Math.atan2(grad.y2 - grad.y1, grad.x2 - grad.x1) * 180) / Math.PI)
      : 0,
  );
  function setAngle(deg: number) {
    if (!grad) return;
    const t = (deg * Math.PI) / 180;
    const c = Math.cos(t) / 2;
    const s = Math.sin(t) / 2;
    editor.setGradient({ ...grad, x1: 0.5 - c, y1: 0.5 - s, x2: 0.5 + c, y2: 0.5 + s });
  }
</script>

<div class="paint">
  <div class="ptop">
    <span class="plabel">{label}</span>
    <div class="pmode">
      {#each [["none", "—"], ["solid", "solid"], ["linear", "linear"], ["radial", "radial"]] as [m, lbl] (m)}
        <button
          class:active={mode === m}
          onclick={() => setMode(m as "none" | "solid" | "linear" | "radial")}>{lbl}</button
        >
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
    <div class="preview" style:background={previewBg}></div>
    {#each grad.stops as s, i (i)}
      <div class="stop">
        <ColorInput
          label=""
          value={s.color}
          editable
          oninput={(v) => updateStop(i, { color: v }, true)}
          onchange={(v) => updateStop(i, { color: v })}
        />
        <input
          class="off"
          type="number"
          min="0"
          max="1"
          step="0.05"
          value={s.offset}
          onchange={(e) => updateStop(i, { offset: Number(e.currentTarget.value) })}
        />
        <button
          class="rm"
          disabled={grad.stops.length <= 2}
          aria-label="remove stop"
          onclick={() => removeStop(i)}>×</button
        >
      </div>
    {/each}
    <div class="grow">
      <button class="addstop" onclick={addStop}>+ stop</button>
      {#if grad.kind === "linear"}
        <label class="angle">
          angle
          <input
            type="number"
            value={angle}
            onchange={(e) => setAngle(Number(e.currentTarget.value))}
          />
        </label>
      {/if}
    </div>
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
    width: 44px;
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

  .preview {
    height: 14px;
    margin-bottom: 6px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
  }

  .stop {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .stop :global(.field) {
    flex: 1;
    margin-bottom: 4px;
  }

  .off {
    width: 52px;
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
    margin-top: 2px;
  }

  .addstop {
    padding: 3px 10px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
    font-size: 12px;
  }

  .angle {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--halo-text-muted);
    font-size: 12px;
  }

  .angle input {
    width: 52px;
  }
</style>
