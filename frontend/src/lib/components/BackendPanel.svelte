<script lang="ts">
  // Connected-mode projects surface (only mounted when the BACKEND flag is on). Lists the user's
  // backend projects, opens one (loading it + attaching the live-sync socket), and creates new ones.
  import { onMount } from "svelte";

  import { account } from "$lib/backend/account.svelte";
  import {
    createProject,
    deleteProject,
    getProject,
    listProjects,
    type ProjectMeta,
    renameProject,
  } from "$lib/backend/client";
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

  // One call on connect: `/api/me` carries identity, the personal token, and the project list —
  // and, being session-authenticated, it's what bounces an unauthenticated visitor to the login.
  onMount(async () => {
    await account.load();
    if (account.me) projects = account.me.projects;
    else if (account.error) error = account.error;
  });

  async function open(id: number) {
    error = null;
    try {
      const p = await getProject(id);
      // Load the native model (ids intact, so structural ops sync correctly); a not-yet-migrated
      // project has no model — fall back to importing its svg (the backend migrates it on connect).
      if (p.model) editor.loadModel(JSON.parse(p.model), p.name);
      else editor.load(p.svg, p.name);
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

  type Menu = { x: number; y: number; project: ProjectMeta };
  let menu = $state<Menu | null>(null);

  function openMenu(e: MouseEvent, project: ProjectMeta) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY, project };
  }

  // Keep the menu on screen and move focus into it, so Escape/arrows work (mirrors Inspector).
  function placeMenu(node: HTMLElement) {
    const r = node.getBoundingClientRect();
    const pad = 8;
    const dx = Math.min(0, window.innerWidth - pad - r.right);
    const dy = Math.min(0, window.innerHeight - pad - r.bottom);
    if (dx || dy) node.style.transform = `translate(${dx}px, ${dy}px)`;
    node.querySelector("button")?.focus();
  }

  function onMenuKeydown(e: KeyboardEvent) {
    const items = [
      ...(e.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("button"),
    ];
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "Escape") {
      menu = null;
      e.stopPropagation();
      e.preventDefault();
    } else if (e.key === "ArrowDown") {
      items[(i + 1) % items.length]?.focus();
      e.preventDefault();
    } else if (e.key === "ArrowUp") {
      items[(i - 1 + items.length) % items.length]?.focus();
      e.preventDefault();
    }
  }

  function autofocus(node: HTMLInputElement) {
    node.select();
  }
</script>

<aside class="backend">
  <div class="head">
    <span class="title">projects</span>
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
            onclick={() => open(p.id)}
            ondblclick={() => startRename(p)}
            oncontextmenu={(e) => openMenu(e, p)}
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

{#if menu}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="ctx-scrim"
    onclick={() => (menu = null)}
    oncontextmenu={(e) => {
      e.preventDefault();
      menu = null;
    }}
  ></div>
  <div
    class="ctx"
    style:left="{menu.x}px"
    style:top="{menu.y}px"
    role="menu"
    tabindex="-1"
    use:placeMenu
    onkeydown={onMenuKeydown}
  >
    <button
      role="menuitem"
      onclick={() => {
        const p = menu?.project;
        menu = null;
        if (p) startRename(p);
      }}>rename</button
    >
    <button
      role="menuitem"
      class="danger"
      onclick={() => {
        const p = menu?.project;
        menu = null;
        if (p) void remove(p);
      }}>delete</button
    >
  </div>
{/if}

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
  .ctx-scrim {
    position: fixed;
    inset: 0;
    z-index: 60;
  }

  .ctx {
    position: fixed;
    z-index: 61;
    min-width: 120px;
    padding: 4px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    box-shadow: var(--halo-shadow, 0 8px 24px rgb(0 0 0 / 0.25));
  }

  .ctx button {
    display: block;
    width: 100%;
    padding: 6px 10px;
    border: none;
    border-radius: var(--halo-radius);
    background: transparent;
    color: var(--halo-text-main);
    text-align: left;
    font-size: 13px;
  }

  .ctx button:hover {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  .ctx button.danger:hover {
    background: var(--halo-accent-soft);
    color: var(--halo-error);
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
