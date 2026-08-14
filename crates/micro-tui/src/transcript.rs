//! What the scrollback shows.
//!
//! The agent reports its progress as a flat stream of events; the transcript folds that
//! stream into the ordered list of things a reader wants to see, and rebuilds the same list
//! from a saved conversation so a resumed session looks identical to a live one.

use micro_types::AgentEvent;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
use micro_types::Message;
use micro_types::StreamEvent;
use micro_types::Usage;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssistantEntry {
    pub text: String,
    pub thinking: String,
    /// True while deltas are still arriving, which is what draws the caret after the text.
    pub streaming: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolEntry {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    /// `None` while the tool is still running.
    pub output: Option<String>,
    pub is_error: bool,
    /// Whether the reader has opened this result up.
    pub expanded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    User(String),
    /// A command the user ran themselves rather than asking the model to run.
    ///
    /// `shared` is whether the model was told: `!` records the command and its output into
    /// the conversation, `!!` runs it and keeps it out, for when the answer is for the user
    /// and would only crowd the model's context.
    Bash { command: String, shared: bool },
    /// An image the user attached, drawn by the terminal when it can and described when it
    /// cannot.
    Image { data: String, mime_type: String },
    /// A stretch of conversation replaced by a summary, shown folded until asked for.
    Compaction { summary: String, expanded: bool },
    Assistant(AssistantEntry),
    Tool(ToolEntry),
    Notice { text: String, level: NoticeLevel },
    /// Something an extension drew itself. micro decides where it goes and how it is
    /// tinted; what it says is the extension's.
    Custom {
        /// What to call it, shown as its label.
        label: String,
        lines: Vec<String>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// Index of the assistant entry currently receiving deltas.
    active: Option<usize>,
    tools: HashMap<String, usize>,
    version: u64,
    /// The earliest entry that has changed since a reader last saw the conversation drawn.
    ///
    /// Everything before it is exactly as it was, so a frame can keep the rows it already
    /// has for those and redraw only from here on. A turn changes the entry it is writing
    /// and nothing else, which is what makes a long conversation cost no more to keep on
    /// screen than a short one.
    dirty_from: usize,
    last_usage: Usage,
    total_usage: Usage,
    model: Option<String>,
}

impl Transcript {
    pub fn new() -> Self {
        Transcript::default()
    }

    /// Rebuild the scrollback from a stored conversation.
    ///
    /// Nothing in a stored conversation is still running, so a call it left unanswered was
    /// abandoned; closing settles those rather than reopening the session with a tool that
    /// appears to be working.
    pub fn from_messages(messages: &[Message]) -> Self {
        let mut transcript = Transcript::new();
        for message in messages {
            transcript.push_message(message);
        }
        transcript.close();
        transcript
    }

    /// The earliest entry whose rows have to be drawn again.
    pub fn dirty_from(&self) -> usize {
        self.dirty_from
    }

    /// Say that everything on screen matches the conversation as it stands.
    pub fn settled(&mut self) {
        self.dirty_from = self.entries.len();
    }

    /// Note that `index` no longer looks the way it was drawn.
    fn touched(&mut self, index: usize) {
        self.dirty_from = self.dirty_from.min(index);
    }

    /// Note that an entry was added, which is a change at the end and nowhere else.
    fn appended(&mut self) {
        self.touched(self.entries.len().saturating_sub(1));
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bumped by every mutation so cached wrapping can be reused between frames.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Usage reported by the most recent assistant message.
    pub fn last_usage(&self) -> Usage {
        self.last_usage
    }

    /// Usage summed over every assistant message in the transcript.
    pub fn total_usage(&self) -> Usage {
        self.total_usage
    }

    /// Model id reported by the most recent assistant message.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Entry indices of every tool result, in the order they appear. These are the entries
    /// focus moves between and the ones that can be expanded.
    pub fn tool_positions(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, Entry::Tool(_)))
            .map(|(index, _)| index)
            .collect()
    }

    /// Open or close one tool result. Returns false when the index is not a tool result.
    pub fn set_expanded(&mut self, index: usize, expanded: bool) -> bool {
        let Some(Entry::Tool(tool)) = self.entries.get_mut(index) else {
            return false;
        };
        if tool.expanded != expanded {
            tool.expanded = expanded;
            self.version += 1;
            self.touched(index);
        }
        true
    }

    /// The most recent answer's text, which is what a copy takes.
    pub fn last_answer(&self) -> Option<String> {
        self.entries.iter().rev().find_map(|entry| match entry {
            Entry::Assistant(assistant) if !assistant.text.trim().is_empty() => {
                Some(assistant.text.clone())
            }
            _ => None,
        })
    }

    /// True when anything that could be opened is still closed.
    ///
    /// What decides which way a global toggle goes: with anything left closed the next
    /// press opens, so a half-open transcript resolves toward open rather than flapping.
    pub fn any_collapsed(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| match entry {
                Entry::Tool(tool) => !tool.expanded,
                Entry::Compaction { expanded, .. } => !expanded,
                _ => false,
            })
    }

    /// Open or close every collapsible entry, which is what ohm's `ctrl+o` does.
    pub fn set_all_expanded(&mut self, expanded: bool) {
        let mut changed = false;
        for entry in &mut self.entries {
            let open = match entry {
                Entry::Tool(tool) => &mut tool.expanded,
                Entry::Compaction { expanded, .. } => expanded,
                _ => continue,
            };
            if *open != expanded {
                *open = expanded;
                changed = true;
            }
        }
        if changed {
            self.version += 1;
            // Opening or closing every result at once changes all of them.
            self.touched(0);
        }
    }

    /// Flip one tool result open or closed, returning its new state.
    pub fn toggle_expanded(&mut self, index: usize) -> Option<bool> {
        let Some(Entry::Tool(tool)) = self.entries.get_mut(index) else {
            return None;
        };
        tool.expanded = !tool.expanded;
        self.version += 1;
        Some(tool.expanded)
    }

    /// Record a summary that replaced a stretch of conversation.
    pub fn push_compaction(&mut self, summary: impl Into<String>) {
        self.entries.push(Entry::Compaction {
            summary: summary.into(),
            expanded: false,
        });
        self.version += 1;
        self.appended();
    }

    pub fn push_user(&mut self, text: impl Into<String>) {
        self.entries.push(Entry::User(text.into()));
        self.version += 1;
        self.appended();
    }

    /// Record a command the user ran themselves, and whether the model was told about it.
    pub fn push_bash(&mut self, command: impl Into<String>, shared: bool) {
        self.entries.push(Entry::Bash {
            command: command.into(),
            shared,
        });
        self.version += 1;
        self.appended();
    }

    /// Show an image the user attached, which is drawn where it was attached rather than
    /// alongside the answer it belongs to.
    pub fn push_image(&mut self, data: impl Into<String>, mime_type: impl Into<String>) {
        self.entries.push(Entry::Image {
            data: data.into(),
            mime_type: mime_type.into(),
        });
        self.version += 1;
        self.appended();
    }

    /// Show something an extension drew.
    pub fn push_custom(&mut self, label: impl Into<String>, lines: Vec<String>) {
        self.entries.push(Entry::Custom {
            label: label.into(),
            lines,
        });
        self.version += 1;
        self.appended();
    }

    pub fn push_notice(&mut self, text: impl Into<String>, level: NoticeLevel) {
        self.entries.push(Entry::Notice {
            text: text.into(),
            level,
        });
        self.version += 1;
        self.appended();
    }

    /// Fold one agent event into the scrollback.
    pub fn apply(&mut self, event: &AgentEvent) {
        // An event that changes nothing on screen leaves the drawing alone: a turn starting
        // or settling is worth knowing about, but there is nothing new to look at.
        if matches!(
            event,
            AgentEvent::AgentStart
                | AgentEvent::TurnStart
                | AgentEvent::TurnEnd { .. }
                | AgentEvent::AgentSettled
        ) {
            return;
        }
        self.version += 1;
        match event {
            // The prompt is echoed the moment it is submitted, and a tool result is already
            // covered by `ToolEnd`, so only assistant messages open an entry here.
            AgentEvent::MessageStart { message } => {
                if matches!(message, Message::Assistant(_)) {
                    self.begin_assistant();
                }
            }
            AgentEvent::MessageDelta { event } => self.apply_delta(event),
            AgentEvent::MessageEnd { message } => match message {
                Message::Assistant(assistant) => self.finish_assistant(assistant),
                // A call left unanswered by an abandoned turn is given a result before the
                // next one starts, outside any turn. It settles a tool still shown as
                // unfinished; one the reader already saw an outcome for keeps it.
                Message::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                    ..
                } => self.answer_unfinished(tool_call_id, &text_of(content), *is_error),
                Message::User { .. } => {}
            },
            AgentEvent::ToolStart {
                id,
                name,
                arguments,
            } => {
                self.tools.insert(id.clone(), self.entries.len());
                self.entries.push(Entry::Tool(ToolEntry {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                    output: None,
                    is_error: false,
                    expanded: false,
                }));
            }
            // What a tool has printed so far replaces what it had printed before, so a
            // long command reads as it runs rather than only once it is over.
            AgentEvent::ToolUpdate { id, name, output } => {
                self.update_tool(id, name, output)
            }
            AgentEvent::ToolEnd {
                id,
                name,
                output,
                is_error,
            } => self.finish_tool(id, name, output, *is_error),
            AgentEvent::Retry {
                attempt,
                max_attempts,
                delay_ms,
            } => self.push_notice(
                format!(
                    "retrying ({attempt}/{max_attempts}) in {}s",
                    delay_ms / 1000
                ),
                NoticeLevel::Warning,
            ),
            // Nothing on screen changes when a turn opens or closes: what a turn produced
            // is already drawn message by message.
            AgentEvent::AgentStart
            | AgentEvent::TurnStart
            | AgentEvent::TurnEnd { .. }
            | AgentEvent::AgentSettled => {}
            AgentEvent::AgentEnd { .. } => self.close(),
        }
    }

    /// Stop the streaming caret, whether the turn ended or was abandoned.
    pub fn close(&mut self) {
        if let Some(index) = self.active.take() {
            if let Some(Entry::Assistant(entry)) = self.entries.get_mut(index) {
                entry.streaming = false;
            }
            self.drop_if_empty(index);
        }
        for entry in &mut self.entries {
            if let Entry::Tool(tool) = entry {
                if tool.output.is_none() {
                    tool.output = Some("interrupted".to_string());
                    tool.is_error = true;
                }
            }
        }
        self.version += 1;
        self.touched(0);
    }

    fn apply_delta(&mut self, event: &StreamEvent) {
        let index = match self.active {
            Some(index) => {
                self.touched(index);
                index
            }
            None => self.begin_assistant(),
        };
        let Some(Entry::Assistant(entry)) = self.entries.get_mut(index) else {
            return;
        };
        match event {
            StreamEvent::TextDelta { delta, .. } => entry.text.push_str(delta),
            StreamEvent::ThinkingDelta { delta, .. } => entry.thinking.push_str(delta),
            _ => {}
        }
    }

    fn begin_assistant(&mut self) -> usize {
        if let Some(index) = self.active {
            return index;
        }
        let index = self.entries.len();
        self.entries.push(Entry::Assistant(AssistantEntry {
            text: String::new(),
            thinking: String::new(),
            streaming: true,
            error: None,
        }));
        self.active = Some(index);
        index
    }

    fn finish_assistant(&mut self, message: &AssistantMessage) {
        let index = match self.active.take() {
            Some(index) => {
                self.touched(index);
                index
            }
            None => {
                self.entries.push(Entry::Assistant(AssistantEntry {
                    text: String::new(),
                    thinking: String::new(),
                    streaming: false,
                    error: None,
                }));
                self.entries.len() - 1
            }
        };

        self.last_usage = message.usage;
        self.total_usage = add(self.total_usage, message.usage);
        if !message.model.is_empty() {
            self.model = Some(message.model.clone());
        }

        if let Some(Entry::Assistant(entry)) = self.entries.get_mut(index) {
            // The final message is authoritative; the streamed copy only fills gaps a
            // provider left, such as a response that failed before any delta arrived.
            let text = message.text();
            if !text.is_empty() {
                entry.text = text;
            }
            let thinking = thinking_of(message);
            if !thinking.is_empty() {
                entry.thinking = thinking;
            }
            entry.streaming = false;
            entry.error = message.error.clone();
        }
        self.drop_if_empty(index);
    }

    /// Settle a tool result that is still waiting, ignoring one that already has an outcome
    /// and one for a call this scrollback never showed.
    fn answer_unfinished(&mut self, id: &str, output: &str, is_error: bool) {
        let Some(index) = self.tools.get(id).copied() else {
            return;
        };
        self.touched(index);
        if let Some(Entry::Tool(tool)) = self.entries.get_mut(index) {
            if tool.output.is_none() {
                tool.output = Some(output.to_string());
                tool.is_error = is_error;
            }
        }
    }

    /// What a tool has produced so far, while it is still running.
    fn update_tool(&mut self, id: &str, name: &str, output: &str) {
        match self.tools.get(id).copied() {
            Some(index) => {
                self.touched(index);
                if let Some(Entry::Tool(tool)) = self.entries.get_mut(index) {
                    tool.output = Some(output.to_string());
                }
            }
            // An update for a call nothing announced still shows: better an entry with no
            // arguments than output nobody can see.
            None => self.finish_tool(id, name, output, false),
        }
    }

    fn finish_tool(&mut self, id: &str, name: &str, output: &str, is_error: bool) {
        match self.tools.get(id).copied() {
            Some(index) => {
                self.touched(index);
                if let Some(Entry::Tool(tool)) = self.entries.get_mut(index) {
                    tool.output = Some(output.to_string());
                    tool.is_error = is_error;
                }
            }
            None => {
                self.tools.insert(id.to_string(), self.entries.len());
                self.entries.push(Entry::Tool(ToolEntry {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: Value::Null,
                    output: Some(output.to_string()),
                    is_error,
                    expanded: false,
                }));
            }
        }
    }

    /// A turn that produced no text, no thinking, and no error leaves nothing to show.
    fn drop_if_empty(&mut self, index: usize) {
        let empty = matches!(
            self.entries.get(index),
            Some(Entry::Assistant(entry))
                if entry.text.is_empty() && entry.thinking.is_empty() && entry.error.is_none()
        );
        if empty && index + 1 == self.entries.len() {
            self.entries.pop();
            // The rows drawn for it have to go with it.
            self.touched(index);
        }
    }

    fn push_message(&mut self, message: &Message) {
        match message {
            Message::User { content, .. } => {
                for block in content {
                    if let ContentBlock::Image { data, mime_type } = block {
                        self.entries.push(Entry::Image {
                            data: data.clone(),
                            mime_type: mime_type.clone(),
                        });
                        self.version += 1;
                        self.appended();
                    }
                }
                let text = text_of(content);
                if text.is_empty() {
                    return;
                }
                // A summary standing in for a compacted stretch is not something the user
                // typed, and is not drawn as though it were.
                match summary_of(&text) {
                    Some(summary) => self.push_compaction(summary),
                    None => self.push_user(text),
                }
            }
            Message::Assistant(assistant) => {
                self.begin_assistant();
                self.finish_assistant(assistant);
                for (id, name, arguments) in assistant.tool_calls() {
                    self.tools.insert(id.to_string(), self.entries.len());
                    self.entries.push(Entry::Tool(ToolEntry {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments: arguments.clone(),
                        output: None,
                        is_error: false,
                        expanded: false,
                    }));
                }
            }
            Message::ToolResult {
                tool_call_id,
                tool_name,
                content,
                is_error,
                ..
            } => self.finish_tool(tool_call_id, tool_name, &text_of(content), *is_error),
        }
        self.version += 1;
    }
}

