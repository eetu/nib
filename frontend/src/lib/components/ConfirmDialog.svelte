<script lang="ts">
  import { answerConfirm, pendingConfirm } from "$lib/confirm.svelte";

  import Modal from "./Modal.svelte";

  // The app's one "are you sure?", mounted at the root and driven by `askConfirm`. Built on Modal
  // like every other dialog, so Escape cancels and Enter confirms with the button guard.
  const ask = $derived(pendingConfirm());
</script>

<Modal
  open={!!ask}
  title={ask?.title ?? ""}
  labelledBy="confirm-title"
  onClose={() => answerConfirm(false)}
  onConfirm={() => answerConfirm(true)}
>
  {#if ask}
    <h2 id="confirm-title">{ask.title}</h2>
    <p class="body">{ask.body}</p>
    <div class="actions">
      <button class="ghost" onclick={() => answerConfirm(false)}>cancel</button>
      <button class="primary" class:danger={ask.danger} onclick={() => answerConfirm(true)}>
        {ask.confirmLabel}
      </button>
    </div>
  {/if}
</Modal>

<style>
  h2 {
    margin: 0 0 8px;
    font-size: 15px;
  }

  .body {
    margin: 0 0 16px;
    color: var(--halo-text-muted);
    font-size: 13px;
    max-width: 42ch;
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }

  .actions button {
    height: 32px;
    padding: 0 14px;
    border-radius: var(--halo-radius);
    border: 1px solid var(--halo-border);
    background: transparent;
    color: var(--halo-text-main);
    font-size: 13px;
  }

  .actions .primary {
    border-color: var(--halo-accent);
    background: var(--halo-accent);
    color: #fff;
  }

  .actions .primary.danger {
    border-color: var(--halo-error);
    background: var(--halo-error);
  }
</style>
