//! A session is a tree, not a list.
//!
//! Every entry records which entry it followed, so a conversation can be picked up again
//! from any point without losing what came after. Asking a question, disliking the answer
//! and asking differently leaves both branches on disk; the conversation the model sees is
//! the path from the root to whichever entry is currently the head.
//!
//! Older logs hold bare messages with no envelope. They read as a straight line, which is
//! exactly what they were.

use micro_types::Message;
use serde::Deserialize;
use serde::Serialize;

/// One entry in the log: a message, and where it sits in the tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// The entry this one followed. `None` for the first entry of a session.
    #[serde(default)]
    pub parent_id: Option<String>,
    pub timestamp: i64,
    pub message: Message,
}

impl Entry {
    pub fn new(id: impl Into<String>, parent_id: Option<String>, message: Message) -> Self {
        Entry {
            id: id.into(),
            parent_id,
            timestamp: micro_types::now_ms(),
            message,
        }
    }
}

/// A line of the log, which is either an entry or a bare message from an older session.
///
/// Untagged so both shapes parse: an envelope has an `id`, a bare message has a `role`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Line {
    Entry(Entry),
    Bare(Message),
}

/// The entries of a session, and which one the conversation currently continues from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    entries: Vec<Entry>,
    head: Option<String>,
}

impl Tree {
    pub fn new() -> Self {
        Tree::default()
    }

    /// Rebuild a tree from what a log held, giving bare messages the straight line they had.
    pub fn from_lines(lines: Vec<Line>) -> Self {
        let mut tree = Tree::new();
        for line in lines {
            match line {
                Line::Entry(entry) => {
                    tree.head = Some(entry.id.clone());
                    tree.entries.push(entry);
                }
                Line::Bare(message) => {
                    tree.push(message);
                }
            }
        }
        tree
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn head(&self) -> Option<&str> {
        self.head.as_deref()
    }

    /// Add a message after the current head, and make it the head.
    pub fn push(&mut self, message: Message) -> Entry {
        let id = format!("{}", self.entries.len() + 1);
        let entry = Entry::new(id.clone(), self.head.clone(), message);
        self.head = Some(id);
        self.entries.push(entry.clone());
        entry
    }

    /// Continue the conversation from `id` instead of from where it left off.
    ///
    /// Nothing is deleted: what came after stays on disk as another branch, which is the
    /// point of keeping a tree rather than a list.
    pub fn branch_from(&mut self, id: &str) -> bool {
        if !self.entries.iter().any(|entry| entry.id == id) {
            return false;
        }
        self.head = Some(id.to_string());
        true
    }

    /// The conversation as the model should see it: the path from the root to the head.
    pub fn path(&self) -> Vec<Message> {
        let mut path = Vec::new();
        let mut cursor = self.head.clone();
        while let Some(id) = cursor {
            let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
                break;
            };
            path.push(entry.message.clone());
            cursor = entry.parent_id.clone();
        }
        path.reverse();
        path
    }

    /// Every entry that continues directly from `id`, oldest first.
    pub fn children(&self, id: Option<&str>) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.parent_id.as_deref() == id)
            .collect()
    }

    /// Whether an entry has more than one continuation, which is what makes it a fork.
    pub fn is_fork(&self, id: &str) -> bool {
        self.children(Some(id)).len() > 1
    }

    /// The tree flattened for display: each entry with how deep it sits and whether the
    /// conversation currently runs through it.
    pub fn outline(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        let on_path: Vec<String> = self.path_ids();
        self.walk(None, 0, &on_path, &mut rows);
        rows
    }

    fn path_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor = self.head.clone();
        while let Some(id) = cursor {
            let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
                break;
            };
            ids.push(entry.id.clone());
            cursor = entry.parent_id.clone();
        }
        ids
    }

    fn walk<'a>(
        &'a self,
        parent: Option<&str>,
        depth: usize,
        on_path: &[String],
        rows: &mut Vec<Row<'a>>,
    ) {
        for child in self.children(parent) {
            rows.push(Row {
                entry: child,
                depth,
                on_path: on_path.contains(&child.id),
                is_head: self.head.as_deref() == Some(child.id.as_str()),
            });
            self.walk(Some(&child.id), depth + 1, on_path, rows);
        }
    }
}

/// One line of a tree view.
#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    pub entry: &'a Entry,
    /// How far from the root, for indenting.
    pub depth: usize,
    /// Whether the conversation currently runs through this entry.
    pub on_path: bool,
    /// Whether this is where the conversation continues from.
    pub is_head: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Message {
        Message::user(text)
    }

    #[test]
    fn a_conversation_with_no_branches_is_a_straight_line() {
        let mut tree = Tree::new();
        tree.push(user("one"));
        tree.push(user("two"));
        tree.push(user("three"));

        let path: Vec<String> = tree
            .path()
            .iter()
            .map(|message| message.content()[0].as_text().to_string())
            .collect();
        assert_eq!(path, vec!["one", "two", "three"]);
    }

    /// Branching keeps what came after: the conversation continues from an earlier point
    /// and the other answer is still there to go back to.
    #[test]
    fn branching_keeps_both_continuations() {
        let mut tree = Tree::new();
        tree.push(user("question"));
        let first = tree.push(user("answer one")).id;
        tree.push(user("follow up"));

        assert!(tree.branch_from("1"));
        tree.push(user("answer two"));

        let path: Vec<String> = tree
            .path()
            .iter()
            .map(|message| message.content()[0].as_text().to_string())
            .collect();
        assert_eq!(path, vec!["question", "answer two"]);
        assert_eq!(tree.entries().len(), 4, "nothing was deleted");
        assert!(tree.is_fork("1"), "the question now has two answers");
        assert_eq!(first, "2");
    }

    #[test]
    fn branching_from_something_that_is_not_there_changes_nothing() {
        let mut tree = Tree::new();
        tree.push(user("only"));
        assert!(!tree.branch_from("nowhere"));
        assert_eq!(tree.head(), Some("1"));
    }

    #[test]
    fn an_outline_shows_every_branch_and_marks_the_one_in_use() {
        let mut tree = Tree::new();
        tree.push(user("root"));
        tree.push(user("first child"));
        tree.branch_from("1");
        tree.push(user("second child"));

        let outline = tree.outline();
        assert_eq!(outline.len(), 3);
        assert_eq!(outline[0].depth, 0);
        assert_eq!(outline[1].depth, 1);
        assert_eq!(outline[2].depth, 1);

        assert!(outline[0].on_path, "the root is on the path");
        assert!(!outline[1].on_path, "the abandoned branch is not");
        assert!(outline[2].is_head);
    }

    /// A log written before sessions had a tree reads as the straight line it was.
    #[test]
    fn an_older_log_reads_as_one_branch() {
        let tree = Tree::from_lines(vec![
            Line::Bare(user("one")),
            Line::Bare(user("two")),
        ]);
        assert_eq!(tree.entries().len(), 2);
        assert_eq!(tree.path().len(), 2);
        assert_eq!(tree.entries()[1].parent_id.as_deref(), Some("1"));
    }

    #[test]
    fn a_mixed_log_joins_up() {
        let tree = Tree::from_lines(vec![
            Line::Bare(user("old")),
            Line::Entry(Entry::new("2", Some("1".into()), user("new"))),
        ]);
        assert_eq!(tree.path().len(), 2);
        assert_eq!(tree.head(), Some("2"));
    }
}
