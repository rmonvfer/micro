//! Turning what the agent reports into what an extension listens for.
//!
//! pi names its events from the session's point of view — a turn starts, a message ends,
//! a tool finished — and an extension subscribes by that name. micro's agent reports the
//! same moments under its own names, and in its own shapes, so both are translated here, in
//! one place, rather than at every point that emits one.
//!
//! micro's own `Message`/`ContentBlock`/`Usage` are Rust-shaped: `snake_case` fields, and a
//! tool result's role written as `tool_result`. pi's extensions are written against
//! TypeScript's shapes: `camelCase` fields, and a tool result's role written as
//! `toolResult`. An extension checking `message.role === "toolResult"` is not being
//! unreasonable — that is what pi hands it — so the conversion here is not cosmetic: a
//! payload that keeps micro's own field names is one an extension silently fails to read.
//!
//! An event micro has no counterpart for is not invented: it simply never fires, and an
//! extension listening for it waits forever rather than being lied to.

use micro_types::AgentEvent;
use micro_types::ContentBlock;
use micro_types::Message;
use micro_types::StopReason;
use micro_types::StreamEvent;
use micro_types::Usage;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;

/// What an extension calls this event, or nothing when it is not one they can hear.
pub fn name_of(event: &AgentEvent) -> Option<&'static str> {
    match event {
        AgentEvent::AgentStart => Some("agent_start"),
        AgentEvent::AgentEnd { .. } => Some("agent_end"),
        AgentEvent::TurnStart => Some("turn_start"),
        AgentEvent::TurnEnd { .. } => Some("turn_end"),
        AgentEvent::AgentSettled => Some("agent_settled"),
        AgentEvent::MessageStart { .. } => Some("message_start"),
        AgentEvent::MessageDelta { .. } => Some("message_update"),
        AgentEvent::MessageEnd { .. } => Some("message_end"),
        AgentEvent::ToolStart { .. } => Some("tool_execution_start"),
        AgentEvent::ToolUpdate { .. } => Some("tool_execution_update"),
        AgentEvent::ToolEnd { .. } => Some("tool_execution_end"),
        // A retry is micro's own business: the request is repeated, and nothing about the
        // conversation changed.
        AgentEvent::Retry { .. } => None,
    }
}

/// Turns the flat stream of `AgentEvent`s micro's agent loop reports into the payload
/// shapes pi's handlers are written against.
///
/// A free function was enough while every event could be translated from itself alone.
/// Two of pi's events cannot be: `turn_end` wants only the messages produced during that
/// turn, split into the turn's own assistant message and its tool results, while
/// `AgentEvent::TurnEnd` carries every message the whole run has produced so far; and
/// `message_update` wants the assistant message accumulated so far even though micro's
/// stream reports only the newest delta, not a rebuilt whole. Both need to remember
/// something about the events that already went by, which is what this keeps: one
/// `Translator` per run, fed every event in order.
#[derive(Debug, Default)]
pub struct Translator {
    /// How many turns have ended so far in this run — pi calls this `turnIndex`. Reset to
    /// zero on `agent_start` and advanced once each `turn_end` is built.
    turn_index: u32,
    /// How many of `AgentEvent::TurnEnd`'s cumulative messages had already been reported
    /// as of the last turn, so this turn's own messages can be sliced out of the rest.
    reported: usize,
    /// The arguments a tool call was started with, kept by call id until the call ends.
    /// `AgentEvent::ToolUpdate` does not carry them itself, and pi's
    /// `tool_execution_update` wants them anyway, so they are remembered from the call's
    /// `tool_execution_start`.
    tool_arguments: HashMap<String, Value>,
    /// The assistant message being assembled from stream deltas. Reset at the stream's
    /// `start` event and cleared once it reaches `done` or `error`.
    partial: Option<PartialAssistant>,
}

