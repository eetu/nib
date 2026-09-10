// The app's one context menu, as state. `ContextMenu.svelte` renders it once at the root; every
// surface that has verbs for a thing calls `openMenu` and never draws a menu of its own.
//
// One menu, one place, for two reasons. Only one can be open at a time, so two implementations
// means two ways to leave one open. And a menu rendered inside a panel is clipped by that panel's
// scroll box — the reason a right-click near the bottom of the LAYERS list used to open a menu you
// could only half see.

/** One line in a menu. A disabled item still renders — greyed, with `hint` saying why. */
export type MenuItem = {
  label: string;
  run: () => void;
  /** A fact: a count, a size, a shortcut, or the reason this item is grey. Never a re-explanation
   *  of the label — if the label needs explaining, the label is wrong. */
  hint?: string;
  /** Greyed and unclickable. Pair it with `hint`: an item that just goes dead teaches nothing. */
  disabled?: boolean;
  /** Destructive — rendered in the error colour. */
  danger?: boolean;
};

export type OpenMenu = {
  x: number;
  y: number;
  /** What the verbs act on ("robe", "die · component"). Four verbs with no subject read as noise. */
  title: string;
  items: MenuItem[];
};

class MenuState {
  current = $state<OpenMenu | null>(null);
}

const state = new MenuState();

/** The open menu, or `null`. Read by `ContextMenu.svelte`. */
export function activeMenu(): OpenMenu | null {
  return state.current;
}

/**
 * Open the shared menu at the event, for the thing `title` names.
 *
 * Takes the event so it can both position the menu and stop the browser's own — a caller that
 * forgets `preventDefault` would otherwise get both menus at once.
 */
export function openMenu(event: MouseEvent, title: string, items: MenuItem[]): void {
  event.preventDefault();
  event.stopPropagation();
  state.current = { x: event.clientX, y: event.clientY, title, items };
}

export function closeMenu(): void {
  state.current = null;
}
