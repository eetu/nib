<script lang="ts">
  /**
   * The NAVIGATOR — the left region, answering "what exists?".
   *
   * Projects, the document's layers, and the open folder's files are all lists you go to, pick
   * from, and leave. Stacked they competed for one column and pushed each other off a laptop
   * screen; as tabs they cost one row of chrome and each gets the whole height. The family layout
   * puts this on the left and the Inspector — "what am I working on?" — on the right.
   *
   * Layers is the default: it's the one tied to the document in front of you rather than to the
   * session around it.
   */
  import PanelLeft from "@lucide/svelte/icons/panel-left";

  import { BACKEND } from "$lib/backend/flag";
  import { loadState, saveState } from "$lib/persistence";
  import { editor } from "$lib/stores/document.svelte";
  import { workspace } from "$lib/stores/workspace.svelte";

  import FileList from "./FileList.svelte";
  import Layers from "./Layers.svelte";

  type TabId = "layers" | "projects" | "files";

  const OPEN_KEY = "nib:navOpen";
  const TAB_KEY = "nib:navTab";

  // Chrome layout is a global pref; which document is open must not change it.
  let open = $state(loadState<boolean>(OPEN_KEY) ?? true);
  let tab = $state<TabId>(loadState<TabId>(TAB_KEY) ?? "layers");

  const tabs = $derived(
    [
      { id: "layers" as const, label: "layers", show: true },
      { id: "projects" as const, label: "projects", show: BACKEND },
      { id: "files" as const, label: "files", show: workspace.files.length > 0 },
    ].filter((t) => t.show),
  );

  // A tab whose reason for existing goes away (the folder is closed) mustn't leave the panel
  // showing nothing — fall back rather than render an empty body.
  const active = $derived(tabs.some((t) => t.id === tab) ? tab : "layers");

  function pick(id: TabId) {
    tab = id;
    saveState(TAB_KEY, id);
  }
  function setOpen(next: boolean) {
    open = next;
    saveState(OPEN_KEY, next);
  }

  // The count a tab can show without being opened — the cheap part of "what's in there".
  const counts = $derived<Partial<Record<TabId, number>>>({
    files: workspace.files.length,
    layers: editor.doc?.paths.filter((p) => !p.deleted).length ?? 0,
  });
</script>

{#if !open}
  <!-- Closed: a rail, not nothing. The way back is where the panel was. -->
  <aside class="navrail">
    <button
      class="reopen"
      aria-label="show panel"
      aria-expanded="false"
      title="show panel"
      onclick={() => setOpen(true)}
    >
      <PanelLeft size={13} />
      <span class="vtitle">{active}</span>
    </button>
  </aside>
{:else}
  <aside class="nav">
    <div class="tabs" role="tablist">
      {#each tabs as t (t.id)}
        <button
          role="tab"
          class:on={active === t.id}
          aria-selected={active === t.id}
          onclick={() => pick(t.id)}
        >
          {t.label}{#if counts[t.id]}<span class="count">{counts[t.id]}</span>{/if}
        </button>
      {/each}
      <button
        class="collapse"
        aria-label="hide panel"
        aria-expanded="true"
        title="hide panel"
        onclick={() => setOpen(false)}>‹</button
      >
    </div>

    <div class="navbody">
      {#if active === "layers"}
        <Layers />
      {:else if active === "files"}
        <FileList />
      {:else if active === "projects" && BACKEND}
        <!-- Dynamically imported, so the standalone build ships none of the backend code. -->
        {#await import("./BackendPanel.svelte") then M}
          <M.default />
        {/await}
      {/if}
    </div>
  </aside>
{/if}

<style>
  .nav {
    display: flex;
    width: 232px;
    flex: none;
    flex-direction: column;
    background: var(--halo-bg-light);
    border-right: 1px solid var(--halo-border);
  }

  .tabs {
    display: flex;
    align-items: stretch;
    flex: none;
    border-bottom: 1px solid var(--halo-border);
  }

  .tabs button {
    display: flex;
    min-width: 0;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 7px 6px;
    border: none;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--halo-text-muted);
    font-family: var(--halo-font-heading);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .tabs button[role="tab"] {
    flex: 1;
  }

  .tabs button:hover {
    color: var(--halo-text-main);
  }

  /* The selected tab is marked by an underline, not a filled pill: it names the panel below it,
     so the mark has to read as "this one continues down there". */
  .tabs button.on {
    border-bottom-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .count {
    color: var(--halo-text-muted);
    font-family: var(--halo-font-main);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0;
  }

  .tabs button.on .count {
    color: var(--halo-accent);
  }

  .collapse {
    flex: none;
    padding: 0 8px;
  }

  /* The body scrolls; the tabs don't go with it. */
  .navbody {
    display: flex;
    flex: 1;
    min-height: 0;
    flex-direction: column;
    overflow-y: auto;
  }

  .navrail {
    display: flex;
    width: 26px;
    flex: none;
    justify-content: center;
    padding: 8px 0;
    background: var(--halo-bg-light);
    border-right: 1px solid var(--halo-border);
  }

  .reopen {
    display: flex;
    width: 100%;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border: none;
    background: transparent;
    color: var(--halo-text-muted);
  }

  .reopen:hover {
    color: var(--halo-accent);
  }

  .vtitle {
    writing-mode: vertical-rl;
    font-family: var(--halo-font-heading);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
</style>
