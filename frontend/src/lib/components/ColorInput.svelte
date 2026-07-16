<script lang="ts">
  type Props = {
    label: string;
    value: string;
    editable: boolean;
    onchange: (value: string) => void;
    /** Live updates while the native picker / alpha slider is dragged (before commit). */
    oninput?: (value: string) => void;
  };

  let { label, value, editable, onchange, oninput }: Props = $props();

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
</script>

<div class="field">
  {#if label}<span class="lbl">{label}</span>{/if}
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
  {#if editable}
    <button
      class="none-btn"
      class:active={isNone}
      title="none"
      aria-label="{label} none"
      onclick={() => onchange(isNone ? "#000000" : "none")}>—</button
    >
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

  .none-btn {
    width: 22px;
    height: 22px;
    flex: none;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-main);
    color: var(--halo-text-muted);
    line-height: 1;
  }

  .none-btn.active {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  /* alpha row — forced onto its own line under the colour row */
  .alpha {
    display: flex;
    flex-basis: 100%;
    align-items: center;
    gap: 6px;
  }

  .albl {
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
