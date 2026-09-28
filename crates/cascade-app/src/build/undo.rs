//! Undo and redo stacks of edit ops.
//!
//! Each entry holds the op that reverses a committed edit. Undoing applies
//! the top undo entry; the inverse of *that* application goes onto the redo
//! stack, and vice versa. The host applies ops (it may fail, e.g. while
//! `edit::apply` is a stub), so taking an entry is two-phase: [`History::peek`]
//! to read it, [`History::complete`] once it applied. A failed application
//! leaves both stacks unchanged.

use cascade_core::edit::EditOp;

/// Oldest entries are dropped past this many.
pub const HISTORY_LIMIT: usize = 200;

/// Which stack an operation reads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Undo,
    Redo,
}

impl Direction {
    pub const fn verb(self) -> &'static str {
        match self {
            Direction::Undo => "Undid",
            Direction::Redo => "Redid",
        }
    }
}

/// One undoable (or redoable) step.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The op that moves the definition one step in this entry's direction.
    pub op: EditOp,
    /// What the original edit did, e.g. "Add state idle2".
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

impl History {
    /// A new edit was committed: remember how to reverse it and forget the
    /// redo stack (it no longer applies).
    pub fn record(&mut self, inverse: EditOp, label: impl Into<String>) {
        self.undo.push(Entry { op: inverse, label: label.into() });
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// The entry an undo or redo would apply.
    pub fn peek(&self, direction: Direction) -> Option<&Entry> {
        match direction {
            Direction::Undo => self.undo.last(),
            Direction::Redo => self.redo.last(),
        }
    }

    /// The peeked entry applied and produced `inverse`: move it to the other
    /// stack. Returns the entry's label, or `None` when the stack was empty.
    pub fn complete(&mut self, direction: Direction, inverse: EditOp) -> Option<String> {
        let (from, to) = match direction {
            Direction::Undo => (&mut self.undo, &mut self.redo),
            Direction::Redo => (&mut self.redo, &mut self.undo),
        };
        let entry = from.pop()?;
        to.push(Entry { op: inverse, label: entry.label.clone() });
        Some(entry.label)
    }

    /// Forget everything (the file changed outside the app).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }

    pub fn can(&self, direction: Direction) -> bool {
        self.peek(direction).is_some()
    }

    pub fn len(&self, direction: Direction) -> usize {
        match direction {
            Direction::Undo => self.undo.len(),
            Direction::Redo => self.redo.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(name: &str) -> EditOp {
        EditOp::RemoveMachine { machine: name.to_owned() }
    }

    #[test]
    fn empty_history_has_nothing_to_undo_or_redo() {
        let mut h = History::default();
        assert!(h.is_empty());
        assert!(!h.can(Direction::Undo));
        assert!(!h.can(Direction::Redo));
        assert_eq!(h.complete(Direction::Undo, op("x")), None);
        assert!(h.is_empty());
    }

    #[test]
    fn undo_then_redo_moves_entries_between_stacks() {
        let mut h = History::default();
        h.record(op("undo-a"), "Add A");
        h.record(op("undo-b"), "Add B");
        assert_eq!(h.peek(Direction::Undo).map(|e| e.op.clone()), Some(op("undo-b")));

        assert_eq!(h.complete(Direction::Undo, op("redo-b")), Some("Add B".to_owned()));
        assert_eq!(h.peek(Direction::Undo).map(|e| e.label.as_str()), Some("Add A"));
        assert_eq!(h.peek(Direction::Redo), Some(&Entry { op: op("redo-b"), label: "Add B".into() }));

        assert_eq!(h.complete(Direction::Redo, op("undo-b2")), Some("Add B".to_owned()));
        assert_eq!(h.peek(Direction::Undo).map(|e| e.op.clone()), Some(op("undo-b2")));
        assert!(!h.can(Direction::Redo));
        assert_eq!(h.len(Direction::Undo), 2);
    }

    #[test]
    fn a_failed_application_leaves_the_stacks_alone() {
        let mut h = History::default();
        h.record(op("undo-a"), "Add A");
        // The host peeks, the application fails, it never calls complete.
        let before = h.clone();
        let _ = h.peek(Direction::Undo);
        assert_eq!(h, before);
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut h = History::default();
        h.record(op("undo-a"), "Add A");
        h.complete(Direction::Undo, op("redo-a"));
        assert!(h.can(Direction::Redo));
        h.record(op("undo-c"), "Add C");
        assert!(!h.can(Direction::Redo));
        assert_eq!(h.len(Direction::Undo), 1);
    }

    #[test]
    fn clear_forgets_both_stacks() {
        let mut h = History::default();
        h.record(op("a"), "A");
        h.record(op("b"), "B");
        h.complete(Direction::Undo, op("b2"));
        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::default();
        for i in 0..HISTORY_LIMIT + 5 {
            h.record(op(&format!("m{i}")), format!("{i}"));
        }
        assert_eq!(h.len(Direction::Undo), HISTORY_LIMIT);
        assert_eq!(h.peek(Direction::Undo).map(|e| e.label.clone()), Some(format!("{}", HISTORY_LIMIT + 4)));
    }

    #[test]
    fn verbs() {
        assert_eq!(Direction::Undo.verb(), "Undid");
        assert_eq!(Direction::Redo.verb(), "Redid");
    }
}
