<script lang="ts">
  import Circle from "@lucide/svelte/icons/circle";
  import Eraser from "@lucide/svelte/icons/eraser";
  import Grid3x3 from "@lucide/svelte/icons/grid-3x3";
  import MousePointer2 from "@lucide/svelte/icons/mouse-pointer-2";
  import PenTool from "@lucide/svelte/icons/pen-tool";
  import Plus from "@lucide/svelte/icons/plus";
  import Scan from "@lucide/svelte/icons/scan";
  import type { Component } from "svelte";

  import { editor } from "$lib/stores/document.svelte";
  import { type ToolId, tools } from "$lib/stores/tool.svelte";
  import { viewport } from "$lib/stores/viewport.svelte";

  type Item = { id: ToolId; label: string; icon: Component };

  // Grouped so the rail stays scannable as tools grow: select · create · nodes.
  // New shape tools slot into the "create" group; new node ops into "nodes".
  const groups: Item[][] = [
    [{ id: "select", label: "Select & move (V)", icon: MousePointer2 }],
    [
      { id: "pen", label: "Draw path / pen (P)", icon: PenTool },
      { id: "circle", label: "Circle (C)", icon: Circle },
    ],
    [
      { id: "add-node", label: "Add node (A)", icon: Plus },
      { id: "delete-node", label: "Delete node (D)", icon: Eraser },
    ],
  ];

  function fit() {
    if (editor.doc) viewport.fitDocument(editor.doc.viewBox);
  }
</script>

<div class="rail">
  {#each groups as group, gi (gi)}
    {#if gi > 0}<div class="sep"></div>{/if}
    {#each group as item (item.id)}
      <button
        class="icon-btn"
        class:active={tools.active === item.id}
        title={item.label}
        aria-label={item.label}
        aria-pressed={tools.active === item.id}
        onclick={() => tools.set(item.id)}
      >
        <item.icon size={18} />
      </button>
    {/each}
  {/each}

  <div class="sep"></div>

  <button
    class="icon-btn"
    class:active={tools.gridEnabled}
    title="Toggle grid"
    aria-label="Toggle grid"
    aria-pressed={tools.gridEnabled}
    onclick={() => (tools.gridEnabled = !tools.gridEnabled)}
  >
    <Grid3x3 size={18} />
  </button>

  <div class="spacer"></div>

  <button
    class="icon-btn"
    title="Fit to view"
    aria-label="Fit to view"
    onclick={fit}
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
</style>