impl Translator {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the event carries, in the shape pi's handlers are written against.
    pub fn payload_of(&mut self, event: &AgentEvent) -> Value {
        match event {
            AgentEvent::AgentStart => {
                self.turn_index = 0;
                self.reported = 0;
                self.tool_arguments.clear();
                self.partial = None;
                json!({})
            }
            AgentEvent::AgentSettled => json!({}),
            AgentEvent::AgentEnd { messages } => json!({
                "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
            }),
            AgentEvent::TurnStart => json!({
                "turnIndex": self.turn_index,
                "timestamp": micro_types::now_ms(),
            }),
            AgentEvent::TurnEnd { messages } => self.turn_end(messages),
            AgentEvent::MessageStart { message } => json!({ "message": message_json(message) }),
            AgentEvent::MessageDelta { event } => self.message_update(event),
            AgentEvent::MessageEnd { message } => json!({ "message": message_json(message) }),
            AgentEvent::ToolStart {
                id,
                name,
                arguments,
            } => {
                self.tool_arguments.insert(id.clone(), arguments.clone());
                json!({ "toolCallId": id, "toolName": name, "args": arguments })
            }
            AgentEvent::ToolUpdate { id, name, output } => json!({
                "toolCallId": id,
                "toolName": name,
                "args": self.tool_arguments.get(id).cloned().unwrap_or_else(|| json!({})),
                "partialResult": tool_result_json(output),
            }),
            AgentEvent::ToolEnd {
                id,
                name,
                output,
                is_error,
            } => {
                self.tool_arguments.remove(id);
                json!({
                    "toolCallId": id,
                    "toolName": name,
                    "result": tool_result_json(output),
                    "isError": is_error,
                })
            }
            AgentEvent::Retry { .. } => json!({}),
        }
    }

    /// `turn_end`'s messages are only the ones this turn added: the assistant's reply, and
    /// whatever tool results followed it. `AgentEvent::TurnEnd` reports the run's entire
    /// output so far, so this turn's share is the part beyond what the last one already
    /// covered.
    fn turn_end(&mut self, messages: &[Message]) -> Value {
        let this_turn = messages.get(self.reported..).unwrap_or(&[]);
        // Exactly one assistant message is produced before this fires, every turn — the
        // fallback below only guards against that invariant ever slipping rather than
        // expecting to be used.
        let message = this_turn
            .iter()
            .find(|message| matches!(message, Message::Assistant(_)))
            .map(message_json)
            .unwrap_or_else(|| json!({ "role": "assistant", "content": [] }));
        let tool_results: Vec<Value> = this_turn
            .iter()
            .filter(|message| matches!(message, Message::ToolResult { .. }))
            .map(message_json)
            .collect();

        let payload = json!({
            "turnIndex": self.turn_index,
            "message": message,
            "toolResults": tool_results,
        });
        self.reported = messages.len();
        self.turn_index += 1;
        payload
    }

    /// `message_update` carries both the raw stream event, translated to pi's shape, and
    /// the message being assembled so far — which for pi is the same accumulated state
    /// the translated event's own `partial` (or, at the end, `message`/`error`) carries.
    fn message_update(&mut self, event: &StreamEvent) -> Value {
        let assistant_message_event = self.stream_event_json(event);
        let message = assistant_message_event
            .get("partial")
            .or_else(|| assistant_message_event.get("message"))
            .or_else(|| assistant_message_event.get("error"))
            .cloned()
            .unwrap_or_else(|| json!({ "role": "assistant", "content": [] }));
        json!({ "message": message, "assistantMessageEvent": assistant_message_event })
    }

