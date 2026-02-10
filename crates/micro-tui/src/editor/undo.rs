//! Undo history for the prompt.
//!
//! Snapshots rather than operations, coalesced the way fish coalesces: a run of word
//! characters is one unit, and whitespace opens the next one. Undoing a sentence therefore
//! takes back a word at a time rather than a keystroke at a time, and the space that
//! separated two words goes back with the word that followed it.

use crate::editor::kill_ring::LastAction;

/// The editor as it stood before an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub lines: Vec<String>,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UndoStack {
    snapshots: Vec<Snapshot>,
}

impl UndoStack {
    pub fn new() -> Self {
        UndoStack::default()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn push(&mut self, snapshot: Snapshot) {
        self.snapshots.push(snapshot);
    }

    pub fn pop(&mut self) -> Option<Snapshot> {
        self.snapshots.pop()
    }

    /// Submitting ends the prompt's history along with the prompt.
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }
}

/// Whether typing `character` should open a new undo unit.
///
/// A word character continues the run it is part of; anything else — a space, punctuation —
/// starts its own, so undo takes back a word and the space before it together.
pub fn opens_new_unit(character: char, last: LastAction) -> bool {
    match is_word_character(character) {
        true => last != LastAction::TypeWord,
        false => true,
    }
}

/// What counts as part of a word while typing. Letters, digits and underscore, matching
/// what the word-motion commands treat as one run.
pub fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str) -> Snapshot {
        Snapshot {
            lines: vec![text.to_string()],
            line: 0,
            column: text.len(),
        }
    }

    #[test]
    fn undo_returns_the_most_recent_snapshot_first() {
        let mut stack = UndoStack::new();
        stack.push(snapshot("one"));
        stack.push(snapshot("two"));

        assert_eq!(stack.pop(), Some(snapshot("two")));
        assert_eq!(stack.pop(), Some(snapshot("one")));
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn a_run_of_word_characters_is_one_unit() {
        assert!(opens_new_unit('h', LastAction::Other));
        assert!(!opens_new_unit('e', LastAction::TypeWord));
        assert!(!opens_new_unit('1', LastAction::TypeWord));
        assert!(!opens_new_unit('_', LastAction::TypeWord));
    }

    #[test]
    fn whitespace_always_opens_its_own_unit() {
        assert!(opens_new_unit(' ', LastAction::TypeWord));
        assert!(opens_new_unit(' ', LastAction::Other));
    }

    #[test]
    fn punctuation_opens_its_own_unit_too() {
        assert!(opens_new_unit('.', LastAction::TypeWord));
        assert!(opens_new_unit('-', LastAction::TypeWord));
    }

    #[test]
    fn submitting_ends_the_history_with_the_prompt() {
        let mut stack = UndoStack::new();
        stack.push(snapshot("gone"));
        stack.clear();
        assert!(stack.is_empty());
    }
}
