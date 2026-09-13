<script lang="ts">
  /**
   * The link between this canvas and a backend project — re-established on boot, and reported.
   *
   * Two jobs, deliberately in one component. A reload used to leave the canvas showing a project
   * (rehydrated from localStorage) with nothing attached to it: every edit went nowhere, and the
   * only cue was an unhighlighted row in a panel you might never open. Putting the reattach
   * inside that panel is what made it silent — it only ran if you happened to be on the projects
   * tab. So the component that ANSWERS "where do my edits go?" is the one that re-establishes the
   * answer on load, and it mounts with the header rather than with a tab.
   *
   * It reports all three states rather than only the bad one: if silence meant "fine", it would
   * also mean "this didn't render", and the state being reported is exactly the one you can't
   * tell by looking at the canvas.
   */
  import { onMount } from "svelte";

  import { account } from "$lib/backend/account.svelte";
  import { sync } from "$lib/backend/sync.svelte";
  import { editor } from "$lib/stores/document.svelte";

  onMount(async () => {
    if (sync.projectId !== null) return; // already attached — an HMR re-mount, not a fresh load
    await account.load();
    // Bounded by the caller's own list, so a project since deleted (or someone else's) is a no-op.
    await sync.restore(account.me?.projects.map((p) => p.id) ?? []);
  });

  const link = $derived.by(() => {
    if (sync.projectId === null)
      return {
        kind: "local",
        label: "local copy",
        hint: "not linked to a project — edits stay in this browser. open one from the projects tab",
      } as const;
    if (sync.status === "connected")
      return {
        kind: "live",
        label: "synced",
        hint: "edits stream to the project, and to anyone else editing it",
      } as const;
    return {
      kind: "down",
      label: sync.status,
      hint: "the project is open but its live connection is down — edits aren't reaching it",
    } as const;
  });
</script>

{#if editor.hasDocument}
  <span class="link {link.kind}" title={link.hint}>
    <span class="dot"></span>{link.label}
  </span>
{/if}

<style>
  /* Sits beside the document name, because it qualifies the name: this title, but whose copy? */
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--halo-text-muted);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentcolor;
  }

  .live {
    color: var(--halo-connected, #2a9d3a);
  }

  /* Attached but not reaching it — the one state that's actively going wrong. */
  .down {
    color: var(--halo-error);
  }
</style>
