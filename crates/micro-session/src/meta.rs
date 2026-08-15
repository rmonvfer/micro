//! Session metadata: the small record `list` reads instead of replaying whole logs.

use micro_types::now_ms;
use micro_types::ContentBlock;
use micro_types::Message;
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;

/// Longest title kept for a session.
pub const MAX_TITLE_CHARS: usize = 60;

/// Everything a listing needs about a session without opening its message log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    /// The schema this record was written against, so a later reader knows what it is looking at.
    #[serde(default)]
    pub v: u32,
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub workspace: PathBuf,
    pub model_id: String,
    /// Derived from the first user message; empty until one is appended.
    pub title: String,
    pub message_count: usize,
    /// The session this one was forked from, when it did not start from scratch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Who the session belongs to, when it belongs to anyone beyond the person who ran it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl SessionMeta {
    pub(crate) fn new(id: String, workspace: PathBuf, model_id: String) -> Self {
        let now = now_ms();
        SessionMeta {
            v: micro_types::SCHEMA_VERSION,
            id,
            created_at: now,
            updated_at: now,
            workspace,
            model_id,
            title: String::new(),
            message_count: 0,
            parent: None,
            org_id: None,
            agent_id: None,
        }
    }

    /// Folds one appended message into the metadata.
    pub(crate) fn record(&mut self, message: &Message) {
        self.message_count += 1;
        self.updated_at = now_ms();
        if self.title.is_empty() {
            if let Message::User { content, .. } = message {
                self.title = derive_title(content);
            }
        }
    }
}

/// Flattens a user message to a single line short enough to list.
fn derive_title(content: &[ContentBlock]) -> String {
    let flattened = content
        .iter()
        .map(ContentBlock::as_text)
        .collect::<Vec<_>>()
        .join(" ");
    let single_line = flattened.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= MAX_TITLE_CHARS {
        return single_line;
    }

    let clipped: String = single_line.chars().take(MAX_TITLE_CHARS).collect();
    
    let boundary = clipped.rfind(' ').filter(|at| *at >= clipped.len() / 2);
    let kept = match boundary {
        Some(at) => &clipped[..at],
        None => clipped.as_str(),
    };
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_first_message_becomes_the_title_verbatim() {
        let mut meta = SessionMeta::new("1".into(), PathBuf::from("/work"), "opus".into());
        meta.record(&Message::user("fix the login bug"));
        assert_eq!(meta.title, "fix the login bug");
        assert_eq!(meta.message_count, 1);
    }

    #[test]
    fn a_multi_line_first_message_collapses_to_one_line() {
        let mut meta = SessionMeta::new("1".into(), PathBuf::from("/work"), "opus".into());
        meta.record(&Message::user("  fix\n\tthe   login\nbug  "));
        assert_eq!(meta.title, "fix the login bug");
    }

    #[test]
    fn a_long_first_message_is_cut_at_a_word_boundary() {
        let mut meta = SessionMeta::new("1".into(), PathBuf::from("/work"), "opus".into());
        meta.record(&Message::user(
            "port the entire session persistence layer from kotlin to rust without losing anything",
        ));
        assert!(meta.title.ends_with('…'));
        assert!(meta.title.chars().count() <= MAX_TITLE_CHARS + 1);
        assert!(!meta.title.contains("  "));
        assert!(meta
            .title
            .starts_with("port the entire session persistence layer"));
    }

    #[test]
    fn only_the_first_user_message_sets_the_title() {
        let mut meta = SessionMeta::new("1".into(), PathBuf::from("/work"), "opus".into());
        meta.record(&Message::user("first"));
        meta.record(&Message::user("second"));
        assert_eq!(meta.title, "first");
        assert_eq!(meta.message_count, 2);
    }

    #[test]
    fn a_leading_tool_result_leaves_the_title_open() {
        let mut meta = SessionMeta::new("1".into(), PathBuf::from("/work"), "opus".into());
        meta.record(&Message::tool_result("call_1", "read", "contents", false));
        assert_eq!(meta.title, "");
        meta.record(&Message::user("now the title"));
        assert_eq!(meta.title, "now the title");
    }
}
