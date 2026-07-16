<script lang="ts">
  import { workspace } from "$lib/stores/workspace.svelte";

  let { open, onClose }: { open: boolean; onClose: () => void } = $props();

  const PLACEHOLDER =
    '<svg viewBox="0 0 100 100">\n  <path d="M10 50 C 30 10, 70 10, 90 50" />\n</svg>';

  let text = $state("");
  let field = $state<HTMLTextAreaElement | null>(null);

  $effect(() => {
    if (open) {
      workspace.error = null;
      field?.focus();
    }
  });

  function load() {
    if (!text.trim()) return;
    workspace.importText(text, "pasted.svg");
    if (!workspace.error) {
      text = "";
      onClose();
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) load();
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={(e) => e.target === e.currentTarget && onClose()}>
    <div
      class="dialog halo-card"
      role="dialog"
      aria-modal="true"
      aria-label="Paste SVG"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2>paste svg</h2>
      <textarea bind:this={field} bind:value={text} spellcheck="false" placeholder={PLACEHOLDER}
      ></textarea>
      {#if workspace.error}<p class="error">{workspace.error}</p>{/if}
      <div class="actions">
        <button class="ghost" onclick={onClose}>cancel</button>
        <button class="primary" onclick={load} disabled={!text.trim()}>load</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.4);
  }

  .dialog {
    width: min(560px, 92vw);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  h2 {
    margin: 0;
    font-family: var(--halo-font-heading);
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--halo-text-muted);
  }

  textarea {
    width: 100%;
    height: 220px;
    resize: vertical;
    font-family: ui-monospace, "SF Mono", monospace;
    font-size: 12.5px;
    line-height: 1.5;
  }

  .error {
    margin: 0;
    color: var(--halo-error);
    font-size: 12.5px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .actions button {
    height: 32px;
    padding: 0 16px;
    border-radius: var(--halo-radius);
    border: 1px solid var(--halo-border);
    background: var(--halo-bg-light);
    color: var(--halo-text-main);
  }

  .actions .primary {
    border-color: var(--halo-accent);
    background: var(--halo-accent);
    color: #fff;
  }
</style>
