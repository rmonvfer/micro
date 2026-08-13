//! The conversation model shared by every layer: content blocks, messages, usage,
//! and the events a provider emits while a response streams in.

mod compat;
mod events;
mod model;

pub use compat::CacheControlFormat;
pub use compat::Compat;
pub use compat::MaxTokensField;
pub use compat::OffLevel;
pub use compat::SessionAffinity;
pub use compat::ThinkingFormat;
pub use events::AgentEvent;
pub use events::StreamEvent;
pub use model::Model;
pub use model::ThinkingLevel;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Milliseconds since the Unix epoch, used to stamp every message.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// A piece of content within a [`Message`]. Providers serialize each variant differently,
/// but the agent layer works with them uniformly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    /// Extended thinking from a reasoning model. The signature is an opaque provider
    /// token that must be replayed verbatim for the block to be accepted next turn.
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// Thinking the provider redacted for safety. Not human-readable, but it must be
    /// preserved and replayed verbatim.
    RedactedThinking {
        data: String,
    },
    Image {
        data: String,
        mime_type: String,
    },
    /// A tool invocation requested by the model. The id is provider-assigned and must be
    /// echoed back in the matching [`Message::ToolResult`].
    ToolCall {
        id: String,
        name: String,
        #[serde(default)]
        arguments: Value,
        /// Provider-issued proof of the reasoning that produced this call. Opaque, and
        /// replayed verbatim or not at all — a provider that issued one validates the
        /// thought against it on the next turn.
        ///
        /// Only Gemini populates this; Anthropic and OpenAI have no equivalent on a tool
        /// call and leave it empty.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text { text: text.into() }
    }

    /// The block's text, or an empty string for blocks that carry none.
    pub fn as_text(&self) -> &str {
        match self {
            ContentBlock::Text { text } => text,
            _ => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
}

impl Usage {
    pub fn total_tokens(&self) -> u32 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// A response from the model. Carries the provider that produced it so thinking
/// signatures can be dropped when a message crosses provider boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
    pub provider: String,
    pub model: String,
    pub usage: Usage,
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub timestamp: i64,
}

impl AssistantMessage {
    /// Every tool call the model requested, in the order it emitted them.
    pub fn tool_calls(&self) -> Vec<(&str, &str, &Value)> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                    ..
                } => Some((id.as_str(), name.as_str(), arguments)),
                _ => None,
            })
            .collect()
    }

    /// Concatenated text of every text block, which is what a headless caller prints.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// A single entry in a conversation. Serialized to JSONL for session persistence and
/// converted to a provider-specific shape before it reaches the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    User {
        content: Vec<ContentBlock>,
        timestamp: i64,
    },
    Assistant(AssistantMessage),
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        content: Vec<ContentBlock>,
        is_error: bool,
        timestamp: i64,
    },
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message::User {
            content: vec![ContentBlock::text(text)],
            timestamp: now_ms(),
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        text: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Message::tool_result_content(
            tool_call_id,
            tool_name,
            vec![ContentBlock::text(text)],
            is_error,
        )
    }

    /// A result the model reads as content rather than as text, which is how a tool hands
    /// back something looked at instead of read.
    pub fn tool_result_content(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: Vec<ContentBlock>,
        is_error: bool,
    ) -> Self {
        Message::ToolResult {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            content,
            is_error,
            timestamp: now_ms(),
        }
    }

    pub fn content(&self) -> &[ContentBlock] {
        match self {
            Message::User { content, .. } => content,
            Message::Assistant(message) => &message.content,
            Message::ToolResult { content, .. } => content,
        }
    }
}

/// A tool as the model sees it: a name, a description, and a JSON Schema for its input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Everything a provider needs to issue one request.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    /// Headers to send along with the request, beyond the ones the provider sets itself.
    /// A header named here replaces the provider's own.
    pub headers: Vec<(String, String)>,
    /// What this conversation is called, for a provider that caches a prompt against it.
    /// Sending the same key for the same conversation is what makes a cache hit possible.
    pub cache_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_blocks_round_trip_through_their_tag() {
        let block = ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({ "path": "a.txt" }),
            signature: None,
        };
        let encoded = serde_json::to_string(&block).unwrap();
        assert!(encoded.contains("\"type\":\"tool_call\""));
        assert_eq!(block, serde_json::from_str(&encoded).unwrap());
    }

    /// A session log written before a tool call could carry a signature has no such key.
    /// Reading one has to keep working: the session store treats a line it cannot parse as
    /// corrupt and skips it, so a missing default would silently drop history a user
    /// recorded rather than fail loudly.
    #[test]
    fn a_tool_call_recorded_without_a_signature_still_loads() {
        let line = r#"{"role":"assistant","content":[{"type":"tool_call","id":"call_1","name":"read","arguments":{"path":"a.txt"}}],"provider":"anthropic","model":"claude-opus-5","usage":{"input":1,"output":2,"cache_read":0,"cache_write":0},"stop_reason":"tool_use","timestamp":1786361585474}"#;

        let decoded: Message = serde_json::from_str(line).expect("an existing log line must load");
        let Message::Assistant(assistant) = &decoded else {
            panic!("expected an assistant message");
        };
        assert_eq!(
            assistant.content,
            vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "read".into(),
                arguments: serde_json::json!({ "path": "a.txt" }),
                signature: None,
            }]
        );
    }

    #[test]
    fn a_call_signature_is_omitted_from_the_log_when_absent() {
        let block = ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({}),
            signature: None,
        };
        assert!(!serde_json::to_string(&block).unwrap().contains("signature"));

        let signed = ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({}),
            signature: Some("sig".into()),
        };
        let encoded = serde_json::to_string(&signed).unwrap();
        assert!(encoded.contains(r#""signature":"sig""#));
        assert_eq!(signed, serde_json::from_str(&encoded).unwrap());
    }

    #[test]
    fn messages_round_trip_through_their_role() {
        let message = Message::tool_result("call_1", "read", "contents", false);
        let encoded = serde_json::to_string(&message).unwrap();
        assert!(encoded.contains("\"role\":\"tool_result\""));
        assert_eq!(message, serde_json::from_str(&encoded).unwrap());
    }

    #[test]
    fn thinking_signature_is_omitted_when_absent() {
        let block = ContentBlock::Thinking {
            thinking: "hmm".into(),
            signature: None,
        };
        let encoded = serde_json::to_string(&block).unwrap();
        assert!(!encoded.contains("signature"));
    }

    #[test]
    fn assistant_message_collects_tool_calls_in_order() {
        let message = AssistantMessage {
            content: vec![
                ContentBlock::text("working"),
                ContentBlock::ToolCall {
                    id: "a".into(),
                    name: "ls".into(),
                    arguments: Value::Null,

                    signature: None,
                },
                ContentBlock::ToolCall {
                    id: "b".into(),
                    name: "read".into(),
                    arguments: Value::Null,

                    signature: None,
                },
            ],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error: None,
            timestamp: 0,
        };
        let ids: Vec<_> = message.tool_calls().iter().map(|(id, ..)| *id).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(message.text(), "working");
    }
}