    /// `AssistantMessageEvent`, as pi names it: `contentIndex` rather than `index`, and a
    /// `partial` — the assistant message assembled so far — on every event but the two
    /// terminal ones, which carry the finished message instead.
    fn stream_event_json(&mut self, event: &StreamEvent) -> Value {
        match event {
            StreamEvent::Start => {
                self.partial = Some(PartialAssistant::default());
                json!({ "type": "start", "partial": self.partial_message() })
            }
            StreamEvent::TextStart { index } => {
                self.block(*index, || json!({ "type": "text", "text": "" }));
                json!({ "type": "text_start", "contentIndex": index, "partial": self.partial_message() })
            }
            StreamEvent::TextDelta { index, delta } => {
                let block = self.block(*index, || json!({ "type": "text", "text": "" }));
                let joined = format!("{}{delta}", block["text"].as_str().unwrap_or_default());
                block["text"] = json!(joined);
                json!({
                    "type": "text_delta",
                    "contentIndex": index,
                    "delta": delta,
                    "partial": self.partial_message(),
                })
            }
            StreamEvent::TextEnd { index, text } => {
                *self.block(*index, || Value::Null) = json!({ "type": "text", "text": text });
                json!({
                    "type": "text_end",
                    "contentIndex": index,
                    "content": text,
                    "partial": self.partial_message(),
                })
            }
            StreamEvent::ThinkingStart { index } => {
                self.block(*index, || json!({ "type": "thinking", "thinking": "" }));
                json!({ "type": "thinking_start", "contentIndex": index, "partial": self.partial_message() })
            }
            StreamEvent::ThinkingDelta { index, delta } => {
                let block = self.block(*index, || json!({ "type": "thinking", "thinking": "" }));
                let joined = format!("{}{delta}", block["thinking"].as_str().unwrap_or_default());
                block["thinking"] = json!(joined);
                json!({
                    "type": "thinking_delta",
                    "contentIndex": index,
                    "delta": delta,
                    "partial": self.partial_message(),
                })
            }
            StreamEvent::ThinkingEnd { index, thinking } => {
                *self.block(*index, || Value::Null) = json!({ "type": "thinking", "thinking": thinking });
                json!({
                    "type": "thinking_end",
                    "contentIndex": index,
                    "content": thinking,
                    "partial": self.partial_message(),
                })
            }
            StreamEvent::ToolCallStart { index, id, name } => {
                let block = self.block(*index, || {
                    json!({ "type": "toolCall", "id": "", "name": "", "arguments": {} })
                });
                block["id"] = json!(id);
                block["name"] = json!(name);
                json!({ "type": "toolcall_start", "contentIndex": index, "partial": self.partial_message() })
            }
            StreamEvent::ToolCallDelta { index, delta } => {
                // The arguments stream as raw, not-yet-parseable JSON text. The partial
                // keeps the last complete shape — empty, until `ToolCallEnd` fills it in —
                // rather than a value that would not parse.
                self.block(*index, || json!({ "type": "toolCall", "id": "", "name": "", "arguments": {} }));
                json!({
                    "type": "toolcall_delta",
                    "contentIndex": index,
                    "delta": delta,
                    "partial": self.partial_message(),
                })
            }
            StreamEvent::ToolCallEnd {
                index,
                id,
                name,
                arguments,
            } => {
                let tool_call = json!({ "type": "toolCall", "id": id, "name": name, "arguments": arguments });
                *self.block(*index, || Value::Null) = tool_call.clone();
                json!({
                    "type": "toolcall_end",
                    "contentIndex": index,
                    "toolCall": tool_call,
                    "partial": self.partial_message(),
                })
            }
            StreamEvent::Done { message } => {
                self.partial = None;
                json!({
                    "type": "done",
                    "reason": stop_reason_json(message.stop_reason),
                    "message": message_json(&Message::Assistant(message.clone())),
                })
            }
            StreamEvent::Error { message } => {
                // Whatever streamed in before the failure is real, so it is kept; nothing
                // else about a completed assistant message is known, so the rest is left
                // empty rather than invented.
                let content = self
                    .partial
                    .take()
                    .map(|partial| partial.content())
                    .unwrap_or_default();
                json!({
                    "type": "error",
                    "reason": "error",
                    "error": {
                        "role": "assistant",
                        "content": content,
                        "provider": "",
                        "model": "",
                        "usage": usage_json(&Usage::default()),
                        "stopReason": "error",
                        "errorMessage": message,
                        "timestamp": micro_types::now_ms(),
                    },
                })
            }
        }
    }

