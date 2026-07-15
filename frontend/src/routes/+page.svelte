<script lang="ts">
  import CommandPalette from "$lib/components/CommandPalette.svelte";
  import EditorCanvas from "$lib/components/EditorCanvas.svelte";
  import FileList from "$lib/components/FileList.svelte";
  import ImportDialog from "$lib/components/ImportDialog.svelte";
  import Inspector from "$lib/components/Inspector.svelte";
  import SettingsDialog from "$lib/components/SettingsDialog.svelte";
  import SourceView from "$lib/components/SourceView.svelte";
  import ToolRail from "$lib/components/ToolRail.svelte";
  import TopBar from "$lib/components/TopBar.svelte";
  import { editor } from "$lib/stores/document.svelte";
  import { interaction } from "$lib/stores/interaction.svelte";
  import { type ToolId, tools } from "$lib/stores/tool.svelte";
  import { workspace } from "$lib/stores/workspace.svelte";
  import { finishPen, getTool, toolShortcuts } from "$lib/tools";
  import { fitToView } from "$lib/view";

  let pasteOpen = $state(false);
  let settingsOpen = $state(false);
  let paletteOpen = $state(false);
  let dragging = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);

  const SAMPLE = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 160">
  <path d="M40 120 C 60 40, 120 40, 140 100 S 210 140, 205 70" fill="none" stroke="#f78f08" stroke-width="4" stroke-linecap="round"/>
