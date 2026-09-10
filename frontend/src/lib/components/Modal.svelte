<script lang="ts">
  import type { Snippet } from "svelte";

  import { focusTrap } from "$lib/actions/focusTrap";
  import { closeMenu } from "$lib/menu.svelte";

  /**
   * The app's one dialog shell: veil, focus, Escape, Enter, geometry.
   *
   * Every dialog is built on this rather than rolling its own, because hand-rolled veils drift —
   * four copies of the same scrim CSS had already appeared here, each with its own idea of
   * padding and its own Escape handling.
   *
   * Escape cancels and Enter confirms, with the guard that matters: Enter on a focused button
   * does nothing extra, or a user tabbing to Cancel and pressing Enter would confirm instead.
   */
  let {
    open,
    title,
    onClose,
    onConfirm,
    children,
    labelledBy,
    align = "center",
  }: {
    open: boolean;
    /** Accessible name; also what a screen reader announces on open. */
    title: string;
    onClose: () => void;
    /** Enter confirms when given — omit for dialogs that only close. */
    onConfirm?: () => void;
    children: Snippet;
    /** Set when the dialog renders its own visible heading to name itself. */
    labelledBy?: string;
    /** Where the panel sits. `top` suits a launcher you type into; questions stay centred. */
    align?: "center" | "top";
  } = $props();

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key === "Enter" && onConfirm) {
      // A focused button owns Enter — let it click itself rather than confirming behind it.
      const el = document.activeElement;
      if (el instanceof HTMLButtonElement || el instanceof HTMLTextAreaElement) return;
      e.preventDefault();
      onConfirm();
    }
  }

  /** Focus the first field, else the dialog itself so Escape lands somewhere. */
  function focusFirst(node: HTMLElement) {
    const field = node.querySelector<HTMLElement>("input, textarea, select");
    (field ?? node).focus();
  }

  // A dialog is a mode: any open context menu belongs to the surface behind it.
  $effect(() => {
    if (open) closeMenu();
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="scrim"
    class:top={align === "top"}
    onclick={(e) => e.target === e.currentTarget && onClose()}
  >
    <div
      class="dialog halo-card"
      role="dialog"
      aria-modal="true"
      aria-label={labelledBy ? undefined : title}
      aria-labelledby={labelledBy}
      tabindex="-1"
      use:focusFirst
      use:focusTrap
      onkeydown={onKeydown}
    >
      {@render children()}
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgb(0 0 0 / 0.35);
  }

  .scrim.top {
    align-items: start;
    padding-top: 12vh;
  }

  .dialog {
    max-width: min(92vw, 520px);
    max-height: 86vh;
    padding: 16px;
    overflow-y: auto;
  }
</style>
