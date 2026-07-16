<script lang="ts">
  // Settings overlay. Everything it shows is shared rune state read directly
  // (settings — theme + canvas backdrop), no props beyond open/onClose.
  import Monitor from "@lucide/svelte/icons/monitor";
  import Moon from "@lucide/svelte/icons/moon";
  import Sun from "@lucide/svelte/icons/sun";

  import { focusTrap } from "$lib/actions/focusTrap";
  import { setCanvasBg, setThemeMode, settings, setUiLevel } from "$lib/stores/settings.svelte";

  let { open, onClose }: { open: boolean; onClose: () => void } = $props();

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }

  // Focus the dialog on open so its Escape handler fires (the +page window handler is inert while
  // the dialog is up) — mirrors ImportDialog focusing its field.
  function autofocus(node: HTMLElement) {
    node.focus();
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={(e) => e.target === e.currentTarget && onClose()}>
    <div
      class="dialog halo-card"
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      tabindex="-1"
      use:autofocus
      use:focusTrap
      onkeydown={onKeydown}
    >
      <h2>settings</h2>

      <div class="setting">
        <span class="setting-label">interface</span>
        <div class="seg">
          <button class:on={settings.uiLevel === "basic"} onclick={() => setUiLevel("basic")}>
            basic
          </button>
          <button class:on={settings.uiLevel === "advanced"} onclick={() => setUiLevel("advanced")}>
            advanced
          </button>
        </div>
        <span class="setting-hint">
          basic shows just touch-up tools; advanced adds shapes, path craft, booleans, gradients +
          groups.
        </span>
      </div>

      <div class="setting">
        <span class="setting-label">theme</span>
        <div class="seg">
          <button class:on={settings.themeMode === "light"} onclick={() => setThemeMode("light")}>
            <Sun size={15} /> light
          </button>
          <button class:on={settings.themeMode === "dark"} onclick={() => setThemeMode("dark")}>
            <Moon size={15} /> dark
          </button>
          <button class:on={settings.themeMode === "auto"} onclick={() => setThemeMode("auto")}>
            <Monitor size={15} /> auto
          </button>
        </div>
      </div>

      <div class="setting">
        <span class="setting-label">canvas background</span>
        <div class="seg">
          <button class:on={settings.canvasBg === "checker"} onclick={() => setCanvasBg("checker")}>
            <span class="swatch checker"></span> checker
          </button>
          <button class:on={settings.canvasBg === "light"} onclick={() => setCanvasBg("light")}>
            <span class="swatch light"></span> light
          </button>
          <button class:on={settings.canvasBg === "dark"} onclick={() => setCanvasBg("dark")}>
            <span class="swatch dark"></span> dark
          </button>
        </div>
        <span class="setting-hint">
          the surface your svg previews against — independent of the ui theme.
        </span>
      </div>

      <div class="actions">
        <button class="primary" onclick={onClose}>done</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.4);
  }

  .dialog {
    width: min(440px, 92vw);
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  h2 {
    margin: 0;
    font-family: var(--halo-font-heading);
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--halo-text-muted);
  }

  .setting {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .setting-label {
    font-size: 12px;
    color: var(--halo-text-muted);
  }

  .setting-hint {
    font-size: 12px;
    color: var(--halo-text-muted);
  }

  .seg {
    display: flex;
    gap: 6px;
  }

  .seg button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    color: var(--halo-text-main);
    white-space: nowrap;
  }

  .seg button.on {
    color: #fff;
    background: var(--halo-accent);
    border-color: var(--halo-accent);
  }

  .swatch {
    width: 13px;
    height: 13px;
    border-radius: 3px;
    box-shadow: inset 0 0 0 1px rgba(128, 128, 128, 0.35);
  }

  .swatch.checker {
    background: repeating-conic-gradient(#c7c7c7 0% 25%, #ffffff 0% 50%) 50% / 7px 7px;
  }

  .swatch.light {
    background: #ffffff;
  }

  .swatch.dark {
    background: #14161a;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }

  .actions .primary {
    height: 32px;
    padding: 0 16px;
    border-radius: var(--halo-radius);
    border: 1px solid var(--halo-accent);
    background: var(--halo-accent);
    color: #fff;
  }
</style>
