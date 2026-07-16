<script lang="ts">
  import { focusTrap } from "$lib/actions/focusTrap";
  import { editor } from "$lib/stores/document.svelte";
  import { tools } from "$lib/stores/tool.svelte";
  import { workspace } from "$lib/stores/workspace.svelte";
  import { TOOL_GROUPS } from "$lib/tools";
  import { fitToView } from "$lib/view";
  import { downloadSvg } from "$lib/workspace/fs";

  // ⌘/Ctrl+K quick actions — a searchable list over the tool + editor action registry (the
  // same op surface the MCP server will expose). Keyboard-first: type, arrow, enter.
  let { open = $bindable(false) }: { open?: boolean } = $props();

  let query = $state("");
  let index = $state(0);

  type Command = { label: string; run: () => void; enabled?: () => boolean };

  const hasSelection = () => editor.selection !== null || editor.selectedPathIndex !== null;

  const commands = $derived<Command[]>([
    ...TOOL_GROUPS.flatMap((g) => g.tools).map((t) => ({
      label: `tool: ${t.label}`,
      run: () => tools.set(t.id),
    })),
    { label: "undo", run: () => editor.undo(), enabled: () => editor.canUndo },
    { label: "redo", run: () => editor.redo(), enabled: () => editor.canRedo },
    { label: "fit to view", run: () => fitToView(), enabled: () => editor.hasDocument },
    { label: "duplicate", run: () => editor.duplicateSelected(), enabled: hasSelection },
    { label: "copy", run: () => editor.copySelected(), enabled: hasSelection },
    { label: "paste", run: () => editor.paste(), enabled: () => editor.canPaste },
    { label: "cut", run: () => editor.cutSelected(), enabled: hasSelection },
    {
      label: "delete selection",
      run: () => {
        if (editor.selection) editor.deleteNode(editor.selection);
        else if (editor.selectedPaths.length > 0) editor.deleteSelectedPaths();
      },
      enabled: hasSelection,
    },
    { label: "deselect", run: () => editor.deselect() },
    {
      label: "group selection",
      run: () => editor.groupSelection(),
      enabled: () => editor.selectedPaths.length > 1,
    },
    {
      label: "ungroup",
      run: () => editor.ungroupSelection(),
      enabled: () => editor.selectedGroupUid !== null,
    },
    ...(
      [
        ["left", "align left"],
        ["hcenter", "align centers (horizontal)"],
        ["right", "align right"],
        ["top", "align top"],
        ["vcenter", "align centers (vertical)"],
        ["bottom", "align bottom"],
      ] as const
    ).map(([edge, label]) => ({
      label,
      run: () => editor.align(edge),
      enabled: () => editor.selectedPaths.length >= 2,
    })),
    {
      label: "distribute horizontally",
      run: () => editor.distribute("h"),
      enabled: () => editor.selectedPaths.length >= 3,
    },
    {
      label: "distribute vertically",
      run: () => editor.distribute("v"),
      enabled: () => editor.selectedPaths.length >= 3,
    },
    { label: "copy style", run: () => editor.copyStyle(), enabled: hasSelection },
    { label: "paste style", run: () => editor.pasteStyle(), enabled: () => editor.canPasteStyle },
    ...(["union", "subtract", "intersect", "exclude"] as const).map((op) => ({
      label: `boolean: ${op}`,
      run: () => editor.booleanOp(op),
      enabled: () => editor.selectedPaths.length >= 2,
    })),
    ...(["union", "subtract", "intersect", "exclude"] as const).map((op) => ({
      label: `live boolean: ${op} (non-destructive)`,
      run: () => editor.makeBooleanGroup(op),
      enabled: () => editor.selectedPaths.length >= 2,
    })),
    {
      label: "make compound path",
      run: () => editor.combinePaths(),
      enabled: () => editor.selectedPaths.length >= 2,
    },
    {
      label: "release compound path",
      run: () => editor.releaseCompound(),
      enabled: () => (editor.selectedPathElement?.subpaths.length ?? 0) > 1,
    },
    {
      label: "simplify path",
      run: () => editor.simplifyPath(),
      enabled: () => editor.selectedPathIndex !== null,
    },
    {
      label: "outline stroke",
      run: () => editor.outlineStroke(),
      enabled: () => editor.selectedPathIndex !== null,
    },
    {
      label: "offset path outward",
      run: () => editor.offsetPath(4),
      enabled: () => editor.selectedPathIndex !== null,
    },
    {
      label: "offset path inward",
      run: () => editor.offsetPath(-4),
      enabled: () => editor.selectedPathIndex !== null,
    },
    { label: "toggle snap to grid", run: () => (tools.gridEnabled = !tools.gridEnabled) },
    { label: "toggle snap to points", run: () => (tools.snapEnabled = !tools.snapEnabled) },
    { label: "toggle smart guides", run: () => (tools.guidesEnabled = !tools.guidesEnabled) },
    { label: "new drawing", run: () => workspace.newDocument() },
    { label: "save", run: () => void workspace.save(), enabled: () => editor.hasDocument },
    { label: "save as…", run: () => void workspace.saveAs(), enabled: () => editor.hasDocument },
    {
      label: "copy svg",
      run: () => void navigator.clipboard.writeText(editor.toSvg()),
      enabled: () => editor.hasDocument,
    },
    {
      label: "copy normalized svg",
      run: () => void navigator.clipboard.writeText(editor.toSvgNormalized()),
      enabled: () => editor.hasDocument,
    },
    {
      label: "export normalized copy",
      run: () => {
        const base = (editor.fileName ?? "nib.svg").replace(/\.svg$/i, "");
        downloadSvg(`${base}-normalized.svg`, editor.toSvgNormalized());
      },
      enabled: () => editor.hasDocument,
    },
  ]);

  const filtered = $derived(
    commands.filter((c) => c.label.toLowerCase().includes(query.toLowerCase().trim())),
  );

  $effect(() => {
    if (index >= filtered.length) index = Math.max(0, filtered.length - 1);
  });

  function close() {
    open = false;
    query = "";
    index = 0;
  }

  function run(c: Command) {
    if (c.enabled && !c.enabled()) return;
    c.run();
    close();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      close();
      e.preventDefault();
    } else if (e.key === "ArrowDown") {
      index = Math.min(index + 1, filtered.length - 1);
      e.preventDefault();
    } else if (e.key === "ArrowUp") {
      index = Math.max(index - 1, 0);
      e.preventDefault();
    } else if (e.key === "Enter") {
      const c = filtered[index];
      if (c) run(c);
      e.preventDefault();
    }
  }

  function autofocus(node: HTMLInputElement) {
    node.focus();
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={close}></div>
  <div class="palette" role="dialog" aria-label="Command palette" use:focusTrap>
    <input
      class="q"
      placeholder="run a command…"
      bind:value={query}
      oninput={() => (index = 0)}
      onkeydown={onKeydown}
      use:autofocus
      spellcheck="false"
    />
    <ul>
      {#each filtered as c, i (c.label)}
        {@const off = !!c.enabled && !c.enabled()}
        <li>
          <button
            class:active={i === index}
            class:off
            disabled={off}
            onmouseenter={() => (index = i)}
            onclick={() => run(c)}
          >
            {c.label}
          </button>
        </li>
      {/each}
      {#if filtered.length === 0}<li class="none">no matches</li>{/if}
    </ul>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgb(0 0 0 / 0.25);
  }

  .palette {
    position: fixed;
    z-index: 41;
    top: 15%;
    left: 50%;
    transform: translateX(-50%);
    width: min(460px, 90vw);
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    box-shadow: var(--halo-shadow, 0 12px 40px rgb(0 0 0 / 0.3));
  }

  .q {
    margin: 8px;
    padding: 8px 10px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-main);
    font-size: 14px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0 8px 8px;
    overflow-y: auto;
  }

  ul button {
    width: 100%;
    padding: 7px 10px;
    border: none;
    border-radius: var(--halo-radius);
    background: transparent;
    color: var(--halo-text-main);
    text-align: left;
    font-size: 13px;
  }

  ul button.active {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  ul button.off {
    opacity: 0.4;
  }

  .none {
    padding: 8px 10px;
    color: var(--halo-text-muted);
    font-style: italic;
  }
</style>
