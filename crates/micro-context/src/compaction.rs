//! Replacing older conversation with a summary once it threatens the context window.

use crate::ContextError;
use crate::Result;
use async_trait::async_trait;
use micro_types::now_ms;
use micro_types::CompactionCost;
use micro_types::ContentBlock;
use micro_types::Message;
use micro_types::StopReason;
use std::collections::HashSet;
use std::sync::Arc;

/// Characters per token.
pub const CHARS_PER_TOKEN: usize = 4;


const IMAGE_CHARS: usize = 4_800;

/// Wraps the summary so it stays recognisable after a round trip through session storage.
pub const SUMMARY_OPEN: &str = "<compaction-summary>";
pub const SUMMARY_CLOSE: &str = "</compaction-summary>";

/// The instruction handed to a summarizer, kept here so every implementation asks for the same
/// shape of summary.
pub const COMPACTION_PROMPT: &str = "\
Summarize the conversation above so it can be continued without the original transcript.

Use this template:

## Goal
[What is the user trying to accomplish?]

## Instructions
[Instructions the user gave that still apply]

## Discoveries
[What was learned, including anything that turned out to be wrong]

## Accomplished
[Work finished, work in progress, work still to do]

## Relevant files
[Files read, edited, or created, with what matters about each]";

/// A summary, and what asking for it cost.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub text: String,
    pub cost: CompactionCost,
}

impl Summary {
    /// A summary that cost nothing, for a summarizer with no request behind it.
    pub fn text(text: impl Into<String>) -> Self {
        Summary {
            text: text.into(),
            cost: CompactionCost::default(),
        }
    }
}

/// Produces the summary that replaces the older half of a conversation.
#[async_trait]
pub trait Summarizer: Send + Sync {
    async fn summarize(&self, messages: &[Message]) -> Result<Summary>;
}

#[async_trait]
impl<S: Summarizer + ?Sized> Summarizer for Arc<S> {
    async fn summarize(&self, messages: &[Message]) -> Result<Summary> {
        (**self).summarize(messages).await
    }
}

/// When compaction fires and how much of the conversation survives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactionConfig {
    /// Share of the context window that triggers compaction.
    pub trigger_fraction: f64,
    /// Share of the context window kept verbatim afterwards.
    pub keep_recent_fraction: f64,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        CompactionConfig {
            trigger_fraction: 0.8,
            keep_recent_fraction: 0.3,
        }
    }
}

impl CompactionConfig {
    pub fn new(trigger_fraction: f64, keep_recent_fraction: f64) -> Result<Self> {
        if !(trigger_fraction > 0.0 && trigger_fraction <= 1.0) {
            return Err(ContextError::InvalidConfig(format!(
                "trigger_fraction must be within (0, 1], got {trigger_fraction}"
            )));
        }
        if !(keep_recent_fraction > 0.0 && keep_recent_fraction < 1.0) {
            return Err(ContextError::InvalidConfig(format!(
                "keep_recent_fraction must be within (0, 1), got {keep_recent_fraction}"
            )));
        }
        if keep_recent_fraction >= trigger_fraction {
            return Err(ContextError::InvalidConfig(format!(
                "keep_recent_fraction {keep_recent_fraction} must be below trigger_fraction \
                 {trigger_fraction}, or compaction cannot shrink the context"
            )));
        }
        Ok(CompactionConfig {
            trigger_fraction,
            keep_recent_fraction,
        })
    }

    pub fn trigger_tokens(&self, context_window: usize) -> usize {
        (context_window as f64 * self.trigger_fraction) as usize
    }

    pub fn keep_recent_tokens(&self, context_window: usize) -> usize {
        (context_window as f64 * self.keep_recent_fraction) as usize
    }
}

/// What a compaction produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Compacted {
    /// The conversation to continue with: the summary followed by the kept messages.
    pub messages: Vec<Message>,
    pub summary: String,
    /// What writing the summary cost, for whoever is accounting for the session.
    pub cost: CompactionCost,
    
    pub replaced: usize,
    pub tokens_before: usize,
    pub tokens_after: usize,
}

