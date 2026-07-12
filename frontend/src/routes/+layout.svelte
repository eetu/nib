<script lang="ts">
  import "$lib/styles/halo.css";

  import type { Snippet } from "svelte";

  import { settings } from "$lib/stores/settings.svelte";

  let { children }: { children: Snippet } = $props();

  // Resolve the chosen theme mode to an effective 'light'/'dark' and apply it as
  // data-theme on <html> (the halo tokens key off it). Only `auto` follows the
  // system, re-resolving live when the OS appearance flips. The inline script in
  // app.html does the same pre-paint, so this only handles later changes.
  $effect(() => {
    const mode = settings.themeMode;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const eff = mode === "auto" ? (mq.matches ? "dark" : "light") : mode;
      document.documentElement.dataset.theme = eff;
      document
        .querySelector('meta[name="theme-color"]')
        ?.setAttribute("content", eff === "dark" ? "#0f0f0f" : "#ffffff");
    };
    apply();
    if (mode === "auto") {
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  });
</script>

{@render children()}

<style>
  /* App-wide element/utility base — one place, per coding-style:svelte. Scoped
     styles don't cross component boundaries, so themed control bases live here
     as :global() rules; components override on specificity. */
  :global(*),
  :global(*::before),
  :global(*::after) {
    box-sizing: border-box;
  }

  :global(html),
  :global(body) {
    height: 100%;
    margin: 0;
  }

  :global(body) {
    background: var(--halo-body);
    color: var(--halo-text-main);
    font-family: var(--halo-font-body);
    font-size: 14px;
    line-height: 1.4;
    -webkit-font-smoothing: antialiased;
    overflow: hidden; /* the editor owns the viewport; panels scroll internally */
  }

  :global(button) {
    font-family: inherit;
    font-size: inherit;
    color: inherit;
    cursor: pointer;
  }

  :global(button:disabled) {
    cursor: default;
    opacity: 0.4;
  }

  /* Square icon button — the tool rail + top bar share this base. */
  :global(.icon-btn) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    padding: 0;
    border: none;
    border-radius: var(--halo-radius);
    background: transparent;
    color: var(--halo-text-muted);
    transition:
      background var(--halo-d-fast),
      color var(--halo-d-fast);
  }

  :global(.icon-btn:hover:not(:disabled)) {
    background: var(--halo-bg-light);
    color: var(--halo-text-main);
  }

  :global(.icon-btn.active) {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  :global(input),
  :global(select) {
    font-family: inherit;
    font-size: inherit;
    color: var(--halo-text-main);
    background: var(--halo-bg-light);
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    padding: 3px 6px;
  }

  :global(input:focus),
  :global(select:focus) {
    outline: none;
    border-color: var(--halo-accent);
  }
</style>
