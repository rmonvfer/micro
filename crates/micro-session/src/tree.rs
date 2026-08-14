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
use serde_json::Value;

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

/// Something recorded in a session that is not part of the conversation.
///
/// An extension keeps state here — what it decided, what it was told — and a label names
/// an entry for whoever reads the tree later. None of it is ever shown to the model: the
/// conversation is what [`Tree::path`] returns, and nothing here is in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomEntry {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub timestamp: i64,
    /// What kind of thing this is, as whoever wrote it named it.
    pub custom_type: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

/// A stretch of the conversation replaced by a summary of it.
///
/// Compaction is a fact about the session rather than about the process that did it: a
/// session reopened later reads the summary and what followed it, and does not pay to
/// summarize the same stretch again. Nothing is deleted — the entries it stands for are
/// still in the log, and a reader looking at the tree can still see them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compaction {
    /// The entry this was recorded after, which is where it sits on the path.
    pub entry_id: String,
    /// The first entry that is still part of the conversation. Everything before it on
    /// the path is what the summary stands for.
    pub first_kept: Option<String>,
    /// What the model said the replaced stretch amounted to.
    pub summary: String,
    pub timestamp: i64,
}

/// A name given to an entry, so a branch can be found again by what it was for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    /// The entry being named.
    pub entry_id: String,
    /// The name, or nothing to take a name away.
    #[serde(default)]
    pub label: Option<String>,
    pub timestamp: i64,
}

/// A line of the log: an entry, something recorded beside the conversation, or a bare
/// message from an older session.
///
/// Untagged so every shape parses: an envelope has an `id` and a `message`, a custom entry
/// has a `custom_type`, a label has an `entry_id`, and a bare message has a `role`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Line {
    Entry(Entry),
    Compaction(Compaction),
    Custom(CustomEntry),
    Label(Label),
    Bare(Message),
}