    /// The content block at this index of the message being streamed, creating it (and the
    /// message, if this is the first block reported) if this is the first event to mention
    /// it.
    fn block(&mut self, index: usize, default: impl FnOnce() -> Value) -> &mut Value {
        self.partial
            .get_or_insert_with(PartialAssistant::default)
            .blocks
            .entry(index)
            .or_insert_with(default)
    }

    /// The assistant message assembled from the stream so far, in pi's shape.
    fn partial_message(&self) -> Value {
        match &self.partial {
            Some(partial) => json!({ "role": "assistant", "content": partial.content() }),
            None => json!({ "role": "assistant", "content": [] }),
        }
    }
}

/// The content blocks of an assistant message being streamed in, keyed by the index the
/// provider assigned them. A map rather than a vector: a block's index is announced once,
/// at its `_start` event, and every later event about it names the same index rather than
/// its position — a `BTreeMap` reads them back out in the order they were assigned.
#[derive(Debug, Default)]
struct PartialAssistant {
    blocks: BTreeMap<usize, Value>,
}

impl PartialAssistant {
    fn content(&self) -> Vec<Value> {
        self.blocks.values().cloned().collect()
    }
}

/// What a tool produced, in the shape pi's tool-execution events carry it.
///
/// pi hands a handler the tool's whole result object rather than its text, so a handler
/// reads the output at `result.content[0].text` and reaches for `details` when a tool
/// offers one. micro's tools return text alone, so the block list has one entry and there
/// are no details to report — but the envelope is what an extension destructures, and one
/// that arrives as a bare string is one every pi-shaped handler misreads.
fn tool_result_json(output: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": output }],
        "details": Value::Null,
    })
}

/// A message in the shape pi's handlers are written against: `camelCase` fields, and a
/// tool result's role spelled `toolResult` rather than micro's own `tool_result`.
pub fn message_json(message: &Message) -> Value {
    match message {
        Message::User { content, timestamp } => json!({
            "role": "user",
            "content": content.iter().map(content_json).collect::<Vec<_>>(),
            "timestamp": timestamp,
        }),
        Message::Assistant(assistant) => json!({
            "role": "assistant",
            "content": assistant.content.iter().map(content_json).collect::<Vec<_>>(),
            "provider": assistant.provider,
            "model": assistant.model,
            "usage": usage_json(&assistant.usage),
            "stopReason": stop_reason_json(assistant.stop_reason),
            "errorMessage": assistant.error,
            "timestamp": assistant.timestamp,
        }),
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            is_error,
            timestamp,
        } => json!({
            "role": "toolResult",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "content": content.iter().map(content_json).collect::<Vec<_>>(),
            "isError": is_error,
            "timestamp": timestamp,
        }),
    }
}

/// A content block in the shape pi's handlers are written against.
///
/// micro keeps redacted thinking as its own variant, carrying only the opaque token a
/// provider must see again; pi folds it into `ThinkingContent` with a `redacted` flag
/// instead, so that is the shape it is written as here.
pub fn content_json(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
        ContentBlock::Thinking { thinking, signature } => {
            let mut value = json!({ "type": "thinking", "thinking": thinking });
            if let Some(signature) = signature {
                value["thinkingSignature"] = json!(signature);
            }
            value
        }
        ContentBlock::RedactedThinking { data } => json!({
            "type": "thinking",
            "thinking": "",
            "thinkingSignature": data,
            "redacted": true,
        }),
        ContentBlock::Image { data, mime_type } => json!({
            "type": "image",
            "data": data,
            "mimeType": mime_type,
        }),
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
            ..
        } => json!({
            "type": "toolCall",
            "id": id,
            "name": name,
            "arguments": arguments,
        }),
    }
}

