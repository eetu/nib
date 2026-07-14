<script lang="ts">
  type Props = {
    label: string;
    value: string;
    editable: boolean;
    onchange: (value: string) => void;
    /** Live updates while the native picker is open (before it closes / commits). */
    oninput?: (value: string) => void;
  };

  let { label, value, editable, onchange, oninput }: Props = $props();

  const HEX = /^#[0-9a-fA-F]{6}$/;
  const isHex = $derived(HEX.test(value));
  const isNone = $derived(value === "none" || value === "");

  function pick(e: Event) {
    onchange((e.currentTarget as HTMLInputElement).value);
  }

  function live(e: Event) {
    oninput?.((e.currentTarget as HTMLInputElement).value);
  }

  function typeHex(e: Event) {
    onchange((e.currentTarget as HTMLInputElement).value.trim());
  }
</script>

<div class="field">
  <span class="lbl">{label}</span>
  <span
    class="swatch"
    class:none={isNone}
    style:background={isHex ? value : value === "currentColor" ? "currentColor" : undefined}
  >
    {#if editable}
      <input
        type="color"
        value={isHex ? value : "#888888"}
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
      onclick={() => onchange(isNone ? "#000000" : "none")}>⁄</button
    >
  {/if}
</div>

<style>
  .field {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }

  .lbl {
    width: 44px;
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
</style>