</svg>`;

  // A tool switch runs the outgoing tool's cleanup (e.g. the pen finishing its path) and
  // clears any live snap aid — the one place for tool-change lifecycle. `tools.active` stays
  // the single source of truth for which tool is selected.
  let prevTool: ToolId = tools.active;
  $effect(() => {
    const active = tools.active;
    if (active !== prevTool) {
      getTool(prevTool).onDeactivate?.();
      interaction.clearDrag();
      editor.exitNodeEdit(); // a tool switch drops back to object mode
      prevTool = active;
    }
  });

  function typing(target: EventTarget | null): boolean {
    const el = target as HTMLElement | null;
    return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA");
  }

  function onKeydown(e: KeyboardEvent) {
    if (pasteOpen || settingsOpen || paletteOpen || typing(e.target)) return;

    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();

    if (mod) {
      if (k === "k") {
        e.preventDefault();
        paletteOpen = true;
      } else if (k === "z") {
        e.preventDefault();
        if (e.shiftKey) editor.redo();
        else editor.undo();
      } else if (k === "y") {
        e.preventDefault();
        editor.redo();
      } else if (k === "c") {
        e.preventDefault();
        editor.copySelected();
      } else if (k === "x") {
        e.preventDefault();
        editor.cutSelected();
      } else if (k === "v") {
        e.preventDefault();
        editor.paste();
      } else if (k === "d") {
        e.preventDefault();
        editor.duplicateSelected();
      }
      return;
    }

    // Escape cancels the current context but keeps the active tool (familiar editor
    // behaviour): finish an in-progress pen path → else leave node-edit mode → else deselect.
    if (e.key === "Escape") {
      if (interaction.penDrawing) finishPen();
      else if (editor.nodeEditIndex !== null) editor.exitNodeEdit();
      else editor.deselect();
      return;
    }
    if (e.key === "Enter" && tools.active === "pen") {
      finishPen();
      return;
    }

    if (e.key === "Delete" || e.key === "Backspace") {
      if (editor.selection) {
        e.preventDefault();
        editor.deleteNode(editor.selection);
      } else if (editor.selectedPaths.length > 0) {
        e.preventDefault();
        editor.deleteSelectedPaths();
      }
      return;
    }

    // Arrow keys nudge the selection (10 units with shift).
    const hasSel = editor.selection !== null || editor.selectedPaths.length > 0;
    if (hasSel && e.key.startsWith("Arrow")) {
      e.preventDefault();
      const step = e.shiftKey ? 10 : 1;
      if (e.key === "ArrowLeft") editor.nudge(-step, 0);
      else if (e.key === "ArrowRight") editor.nudge(step, 0);
      else if (e.key === "ArrowUp") editor.nudge(0, -step);
      else if (e.key === "ArrowDown") editor.nudge(0, step);
      return;
    }

    const tool = toolShortcuts[k];
    if (tool) tools.set(tool);
    if (e.key === "0") fitToView();
  }

  function isSvgFile(file: File): boolean {
    return file.type === "image/svg+xml" || file.name.toLowerCase().endsWith(".svg");
  }

  // Prefer the File System Access picker (Chromium → save-back); otherwise fall
  // back to a classic file input, which works in every browser.
  async function openFile() {
    if (workspace.filePickerSupported) await workspace.openSingleFile();
    else fileInput?.click();
  }

  function onFileInput(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (file) void workspace.importFile(file);
    input.value = ""; // let the same file be picked again
  }

  function onDragOver(e: DragEvent) {
    if (!e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    dragging = true;
  }

  function onDragLeave(e: DragEvent) {
    if (e.relatedTarget === null) dragging = false; // left the window
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    const file = e.dataTransfer?.files?.[0];
    if (file && isSvgFile(file)) void workspace.importFile(file);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<!-- Drop-to-load is a progressive enhancement; the keyboard-accessible "open
     file" button covers the same action, so the shell needs no ARIA role. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="app" ondragover={onDragOver} ondragleave={onDragLeave} ondrop={onDrop}>
  <TopBar
    onPaste={() => (pasteOpen = true)}
    onOpenFile={openFile}
    onSettings={() => (settingsOpen = true)}
  />

  <div class="body">
    {#if workspace.files.length}
      <FileList />
    {/if}
    <ToolRail />

    <div class="center">
      {#if editor.hasDocument}
        <EditorCanvas />
      {:else}
        <div class="empty">
          <div class="empty-card">
            <p class="lead">no svg loaded</p>
            <p class="hint">
              refine an LLM's paths: open or drop a file, open a folder, or paste markup.
            </p>
            <div class="empty-actions">
              {#if workspace.foldersSupported}
                <button onclick={() => workspace.openFolder()}>open folder</button>
              {/if}
              <button onclick={openFile}>open file</button>
              <button onclick={() => (pasteOpen = true)}>paste svg</button>
              <button
                onclick={() => {
                  editor.ensureBlank();
                  tools.set("pen");
                }}>new drawing</button
              >
              <button class="sample" onclick={() => workspace.importText(SAMPLE, "sample.svg")}
                >load sample</button
              >
            </div>
            {#if workspace.error}<p class="err">{workspace.error}</p>{/if}
          </div>
        </div>
      {/if}
      <SourceView />
    </div>

    <Inspector />
  </div>

  {#if dragging}
    <div class="dropzone">drop svg to load</div>
  {/if}
</div>

<ImportDialog open={pasteOpen} onClose={() => (pasteOpen = false)} />
<SettingsDialog open={settingsOpen} onClose={() => (settingsOpen = false)} />
<CommandPalette bind:open={paletteOpen} />

<input
  class="hidden-file"
  type="file"
  accept=".svg,image/svg+xml"
  bind:this={fileInput}
  onchange={onFileInput}
/>

<style>
  .app {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .hidden-file {
    display: none;
  }

  .dropzone {
    position: absolute;
    inset: 8px;
    z-index: 15;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    border: 2px dashed var(--halo-accent);
    border-radius: var(--halo-radius);
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
    font-family: var(--halo-font-heading);
    font-size: 14px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .center {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .empty {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    background: var(--halo-body);
  }

  .empty-card {
    text-align: center;
    max-width: 360px;
    padding: 24px;
  }

  .lead {
    margin: 0 0 4px;
    font-family: var(--halo-font-heading);
    font-size: 15px;
    color: var(--halo-text-main);
  }

  .hint {
    margin: 0 0 16px;
    color: var(--halo-text-muted);
  }

  .empty-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }

  .empty-actions button {
    height: 32px;
    padding: 0 14px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
  }

  .empty-actions button:hover {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .empty-actions .sample {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .err {
    margin: 12px 0 0;
    color: var(--halo-error);
  }
</style>
