// Asking "are you sure?" without the browser's `confirm()`, which blocks the page, ignores the
// app's type and colours, and reads as the browser interrupting rather than nib asking.
//
// Only for what undo cannot take back — Revert, Discard, deleting a file. Anything undoable acts
// immediately and lets undo be the answer.

type Ask = {
  title: string;
  /** One line of consequence: what will be lost, in facts. */
  body: string;
  /** The verb on the confirming button ("revert", "discard"), never "OK". */
  confirmLabel: string;
  /** Destructive framing for the confirm button. */
  danger?: boolean;
  resolve: (ok: boolean) => void;
};

class ConfirmState {
  current = $state<Ask | null>(null);
}

const state = new ConfirmState();

export function pendingConfirm(): Ask | null {
  return state.current;
}

/** Ask, and resolve with the answer. A second ask while one is open answers the first with no. */
export function askConfirm(question: Omit<Ask, "resolve">): Promise<boolean> {
  state.current?.resolve(false);
  return new Promise<boolean>((resolve) => {
    state.current = { ...question, resolve };
  });
}

export function answerConfirm(ok: boolean): void {
  const ask = state.current;
  state.current = null;
  ask?.resolve(ok);
}
