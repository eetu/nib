<script lang="ts">
  import { editor } from "$lib/stores/document.svelte";
  import { workspace } from "$lib/stores/workspace.svelte";
  import type { WorkspaceFile } from "$lib/workspace/fs";

  function open(file: WorkspaceFile) {
    if (file.name === editor.fileName) return;
    if (editor.dirty && !window.confirm(`Discard unsaved changes to ${editor.fileName}?`)) return;
    workspace.openFile(file);
  }
</script>

<nav class="filelist">
  <div class="head">
    <span class="dir" title={workspace.dirName ?? ""}>{workspace.dirName}</span>
    <span class="count">{workspace.files.length}</span>
  </div>
  <ul>
    {#each workspace.files as file (file.name)}
      {@const active = file.name === editor.fileName}
      <li>
        <button class:active onclick={() => open(file)} title={file.name}>
          <span class="name">{file.name}</span>
          {#if active && editor.dirty}<span class="dot" title="unsaved changes"></span>{/if}
        </button>
      </li>
    {/each}
  </ul>
</nav>

<style>
  .filelist {
    display: flex;
    flex-direction: column;
    width: 200px;
    background: var(--halo-bg-light);
    border-right: 1px solid var(--halo-border);
    overflow: hidden;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--halo-border);
    font-family: var(--halo-font-heading);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--halo-text-muted);
  }

  .dir {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .count {
    margin-left: auto;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 4px;
    overflow-y: auto;
  }

  li button {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 8px;
    border: none;
    border-radius: var(--halo-radius-pill);
    background: transparent;
    text-align: left;
    color: var(--halo-text-main);
  }

  li button:hover {
    background: var(--halo-bg-main);
  }

  li button.active {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot {
    margin-left: auto;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--halo-accent);
    flex: none;
  }
</style>