/// Compacts a conversation using a caller-supplied [`Summarizer`].
pub struct Compactor<S> {
    summarizer: S,
    config: CompactionConfig,
}

impl<S: Summarizer> Compactor<S> {
    pub fn new(summarizer: S, config: CompactionConfig) -> Self {
        Compactor { summarizer, config }
    }

    pub fn config(&self) -> &CompactionConfig {
        &self.config
    }

    pub fn should_compact(&self, messages: &[Message], context_window: usize) -> bool {
        estimate_context_tokens(messages) > self.config.trigger_tokens(context_window)
    }

    /// Summarizes everything before the cut point and returns the conversation to continue with.
    pub async fn compact(&self, messages: &[Message], context_window: usize) -> Result<Compacted> {
        let cut = find_cut(messages, self.config.keep_recent_tokens(context_window));
        if cut == 0 {
            return Err(ContextError::NothingToCompact);
        }

        let tokens_before = estimate_context_tokens(messages);
        let summary = self.summarizer.summarize(&messages[..cut]).await?;

        let mut compacted = Vec::with_capacity(messages.len() - cut + 1);
        compacted.push(summary_message(&summary.text));
        compacted.extend_from_slice(&messages[cut..]);

        Ok(Compacted {
            tokens_after: estimate_tokens(&compacted),
            messages: compacted,
            summary: summary.text,
            cost: summary.cost,
            replaced: cut,
            tokens_before,
        })
    }

    /// Compacts only when the conversation has grown past the trigger.
    pub async fn compact_if_needed(
        &self,
        messages: &[Message],
        context_window: usize,
    ) -> Result<Option<Compacted>> {
        if !self.should_compact(messages, context_window) {
            return Ok(None);
        }
        match self.compact(messages, context_window).await {
            Ok(compacted) => Ok(Some(compacted)),
            Err(ContextError::NothingToCompact) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// The index the kept history starts at.
pub fn find_cut(messages: &[Message], keep_recent_tokens: usize) -> usize {
    let mut orphans: HashSet<&str> = HashSet::new();
    let mut kept = 0usize;
    let mut cut = messages.len();

    for index in (0..messages.len()).rev() {
        let estimate = estimate_message(&messages[index]);
        
        if kept + estimate > keep_recent_tokens && orphans.is_empty() && cut < messages.len() {
            break;
        }

        match &messages[index] {
            Message::Assistant(assistant) => {
                for (id, ..) in assistant.tool_calls() {
                    orphans.remove(id);
                }
            }
            Message::ToolResult { tool_call_id, .. } => {
                orphans.insert(tool_call_id.as_str());
            }
            Message::User { .. } => {}
        }

        kept += estimate;
        cut = index;
    }

    cut
}

/// Whether a conversation can be sent as it stands: every tool result it holds is answered by a
/// tool call that appears before it.
pub fn is_self_contained(messages: &[Message]) -> bool {
    let mut called: HashSet<&str> = HashSet::new();
    for message in messages {
        match message {
            Message::Assistant(assistant) => {
                for (id, ..) in assistant.tool_calls() {
                    called.insert(id);
                }
            }
            Message::ToolResult { tool_call_id, .. } => {
                if !called.contains(tool_call_id.as_str()) {
                    return false;
                }
            }
            Message::User { .. } => {}
        }
    }
    true
}

/// The characters-per-token estimate for a whole conversation.
pub fn estimate_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message).sum()
}

/// The characters-per-token estimate for one message, rounded up: part of a token still occupies a
/// token.
pub fn estimate_message(message: &Message) -> usize {
    let characters = match message {
        Message::User { content, .. } => content_chars(content),
        Message::Assistant(assistant) => content_chars(&assistant.content),
        Message::ToolResult {
            tool_name, content, ..
        } => tool_name.len() + content_chars(content),
    };
    characters.div_ceil(CHARS_PER_TOKEN)
}

/// What the next request will carry, in the most accurate form available.
pub fn estimate_context_tokens(messages: &[Message]) -> usize {
    for (index, message) in messages.iter().enumerate().rev() {
        let Message::Assistant(assistant) = message else {
            continue;
        };
        
        if matches!(
            assistant.stop_reason,
            StopReason::Error | StopReason::Aborted
        ) {
            continue;
        }
        let reported = assistant.usage.total_tokens() as usize;
        if reported == 0 {
            continue;
        }
        return reported + estimate_tokens(&messages[index + 1..]);
    }
    estimate_tokens(messages)
}

fn content_chars(content: &[ContentBlock]) -> usize {
    content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => text.len(),
            ContentBlock::Thinking { thinking, .. } => thinking.len(),
            ContentBlock::RedactedThinking { data } => data.len(),
            ContentBlock::Image { .. } => IMAGE_CHARS,
            
            ContentBlock::ToolCall {
                name,
                arguments,
                signature,
                ..
            } => {
                name.len() + arguments.to_string().len() + signature.as_ref().map_or(0, String::len)
            }
        })
        .sum()
}

