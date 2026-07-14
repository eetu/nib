<script lang="ts">
  import type { Component, Snippet } from "svelte";

  // A small top-bar dropdown: an icon-button trigger + a panel of `children`, with
  // click-outside to close. The reusable pattern for top-bar settings/context menus.
  let {
    icon,
    title,
    align = "left",
    children,
  }: {
    icon: Component;
    title: string;
    align?: "left" | "right";
    children: Snippet;
  } = $props();

  let open = $state(false);
  let anchor = $state<HTMLElement>();

  $effect(() => {
    if (!open || !anchor) return;
    const el = anchor;
    function onDown(e: PointerEvent) {
      if (!el.contains(e.target as Node)) open = false;
    }
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  });

  const Icon = $derived(icon);
</script>

<div class="pop" bind:this={anchor}>
  <button
    class="icon-btn"
    class:active={open}
    {title}
    aria-label={title}
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <Icon size={18} />
  </button>
  {#if open}
    <div class="panel" class:right={align === "right"} role="menu">
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .pop {
    position: relative;
    display: flex;
  }

  .panel {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 30;
    min-width: 180px;
    padding: 10px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    box-shadow: var(--halo-shadow, 0 6px 20px rgb(0 0 0 / 0.2));
  }

  .panel.right {
    left: auto;
    right: 0;
  }
</style>
