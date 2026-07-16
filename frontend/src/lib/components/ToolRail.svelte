<script lang="ts">
  import Grid3x3 from "@lucide/svelte/icons/grid-3x3";
  import Scan from "@lucide/svelte/icons/scan";

  import { editor } from "$lib/stores/document.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { type ToolId, tools } from "$lib/stores/tool.svelte";
  import { TOOL_GROUPS, type ToolGroup } from "$lib/tools";
  import { fitToView } from "$lib/view";

  // Which flyout group's popup is open (by group name), if any.
  let openGroup = $state<string | null>(null);

  // Basic (touch-up) mode hides advanced groups (shape primitives); the full surface shows
  // in advanced mode. The engine keeps every tool regardless.
  const groups = $derived(
    TOOL_GROUPS.filter((g) => settings.uiLevel === "advanced" || !g.advanced),
  );

  // A flyout group only collapses into a popup once it holds more than one tool — so a
  // single-tool "shapes" group renders as a plain button now, and gains the flyout for free
  // when rect/polygon/star land.
  function isFlyout(g: ToolGroup): boolean {
    return !!g.flyout && g.tools.length > 1;
  }

  function shortcutLabel(shortcut?: string): string {
    return shortcut ? ` (${shortcut.toUpperCase()})` : "";
  }

  function pick(id: ToolId): void {
    tools.set(id);
    openGroup = null;
  }

  // Close an open flyout when clicking anywhere outside a flyout anchor.
  $effect(() => {
    if (!openGroup) return;
    function onDown(e: PointerEvent) {
      if (!(e.target as HTMLElement).closest(".flyout-anchor")) openGroup = null;
    }
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  });
</script>

<div class="rail">
  {#each groups as group, gi (group.name)}
    {#if gi > 0}<div class="sep"></div>{/if}
    {#if isFlyout(group)}
      {@const shown = group.tools.find((t) => t.id === tools.active) ?? group.tools[0]}
      <div class="flyout-anchor">
        <button
          class="icon-btn flyout-btn"
          class:active={group.tools.some((t) => t.id === tools.active)}
          title={shown.label + shortcutLabel(shown.shortcut)}
          aria-label={`${group.name} tools`}
          aria-haspopup="menu"
          aria-expanded={openGroup === group.name}
          onclick={() => (openGroup = openGroup === group.name ? null : group.name)}
        >
          <shown.icon size={18} />
          <span class="flyout-mark"></span>
        </button>
        {#if openGroup === group.name}
          <div class="flyout" role="menu">
            {#each group.tools as t (t.id)}
              <button
                class="flyout-item"
                class:active={tools.active === t.id}
                role="menuitem"
                onclick={() => pick(t.id)}
              >
                <t.icon size={16} />
                <span>{t.label}{shortcutLabel(t.shortcut)}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      {#each group.tools as t (t.id)}
        <button
          class="icon-btn"
          class:active={tools.active === t.id}
          title={t.label + shortcutLabel(t.shortcut)}
          aria-label={t.label}
          aria-pressed={tools.active === t.id}
          onclick={() => tools.set(t.id)}
        >
          <t.icon size={18} />
        </button>
      {/each}
    {/if}
  {/each}

  <div class="sep"></div>

  <button
    class="icon-btn"
    class:active={tools.gridEnabled}
    title="snap to grid"
    aria-label="snap to grid"
    aria-pressed={tools.gridEnabled}
    onclick={() => (tools.gridEnabled = !tools.gridEnabled)}
  >
    <Grid3x3 size={18} />
  </button>

  <div class="spacer"></div>

  <button
    class="icon-btn"
    title="fit to view (0)"
    aria-label="fit to view"
    onclick={fitToView}
    disabled={!editor.hasDocument}
  >
    <Scan size={18} />
  </button>
</div>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 8px 6px;
    background: var(--halo-bg-light);
    border-right: 1px solid var(--halo-border);
  }

  .sep {
    width: 20px;
    height: 1px;
    margin: 4px 0;
    background: var(--halo-border);
  }

  .spacer {
    flex: 1;
  }

  /* Flyout: a single rail slot that pops a menu of its group's tools to the right. */
  .flyout-anchor {
    position: relative;
    display: flex;
  }

  /* Corner mark hinting the button expands into a flyout. */
  .flyout-mark {
    position: absolute;
    right: 3px;
    bottom: 3px;
    width: 0;
    height: 0;
    border-left: 4px solid transparent;
    border-bottom: 4px solid var(--halo-text-muted);
  }

  .flyout {
    position: absolute;
    left: calc(100% + 6px);
    top: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    box-shadow: var(--halo-shadow, 0 4px 16px rgb(0 0 0 / 0.18));
    white-space: nowrap;
  }

  .flyout-item {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px 6px 8px;
    border: none;
    border-radius: var(--halo-radius);
    background: transparent;
    color: var(--halo-text-main);
    text-align: left;
  }

  .flyout-item:hover {
    background: var(--halo-bg-main);
  }

  .flyout-item.active {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }
</style>