/// Builds the message that stands in for everything a compaction replaced.
pub fn summary_message(summary: &str) -> Message {
    Message::User {
        content: vec![ContentBlock::text(format!(
            "{SUMMARY_OPEN}\n{}\n{SUMMARY_CLOSE}",
            summary.trim()
        ))],
        timestamp: now_ms(),
    }
}

/// The summary a message carries, or nothing if it is an ordinary message.
pub fn summary_text(message: &Message) -> Option<&str> {
    let Message::User { content, .. } = message else {
        return None;
    };
    let ContentBlock::Text { text } = content.first()? else {
        return None;
    };
    let body = text.trim().strip_prefix(SUMMARY_OPEN)?;
    Some(body.strip_suffix(SUMMARY_CLOSE)?.trim())
}

pub fn is_summary(message: &Message) -> bool {
    summary_text(message).is_some()
}

/// Flattens a conversation into the transcript a summarizer reads.
pub fn render_transcript(messages: &[Message]) -> String {
    const ARGUMENT_PREVIEW: usize = 200;
    const RESULT_PREVIEW: usize = 300;

    let mut transcript = String::new();
    for message in messages {
        if let Some(summary) = summary_text(message) {
            transcript.push_str("[earlier summary]\n");
            transcript.push_str(summary);
            transcript.push_str("\n\n");
            continue;
        }

        match message {
            Message::User { content, .. } => {
                transcript.push_str("[user]\n");
                transcript.push_str(&text_of(content));
            }
            Message::Assistant(assistant) => {
                transcript.push_str("[assistant]\n");
                transcript.push_str(&text_of(&assistant.content));
                for (_, name, arguments) in assistant.tool_calls() {
                    transcript.push_str(&format!(
                        "\n  calls {name}({})",
                        clip(&arguments.to_string(), ARGUMENT_PREVIEW)
                    ));
                }
            }
            Message::ToolResult {
                tool_name,
                content,
                is_error,
                ..
            } => {
                let outcome = if *is_error { "failed" } else { "ok" };
                transcript.push_str(&format!("[{tool_name} {outcome}]\n"));
                transcript.push_str(&clip(&text_of(content), RESULT_PREVIEW));
            }
        }
        transcript.push_str("\n\n");
    }
    transcript
}

