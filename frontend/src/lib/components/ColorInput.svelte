<script lang="ts">
  import Pipette from "@lucide/svelte/icons/pipette";

  type Props = {
    /** Names the field — shown in the label column, and the accessible name of every control in
     *  it. A caller that draws the label itself passes `showLabel: false`; the name is still
     *  needed, or the controls announce as a bare "value" / "alpha". */
    label: string;
    showLabel?: boolean;
    value: string;
    editable: boolean;
    onchange: (value: string) => void;
    /** Live updates while the native picker / alpha slider is dragged (before commit). */
    oninput?: (value: string) => void;
    /** Arms the eyedropper for this paint, when there's one to arm. Rendered as a button in the
     *  row, because "what colour?" is answered by pointing at one — next to the field it fills
     *  in, not across the window in the tool rail. */
    onsample?: () => void;
    /** The eyedropper is armed for THIS paint — the next canvas click lands here. Lighting the
     *  button is what says which of fill/stroke is waiting, with two of them on screen. */
    armed?: boolean;
    /**
     * The SVG paint keywords this field offers besides a colour — values that aren't a colour at
     * all and so can't be reached through the swatch.
     *
     * A caller that already offers one elsewhere leaves it out: `PaintInput`'s segmented control
     * has a `—` chip for `none`, so its field only offers `currentColor`. Two controls for one
     * meaning, stacked on adjacent rows, read as a mistake even when both work.
     */
    keywords?: string[];
  };

  let {
    label,
    showLabel = true,
    value,
    editable,
    onchange,
    oninput,
    onsample,
    armed = false,
    keywords = ["none", "currentColor"],
  }: Props = $props();

  const isNone = $derived(value === "none" || value === "");

  // Parse the value into a 6-digit base colour + alpha (0..1). Handles #rgb / #rgba / #rrggbb /
  // #rrggbbaa; non-hex paints (currentColor / url(#…) / named) report `hex: false`.
  const parsed = $derived.by(() => {
    const m = value.trim().match(/^#([0-9a-f]{3,8})$/i);
    let h = m?.[1];
    if (!h || ![3, 4, 6, 8].includes(h.length)) return { base: "#888888", alpha: 1, hex: false };
    if (h.length === 3 || h.length === 4) h = [...h].map((c) => c + c).join(""); // expand shorthand
    const base = "#" + h.slice(0, 6);
    const alpha = h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1;
    return { base, alpha, hex: true };
  });
  const alphaPct = $derived(Math.round(parsed.alpha * 100));

  // Combine a 6-digit base + alpha into #rrggbb (opaque) or #rrggbbaa.
  function withAlpha(base: string, alpha: number): string {
    if (alpha >= 1) return base;
    const a = Math.round(Math.max(0, alpha) * 255)
      .toString(16)
      .padStart(2, "0");
    return base + a;
  }

  const CHECKER =
    "repeating-conic-gradient(var(--halo-off-bg) 0% 25%, transparent 0% 50%) 50% / 8px 8px";
  // The swatch: the colour (with its alpha) layered over a checker so transparency reads.
  const swatchBg = $derived(
    isNone
      ? undefined
      : parsed.hex
        ? `linear-gradient(${value}, ${value}), ${CHECKER}, var(--halo-bg-main)`
        : value === "currentColor"
          ? "currentColor"
          : undefined,
  );

  function pick(e: Event) {
    onchange(withAlpha((e.currentTarget as HTMLInputElement).value, parsed.alpha));
  }
  function live(e: Event) {
    oninput?.(withAlpha((e.currentTarget as HTMLInputElement).value, parsed.alpha));
  }
  function typeHex(e: Event) {
    onchange((e.currentTarget as HTMLInputElement).value.trim());
  }
  function alphaLive(e: Event) {
    oninput?.(withAlpha(parsed.base, Number((e.currentTarget as HTMLInputElement).value) / 100));
  }
  function alphaCommit(e: Event) {
    onchange(withAlpha(parsed.base, Number((e.currentTarget as HTMLInputElement).value) / 100));
  }

  // --- the keyword picker -------------------------------------------------
  // Which of the offered values is in force — a keyword, else an ordinary colour.
  const kind = $derived(keywords.find((k) => k === value) ?? "color");

  // The colour to come back to when a keyword is switched off. Remembered, because
  // keyword → colour → keyword otherwise loses the colour you had and hands back black.
  let lastColor = $state("#000000");
  $effect(() => {
    if (parsed.hex) lastColor = value;
  });

  function pickKind(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    onchange(v === "color" ? lastColor : v);
  }
</script>

<div class="field">
  <!-- The label column is drawn only when this field owns its label. A caller that draws its own
       (PaintInput, which puts it on the mode row above) used to get an empty 50px column for
       alignment — 56px of a 232px panel spent on nothing, which is why the row had no space for
       the eyedropper that belongs in it. -->
  {#if showLabel}<span class="lbl">{label}</span>{/if}
  <span class="swatch" class:none={isNone} style:background={swatchBg}>
    {#if editable}
      <input
        type="color"
        value={parsed.hex ? parsed.base : "#888888"}
        oninput={live}
        onchange={pick}
        disabled={isNone}
      />
    {/if}
  </span>
  <input
    class="hex"
    type="text"
    {value}
    onchange={typeHex}
    disabled={!editable}
    spellcheck="false"
  />
  {#if editable && onsample}
    <button
      class="sample"
      class:on={armed}
      title="eyedropper — click a shape to take its colour"
      aria-label="{label} eyedropper"
      onclick={onsample}
    >
      <Pipette size={13} />
    </button>
  {/if}
  {#if editable && keywords.length}
    <!-- A native <select> under a caret, the same overlay trick the swatch uses for its colour
         input: the keyboard, dismissal and placement come free, and the row keeps the footprint of
         the single button this replaced. The chosen value is already legible twice over — the
         swatch and the field both show it — so the trigger itself only needs to say "there are
         other values here". -->
    <span class="kw" class:keyword={kind !== "color"}>
      <span class="caret" aria-hidden="true">▾</span>
      <select
        aria-label="{label} value"
        title="paint value — a colour, or an SVG keyword"
        value={kind}
        onchange={pickKind}
      >
        <option value="color">colour</option>
        {#each keywords as k (k)}
          <option value={k}>{k}</option>
        {/each}
      </select>
    </span>
  {/if}
  {#if editable && parsed.hex}
    <!-- alpha slider — wraps to its own line under the colour row -->
    <label class="alpha">
      <span class="albl">alpha</span>
      <input
        type="range"
        min="0"
        max="100"
        aria-label="{label} alpha"
        value={alphaPct}
        oninput={alphaLive}
        onchange={alphaCommit}
      />
      <span class="apct">{alphaPct}%</span>
    </label>
  {/if}
</div>

<style>
  .field {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    row-gap: 4px;
    margin-bottom: 6px;
  }

  .lbl {
    width: 50px;
    color: var(--halo-text-muted);
  }

  .swatch {
    position: relative;
    width: 20px;
    height: 20px;
    flex: none;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    overflow: hidden;
  }

  /* transparent checker for "none" */
  .swatch.none {
    background: repeating-conic-gradient(var(--halo-off-bg) 0% 25%, transparent 0% 50%) 50% / 8px
      8px;
    background-color: var(--halo-bg-main);
  }

  .swatch input[type="color"] {
    position: absolute;
    inset: -4px;
    width: calc(100% + 8px);
    height: calc(100% + 8px);
    padding: 0;
    border: none;
    background: none;
    cursor: pointer;
    opacity: 0;
  }

  .hex {
    flex: 1;
    min-width: 0;
    font-size: 12px;
  }

  .sample {
    display: grid;
    width: 22px;
    height: 22px;
    flex: none;
    place-items: center;
    padding: 0;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
  }

  .sample:hover {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  /* armed: the next canvas click takes a colour into this field */
  .sample.on {
    border-color: var(--halo-accent);
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  .kw {
    position: relative;
    display: grid;
    width: 22px;
    height: 22px;
    flex: none;
    place-items: center;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
  }

  /* a keyword is in force, not a colour — the same accent the mode chips use */
  .kw.keyword {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  /* the invisible select is what's focused, so the ring has to be drawn on its wrapper */
  .kw:has(select:focus-visible) {
    outline: 2px solid var(--halo-accent);
    outline-offset: 1px;
  }

  .caret {
    font-size: 10px;
    line-height: 1;
    pointer-events: none;
  }

  .kw select {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    padding: 0;
    border: none;
    appearance: none;
    background: none;
    cursor: pointer;
    opacity: 0;
  }

  /* alpha row — forced onto its own line under the colour row */
  .alpha {
    display: flex;
    flex-basis: 100%;
    min-width: 0; /* let the wrapped line shrink to the field width so the % isn't pushed off */
    align-items: center;
    gap: 6px;
  }

  .albl {
    width: 50px;
    flex: none;
    color: var(--halo-text-muted);
    font-size: 11px;
  }

  .alpha input[type="range"] {
    flex: 1;
    min-width: 0;
  }

  .apct {
    width: 34px;
    text-align: right;
    color: var(--halo-text-muted);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
  }
</style>