/// A message as an extension handed it back — for `context`, which may return a rewritten
/// `messages` array, and for `tool_result`, whose `content` extensions write in this same
/// shape. `None` for anything that does not parse as one of pi's three message roles,
/// which a malformed or half-written answer is left as rather than guessed at.
pub fn message_from_json(value: &Value) -> Option<Message> {
    let timestamp = || value.get("timestamp").and_then(Value::as_i64).unwrap_or_else(micro_types::now_ms);
    let content = || -> Vec<ContentBlock> {
        value
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| blocks.iter().filter_map(content_from_json).collect())
            .unwrap_or_default()
    };

    match value.get("role").and_then(Value::as_str)? {
        "user" => Some(Message::User {
            content: content(),
            timestamp: timestamp(),
        }),
        "assistant" => Some(Message::Assistant(micro_types::AssistantMessage {
            content: content(),
            provider: text(value, "provider"),
            model: text(value, "model"),
            usage: usage_from_json(value.get("usage")),
            stop_reason: stop_reason_from_json(value.get("stopReason").and_then(Value::as_str)),
            error: value.get("errorMessage").and_then(Value::as_str).map(str::to_string),
            timestamp: timestamp(),
        })),
        "toolResult" => Some(Message::ToolResult {
            tool_call_id: value.get("toolCallId")?.as_str()?.to_string(),
            tool_name: value.get("toolName")?.as_str()?.to_string(),
            content: content(),
            is_error: value.get("isError").and_then(Value::as_bool).unwrap_or(false),
            timestamp: timestamp(),
        }),
        _ => None,
    }
}

/// A content block as an extension handed it back. The inverse of [`content_json`], and
/// deliberately more forgiving than it: a field an extension left off is defaulted rather
/// than failing the whole block, since a handler rewriting a message is rarely rebuilding
/// every field of every block from scratch.
pub fn content_from_json(value: &Value) -> Option<ContentBlock> {
    match value.get("type").and_then(Value::as_str)? {
        "text" => Some(ContentBlock::Text { text: text(value, "text") }),
        "thinking" if value.get("redacted").and_then(Value::as_bool) == Some(true) => {
            Some(ContentBlock::RedactedThinking {
                data: text(value, "thinkingSignature"),
            })
        }
        "thinking" => Some(ContentBlock::Thinking {
            thinking: text(value, "thinking"),
            signature: value.get("thinkingSignature").and_then(Value::as_str).map(str::to_string),
        }),
        "image" => Some(ContentBlock::Image {
            data: text(value, "data"),
            mime_type: text(value, "mimeType"),
        }),
        "toolCall" => Some(ContentBlock::ToolCall {
            id: text(value, "id"),
            name: text(value, "name"),
            arguments: value.get("arguments").cloned().unwrap_or_else(|| json!({})),
            signature: None,
        }),
        _ => None,
    }
}

