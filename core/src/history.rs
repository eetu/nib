//! A generic snapshot-based undo/redo stack — ported from `stores/history.svelte.ts`. The
//! `Editor` owns one and decides what a snapshot is + how to restore it.

pub struct History<T> {
    past: Vec<T>,
    future: Vec<T>,
    present: Option<T>,
}

impl<T: Clone> History<T> {
    pub fn new() -> Self {
        History {
            past: Vec::new(),
            future: Vec::new(),
            present: None,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// The current committed snapshot (used to revert an in-flight gesture).
    pub fn current(&self) -> Option<&T> {
        self.present.as_ref()
    }

    /// Discard all history and seed the initial state.
    pub fn reset(&mut self, initial: T) {
        self.past.clear();
        self.future.clear();
        self.present = Some(initial);
    }

    /// Record a new committed state; the old present becomes undoable.
    pub fn commit(&mut self, next: T) {
        if let Some(present) = self.present.take() {
            self.past.push(present);
        }
        self.present = Some(next);
        self.future.clear();
    }

    pub fn undo(&mut self) -> Option<&T> {
        let prev = self.past.pop()?;
        if let Some(present) = self.present.take() {
            self.future.push(present);
        }
        self.present = Some(prev);
        self.present.as_ref()
    }

    pub fn redo(&mut self) -> Option<&T> {
        let next = self.future.pop()?;
        if let Some(present) = self.present.take() {
            self.past.push(present);
        }
        self.present = Some(next);
        self.present.as_ref()
    }
}

impl<T: Clone> Default for History<T> {
    fn default() -> Self {
        Self::new()
    }
}
