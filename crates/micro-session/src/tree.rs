//! A session is a tree, not a list.

use micro_types::LedgerEvent;
use micro_types::Message;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// One entry in the log: a message, and where it sits in the tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// The entry this one followed.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compaction {
    /// The entry this was recorded after, which is where it sits on the path.
    pub entry_id: String,
    /// The first entry that is still part of the conversation.
    pub first_kept: Option<String>,
    /// What the model said the replaced stretch amounted to.
    pub summary: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    /// The entry being named.
    pub entry_id: String,
    /// The name, or nothing to take a name away.
    #[serde(default)]
    pub label: Option<String>,
    pub timestamp: i64,
}

/// Something the session recorded that is not part of the conversation, in the envelope the ledger
/// is written in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerLine {
    pub v: u32,
    pub seq: u64,
    pub ts: i64,
    pub event: LedgerEvent,
}

/// A line of the log: a ledger event, an entry, something recorded beside the conversation, or a
/// bare message from an older session.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Line {
    Ledger(LedgerLine),
    Entry(Entry),
    Compaction(Compaction),
    Custom(CustomEntry),
    Label(Label),
    Bare(Message),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    entries: Vec<Entry>,
    /// Everything recorded beside the conversation, in the order it was written.
    customs: Vec<CustomEntry>,
    /// What each entry has been named, by entry id.
    labels: std::collections::BTreeMap<String, String>,
    /// Every compaction recorded, oldest first.
    compactions: Vec<Compaction>,
    /// Every fact recorded beside the conversation, in the order it happened.
    ledger: Vec<LedgerLine>,
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
            tree.apply(line);
        }
        tree
    }

    /// Fold one line of a log in, in the order it was written.
    pub(crate) fn apply(&mut self, line: Line) {
        match line {
            Line::Ledger(recorded) => {
                if let LedgerEvent::HeadMoved { entry_id } = &recorded.event {
                    self.branch_from(entry_id);
                }
                self.ledger.push(recorded);
            }
            Line::Entry(entry) => {
                self.head = Some(entry.id.clone());
                self.entries.push(entry);
            }
            Line::Compaction(compaction) => self.compactions.push(compaction),
            Line::Custom(custom) => self.customs.push(custom),
            Line::Label(label) => match label.label {
                Some(name) => {
                    self.labels.insert(label.entry_id, name);
                }
                None => {
                    self.labels.remove(&label.entry_id);
                }
            },
            Line::Bare(message) => {
                self.push(message);
            }
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Everything recorded beside the conversation.
    pub fn customs(&self) -> &[CustomEntry] {
        &self.customs
    }

    /// Every compaction recorded, oldest first.
    pub fn compactions(&self) -> &[Compaction] {
        &self.compactions
    }

    /// Every fact recorded beside the conversation, oldest first.
    pub fn ledger(&self) -> &[LedgerLine] {
        &self.ledger
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

    /// Name an entry, or take its name away.
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

    /// Verify that every conversation entry has one unique id, a known parent, and an acyclic
    /// route to a root entry.
    pub(crate) fn validate(&self) -> std::result::Result<(), String> {
        use std::collections::HashMap;

        let entries: HashMap<&str, &Entry> = self
            .entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry))
            .collect();
        if entries.len() != self.entries.len() {
            return Err("duplicate entry id".into());
        }

        for entry in &self.entries {
            if let Some(parent_id) = entry.parent_id.as_deref() {
                if !entries.contains_key(parent_id) {
                    return Err(format!(
                        "entry {} references missing parent {parent_id}",
                        entry.id
                    ));
                }
            }
        }

        let mut states = HashMap::new();
        for entry in &self.entries {
            let mut cursor = Some(entry.id.as_str());
            let mut walked = Vec::new();
            while let Some(id) = cursor {
                match states.get(id).copied() {
                    Some(1) => return Err(format!("cycle involving entry {id}")),
                    Some(2) => break,
                    _ => {
                        states.insert(id, 1);
                        walked.push(id);
                        cursor = entries
                            .get(id)
                            .and_then(|current| current.parent_id.as_deref());
                    }
                }
            }
            for id in walked {
                states.insert(id, 2);
            }
        }

        Ok(())
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
    pub fn branch_from(&mut self, id: &str) -> bool {
        if !self.entries.iter().any(|entry| entry.id == id) {
            return false;
        }
        self.head = Some(id.to_string());
        true
    }

    /// Record that a stretch of the conversation has been summarized.
    pub fn push_compaction(&mut self, summary: impl Into<String>, kept: usize) -> Compaction {
        let ids = self.path_ids();
        let first_kept = match kept >= ids.len() {
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

    pub fn path(&self) -> Vec<Message> {
        use std::collections::HashSet;

        let mut path = Vec::new();
        let mut cursor = self.head.clone();
        let mut visited = HashSet::new();
        while let Some(id) = cursor {
            if !visited.insert(id.clone()) {
                break;
            }
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

        let from = compaction
            .first_kept
            .as_ref()
            .and_then(|kept| path.iter().position(|(id, _)| id == kept))
            .unwrap_or(0);
        let mut read = vec![micro_context::summary_message(&compaction.summary)];
        read.extend(path.into_iter().skip(from).map(|(_, message)| message));
        read
    }

    /// The ids of the entries [`Tree::path`] reads, in the same order.
    pub fn path_entry_ids(&self) -> Vec<String> {
        let ids = self.path_ids();
        let Some(compaction) = self.active_compaction() else {
            return ids;
        };
        let from = compaction
            .first_kept
            .as_ref()
            .and_then(|kept| ids.iter().position(|id| id == kept))
            .unwrap_or(0);
        ids.into_iter().skip(from).collect()
    }

    /// Where an entry sits along the path from the root to the head, counting from zero.
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
        use std::collections::HashSet;

        let mut ids = Vec::new();
        let mut cursor = self.head.clone();
        let mut visited = HashSet::new();
        while let Some(id) = cursor {
            if !visited.insert(id.clone()) {
                break;
            }
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

    /// What is kept beside the conversation is never in it: the model sees the path, and the path
    /// holds messages only.
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

    /// The line kinds are told apart by their fields alone, and a label matches anything carrying
    /// an entry id.
    #[test]
    fn a_ledger_line_is_never_read_as_something_else() {
        let written = serde_json::to_string(&LedgerLine {
            v: micro_types::SCHEMA_VERSION,
            seq: 1,
            ts: 1786361585474,
            event: LedgerEvent::HeadMoved {
                entry_id: "3".into(),
            },
        })
        .unwrap();

        let line: Line = serde_json::from_str(&written).expect("a ledger line reads back");
        let Line::Ledger(recorded) = line else {
            panic!("expected a ledger line, got {line:?}");
        };
        assert_eq!(recorded.seq, 1);

        let mut tree = Tree::from_lines(vec![
            Line::Entry(Entry::new("1", None, user("a question"))),
            Line::Label(Label {
                entry_id: "1".into(),
                label: Some("the good branch".into()),
                timestamp: 0,
            }),
        ]);
        tree.apply(serde_json::from_str(&written).unwrap());
        assert_eq!(tree.label("1"), Some("the good branch"));
        assert_eq!(tree.ledger().len(), 1);
    }

    #[test]
    fn a_head_move_replays_at_the_point_it_was_written() {
        let tree = Tree::from_lines(vec![
            Line::Entry(Entry::new("1", None, user("question"))),
            Line::Entry(Entry::new("2", Some("1".into()), user("first answer"))),
            Line::Ledger(LedgerLine {
                v: micro_types::SCHEMA_VERSION,
                seq: 1,
                ts: 0,
                event: LedgerEvent::HeadMoved {
                    entry_id: "1".into(),
                },
            }),
        ]);

        assert_eq!(tree.head(), Some("1"));
        assert_eq!(tree.path(), vec![user("question")]);
    }

    #[test]
    fn graph_validation_rejects_duplicate_ids_missing_parents_and_cycles() {
        let duplicate = Tree::from_lines(vec![
            Line::Entry(Entry::new("1", None, user("first"))),
            Line::Entry(Entry::new("1", None, user("second"))),
        ]);
        assert!(duplicate.validate().unwrap_err().contains("duplicate"));

        let missing_parent = Tree::from_lines(vec![Line::Entry(Entry::new(
            "1",
            Some("missing".into()),
            user("orphan"),
        ))]);
        assert!(missing_parent
            .validate()
            .unwrap_err()
            .contains("missing parent"));

        let cycle = Tree::from_lines(vec![
            Line::Entry(Entry::new("1", Some("2".into()), user("first"))),
            Line::Entry(Entry::new("2", Some("1".into()), user("second"))),
        ]);
        assert!(cycle.validate().unwrap_err().contains("cycle"));
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

    /// After compacting, the conversation reads as the summary and what was kept.
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

    /// A compaction recorded on a branch that was left behind says nothing about the one being read
    /// now.
    #[test]
    fn a_compaction_on_another_branch_is_ignored() {
        let mut tree = conversation();
        tree.push_compaction("on the abandoned branch", 2);

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
