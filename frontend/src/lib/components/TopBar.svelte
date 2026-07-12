<script lang="ts">
  import ClipboardPaste from "@lucide/svelte/icons/clipboard-paste";
  import Copy from "@lucide/svelte/icons/copy";
  import Download from "@lucide/svelte/icons/download";
  import FilePlus from "@lucide/svelte/icons/file-plus";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import Redo2 from "@lucide/svelte/icons/redo-2";
  import Save from "@lucide/svelte/icons/save";
  import Settings from "@lucide/svelte/icons/settings";
  import Undo2 from "@lucide/svelte/icons/undo-2";

  import { editor } from "$lib/stores/document.svelte";
  import { workspace } from "$lib/stores/workspace.svelte";

  import Wordmark from "./Wordmark.svelte";

  let {
    onPaste,
    onOpenFile,
    onSettings,
  }: { onPaste: () => void; onOpenFile: () => void; onSettings: () => void } = $props();

  let copied = $state(false);

  async function copySvg() {
    await navigator.clipboard.writeText(editor.toSvg());
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }
</script>

<header class="topbar">
  <Wordmark />

  <div class="group">
    {#if workspace.foldersSupported}
      <button
        class="icon-btn"
        title="Open folder"
        aria-label="Open folder"
        onclick={() => workspace.openFolder()}
      >
        <FolderOpen size={18} />
      </button>
    {/if}
    <button class="icon-btn" title="Open file" aria-label="Open file" onclick={onOpenFile}>
      <FilePlus size={18} />
    </button>
    <button class="icon-btn" title="Paste SVG" aria-label="Paste SVG" onclick={onPaste}>
      <ClipboardPaste size={18} />
    </button>
  </div>

  <div class="group">
    <button
      class="icon-btn"
      title="Undo (⌘Z)"
      aria-label="Undo"
      onclick={() => editor.undo()}
      disabled={!editor.canUndo}
    >
      <Undo2 size={18} />
    </button>
    <button
      class="icon-btn"
      title="Redo (⇧⌘Z)"
      aria-label="Redo"
      onclick={() => editor.redo()}
      disabled={!editor.canRedo}
    >
      <Redo2 size={18} />
    </button>
  </div>

  <div class="filename">
    {#if editor.hasDocument}
      <span class="name">{editor.fileName ?? "untitled.svg"}</span>
      {#if editor.dirty}<span class="dot" title="unsaved changes"></span>{/if}
    {/if}
  </div>

  <div class="group right">
    <button class="icon-btn" title="Settings" aria-label="Settings" onclick={onSettings}>
      <Settings size={18} />
    </button>
    <button
      class="icon-btn"
      title={copied ? "Copied" : "Copy SVG"}
      aria-label="Copy SVG"
      onclick={copySvg}
      disabled={!editor.hasDocument}
      class:ok={copied}
    >
      <Copy size={18} />
    </button>
    <button
      class="save"
      onclick={() => workspace.save()}
      disabled={!editor.hasDocument || workspace.busy}
    >
      {#if workspace.savesInPlace}<Save size={16} />Save{:else}<Download size={16} />Download{/if}
    </button>
  </div>
</header>

<style>
  .topbar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 6px 12px;
    height: 46px;
    background: var(--halo-bg-main);
    border-bottom: 1px solid var(--halo-border);
  }

  .group {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .group.right {
    margin-left: auto;
  }

  .filename {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--halo-text-muted);
    font-family: var(--halo-font-heading);
    font-size: 12px;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--halo-accent);
  }

  .icon-btn.ok {
    color: var(--halo-connected);
  }

  .save {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border: none;
    border-radius: var(--halo-radius);
    background: var(--halo-accent);
    color: #fff;
    font-weight: 500;
  }

  .save:hover:not(:disabled) {
    filter: brightness(1.05);
  }
</style>
