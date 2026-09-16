<script lang="ts">
  import CommandPalette from "$lib/components/CommandPalette.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import EditorCanvas from "$lib/components/EditorCanvas.svelte";
  import ImportDialog from "$lib/components/ImportDialog.svelte";
  import Inspector from "$lib/components/Inspector.svelte";
  import SettingsDialog from "$lib/components/SettingsDialog.svelte";
  import SidePanel from "$lib/components/SidePanel.svelte";
  import SourceView from "$lib/components/SourceView.svelte";
  import ToolRail from "$lib/components/ToolRail.svelte";
  import TopBar from "$lib/components/TopBar.svelte";
  import WelcomeDialog from "$lib/components/WelcomeDialog.svelte";
  import { canvas } from "$lib/stores/canvas.svelte";
  import { editor } from "$lib/stores/document.svelte";
  import { interaction } from "$lib/stores/interaction.svelte";
  import { settings } from "$lib/stores/settings.svelte";
  import { type ToolId, tools } from "$lib/stores/tool.svelte";
  import { workspace } from "$lib/stores/workspace.svelte";
  import { ADVANCED_TOOL_IDS, finishPen, getTool, toolShortcuts } from "$lib/tools";
  import { fitToView } from "$lib/view";

  let pasteOpen = $state(false);
  let settingsOpen = $state(false);
  let paletteOpen = $state(false);
  // First-run interface chooser — open until the user has ever picked a UI level (then Settings-only).
  const welcomeOpen = $derived(!settings.uiLevelChosen);
  let dragging = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);

  const SAMPLE = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 160">
  <path d="M40 120 C 60 40, 120 40, 140 100 S 210 140, 205 70" fill="none" stroke="#f78f08" stroke-width="4" stroke-linecap="round"/>
