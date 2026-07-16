/** A Svelte action for modal dialogs: keep Tab focus inside `node` (so it can't wander to the
 *  page behind), and restore focus to the previously-focused element (the invoking control) when
 *  the dialog closes. Initial focus is left to the caller (each dialog focuses its primary field). */
export function focusTrap(node: HTMLElement) {
  const previous = document.activeElement as HTMLElement | null;

  const focusable = (): HTMLElement[] =>
    [
      ...node.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ].filter((el) => el.offsetParent !== null); // skip hidden

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== "Tab") return;
    const items = focusable();
    if (items.length === 0) {
      e.preventDefault();
      node.focus();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === node)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  node.addEventListener("keydown", onKeydown);
  return {
    destroy() {
      node.removeEventListener("keydown", onKeydown);
      previous?.focus?.();
    },
  };
}
