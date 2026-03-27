use crate::AssistantMessage;
use crate::Message;
use serde::Serialize;
use serde_json::Value;

/// What a provider emits while a response streams in.
///
/// Events carry deltas rather than a rebuilt partial message. Consumers that need the
/// message so far accumulate it themselves, which keeps a provider from cloning the
/// whole message on every token.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// The response has begun; no content has arrived yet.
    Start,
    TextStart {
        index: usize,
    },
    TextDelta {
        index: usize,
        delta: String,
    },
    TextEnd {
        index: usize,
        text: String,
    },
    ThinkingStart {
        index: usize,
    },
    ThinkingDelta {
        index: usize,
        delta: String,
    },
    ThinkingEnd {
        index: usize,
        thinking: String,
    },
    ToolCallStart {
        index: usize,
        id: String,
        name: String,
    },
    /// A fragment of the tool call's JSON arguments, streamed as raw text.
    ToolCallDelta {
        index: usize,
        delta: String,
    },
    ToolCallEnd {
        index: usize,
        id: String,
        name: String,
        arguments: Value,
    },
    /// Terminal: the complete assembled message. Always the last event on success.
    Done {
        message: AssistantMessage,
    },
    /// Terminal: the request failed. Always the last event on failure.
    Error {
        message: String,
    },
}

/// What the agent loop emits to whatever is driving it — a TUI, a headless CLI, or a test.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    TurnStart,
    /// One exchange with the model is over, along with everything it produced.
    TurnEnd {
        messages: Vec<Message>,
    },
    /// A message entered the conversation. Assistant messages emit this before streaming.
    MessageStart {
        message: Message,
    },
    /// A streaming fragment of the assistant message currently being produced.
    MessageDelta {
        event: StreamEvent,
    },
    /// A message is complete and final.
    MessageEnd {
        message: Message,
    },
    ToolStart {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolEnd {
        id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    /// A transient failure is being retried after `delay_ms`.
    Retry {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
    },
    /// A tool has said what it has done so far. `output` is everything it has produced,
    /// not only the newest part, so a consumer never has to accumulate.
    ToolUpdate {
        id: String,
        name: String,
        output: String,
    },
    /// Terminal: every message the loop produced this run.
    AgentEnd {
        messages: Vec<Message>,
    },
    /// The agent has nothing left to do and nothing queued behind it.
    AgentSettled,
}