fn text(value: &Value, field: &str) -> String {
    value.get(field).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn usage_from_json(value: Option<&Value>) -> Usage {
    let field = |name: &str| {
        value
            .and_then(|value| value.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
    };
    Usage {
        input: field("input"),
        output: field("output"),
        cache_read: field("cacheRead"),
        cache_write: field("cacheWrite"),
    }
}

fn stop_reason_from_json(value: Option<&str>) -> StopReason {
    match value {
        Some("length") => StopReason::Length,
        Some("toolUse") => StopReason::ToolUse,
        Some("error") => StopReason::Error,
        Some("aborted") => StopReason::Aborted,
        // "stop" and anything unrecognized both mean the ordinary case: nothing else
        // about finishing early was said.
        _ => StopReason::Stop,
    }
}

/// Usage in pi's shape.
///
/// pi's `Usage` also carries `cost`, priced from the model's per-token rates. Nothing at
/// this layer knows which model produced a given message — that lives in the catalog, well
/// above where an `AgentEvent` is translated — so `cost` is left off rather than reported
/// as zero, which would read as "this was free" instead of "this was not priced here".
fn usage_json(usage: &Usage) -> Value {
    json!({
        "input": usage.input,
        "output": usage.output,
        "cacheRead": usage.cache_read,
        "cacheWrite": usage.cache_write,
        "totalTokens": usage.total_tokens(),
    })
}

fn stop_reason_json(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Stop => "stop",
        StopReason::Length => "length",
        StopReason::ToolUse => "toolUse",
        StopReason::Error => "error",
        StopReason::Aborted => "aborted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_moments_an_extension_listens_for_are_named_as_pi_names_them() {
        assert_eq!(name_of(&AgentEvent::AgentStart), Some("agent_start"));
        assert_eq!(name_of(&AgentEvent::TurnStart), Some("turn_start"));
        assert_eq!(
            name_of(&AgentEvent::TurnEnd {
                messages: Vec::new()
            }),
            Some("turn_end")
        );
        assert_eq!(name_of(&AgentEvent::AgentSettled), Some("agent_settled"));
        assert_eq!(
            name_of(&AgentEvent::ToolStart {
                id: "call_1".into(),
                name: "read".into(),
                arguments: json!({}),
            }),
            Some("tool_execution_start")
        );
        assert_eq!(
            name_of(&AgentEvent::AgentEnd {
                messages: Vec::new()
            }),
            Some("agent_end")
        );
    }

    /// A retry is not a moment in the conversation, so nothing is told about it.
    #[test]
    fn a_retry_is_not_an_event_an_extension_hears() {
        assert_eq!(
            name_of(&AgentEvent::Retry {
                attempt: 1,
                max_attempts: 3,
                delay_ms: 100,
            }),
            None
        );
    }

    #[test]
    fn a_tool_carries_what_it_was_called_with_and_what_it_returned() {
        let mut translator = Translator::new();
        let started = translator.payload_of(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "read".into(),
            arguments: json!({ "path": "src/main.rs" }),
        });
        assert_eq!(started["toolCallId"], "call_1");
        assert_eq!(started["toolName"], "read");
        assert_eq!(started["args"]["path"], "src/main.rs");

        let ended = translator.payload_of(&AgentEvent::ToolEnd {
            id: "call_1".into(),
            name: "read".into(),
            output: "fn main() {}".into(),
            is_error: false,
        });
        assert_eq!(ended["result"]["content"][0]["type"], "text");
        assert_eq!(ended["result"]["content"][0]["text"], "fn main() {}");
        assert_eq!(ended["isError"], false);
    }

    /// A handler reads a tool's output off the result object pi hands it, so the output
    /// travels inside a block list rather than as the field itself.
    #[test]
    fn a_tool_result_is_shaped_the_way_a_handler_destructures_it() {
        let mut translator = Translator::new();
        let ended = translator.payload_of(&AgentEvent::ToolEnd {
            id: "call_1".into(),
            name: "read".into(),
            output: "fn main() {}".into(),
            is_error: false,
        });
        assert!(ended["result"]["content"].is_array());
        assert!(ended["result"].get("details").is_some());
        assert!(!ended["result"].is_string());
    }

    /// `tool_execution_update` cannot see the call's own arguments — `AgentEvent::ToolUpdate`
    /// does not carry them — so they have to be remembered from the call's start.
    #[test]
    fn a_tool_update_carries_the_arguments_it_was_started_with() {
        let mut translator = Translator::new();
        translator.payload_of(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "bash".into(),
            arguments: json!({ "command": "ls" }),
        });
        let update = translator.payload_of(&AgentEvent::ToolUpdate {
            id: "call_1".into(),
            name: "bash".into(),
            output: "src\n".into(),
        });
        assert_eq!(update["args"]["command"], "ls");
        assert_eq!(update["partialResult"]["content"][0]["text"], "src\n");
    }

    #[test]
    fn a_message_carries_the_message() {
        let mut translator = Translator::new();
        let payload = translator.payload_of(&AgentEvent::MessageStart {
            message: micro_types::Message::user("hello"),
        });
        assert_eq!(payload["message"]["role"], "user");
    }

    /// pi reads a tool result's role as `toolResult`; micro's own serialization would
    /// write it `tool_result`, which is silently a different string to a `===` check.
    #[test]
    fn a_tool_results_role_is_spelled_the_way_pi_spells_it() {
        let message = Message::ToolResult {
            tool_call_id: "call_1".into(),
            tool_name: "read".into(),
            content: vec![ContentBlock::text("contents")],
            is_error: false,
            timestamp: 0,
        };
        let json = message_json(&message);
        assert_eq!(json["role"], "toolResult");
        assert_eq!(json["toolCallId"], "call_1");
        assert_eq!(json["toolName"], "read");
    }

    /// An assistant message's fields are written the way pi's `AssistantMessage` names
    /// them, not the way micro's own `AssistantMessage` does.
    #[test]
    fn an_assistant_messages_fields_are_camel_cased() {
        let assistant = micro_types::AssistantMessage {
            content: vec![ContentBlock::text("hi")],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage {
                input: 10,
                output: 5,
                cache_read: 2,
                cache_write: 1,
            },
            stop_reason: StopReason::ToolUse,
            error: None,
            timestamp: 123,
        };
        let json = message_json(&Message::Assistant(assistant));
        assert_eq!(json["stopReason"], "toolUse");
        assert_eq!(json["usage"]["cacheRead"], 2);
        assert_eq!(json["usage"]["cacheWrite"], 1);
        assert_eq!(json["usage"]["totalTokens"], 18);
    }

    /// `turn_end` reports only the messages this turn added, split into the assistant's
    /// reply and the tool results that followed it — not the whole run's output so far,
    /// which is what `AgentEvent::TurnEnd` itself carries.
    #[test]
    fn turn_end_reports_only_this_turns_messages_split_by_kind() {
        let mut translator = Translator::new();
        translator.payload_of(&AgentEvent::AgentStart);
        translator.payload_of(&AgentEvent::TurnStart);

        let first_reply = Message::Assistant(micro_types::AssistantMessage {
            content: vec![ContentBlock::text("running a tool")],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error: None,
            timestamp: 1,
        });
        let tool_result = Message::ToolResult {
            tool_call_id: "call_1".into(),
            tool_name: "read".into(),
            content: vec![ContentBlock::text("file contents")],
            is_error: false,
            timestamp: 2,
        };
        let first_turn = translator.payload_of(&AgentEvent::TurnEnd {
            messages: vec![first_reply.clone(), tool_result.clone()],
        });
        assert_eq!(first_turn["turnIndex"], 0);
        assert_eq!(first_turn["message"]["content"][0]["text"], "running a tool");
        assert_eq!(first_turn["toolResults"].as_array().unwrap().len(), 1);
        assert_eq!(first_turn["toolResults"][0]["toolCallId"], "call_1");

        translator.payload_of(&AgentEvent::TurnStart);
        let second_reply = Message::Assistant(micro_types::AssistantMessage {
            content: vec![ContentBlock::text("done")],
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 3,
        });
        let second_turn = translator.payload_of(&AgentEvent::TurnEnd {
            messages: vec![first_reply, tool_result, second_reply],
        });
        assert_eq!(second_turn["turnIndex"], 1);
        assert_eq!(second_turn["message"]["content"][0]["text"], "done");
        assert_eq!(
            second_turn["toolResults"].as_array().unwrap().len(),
            0,
            "the first turn's tool result was already reported and is not repeated"
        );
    }

    /// A streamed message is assembled from its deltas, so `message_update` can hand back
    /// the whole thing so far even though each `StreamEvent` only carries the newest piece.
    #[test]
    fn a_streamed_message_is_assembled_from_its_deltas() {
        let mut translator = Translator::new();
        translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::Start,
        });
        translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::TextStart { index: 0 },
        });
        translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::TextDelta {
                index: 0,
                delta: "Hel".into(),
            },
        });
        let after_second_delta = translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::TextDelta {
                index: 0,
                delta: "lo".into(),
            },
        });
        assert_eq!(after_second_delta["message"]["content"][0]["text"], "Hello");
        assert_eq!(
            after_second_delta["assistantMessageEvent"]["type"],
            "text_delta"
        );
        assert_eq!(after_second_delta["assistantMessageEvent"]["contentIndex"], 0);
        assert_eq!(after_second_delta["assistantMessageEvent"]["delta"], "lo");

        let ended = translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::TextEnd {
                index: 0,
                text: "Hello".into(),
            },
        });
        assert_eq!(ended["message"]["content"][0]["text"], "Hello");

        let done = translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::Done {
                message: micro_types::AssistantMessage {
                    content: vec![ContentBlock::text("Hello")],
                    provider: "anthropic".into(),
                    model: "claude-opus-5".into(),
                    usage: Usage::default(),
                    stop_reason: StopReason::Stop,
                    error: None,
                    timestamp: 4,
                },
            },
        });
        assert_eq!(done["assistantMessageEvent"]["type"], "done");
        assert_eq!(done["assistantMessageEvent"]["reason"], "stop");
        assert_eq!(done["message"]["stopReason"], "stop");
    }

    /// A stream that fails partway keeps what it had already produced, rather than losing
    /// it or inventing what the rest would have said.
    #[test]
    fn a_failed_stream_keeps_what_it_had_produced() {
        let mut translator = Translator::new();
        translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::Start,
        });
        translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::TextStart { index: 0 },
        });
        translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::TextDelta {
                index: 0,
                delta: "partial".into(),
            },
        });
        let failed = translator.payload_of(&AgentEvent::MessageDelta {
            event: StreamEvent::Error {
                message: "connection reset".into(),
            },
        });
        assert_eq!(failed["assistantMessageEvent"]["type"], "error");
        assert_eq!(
            failed["assistantMessageEvent"]["error"]["errorMessage"],
            "connection reset"
        );
        assert_eq!(
            failed["assistantMessageEvent"]["error"]["content"][0]["text"],
            "partial"
        );
    }

    /// A message survives being written out for an extension and read back from what it
    /// answers — round-tripping through the same shape pi itself would produce, not
    /// micro's own.
    #[test]
    fn a_message_round_trips_through_pis_shape() {
        let messages = vec![
            Message::User {
                content: vec![ContentBlock::text("hello")],
                timestamp: 10,
            },
            Message::Assistant(micro_types::AssistantMessage {
                content: vec![
                    ContentBlock::text("thinking about it"),
                    ContentBlock::ToolCall {
                        id: "call_1".into(),
                        name: "read".into(),
                        arguments: json!({ "path": "a.txt" }),
                        signature: None,
                    },
                ],
                provider: "anthropic".into(),
                model: "claude-opus-5".into(),
                usage: Usage {
                    input: 4,
                    output: 2,
                    cache_read: 0,
                    cache_write: 0,
                },
                stop_reason: StopReason::ToolUse,
                error: None,
                timestamp: 11,
            }),
            Message::ToolResult {
                tool_call_id: "call_1".into(),
                tool_name: "read".into(),
                content: vec![ContentBlock::text("contents of a.txt")],
                is_error: false,
                timestamp: 12,
            },
        ];

        for message in &messages {
            let json = message_json(message);
            let parsed = message_from_json(&json).expect("a message an extension wrote back parses");
            assert_eq!(&parsed, message, "{json}");
        }
    }

    /// Redacted thinking is folded into pi's `ThinkingContent` shape on the way out, and
    /// unfolded back into micro's own variant on the way back in.
    #[test]
    fn redacted_thinking_round_trips_through_pis_shape() {
        let block = ContentBlock::RedactedThinking {
            data: "opaque-token".into(),
        };
        let json = content_json(&block);
        assert_eq!(json["redacted"], true);
        assert_eq!(content_from_json(&json), Some(block));
    }

    /// An answer missing the fields a message needs is not a message at all, rather than
    /// one guessed at from what little is there.
    #[test]
    fn an_unrecognized_role_does_not_parse() {
        assert_eq!(message_from_json(&json!({ "role": "system" })), None);
        assert_eq!(message_from_json(&json!({})), None);
    }
}
