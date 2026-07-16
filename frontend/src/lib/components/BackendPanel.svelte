<script lang="ts">
  // Connected-mode projects surface (only mounted when the BACKEND flag is on). Lists the user's
  // backend projects, opens one (loading it + attaching the live-sync socket), and creates new ones.
  import { onMount } from "svelte";

  import { createProject, getProject, listProjects, type ProjectMeta } from "$lib/backend/client";
  import { sync } from "$lib/backend/sync.svelte";
  import { editor } from "$lib/stores/document.svelte";

  let projects = $state<ProjectMeta[]>([]);
  let error = $state<string | null>(null);

  async function refresh() {
    error = null;
    try {
      projects = await listProjects();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  onMount(() => void refresh());

  async function open(id: number) {
    error = null;
    try {
      const p = await getProject(id);
      editor.load(p.svg, p.name);
      sync.connect(id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function create() {
    const name = prompt("new project name:", "untitled");
    if (!name) return;
    error = null;
    try {
      const { id } = await createProject(name);
      await refresh();
      await open(id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
</script>

<aside class="backend">
  <div class="head">
    <span class="title">projects</span>
    <span class="status {sync.status}" title="live sync">{sync.status}</span>
  </div>
  {#if error}<p class="err">{error}</p>{/if}
  <ul class="list">
    {#each projects as p (p.id)}
      <li>
        <button class="row" class:active={sync.projectId === p.id} onclick={() => open(p.id)}>
          {p.name}
        </button>
      </li>
    {/each}
    {#if projects.length === 0 && !error}<li class="empty">no projects yet</li>{/if}
  </ul>
  <div class="actions">
    <button onclick={create}>new</button>
    <button onclick={refresh}>refresh</button>
  </div>
</aside>

<style>
  .backend {
    display: flex;
    flex-direction: column;
    width: 200px;
    flex: none;
    padding: 8px;
    gap: 6px;
    background: var(--halo-bg-light);
    border-right: 1px solid var(--halo-border);
    overflow-y: auto;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .title {
    font-family: var(--halo-font-heading);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--halo-text-muted);
  }

  .status {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--halo-text-muted);
  }

  .status.connected {
    color: var(--halo-connected, #2a9d3a);
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-height: 0;
  }

  .row {
    width: 100%;
    padding: 5px 8px;
    border: none;
    border-radius: var(--halo-radius);
    background: transparent;
    color: var(--halo-text-main);
    text-align: left;
    font-size: 13px;
  }

  .row:hover {
    background: var(--halo-bg-main);
  }

  .row.active {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  .empty {
    padding: 6px 8px;
    color: var(--halo-text-muted);
    font-style: italic;
    font-size: 12px;
  }

  .err {
    margin: 0;
    color: var(--halo-error);
    font-size: 12px;
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  .actions button {
    flex: 1;
    height: 28px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
    font-size: 12px;
  }

  .actions button:hover {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }
</style>