</svg>`;

  // A tool switch runs the outgoing tool's cleanup (e.g. the pen finishing its path) and
  // clears any live snap aid — the one place for tool-change lifecycle. `tools.active` stays
  // the single source of truth for which tool is selected.
  //
  // A BORROW is not a switch. Stepping onto a momentary tool (the eyedropper) and stepping back
  // off it must not run cleanup: the pen's cleanup finishes its path, so arming the eyedropper to
  // pick a colour mid-path used to end the path you were drawing.
  let prevTool: ToolId = tools.active;
  let prevHost: ToolId | null = tools.host;
  $effect(() => {
    const active = tools.active;
    const host = tools.host;
    if (active !== prevTool) {
      const borrowing = host === prevTool; // stepped onto a borrowed tool
      const releasing = prevHost === active; // stepped back onto its host
      if (!borrowing && !releasing) {
        getTool(prevTool).onDeactivate?.();
        interaction.clearDrag();
        editor.exitNodeEdit(); // a tool switch drops back to object mode
      }
      prevTool = active;
    }
    prevHost = host;
  });

  function typing(target: EventTarget | null): boolean {
    const el = target as HTMLElement | null;
    return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA");
  }

  function onKeydown(e: KeyboardEvent) {
    if (welcomeOpen || pasteOpen || settingsOpen || paletteOpen || typing(e.target)) return;

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
      } else if (k === "a") {
        e.preventDefault();
        editor.selectAll();
      } else if (k === "]" || k === "}") {
        // ⌘] forward · ⌘⇧] to front (Shift maps ] → } on US layouts, so accept both).
        e.preventDefault();
        editor.reorderSelection(true, e.shiftKey);
      } else if (k === "[" || k === "{") {
        // ⌘[ backward · ⌘⇧[ to back.
        e.preventDefault();
        editor.reorderSelection(false, e.shiftKey);
      } else if (k === "g" && settings.uiLevel === "advanced") {
        // Group / ungroup the selection — a pro feature, so inert in basic (touch-up) mode.
        e.preventDefault();
        if (e.shiftKey) editor.ungroupSelection();
        else editor.groupSelection();
      }
      return;
    }

    // Escape cancels the current context but keeps the active tool (familiar editor
    // behaviour). A drag/pan in flight? Cancel just that (the gesture machine owns it) and stop —
    // so one Esc does one thing, not also stepping out of node-edit / deselecting. Otherwise:
    // finish an in-progress pen path → else leave node-edit mode → else deselect.
    if (e.key === "Escape") {
      if (!canvas.idle) {
        canvas.send({ type: "CANCEL" });
        return;
      }
      // A borrowed tool is the most recently entered thing, so it's the first rung: Escape
      // un-arms the eyedropper and hands the pen back, rather than falling through and ending
      // the very path you were colouring.
      if (tools.host) tools.release();
      else if (interaction.penDrawing) finishPen();
      else if (editor.nodeEditIndex !== null) editor.exitNodeEdit();
      else editor.deselect();
      return;
    }
    if (e.key === "Enter" && tools.subject === "pen") {
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
      } else if (editor.selectedElementUid) {
        // A selected label/image/use — it carries no path index, so it needs the tree delete.
        e.preventDefault();
        editor.deleteTreeNode(editor.selectedElementUid);
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

    // Shift+H / Shift+V flip the selection horizontally / vertically (Figma-style), before the
    // bare-key tool shortcuts so plain "v" (select) / "h" still switch tools.
    if (e.shiftKey && (k === "h" || k === "v") && hasSel) {
      e.preventDefault();
      editor.flip(k === "h" ? "h" : "v");
      return;
    }

    const tool = toolShortcuts[k];
    // In basic (touch-up) mode, advanced-tool shortcuts are inert so you never land on an
    // off-screen tool.
    if (tool && (settings.uiLevel === "advanced" || !ADVANCED_TOOL_IDS.has(tool))) tools.set(tool);
    if (e.key === "0") fitToView();
  }

  /**
   * The browser own menu never appears over nib surfaces — a drawing tool invites right-click
   * constantly, and Back/Reload/Save-image-as is never the answer to "what can I do with this
   * shape?". Half-suppression is worse than none: a user who sometimes gets nib menu and
   * sometimes the browser one stops trying.
   *
   * Text fields are the exception, and the exception has to actually win — a number input inside a
   * row that carries its own menu must keep paste and spellcheck. This runs at the window, so it
   * sees the real target rather than whatever ancestor handled the event.
   */
  function onContextMenu(e: MouseEvent) {
    const el = e.target as HTMLElement | null;
    if (el?.closest("input, textarea, [contenteditable]")) return;
    e.preventDefault();
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

<svelte:window onkeydown={onKeydown} oncontextmenu={onContextMenu} />

<!-- Drop-to-load is a progressive enhancement; the keyboard-accessible "open
     file" button covers the same action, so the shell needs no ARIA role. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="app" ondragover={onDragOver} ondragleave={onDragLeave} ondrop={onDrop}>
  <TopBar
    onPaste={() => (pasteOpen = true)}
    onOpenFile={openFile}
    onSettings={() => (settingsOpen = true)}
  />

  <!-- The font picker for "convert to outlines" — how every browser without the Local Font Access
       API supplies a face, and the fallback when a family isn't installed. It lives in the DOM
       (visually hidden, not `display:none`, so the click still opens the dialog) rather than being
       created per use, so one element serves every call site. The accept list names MIME types
       as well as extensions, because Safari matches on type and greys out every file when it
       cannot map one. -->
  <input
    class="offscreen"
    type="file"
    accept="font/ttf,font/otf,font/collection,font/woff2,.ttf,.otf,.ttc,.woff2"
    data-font-picker
    aria-hidden="true"
    tabindex="-1"
  />

  <!-- A workspace error (failed save/open, permission denied, bad markup) — shown regardless of
       whether a document is loaded, so save-back failures aren't silent. Clears on the next op. -->
  {#if workspace.error}
    <div class="errbar" role="alert">
      <span>{workspace.error}</span>
      <button class="errclose" aria-label="dismiss error" onclick={() => workspace.dismissError()}
        >×</button
      >
    </div>
  {/if}

  <!-- A notice: something worth knowing that isn't a failure (a label outlined with a substitute
       font, say). Same bar, calmer colours, and `status` rather than `alert` — it doesn't interrupt
       a screen reader mid-task. -->
  {#if workspace.notice}
    <div class="errbar notice" role="status">
      <span>{workspace.notice}</span>
      <button class="errclose" aria-label="dismiss notice" onclick={() => workspace.dismissNotice()}
        >×</button
      >
    </div>
  {/if}

  <div class="body">
    <!-- Navigate: what exists (layers · projects · files), as tabs. -->
    <SidePanel />

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
          </div>
        </div>
      {/if}
      <SourceView />
    </div>

    <!-- Subject, then tools. The rail sits beside the panel that describes what it makes, which
         is the whole point of pairing them: pick a shape tool and its style is the next thing
         along, not across the window. -->
    <Inspector />
    <ToolRail />
  </div>

  {#if dragging}
    <div class="dropzone">drop svg to load</div>
  {/if}
</div>

<WelcomeDialog open={welcomeOpen} />
<ImportDialog open={pasteOpen} onClose={() => (pasteOpen = false)} />
<SettingsDialog open={settingsOpen} onClose={() => (settingsOpen = false)} />
<CommandPalette bind:open={paletteOpen} />

<!-- The app one context menu: mounted at the root so no panel can clip it. -->
<ContextMenu />

<!-- The app's one "are you sure?", for the few things undo can't undo. -->
<ConfirmDialog />

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

  /* Out of sight but still clickable by script — `display:none` would make the file dialog a no-op
     in some browsers. */
  .offscreen {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
  }

  .errbar.notice {
    background: var(--halo-accent-soft);
    color: var(--halo-text-main);
    border-bottom: 1px solid var(--halo-border);
  }

  .errbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 12px;
    background: var(--halo-error-soft, rgb(220 50 50 / 0.12));
    color: var(--halo-error);
    border-bottom: 1px solid var(--halo-error);
    font-size: 13px;
  }

  .errclose {
    flex: none;
    border: none;
    background: none;
    color: inherit;
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
  }
</style>