fn text_of(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::AssistantMessage;
    use micro_types::Usage;
    use serde_json::json;

    struct Canned(&'static str);

    #[async_trait]
    impl Summarizer for Canned {
        async fn summarize(&self, _messages: &[Message]) -> Result<Summary> {
            Ok(Summary::text(self.0))
        }
    }

    struct Failing;

    #[async_trait]
    impl Summarizer for Failing {
        async fn summarize(&self, _messages: &[Message]) -> Result<Summary> {
            Err(ContextError::summarizer("the provider said no"))
        }
    }

    /// Records the messages it was handed so a test can inspect what got summarized.
    struct Recording(std::sync::Mutex<Vec<Message>>);

    #[async_trait]
    impl Summarizer for Recording {
        async fn summarize(&self, messages: &[Message]) -> Result<Summary> {
            *self.0.lock().unwrap() = messages.to_vec();
            Ok(Summary::text("summary"))
        }
    }

    fn user(text: &str) -> Message {
        Message::User {
            content: vec![ContentBlock::text(text)],
            timestamp: 0,
        }
    }

    fn assistant(text: &str) -> Message {
        Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text(text)],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        })
    }

    fn calls(ids: &[&str]) -> Message {
        Message::Assistant(AssistantMessage {
            content: ids
                .iter()
                .map(|id| ContentBlock::ToolCall {
                    id: (*id).into(),
                    name: "read".into(),
                    arguments: json!({ "path": "a.txt" }),

                    signature: None,
                })
                .collect(),
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error: None,
            timestamp: 0,
        })
    }

    fn result(id: &str, text: &str) -> Message {
        Message::tool_result(id, "read", text, false)
    }

    /// One user turn answered by a tool call, its result, and a reply.
    fn turn(index: usize, padding: usize) -> Vec<Message> {
        let id = format!("call_{index}");
        vec![
            user(&"u".repeat(padding)),
            calls(&[&id]),
            result(&id, &"r".repeat(padding)),
            assistant(&"a".repeat(padding)),
        ]
    }

    fn conversation(turns: usize, padding: usize) -> Vec<Message> {
        (0..turns).flat_map(|index| turn(index, padding)).collect()
    }

    #[test]
    fn the_estimate_divides_characters_by_the_documented_ratio() {
        assert_eq!(estimate_message(&user(&"a".repeat(400))), 100);
        
        assert_eq!(estimate_message(&user("abcde")), 2);
        assert_eq!(estimate_message(&user("")), 0);
    }

    #[test]
    fn an_image_counts_as_a_fixed_cost() {
        let message = Message::User {
            content: vec![ContentBlock::Image {
                data: "tiny".into(),
                mime_type: "image/png".into(),
            }],
            timestamp: 0,
        };
        assert_eq!(estimate_message(&message), IMAGE_CHARS / CHARS_PER_TOKEN);
    }

    #[test]
    fn a_tool_call_counts_its_name_and_arguments() {
        let message = calls(&["call_1"]);
        let expected =
            ("read".len() + json!({ "path": "a.txt" }).to_string().len()).div_ceil(CHARS_PER_TOKEN);
        assert_eq!(estimate_message(&message), expected);
    }

    #[test]
    fn a_whole_conversation_is_the_sum_of_its_messages() {
        let messages = conversation(3, 100);
        let expected: usize = messages.iter().map(estimate_message).sum();
        assert_eq!(estimate_tokens(&messages), expected);
    }

    #[test]
    fn reported_usage_anchors_the_context_estimate() {
        let mut messages = conversation(2, 100);
        let anchored = Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text("done")],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage {
                input: 5_000,
                output: 200,
                cache_read: 1_000,
                cache_write: 0,
            },
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        });
        messages.push(anchored);
        messages.push(user("and one more"));

        let trailing = estimate_message(&user("and one more"));
        assert_eq!(estimate_context_tokens(&messages), 6_200 + trailing);
    }

    #[test]
    fn a_failed_turn_does_not_anchor_the_estimate() {
        let mut messages = vec![user("hello")];
        messages.push(Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text("boom")],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage {
                input: 900_000,
                ..Usage::default()
            },
            stop_reason: StopReason::Error,
            error: Some("boom".into()),
            timestamp: 0,
        }));
        assert_eq!(
            estimate_context_tokens(&messages),
            estimate_tokens(&messages)
        );
    }

    #[test]
    fn compaction_triggers_on_a_fraction_of_the_window() {
        let compactor = Compactor::new(Canned("s"), CompactionConfig::default());
        
        let messages = vec![user(&"a".repeat(4_000))];

        assert!(!compactor.should_compact(&messages, 2_000));
        assert!(compactor.should_compact(&messages, 1_200));
    }

    #[test]
    fn the_trigger_fraction_is_configurable() {
        
        let messages = vec![user(&"a".repeat(4_000))];
        let eager = Compactor::new(Canned("s"), CompactionConfig::new(0.04, 0.02).unwrap());
        let patient = Compactor::new(Canned("s"), CompactionConfig::new(0.9, 0.3).unwrap());

        assert!(eager.should_compact(&messages, 20_000));
        assert!(!patient.should_compact(&messages, 20_000));
    }

    #[test]
    fn a_config_that_cannot_shrink_the_context_is_rejected() {
        assert!(CompactionConfig::new(0.5, 0.6).is_err());
        assert!(CompactionConfig::new(0.5, 0.5).is_err());
        assert!(CompactionConfig::new(0.0, 0.1).is_err());
        assert!(CompactionConfig::new(1.5, 0.1).is_err());
        assert!(CompactionConfig::new(0.8, 0.0).is_err());
        assert!(CompactionConfig::new(0.8, 0.3).is_ok());
    }

    #[test]
    fn the_cut_never_lands_between_a_tool_call_and_its_result() {
        let messages = vec![
            user("start"),
            calls(&["call_1"]),
            result("call_1", &"r".repeat(4_000)),
            assistant("done"),
        ];

        
        let cut = find_cut(&messages, 1_005);
        assert_eq!(
            cut, 1,
            "the cut must include the assistant that made the call"
        );
        assert!(is_self_contained(&messages[cut..]));
    }

    #[test]
    fn a_cut_inside_a_parallel_tool_batch_moves_past_the_whole_batch() {
        let messages = vec![
            user("start"),
            calls(&["call_1", "call_2", "call_3"]),
            result("call_1", &"r".repeat(2_000)),
            result("call_2", &"r".repeat(2_000)),
            result("call_3", &"r".repeat(2_000)),
            assistant("done"),
        ];

        let cut = find_cut(&messages, 600);
        assert_eq!(cut, 1);
        assert!(is_self_contained(&messages[cut..]));
    }

    #[test]
    fn every_budget_produces_a_self_contained_suffix() {
        let messages = conversation(12, 120);
        for budget in 0..=estimate_tokens(&messages) + 50 {
            let cut = find_cut(&messages, budget);
            assert!(
                is_self_contained(&messages[cut..]),
                "budget {budget} cut at {cut} orphaned a tool result"
            );
        }
    }

    #[test]
    fn at_least_one_message_survives_any_budget() {
        let messages = conversation(4, 200);
        assert_eq!(find_cut(&messages, 0), messages.len() - 1);
    }

    #[test]
    fn a_short_conversation_keeps_everything() {
        let messages = conversation(2, 10);
        assert_eq!(find_cut(&messages, 100_000), 0);
    }

    #[test]
    fn an_empty_conversation_has_nothing_to_cut() {
        assert_eq!(find_cut(&[], 1_000), 0);
    }

    #[test]
    fn an_orphaned_result_is_recognised() {
        assert!(is_self_contained(&[calls(&["a"]), result("a", "ok")]));
        assert!(!is_self_contained(&[result("a", "ok")]));
        assert!(!is_self_contained(&[result("a", "ok"), calls(&["a"])]));
    }

    #[tokio::test]
    async fn compaction_replaces_the_older_half_with_a_summary() {
        let messages = conversation(10, 400);
        let compactor = Compactor::new(Canned("what happened"), CompactionConfig::default());

        let compacted = compactor.compact(&messages, 2_000).await.unwrap();

        assert_eq!(summary_text(&compacted.messages[0]), Some("what happened"));
        assert!(compacted.replaced > 0);
        assert_eq!(
            compacted.messages.len(),
            messages.len() - compacted.replaced + 1
        );
        assert!(compacted.tokens_after < compacted.tokens_before);
        assert!(is_self_contained(&compacted.messages));
    }

    #[tokio::test]
    async fn the_most_recent_messages_survive_verbatim() {
        let messages = conversation(10, 400);
        let compactor = Compactor::new(Canned("s"), CompactionConfig::default());

        let compacted = compactor.compact(&messages, 2_000).await.unwrap();

        assert_eq!(&compacted.messages[1..], &messages[compacted.replaced..]);
    }

    #[tokio::test]
    async fn the_summarized_half_carries_whole_tool_exchanges() {
        let messages = conversation(10, 400);
        let recording = Recording(std::sync::Mutex::new(Vec::new()));
        let compactor = Compactor::new(recording, CompactionConfig::default());

        let compacted = compactor.compact(&messages, 2_000).await.unwrap();
        let summarized = compactor.summarizer.0.lock().unwrap().clone();

        assert_eq!(summarized, messages[..compacted.replaced]);
        
        assert!(is_self_contained(&summarized));
        assert!(is_self_contained(&messages[compacted.replaced..]));
    }

    #[tokio::test]
    async fn a_conversation_that_fits_is_not_compacted() {
        let messages = conversation(2, 10);
        let compactor = Compactor::new(Canned("s"), CompactionConfig::default());

        assert!(matches!(
            compactor.compact(&messages, 1_000_000).await.unwrap_err(),
            ContextError::NothingToCompact
        ));
        assert!(compactor
            .compact_if_needed(&messages, 1_000_000)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn compact_if_needed_fires_once_the_trigger_is_crossed() {
        let messages = conversation(10, 400);
        let compactor = Compactor::new(Canned("s"), CompactionConfig::default());

        assert!(compactor
            .compact_if_needed(&messages, 1_000_000)
            .await
            .unwrap()
            .is_none());
        assert!(compactor
            .compact_if_needed(&messages, 2_000)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn a_summarizer_failure_is_reported() {
        let messages = conversation(10, 400);
        let compactor = Compactor::new(Failing, CompactionConfig::default());

        let error = compactor.compact(&messages, 2_000).await.unwrap_err();
        assert!(error.to_string().contains("the provider said no"));
    }

    #[tokio::test]
    async fn compacting_twice_folds_the_earlier_summary_in() {
        let messages = conversation(10, 400);
        let compactor = Compactor::new(Canned("first pass"), CompactionConfig::default());

        let once = compactor.compact(&messages, 2_000).await.unwrap();
        let mut grown = once.messages.clone();
        grown.extend(conversation(10, 400));

        let recording = Recording(std::sync::Mutex::new(Vec::new()));
        let second = Compactor::new(recording, CompactionConfig::default());
        let twice = second.compact(&grown, 2_000).await.unwrap();

        let summarized = second.summarizer.0.lock().unwrap().clone();
        assert!(summarized.iter().any(is_summary));
        assert_eq!(summary_text(&twice.messages[0]), Some("summary"));
        assert!(is_self_contained(&twice.messages));
    }

    #[test]
    fn a_summary_message_is_distinguishable_from_a_user_message() {
        let summary = summary_message("what happened");
        assert!(is_summary(&summary));
        assert_eq!(summary_text(&summary), Some("what happened"));
        assert!(!is_summary(&user("what happened")));
        assert!(!is_summary(&assistant("what happened")));
    }

    #[test]
    fn a_summary_survives_a_round_trip_through_json() {
        let summary = summary_message("what happened");
        let encoded = serde_json::to_string(&summary).unwrap();
        let decoded: Message = serde_json::from_str(&encoded).unwrap();
        assert_eq!(summary_text(&decoded), Some("what happened"));
    }

    #[test]
    fn a_transcript_names_every_speaker_and_previews_tool_output() {
        let messages = vec![
            user("find the bug"),
            calls(&["call_1"]),
            result("call_1", &"x".repeat(1_000)),
            assistant("found it"),
            summary_message("earlier work"),
        ];

        let transcript = render_transcript(&messages);
        assert!(transcript.contains("[user]\nfind the bug"));
        assert!(transcript.contains("calls read("));
        assert!(transcript.contains("[read ok]"));
        assert!(transcript.contains("[earlier summary]\nearlier work"));
        assert!(transcript.contains('…'), "long output must be previewed");
        assert!(transcript.len() < 1_000);
    }
}