/// The entries of a session, and which one the conversation currently continues from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    entries: Vec<Entry>,
    /// Everything recorded beside the conversation, in the order it was written.
    customs: Vec<CustomEntry>,
    /// What each entry has been named, by entry id.
    labels: std::collections::BTreeMap<String, String>,
    /// Every compaction recorded, oldest first. The newest one on the current path is
    /// what the conversation is read through.
    compactions: Vec<Compaction>,
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
                Line::Compaction(compaction) => tree.compactions.push(compaction),
                Line::Custom(custom) => tree.customs.push(custom),
                Line::Label(label) => match label.label {
                    Some(name) => {
                        tree.labels.insert(label.entry_id, name);
                    }
                    None => {
                        tree.labels.remove(&label.entry_id);
                    }
                },
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

    /// Everything recorded beside the conversation.
    pub fn customs(&self) -> &[CustomEntry] {
        &self.customs
    }

    /// Every compaction recorded, oldest first. Most callers want [`Tree::path`], which
    /// already reads the conversation through whichever of these is active; this is for a
    /// reader that wants the record of compactions itself, not just its effect.
    pub fn compactions(&self) -> &[Compaction] {
        &self.compactions
    }

    /// What an entry has been named, if anything.
    pub fn label(&self, entry_id: &str) -> Option<&str> {
        self.labels.get(entry_id).map(String::as_str)
    }

    /// Record something beside the conversation, hanging off wherever it currently is.
    pub fn push_custom(&mut self, custom_type: impl Into<String>, data: Value) -> CustomEntry {
        let custom = CustomEntry {
            id: format!("c{}", self.customs.len() + 1),
            parent_id: self.head.clone(),
            timestamp: micro_types::now_ms(),
            custom_type: custom_type.into(),
            data,
        };
        self.customs.push(custom.clone());
        custom
    }

    /// Name an entry, or take its name away. Naming something that is not there does
    /// nothing, which is what makes a stale id harmless.
    pub fn set_label(&mut self, entry_id: &str, label: Option<String>) -> bool {
        if !self.entries.iter().any(|entry| entry.id == entry_id) {
            return false;
        }
        match label {
            Some(label) => {
                self.labels.insert(entry_id.to_string(), label);
            }
            None => {
                self.labels.remove(entry_id);
            }
        }
        true
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

    /// Record that a stretch of the conversation has been summarized.
    ///
    /// `kept` is how many of the most recent entries on the path are still part of the
    /// conversation; everything before them is what the summary stands for.
    pub fn push_compaction(&mut self, summary: impl Into<String>, kept: usize) -> Compaction {
        let ids = self.path_ids();
        let first_kept = match kept >= ids.len() {
            // Nothing was replaced, which is a compaction that changed nothing.
            true => ids.first().cloned(),
            false => ids.get(ids.len() - kept).cloned(),
        };
        let compaction = Compaction {
            entry_id: self.head.clone().unwrap_or_default(),
            first_kept,
            summary: summary.into(),
            timestamp: micro_types::now_ms(),
        };
        self.compactions.push(compaction.clone());
        compaction
    }

    /// The compaction the conversation is currently read through, if any.
    ///
    /// The newest one whose kept entry is still on the path: a compaction recorded on a
    /// branch that has since been left behind says nothing about this one.
    fn active_compaction(&self) -> Option<&Compaction> {
        let ids = self.path_ids();
        self.compactions
            .iter()
            .rev()
            .find(|compaction| match &compaction.first_kept {
                Some(kept) => ids.iter().any(|id| id == kept),
                None => false,
            })
    }

    /// The conversation as the model should see it: the path from the root to the head,
    /// with any summarized stretch standing in for what it replaced.
    pub fn path(&self) -> Vec<Message> {
        let mut path = Vec::new();
        let mut cursor = self.head.clone();
        while let Some(id) = cursor {
            let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
                break;
            };
            path.push((entry.id.clone(), entry.message.clone()));
            cursor = entry.parent_id.clone();
        }
        path.reverse();

        let Some(compaction) = self.active_compaction() else {
            return path.into_iter().map(|(_, message)| message).collect();
        };

        // Everything before the first kept entry is what the summary stands for, so the
        // summary is read in its place.
        let from = compaction
            .first_kept
            .as_ref()
            .and_then(|kept| path.iter().position(|(id, _)| id == kept))
            .unwrap_or(0);
        let mut read = vec![micro_context::summary_message(&compaction.summary)];
        read.extend(path.into_iter().skip(from).map(|(_, message)| message));
        read
    }

    /// Where an entry sits along the path from the root to the head, counting from zero.
    ///
    /// An entry on another branch has no position here: it is not part of the
    /// conversation as it currently stands.
    pub fn position_on_path(&self, id: &str) -> Option<usize> {
        self.path_ids().iter().position(|entry_id| entry_id == id)
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

    /// The ids along the path from the root to the head, oldest first.
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
        ids.reverse();
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
                label: self.labels.get(&child.id).map(String::as_str),
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
    /// The name this entry was given, when it has one.
    pub label: Option<&'a str>,
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
        let tree = Tree::from_lines(vec![Line::Bare(user("one")), Line::Bare(user("two"))]);
        assert_eq!(tree.entries().len(), 2);
        assert_eq!(tree.path().len(), 2);
        assert_eq!(tree.entries()[1].parent_id.as_deref(), Some("1"));
    }

    /// What is kept beside the conversation is never in it: the model sees the path, and
    /// the path holds messages only.
    #[test]
    fn something_kept_beside_the_conversation_stays_out_of_it() {
        let mut tree = Tree::new();
        tree.push(user("a question"));
        let kept = tree.push_custom("note", serde_json::json!({ "seen": true }));

        assert_eq!(tree.path().len(), 1, "the model sees the question alone");
        assert_eq!(tree.customs().len(), 1);
        assert_eq!(tree.customs()[0].custom_type, "note");
        assert_eq!(kept.parent_id.as_deref(), Some("1"));
    }

    #[test]
    fn an_entry_can_be_named_and_unnamed() {
        let mut tree = Tree::new();
        tree.push(user("a question"));

        assert!(tree.set_label("1", Some("the good branch".into())));
        assert_eq!(tree.label("1"), Some("the good branch"));

        assert!(tree.set_label("1", None));
        assert_eq!(tree.label("1"), None);
        assert!(
            !tree.set_label("nowhere", Some("x".into())),
            "a stale id is harmless"
        );
    }

    /// Everything written to the log comes back, each kind as what it was.
    #[test]
    fn a_log_holding_every_kind_of_line_reads_back() {
        let tree = Tree::from_lines(vec![
            Line::Entry(Entry::new("1", None, user("a question"))),
            Line::Custom(CustomEntry {
                id: "c1".into(),
                parent_id: Some("1".into()),
                timestamp: 0,
                custom_type: "note".into(),
                data: serde_json::json!({ "seen": true }),
            }),
            Line::Label(Label {
                entry_id: "1".into(),
                label: Some("the good branch".into()),
                timestamp: 0,
            }),
        ]);

        assert_eq!(tree.path().len(), 1);
        assert_eq!(tree.customs()[0].data["seen"], true);
        assert_eq!(tree.label("1"), Some("the good branch"));
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

#[cfg(test)]
mod compaction {
    use super::*;

    fn conversation() -> Tree {
        let mut tree = Tree::new();
        for index in 0..6 {
            tree.push(Message::user(format!("message {index}")));
        }
        tree
    }

    fn text(message: &Message) -> String {
        message
            .content()
            .iter()
            .map(micro_types::ContentBlock::as_text)
            .collect()
    }

    /// After compacting, the conversation reads as the summary and what was kept — the
    /// replaced stretch is still in the log but no longer part of what the model sees.
    #[test]
    fn the_conversation_reads_through_the_summary() {
        let mut tree = conversation();
        tree.push_compaction("what happened before", 2);

        let path = tree.path();
        assert_eq!(path.len(), 3, "the summary and the two kept messages");
        assert!(micro_context::is_summary(&path[0]));
        assert!(text(&path[0]).contains("what happened before"));
        assert_eq!(text(&path[1]), "message 4");
        assert_eq!(text(&path[2]), "message 5");
    }

    /// Nothing is deleted: the entries the summary stands for are still on the tree.
    #[test]
    fn nothing_is_removed_from_the_log() {
        let mut tree = conversation();
        tree.push_compaction("earlier", 2);
        assert_eq!(tree.entries().len(), 6);
    }

    /// The conversation carries on after a compaction, and what follows joins what was
    /// kept rather than what was replaced.
    #[test]
    fn the_conversation_continues_after_a_compaction() {
        let mut tree = conversation();
        tree.push_compaction("earlier", 2);
        tree.push(Message::user("after"));

        let path = tree.path();
        assert_eq!(text(path.last().unwrap()), "after");
        assert!(micro_context::is_summary(&path[0]));
        assert_eq!(path.len(), 4);
    }

    /// Compacting twice reads through the newer summary alone, not through both.
    #[test]
    fn the_newest_compaction_is_the_one_that_counts() {
        let mut tree = conversation();
        tree.push_compaction("first summary", 4);
        tree.push(Message::user("more"));
        tree.push_compaction("second summary", 2);

        let path = tree.path();
        assert!(text(&path[0]).contains("second summary"));
        assert!(!path
            .iter()
            .skip(1)
            .any(|message| text(message).contains("first summary")));
    }

    /// A compaction recorded on a branch that was left behind says nothing about the one
    /// being read now.
    #[test]
    fn a_compaction_on_another_branch_is_ignored() {
        let mut tree = conversation();
        tree.push_compaction("on the abandoned branch", 2);
        // Go back to before everything the compaction kept.
        assert!(tree.branch_from("2"));

        let path = tree.path();
        assert!(
            !micro_context::is_summary(&path[0]),
            "the conversation reads plainly again: {path:?}",
        );
        assert_eq!(path.len(), 2);
    }

    /// A session that never compacted reads exactly as it always did.
    #[test]
    fn a_plain_conversation_is_unchanged() {
        let tree = conversation();
        assert_eq!(tree.path().len(), 6);
        assert_eq!(text(&tree.path()[0]), "message 0");
    }
}