fn add(left: Usage, right: Usage) -> Usage {
    Usage {
        input: left.input + right.input,
        output: left.output + right.output,
        cache_read: left.cache_read + right.cache_read,
        cache_write: left.cache_write + right.cache_write,
    }
}

fn text_of(content: &[ContentBlock]) -> String {
    content
        .iter()
        .map(ContentBlock::as_text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn thinking_of(message: &AssistantMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The summary inside a compaction message, if that is what this text is.
fn summary_of(text: &str) -> Option<String> {
    let inner = text
        .trim()
        .strip_prefix(micro_context::SUMMARY_OPEN)?
        .strip_suffix(micro_context::SUMMARY_CLOSE)?;
    Some(inner.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::StopReason;
    use serde_json::json;

    fn assistant(text: &str, tool_calls: Vec<(&str, &str)>) -> AssistantMessage {
        let mut content: Vec<ContentBlock> = Vec::new();
        if !text.is_empty() {
            content.push(ContentBlock::text(text));
        }
        for (id, name) in tool_calls {
            content.push(ContentBlock::ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: json!({}),

                signature: None,
            });
        }
        AssistantMessage {
            content,
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
            },
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        }
    }

    fn delta(text: &str) -> AgentEvent {
        AgentEvent::MessageDelta {
            event: StreamEvent::TextDelta {
                index: 0,
                delta: text.to_string(),
            },
        }
    }

    #[test]
    fn streaming_text_accumulates_into_one_entry() {
        let mut transcript = Transcript::new();
        transcript.push_user("hi");
        transcript.apply(&AgentEvent::MessageStart {
            message: Message::Assistant(assistant("", vec![])),
        });
        transcript.apply(&delta("he"));
        transcript.apply(&delta("llo"));

        assert_eq!(transcript.entries().len(), 2);
        let Entry::Assistant(entry) = &transcript.entries()[1] else {
            panic!("expected an assistant entry");
        };
        assert_eq!(entry.text, "hello");
        assert!(entry.streaming);
    }

    #[test]
    fn the_final_message_replaces_the_streamed_text() {
        let mut transcript = Transcript::new();
        transcript.apply(&delta("partial"));
        transcript.apply(&AgentEvent::MessageEnd {
            message: Message::Assistant(assistant("complete answer", vec![])),
        });

        let Entry::Assistant(entry) = &transcript.entries()[0] else {
            panic!("expected an assistant entry");
        };
        assert_eq!(entry.text, "complete answer");
        assert!(!entry.streaming);
        assert_eq!(transcript.last_usage().output, 5);
        assert_eq!(transcript.model(), Some("claude-opus-5"));
    }

    #[test]
    fn thinking_is_kept_apart_from_the_answer() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::MessageDelta {
            event: StreamEvent::ThinkingDelta {
                index: 0,
                delta: "considering".into(),
            },
        });
        transcript.apply(&delta("done"));

        let Entry::Assistant(entry) = &transcript.entries()[0] else {
            panic!("expected an assistant entry");
        };
        assert_eq!(entry.thinking, "considering");
        assert_eq!(entry.text, "done");
    }

    #[test]
    fn a_tool_call_gains_its_output_when_it_finishes() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "read".into(),
            arguments: json!({ "path": "a.txt" }),
        });
        let Entry::Tool(tool) = &transcript.entries()[0] else {
            panic!("expected a tool entry");
        };
        assert!(tool.output.is_none());

        transcript.apply(&AgentEvent::ToolEnd {
            id: "call_1".into(),
            name: "read".into(),
            output: "contents".into(),
            is_error: false,
        });
        let Entry::Tool(tool) = &transcript.entries()[0] else {
            panic!("expected a tool entry");
        };
        assert_eq!(tool.output.as_deref(), Some("contents"));
        assert_eq!(transcript.entries().len(), 1);
    }

    #[test]
    fn an_empty_assistant_turn_leaves_no_entry() {
        let mut transcript = Transcript::new();
        transcript.push_user("go");
        transcript.apply(&AgentEvent::MessageStart {
            message: Message::Assistant(assistant("", vec![])),
        });
        transcript.apply(&AgentEvent::MessageEnd {
            message: Message::Assistant(assistant("", vec![])),
        });
        assert_eq!(transcript.entries().len(), 1);
    }

    #[test]
    fn a_failed_turn_keeps_its_error() {
        let mut transcript = Transcript::new();
        let mut failed = assistant("", vec![]);
        failed.error = Some("Anthropic returned 500".into());
        transcript.apply(&AgentEvent::MessageEnd {
            message: Message::Assistant(failed),
        });

        let Entry::Assistant(entry) = &transcript.entries()[0] else {
            panic!("expected an assistant entry");
        };
        assert_eq!(entry.error.as_deref(), Some("Anthropic returned 500"));
    }

    #[test]
    fn a_retry_shows_as_a_notice() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::Retry {
            attempt: 2,
            max_attempts: 5,
            delay_ms: 2_000,
        });
        assert_eq!(
            transcript.entries()[0],
            Entry::Notice {
                text: "retrying (2/5) in 2s".into(),
                level: NoticeLevel::Warning,
            }
        );
    }

    #[test]
    fn closing_marks_unfinished_tools_as_interrupted() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "bash".into(),
            arguments: json!({ "command": "sleep 60" }),
        });
        transcript.apply(&delta("working"));
        transcript.close();

        let Entry::Tool(tool) = &transcript.entries()[0] else {
            panic!("expected a tool entry");
        };
        assert!(tool.is_error);
        assert_eq!(tool.output.as_deref(), Some("interrupted"));
        let Entry::Assistant(entry) = &transcript.entries()[1] else {
            panic!("expected an assistant entry");
        };
        assert!(!entry.streaming);
    }

    #[test]
    fn a_stored_conversation_rebuilds_in_order() {
        let messages = vec![
            Message::user("read a.txt"),
            Message::Assistant(assistant("on it", vec![("call_1", "read")])),
            Message::tool_result("call_1", "read", "file contents", false),
            Message::Assistant(assistant("here it is", vec![])),
        ];
        let transcript = Transcript::from_messages(&messages);

        assert_eq!(transcript.entries().len(), 4);
        assert_eq!(transcript.entries()[0], Entry::User("read a.txt".into()));
        let Entry::Tool(tool) = &transcript.entries()[2] else {
            panic!("expected a tool entry");
        };
        assert_eq!(tool.name, "read");
        assert_eq!(tool.output.as_deref(), Some("file contents"));
        assert_eq!(transcript.total_usage().input, 20);
    }

    /// The agent settles calls an abandoned turn left open before the next turn starts, so
    /// these arrive outside any turn, ahead of `AgentStart`.
    fn repair(id: &str, text: &str) -> AgentEvent {
        AgentEvent::MessageEnd {
            message: Message::tool_result(id, "bash", text, true),
        }
    }

    #[test]
    fn a_repair_settles_a_call_the_scrollback_still_shows_as_running() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "bash".into(),
            arguments: json!({ "command": "sleep 60" }),
        });

        transcript.apply(&repair("call_1", "abandoned"));

        let Entry::Tool(tool) = &transcript.entries()[0] else {
            panic!("expected a tool entry");
        };
        assert_eq!(tool.output.as_deref(), Some("abandoned"));
        assert!(tool.is_error);
        assert_eq!(transcript.entries().len(), 1, "no entry was invented");
    }

    #[test]
    fn a_repair_leaves_an_outcome_the_reader_already_saw() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "bash".into(),
            arguments: json!({ "command": "sleep 60" }),
        });
        transcript.close();

        transcript.apply(&repair("call_1", "abandoned"));

        let Entry::Tool(tool) = &transcript.entries()[0] else {
            panic!("expected a tool entry");
        };
        assert_eq!(tool.output.as_deref(), Some("interrupted"));
    }

    #[test]
    fn a_repair_for_an_unknown_call_is_ignored() {
        let mut transcript = Transcript::new();
        transcript.push_user("go");
        transcript.apply(&repair("call_from_a_compacted_turn", "abandoned"));
        assert_eq!(transcript.entries().len(), 1);
    }

    #[test]
    fn a_normal_tool_result_does_not_disturb_what_tool_end_recorded() {
        let mut transcript = Transcript::new();
        transcript.apply(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "bash".into(),
            arguments: json!({ "command": "true" }),
        });
        transcript.apply(&AgentEvent::ToolEnd {
            id: "call_1".into(),
            name: "bash".into(),
            output: "done".into(),
            is_error: false,
        });
        // The agent reports every tool result as a message as well as an event.
        transcript.apply(&AgentEvent::MessageEnd {
            message: Message::tool_result("call_1", "bash", "done", false),
        });

        let Entry::Tool(tool) = &transcript.entries()[0] else {
            panic!("expected a tool entry");
        };
        assert_eq!(tool.output.as_deref(), Some("done"));
        assert!(!tool.is_error);
        assert_eq!(transcript.entries().len(), 1);
    }

    #[test]
    fn a_stored_conversation_settles_a_call_it_left_open() {
        let messages = vec![
            Message::user("run it"),
            Message::Assistant(assistant("on it", vec![("call_1", "bash")])),
        ];
        let transcript = Transcript::from_messages(&messages);

        let Entry::Tool(tool) = &transcript.entries()[2] else {
            panic!("expected a tool entry");
        };
        assert_eq!(tool.output.as_deref(), Some("interrupted"));
        assert!(tool.is_error);
    }

    #[test]
    fn tool_results_can_be_opened_and_closed() {
        let mut transcript = Transcript::new();
        transcript.push_user("go");
        transcript.apply(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "read".into(),
            arguments: json!({ "path": "a.rs" }),
        });

        assert_eq!(transcript.tool_positions(), vec![1]);
        assert_eq!(transcript.toggle_expanded(1), Some(true));
        assert_eq!(transcript.toggle_expanded(1), Some(false));
        assert_eq!(
            transcript.toggle_expanded(0),
            None,
            "a prompt cannot expand"
        );

        let before = transcript.version();
        assert!(transcript.set_expanded(1, true));
        assert!(transcript.version() > before);
    }

    #[test]
    fn every_mutation_bumps_the_version() {
        let mut transcript = Transcript::new();
        let before = transcript.version();
        transcript.push_user("hi");
        assert!(transcript.version() > before);
    }
}
