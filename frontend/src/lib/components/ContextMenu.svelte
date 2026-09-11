<script lang="ts">
  import { activeMenu, closeMenu } from "$lib/menu.svelte";
  import { editor } from "$lib/stores/document.svelte";

  // The app's only context menu, mounted once at the root. Every surface with verbs for a thing
  // calls `openMenu`; nothing renders a menu of its own. Being at the root is what keeps a menu
  // opened near the bottom of a scrolling panel from being clipped by it.
  const menu = $derived(activeMenu());

  /** Keep the menu on screen: nudge it back inside the viewport, then focus its first item. */
  function place(node: HTMLElement) {
    const r = node.getBoundingClientRect();
    const pad = 8;
    const dx = Math.min(0, window.innerWidth - pad - r.right);
    const dy = Math.min(0, window.innerHeight - pad - r.bottom);
    if (dx || dy) node.style.transform = `translate(${dx}px, ${dy}px)`;
    node.querySelector<HTMLButtonElement>("button:not([disabled])")?.focus();
  }

  function items(root: HTMLElement): HTMLButtonElement[] {
    return [...root.querySelectorAll<HTMLButtonElement>("button:not([disabled])")];
  }

  function onMenuKeydown(e: KeyboardEvent) {
    const list = items(e.currentTarget as HTMLElement);
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown") {
      list[(i + 1) % list.length]?.focus();
      e.preventDefault();
    } else if (e.key === "ArrowUp") {
      list[(i - 1 + list.length) % list.length]?.focus();
      e.preventDefault();
    }
  }

  // No scrim: a click outside closes the menu, but the event still reaches whatever it landed on.
  // That's what lets a right-click on a second shape *move* the menu to it — with a scrim in the
  // way, the first click only dismissed, and every "open the menu on that one instead" took two.
  $effect(() => {
    if (!menu) return;

    const outside = (e: Event) => {
      if (!(e.target as HTMLElement | null)?.closest("[role='menu']")) closeMenu();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      closeMenu();
      // The menu is the outermost rung of the Escape ladder — one press closes it and nothing
      // else. Capture-phase, so the window handler that would also deselect never runs.
      e.stopPropagation();
      e.preventDefault();
    };

    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", closeMenu);
    window.addEventListener("resize", closeMenu);
    window.addEventListener("wheel", closeMenu, { passive: true });
    return () => {
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", closeMenu);
      window.removeEventListener("resize", closeMenu);
      window.removeEventListener("wheel", closeMenu);
    };
  });

  // A menu describes the document as it was when it opened; if the document changes underneath —
  // a sync from a co-author, an undo — its verbs may no longer mean what they say.
  //
  // Compared by *value*, not by reading the store and reacting: every selection refreshes the
  // mirror with a new `doc` object, so an identity dependency would close the menu on the very
  // selection that opened it.
  let seen = { version: -1, source: "" };
  $effect(() => {
    const version = editor.treeVersion;
    const source = editor.doc?.source ?? "";
    if (version === seen.version && source === seen.source) return;
    seen = { version, source };
    closeMenu();
  });
</script>

{#if menu}
  <div
    class="ctx"
    style:left="{menu.x}px"
    style:top="{menu.y}px"
    role="menu"
    tabindex="-1"
    use:place
    onkeydown={onMenuKeydown}
  >
    <!-- The subject, so a stack of verbs says what it acts on. -->
    <p class="ctx-title">{menu.title}</p>
    {#each menu.items as it (it.label)}
      <button
        role="menuitem"
        class:danger={it.danger}
        disabled={it.disabled}
        title={it.hint}
        onclick={() => {
          it.run();
          closeMenu();
        }}
      >
        <span class="lbl">{it.label}</span>
        {#if it.hint}<span class="hint">{it.hint}</span>{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .ctx {
    position: fixed;
    z-index: 61;
    min-width: 150px;
    padding: 4px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-light);
    box-shadow: var(--halo-shadow, 0 8px 24px rgb(0 0 0 / 0.25));
  }

  .ctx-title {
    margin: 2px 10px 4px;
    color: var(--halo-text-muted);
    font-size: 11px;
    white-space: nowrap;
  }

  .ctx button {
    display: flex;
    gap: 12px;
    align-items: baseline;
    justify-content: space-between;
    width: 100%;
    padding: 6px 10px;
    border: none;
    border-radius: var(--halo-radius);
    background: transparent;
    color: var(--halo-text-main);
    text-align: left;
    font-size: 13px;
  }

  .ctx button:hover:not([disabled]) {
    background: var(--halo-accent-soft);
    color: var(--halo-accent);
  }

  /* A verb that doesn't apply stays visible and says why — an item that vanishes teaches that it
     doesn't exist. */
  .ctx button[disabled] {
    color: var(--halo-text-muted);
    cursor: default;
  }

  .ctx button.danger:not([disabled]) {
    color: var(--halo-error);
  }

  .hint {
    color: var(--halo-text-muted);
    font-size: 11px;
  }
</style>
