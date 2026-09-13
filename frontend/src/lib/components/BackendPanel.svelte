<script lang="ts">
  // Connected-mode projects surface (only mounted when the BACKEND flag is on). Lists the user's
  // backend projects, opens one (loading it + attaching the live-sync socket), and creates new ones.
  import { onMount } from "svelte";

  import { account } from "$lib/backend/account.svelte";
  import {
    createProject,
    deleteProject,
    listProjects,
    type ProjectMeta,
    renameProject,
  } from "$lib/backend/client";
  import { sync } from "$lib/backend/sync.svelte";
  import { openMenu } from "$lib/menu.svelte";
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

  // One call on connect: `/api/me` carries identity, the personal token, and the project list —
  // and, being session-authenticated, it's what bounces an unauthenticated visitor to the login.
  onMount(async () => {
    await account.load();
    if (account.me) projects = account.me.projects;
    else if (account.error) error = account.error;
  });

  // Loading-then-attaching lives in the sync store, not here: a reload has to do exactly the same
  // thing with no panel mounted, and two copies of it are two chances to drift.
  async function openProject(id: number) {
    error = null;
    try {
      await sync.open(id);
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
      await openProject(id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  // --- rename / delete -----------------------------------------------------
  // Same shape as the Inspector's LAYERS rows: double-click (or the context menu) renames in
  // place, right-click offers the rest. Delete is the one destructive action in this panel, so
  // it confirms by name and is styled as danger.
  let renaming = $state<number | null>(null);
  let renameValue = $state("");

  function startRename(p: ProjectMeta) {
    renaming = p.id;
    renameValue = p.name;
  }

  async function commitRename(id: number) {
    if (renaming !== id) return;
    const name = renameValue.trim();
    renaming = null;
    const current = projects.find((p) => p.id === id)?.name;
    if (!name || name === current) return; // nothing to do — don't spend a request
    error = null;
    try {
      await renameProject(id, name);
      await refresh();
      // The header shows the open document's name (seeded from the project name on open), so
      // keep it in step rather than leaving a stale title above a renamed project.
      if (sync.projectId === id) editor.fileName = name;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function remove(p: ProjectMeta) {
    if (!confirm(`Delete "${p.name}"? This can't be undone.`)) return;
    error = null;
    try {
      await deleteProject(p.id);
      // Stop streaming edits into a project that no longer exists. The document stays open on the
      // canvas as an unsaved local copy rather than vanishing under the user.
      if (sync.projectId === p.id) sync.disconnect();
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  /** The verbs for one project row — the same list the "⋯" button opens. */
  function projectMenu(e: MouseEvent, project: ProjectMeta) {
    openMenu(e, project.name, [
      { label: "open", run: () => void openProject(project.id) },
      { label: "rename", run: () => startRename(project) },
      { label: "delete", danger: true, run: () => void remove(project) },
    ]);
  }

  function autofocus(node: HTMLInputElement) {
    node.select();
  }
</script>

<aside class="backend">
  <!-- No title: the tab says "projects". The sync state stays, because it's the one thing here
       that changes without anyone touching it. -->
  <div class="head">
    <span class="status {sync.status}" title="live sync">{sync.status}</span>
  </div>
  {#if error}<p class="err">{error}</p>{/if}
  {#if sync.error}<p class="err">{sync.error}</p>{/if}
  <ul class="list">
    {#each projects as p (p.id)}
      <li>
        {#if renaming === p.id}
          <input
            class="rename"
            bind:value={renameValue}
            use:autofocus
            onblur={() => commitRename(p.id)}
            onkeydown={(e) => {
              if (e.key === "Enter") commitRename(p.id);
              else if (e.key === "Escape") renaming = null;
            }}
          />
        {:else}
          <button
            class="row"
            class:active={sync.projectId === p.id}
            onclick={() => openProject(p.id)}
            ondblclick={() => startRename(p)}
            oncontextmenu={(e) => projectMenu(e, p)}
            title="click to open · double-click to rename · right-click for more"
          >
            {p.name}
          </button>
        {/if}
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
  /* Tab content now — the panel around it owns the column, the background and the border. */
  .backend {
    display: flex;
    flex: 1;
    min-height: 0;
    flex-direction: column;
    padding: 8px;
    gap: 6px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 6px;
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

  .rename {
    width: 100%;
    margin: 1px 0;
    font-size: 13px;
  }

  .empty {
    padding: 6px 8px;
    color: var(--halo-text-muted);
    font-style: italic;
    font-size: 12px;
  }

  /* right-click context menu — matches the Inspector's LAYERS rows */

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
