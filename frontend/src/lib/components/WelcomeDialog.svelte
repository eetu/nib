<script lang="ts">
  // First-run interface chooser. Shown once (when the user has never explicitly picked a UI
  // level); after either choice it never returns — the same toggle then lives in Settings.
  // Basic declutters to touch-up tools; advanced is the full pro surface (and the default, so
  // Escape / scrim-click just keeps it). Voice: terse, lowercase, geometry-forward.
  import { focusTrap } from "$lib/actions/focusTrap";
  import { settings, setUiLevel, type UiLevel } from "$lib/stores/settings.svelte";

  let { open }: { open: boolean } = $props();

  function choose(level: UiLevel) {
    setUiLevel(level);
  }

  // Escape or scrim = keep the current default (advanced) but stop asking — persisting the level
  // is what marks the choice made, so re-selecting it retires the chooser too.
  function dismiss() {
    setUiLevel(settings.uiLevel);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") dismiss();
  }

  function autofocus(node: HTMLElement) {
    node.focus();
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={(e) => e.target === e.currentTarget && dismiss()}>
    <div
      class="dialog halo-card"
      role="dialog"
      aria-modal="true"
      aria-label="Choose your workspace"
      tabindex="-1"
      use:autofocus
      use:focusTrap
      onkeydown={onKeydown}
    >
      <div class="head">
        <span class="brand">nib<span class="period">.</span></span>
        <h2>choose your workspace</h2>
        <p class="sub">you can switch anytime in settings.</p>
      </div>

      <div class="choices">
        <button class="choice" onclick={() => choose("basic")}>
          <span class="name">basic</span>
          <span class="desc">touch-up essentials</span>
          <span class="tools">select · node-edit · style · save</span>
        </button>
        <button class="choice reco" onclick={() => choose("advanced")}>
          <span class="badge">recommended</span>
          <span class="name">advanced</span>
          <span class="desc">the full toolset</span>
          <span class="tools">shapes · path craft · booleans · gradients · groups</span>
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.4);
  }

  .dialog {
    width: min(520px, 94vw);
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .head {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .brand {
    font-family: var(--halo-font-heading);
    font-weight: 600;
    font-size: 18px;
    letter-spacing: -0.04em;
    color: var(--halo-text-main);
  }

  .brand .period {
    color: var(--halo-accent);
  }

  h2 {
    margin: 6px 0 0;
    font-family: var(--halo-font-heading);
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--halo-text-muted);
  }

  .sub {
    margin: 0;
    font-size: 12px;
    color: var(--halo-text-muted);
  }

  .choices {
    display: flex;
    gap: 10px;
  }

  .choice {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 16px 14px;
    text-align: left;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    color: var(--halo-text-main);
  }

  .choice:hover {
    border-color: var(--halo-accent);
    background: var(--halo-bg-main);
  }

  .choice.reco {
    border-color: var(--halo-accent);
  }

  .badge {
    position: absolute;
    top: 10px;
    right: 10px;
    padding: 2px 6px;
    border-radius: 999px;
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .name {
    font-family: var(--halo-font-heading);
    font-size: 15px;
    font-weight: 600;
  }

  .desc {
    font-size: 12px;
    color: var(--halo-text-main);
  }

  .tools {
    margin-top: 4px;
    font-size: 11px;
    color: var(--halo-text-muted);
  }

  @media (max-width: 480px) {
    .choices {
      flex-direction: column;
    }
  }
</style>
