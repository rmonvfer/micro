//! The OpenAI chat-completions API, and the services that speak it.
//!
//! One implementation serves every deployment of this wire format; a [`Flavor`] carries
//! what differs between them — the endpoint, the headers the service demands, and the few
//! fields it spells its own way.

use crate::json::parse_arguments;
use crate::json::read_u32;
use crate::sse::read_sse;
use crate::Provider;
use crate::SseEvent;
use micro_types::now_ms;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::StreamEvent;
use micro_types::ThinkingLevel;
use micro_types::Usage;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

pub(crate) const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub(crate) const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub(crate) const COPILOT_BASE_URL: &str = "https://api.individual.githubcopilot.com";

/// The last line of a chat-completions stream, which carries no JSON.
const DONE_SENTINEL: &str = "[DONE]";

/// What one service needs on top of the shared wire format.
pub struct Flavor {
    /// The provider id, used to select this flavor and to label its errors.
    name: &'static str,
    base_url: &'static str,
    /// Newer OpenAI models take `max_completion_tokens`; everyone else takes `max_tokens`.
    max_tokens_field: &'static str,
    supports_stream_options: bool,
    supports_reasoning_effort: bool,
    headers: &'static [(&'static str, &'static str)],
}

static OPENAI: Flavor = Flavor {
    name: "openai",
    base_url: OPENAI_BASE_URL,
    max_tokens_field: "max_completion_tokens",
    supports_stream_options: true,
    supports_reasoning_effort: true,
    headers: &[],
};

/// OpenRouter attributes requests to the app that made them through these two headers,
/// which is what puts micro on its leaderboards.
static OPENROUTER: Flavor = Flavor {
    name: "openrouter",
    base_url: OPENROUTER_BASE_URL,
    max_tokens_field: "max_tokens",
    supports_stream_options: true,
    supports_reasoning_effort: true,
    headers: &[
        ("http-referer", "https://github.com/agentmode/micro"),
        ("x-title", "micro"),
    ],
};

/// Copilot serves whichever editor its headers describe, and rejects requests that do not
/// describe one. It exposes no reasoning-effort control.
static COPILOT: Flavor = Flavor {
    name: "github-copilot",
    base_url: COPILOT_BASE_URL,
    max_tokens_field: "max_tokens",
    supports_stream_options: true,
    supports_reasoning_effort: false,
    headers: &[
        ("user-agent", "GitHubCopilotChat/0.35.0"),
        ("editor-version", "vscode/1.107.0"),
        ("editor-plugin-version", "copilot-chat/0.35.0"),
        ("copilot-integration-id", "vscode-chat"),
        ("openai-intent", "conversation-edits"),
    ],
};

#[derive(Clone)]
pub struct OpenAi {
    flavor: &'static Flavor,
    client: reqwest::Client,
}

impl OpenAi {
    /// OpenAI itself. Use [`OpenAi::openrouter`] or [`OpenAi::copilot`] for the services
    /// that reimplement the same format.
    pub fn new() -> Self {
        OpenAi::with_flavor(&OPENAI)
    }

    pub fn openrouter() -> Self {
        OpenAi::with_flavor(&OPENROUTER)
    }

    pub fn copilot() -> Self {
        OpenAi::with_flavor(&COPILOT)
    }

    fn with_flavor(flavor: &'static Flavor) -> Self {
        OpenAi {
            flavor,
            client: crate::http_client(),
        }
    }

    /// The endpoint this flavor talks to, for callers assembling a [`Model`].
    pub fn base_url(&self) -> &'static str {
        self.flavor.base_url
    }
}

impl Default for OpenAi {
    fn default() -> Self {
        OpenAi::new()
    }
}

impl Provider for OpenAi {
    fn name(&self) -> &str {
        self.flavor.name
    }

    fn stream(
        &self,
        model: Model,
        context: Context,
        api_key: String,
    ) -> UnboundedReceiver<StreamEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let client = self.client.clone();
        let flavor = self.flavor;

        tokio::spawn(async move {
            if let Err(message) = run(client, flavor, model, context, api_key, &sender).await {
                let _ = sender.send(StreamEvent::Error { message });
            }
        });

        receiver
    }
}

async fn run(
    client: reqwest::Client,
    flavor: &'static Flavor,
    model: Model,
    context: Context,
    api_key: String,
    sender: &UnboundedSender<StreamEvent>,
) -> Result<(), String> {
    let payload = build_payload(flavor, &model, &context);
    let mut request = client
        .post(endpoint(&model.base_url))
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream");
    for (name, value) in flavor.headers {
        request = request.header(*name, *value);
    }

    let response = request
        .json(&payload)
        .send()
        .await
        .map_err(|error| format!("{} request failed: {error}", flavor.name))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "{} returned {}: {}",
            flavor.name,
            status.as_u16(),
            body.trim()
        ));
    }

    let mut state = Accumulator::new(flavor.name, &model);
    read_sse(response, |event| state.handle(event, sender))
        .await
        .map_err(|error| format!("{} stream failed: {error}", flavor.name))?;

    if !state.finished {
        state.finish(sender);
    }
    Ok(())
}

fn endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

/// A content block being assembled from deltas. The index is assigned on first sight, so
/// reasoning, text, and tool calls never collide on index zero.
#[derive(Default)]
struct OpenText {
    index: Option<usize>,
    buffer: String,
}

impl OpenText {
    /// The block's index, allocating one the first time it is asked for.
    fn open(&mut self, next: &mut usize) -> usize {
        *self.index.get_or_insert_with(|| {
            let index = *next;
            *next += 1;
            index
        })
    }
}

struct OpenTool {
    index: usize,
    id: String,
    name: String,
    arguments: String,
}

struct Accumulator {
    label: &'static str,
    provider: String,
    model_id: String,
    blocks: Vec<ContentBlock>,
    usage: Usage,
    stop_reason: StopReason,
    next_index: usize,
    text: OpenText,
    thinking: OpenText,
    /// Keyed by the wire's tool-call index, which is how fragments find their call.
    tools: BTreeMap<u64, OpenTool>,
    started: bool,
    saw_finish_reason: bool,
    finished: bool,
}

impl Accumulator {
    fn new(label: &'static str, model: &Model) -> Self {
        Accumulator {
            label,
            provider: model.provider.clone(),
            model_id: model.id.clone(),
            blocks: Vec::new(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            next_index: 0,
            text: OpenText::default(),
            thinking: OpenText::default(),
            tools: BTreeMap::new(),
            started: false,
            saw_finish_reason: false,
            finished: false,
        }
    }

    fn build(&self) -> AssistantMessage {
        AssistantMessage {
            content: self.blocks.clone(),
            provider: self.provider.clone(),
            model: self.model_id.clone(),
            usage: self.usage,
            stop_reason: self.stop_reason,
            error: None,
            timestamp: now_ms(),
        }
    }

    fn handle(&mut self, event: SseEvent, sender: &UnboundedSender<StreamEvent>) {
        let data = event.data.trim();
        if data.is_empty() || data == DONE_SENTINEL {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };

        // Rate limits and upstream failures arrive as a chunk rather than a status code.
        if let Some(message) = stream_error(self.label, &value) {
            self.finished = true;
            let _ = sender.send(StreamEvent::Error { message });
            return;
        }

        if !self.started {
            self.started = true;
            let _ = sender.send(StreamEvent::Start);
        }

        // Usage rides along on any chunk once `include_usage` is set, commonly the last.
        if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
            self.read_usage(usage);
        }

        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };

        if let Some(delta) = choice.get("delta") {
            // Reasoning goes by `reasoning_content` on DeepSeek-style models and plain
            // `reasoning` on the gateways that re-expose them.
            let reasoning = non_empty(delta.get("reasoning_content"))
                .or_else(|| non_empty(delta.get("reasoning")));
            if let Some(fragment) = reasoning {
                let index = self.thinking.open(&mut self.next_index);
                if self.thinking.buffer.is_empty() {
                    let _ = sender.send(StreamEvent::ThinkingStart { index });
                }
                self.thinking.buffer.push_str(&fragment);
                let _ = sender.send(StreamEvent::ThinkingDelta {
                    index,
                    delta: fragment,
                });
            }

            if let Some(fragment) = non_empty(delta.get("content")) {
                let index = self.text.open(&mut self.next_index);
                if self.text.buffer.is_empty() {
                    let _ = sender.send(StreamEvent::TextStart { index });
                }
                self.text.buffer.push_str(&fragment);
                let _ = sender.send(StreamEvent::TextDelta {
                    index,
                    delta: fragment,
                });
            }

            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    self.apply_tool_delta(call, sender);
                }
            }
        }

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.saw_finish_reason = true;
            self.stop_reason = map_finish_reason(reason);
        }
    }

    /// A tool call is spread across deltas: the first names it, the rest carry slices of
    /// its JSON arguments. They are joined by the index the wire assigns.
    fn apply_tool_delta(&mut self, call: &Value, sender: &UnboundedSender<StreamEvent>) {
        let key = call.get("index").and_then(Value::as_u64).unwrap_or(0);
        let fresh = !self.tools.contains_key(&key);

        let next_index = &mut self.next_index;
        let tool = self.tools.entry(key).or_insert_with(|| {
            let index = *next_index;
            *next_index += 1;
            OpenTool {
                index,
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            }
        });

        if let Some(id) = non_empty(call.get("id")) {
            tool.id = id;
        }
        if let Some(name) = non_empty(call.pointer("/function/name")) {
            tool.name = name;
        }

        if fresh {
            let _ = sender.send(StreamEvent::ToolCallStart {
                index: tool.index,
                id: tool.id.clone(),
                name: tool.name.clone(),
            });
        }

        if let Some(fragment) = non_empty(call.pointer("/function/arguments")) {
            tool.arguments.push_str(&fragment);
            let _ = sender.send(StreamEvent::ToolCallDelta {
                index: tool.index,
                delta: fragment,
            });
        }
    }

    /// `prompt_tokens` counts cached tokens too, so the billable input excludes them.
    fn read_usage(&mut self, usage: &Value) {
        if let Some(cached) = usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
        {
            self.usage.cache_read = cached as u32;
        }
        let prompt = read_u32(usage, "prompt_tokens");
        if prompt > 0 {
            self.usage.input = prompt.saturating_sub(self.usage.cache_read);
        }
        let completion = read_u32(usage, "completion_tokens");
        if completion > 0 {
            self.usage.output = completion;
        }
    }

    /// Close every open block and emit the assembled message. Blocks are finalized in
    /// index order so the content lines up with the events the consumer already saw.
    fn finish(&mut self, sender: &UnboundedSender<StreamEvent>) {
        self.finished = true;
        let mut finalized: Vec<(usize, ContentBlock, StreamEvent)> = Vec::new();

        if let Some(index) = self.thinking.index {
            let thinking = std::mem::take(&mut self.thinking.buffer);
            if !thinking.trim().is_empty() {
                finalized.push((
                    index,
                    ContentBlock::Thinking {
                        thinking: thinking.clone(),
                        signature: None,
                    },
                    StreamEvent::ThinkingEnd { index, thinking },
                ));
            }
        }

        if let Some(index) = self.text.index {
            let text = std::mem::take(&mut self.text.buffer);
            if !text.trim().is_empty() {
                finalized.push((
                    index,
                    ContentBlock::Text { text: text.clone() },
                    StreamEvent::TextEnd { index, text },
                ));
            }
        }

        let has_tool_calls = !self.tools.is_empty();
        for tool in std::mem::take(&mut self.tools).into_values() {
            let arguments = parse_arguments(&tool.arguments);
            finalized.push((
                tool.index,
                ContentBlock::ToolCall {
                    id: tool.id.clone(),
                    name: tool.name.clone(),
                    arguments: arguments.clone(),

                    signature: None,
                },
                StreamEvent::ToolCallEnd {
                    index: tool.index,
                    id: tool.id,
                    name: tool.name,
                    arguments,
                },
            ));
        }

        finalized.sort_by_key(|(index, ..)| *index);
        for (_, block, event) in finalized {
            self.blocks.push(block);
            let _ = sender.send(event);
        }

        if !self.saw_finish_reason {
            // A stream that ended without a finish reason was cut short.
            self.stop_reason = StopReason::Error;
        } else if self.stop_reason == StopReason::Stop && has_tool_calls {
            // Some gateways report `stop` even when they hand back tool calls.
            self.stop_reason = StopReason::ToolUse;
        }

        let _ = sender.send(StreamEvent::Done {
            message: self.build(),
        });
    }
}

/// A chunk that reports a failure instead of content. The status code, when present, is
/// kept in the message so a transient one can be retried.
fn stream_error(label: &str, value: &Value) -> Option<String> {
    let error = value.get("error").filter(|error| !error.is_null())?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string());

    match error.get("code").and_then(Value::as_u64) {
        Some(code) => Some(format!("{label} returned {code}: {message}")),
        None => Some(format!("{label} stream error: {message}")),
    }
}

/// A JSON string with something in it.
fn non_empty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "length" => StopReason::Length,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "content_filter" => StopReason::Error,
        _ => StopReason::Stop,
    }
}

fn reasoning_effort(level: ThinkingLevel) -> Option<&'static str> {
    match level {
        ThinkingLevel::Off => None,
        ThinkingLevel::Low => Some("low"),
        ThinkingLevel::Medium => Some("medium"),
        ThinkingLevel::High => Some("high"),
    }
}

pub(crate) fn build_payload(flavor: &Flavor, model: &Model, context: &Context) -> Value {
    let mut payload = Map::new();
    payload.insert("model".into(), json!(model.id));
    payload.insert("messages".into(), Value::Array(build_messages(context)));
    payload.insert("stream".into(), json!(true));
    payload.insert(flavor.max_tokens_field.into(), json!(model.max_tokens));

    if flavor.supports_stream_options {
        payload.insert("stream_options".into(), json!({ "include_usage": true }));
    }

    if flavor.supports_reasoning_effort {
        if let Some(effort) = reasoning_effort(model.thinking) {
            payload.insert("reasoning_effort".into(), json!(effort));
        }
    }

    if !context.tools.is_empty() {
        let tools: Vec<Value> = context
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    },
                })
            })
            .collect();
        payload.insert("tools".into(), Value::Array(tools));
    }

    Value::Object(payload)
}

/// Convert the conversation to the chat-completions shape: the system prompt as the
/// leading message, and every tool result as its own `tool` turn keyed by call id.
fn build_messages(context: &Context) -> Vec<Value> {
    let mut wire: Vec<Value> = Vec::new();

    if let Some(system) = &context.system_prompt {
        wire.push(json!({ "role": "system", "content": system }));
    }

    for message in &context.messages {
        match message {
            Message::User { content, .. } => {
                wire.push(json!({ "role": "user", "content": user_content(content) }));
            }
            Message::Assistant(assistant) => {
                if let Some(encoded) = assistant_message(assistant) {
                    wire.push(encoded);
                }
            }
            Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                wire.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": join_text(content),
                }));
            }
        }
    }

    wire
}

/// Plain text stays a string, which every service accepts; images force the multi-part
/// form, where each image is an inline data URL.
fn user_content(blocks: &[ContentBlock]) -> Value {
    let has_image = blocks
        .iter()
        .any(|block| matches!(block, ContentBlock::Image { .. }));
    if !has_image {
        return json!(join_text(blocks));
    }

    let parts: Vec<Value> = blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
            ContentBlock::Image { data, mime_type } => Some(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{mime_type};base64,{data}") },
            })),
            _ => None,
        })
        .collect();
    Value::Array(parts)
}

/// Reasoning has no representation on the request side of this format, so it is dropped;
/// only the text and the tool calls are replayed.
fn assistant_message(assistant: &AssistantMessage) -> Option<Value> {
    let text = join_text(&assistant.content);
    let calls: Vec<Value> = assistant
        .content
        .iter()
        .filter_map(|block| match block {
            // A reasoning signature on a call is Gemini's, and this format has nowhere to
            // put one, so it is left behind rather than replayed into a format that would
            // reject it.
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some(json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": encode_arguments(arguments),
                },
            })),
            _ => None,
        })
        .collect();

    if text.is_empty() && calls.is_empty() {
        return None;
    }

    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    message.insert(
        "content".into(),
        if text.is_empty() {
            Value::Null
        } else {
            json!(text)
        },
    );
    if !calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(calls));
    }
    Some(Value::Object(message))
}

/// Arguments travel as a JSON string, and a call with none must still send an object.
fn encode_arguments(arguments: &Value) -> String {
    if arguments.is_null() {
        "{}".to_string()
    } else {
        arguments.to_string()
    }
}

fn join_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::ToolDefinition;

    fn model() -> Model {
        Model {
            id: "gpt-5".into(),
            provider: "openrouter".into(),
            base_url: OPENROUTER_BASE_URL.into(),
            max_tokens: 4_096,
            thinking: ThinkingLevel::Off,
        }
    }

    /// Drive the accumulator with the chunks a service would send, and collect what a
    /// consumer would see.
    fn drain(chunks: &[Value]) -> Vec<StreamEvent> {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut state = Accumulator::new("openrouter", &model());
        for chunk in chunks {
            state.handle(
                SseEvent {
                    event: None,
                    data: chunk.to_string(),
                },
                &sender,
            );
        }
        if !state.finished {
            state.finish(&sender);
        }
        drop(sender);

        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    fn done(events: &[StreamEvent]) -> AssistantMessage {
        match events.last() {
            Some(StreamEvent::Done { message }) => message.clone(),
            other => panic!("expected a done event, got {other:?}"),
        }
    }

    fn text_chunk(content: &str) -> Value {
        json!({ "choices": [{ "delta": { "content": content } }] })
    }

    fn stop_chunk(reason: &str) -> Value {
        json!({ "choices": [{ "delta": {}, "finish_reason": reason }] })
    }

    /// This format takes JSON Schema as it is written, so nothing here may filter it. A
    /// parameter named after a schema keyword is the case that catches a filter creeping in.
    #[test]
    fn tool_schemas_reach_the_service_exactly_as_written() {
        let parameters = json!({
            "type": "object",
            "properties": { "pattern": { "type": "string", "pattern": "^a" } },
            "required": ["pattern"],
        });
        let context = Context {
            tools: vec![ToolDefinition {
                name: "grep".into(),
                description: "search".into(),
                parameters: parameters.clone(),
            }],
            ..Context::default()
        };
        let payload = build_payload(&OPENAI, &model(), &context);

        assert_eq!(payload["tools"][0]["function"]["parameters"], parameters);
    }

    #[test]
    fn endpoint_is_not_doubled_up() {
        assert_eq!(
            endpoint(OPENROUTER_BASE_URL),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            endpoint("https://openrouter.ai/api/v1/chat/completions/"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }

    #[test]
    fn each_flavor_spells_its_output_limit_its_own_way() {
        let payload = build_payload(&OPENROUTER, &model(), &Context::default());
        assert_eq!(payload["max_tokens"], 4_096);
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["stream_options"]["include_usage"], true);

        let payload = build_payload(&OPENAI, &model(), &Context::default());
        assert_eq!(payload["max_completion_tokens"], 4_096);
        assert!(payload.get("max_tokens").is_none());
    }

    #[test]
    fn reasoning_effort_is_sent_only_where_it_is_supported() {
        let thinking = model().with_thinking(ThinkingLevel::High);

        assert_eq!(
            build_payload(&OPENROUTER, &thinking, &Context::default())["reasoning_effort"],
            "high"
        );
        assert!(build_payload(&COPILOT, &thinking, &Context::default())
            .get("reasoning_effort")
            .is_none());
        assert!(build_payload(&OPENROUTER, &model(), &Context::default())
            .get("reasoning_effort")
            .is_none());
    }

    #[test]
    fn tools_are_wrapped_in_a_function_envelope() {
        let context = Context {
            tools: vec![ToolDefinition {
                name: "read".into(),
                description: "read a file".into(),
                parameters: json!({ "type": "object" }),
            }],
            ..Context::default()
        };
        let payload = build_payload(&OPENROUTER, &model(), &context);

        assert_eq!(payload["tools"][0]["type"], "function");
        assert_eq!(payload["tools"][0]["function"]["name"], "read");
        assert_eq!(
            payload["tools"][0]["function"]["parameters"]["type"],
            "object"
        );
    }

    #[test]
    fn the_system_prompt_leads_and_tool_results_get_their_own_turn() {
        let context = Context {
            system_prompt: Some("be brief".into()),
            messages: vec![
                Message::user("go"),
                Message::Assistant(AssistantMessage {
                    content: vec![ContentBlock::ToolCall {
                        id: "call_1".into(),
                        name: "read".into(),
                        arguments: json!({ "path": "a.txt" }),

                        signature: None,
                    }],
                    provider: "openrouter".into(),
                    model: "gpt-5".into(),
                    usage: Usage::default(),
                    stop_reason: StopReason::ToolUse,
                    error: None,
                    timestamp: 0,
                }),
                Message::tool_result("call_1", "read", "contents", false),
            ],
            tools: Vec::new(),
        };
        let wire = build_messages(&context);

        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[1]["role"], "user");
        assert_eq!(wire[1]["content"], "go");
        assert_eq!(wire[2]["role"], "assistant");
        assert_eq!(wire[2]["content"], Value::Null);
        assert_eq!(wire[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            wire[2]["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a.txt\"}"
        );
        assert_eq!(wire[3]["role"], "tool");
        assert_eq!(wire[3]["tool_call_id"], "call_1");
        assert_eq!(wire[3]["content"], "contents");
    }

    #[test]
    fn a_call_without_arguments_still_sends_an_object() {
        assert_eq!(encode_arguments(&Value::Null), "{}");
        assert_eq!(encode_arguments(&json!({ "a": 1 })), "{\"a\":1}");
    }

    #[test]
    fn images_switch_a_user_turn_to_inline_parts() {
        let blocks = vec![
            ContentBlock::text("look"),
            ContentBlock::Image {
                data: "AAAA".into(),
                mime_type: "image/png".into(),
            },
        ];
        let content = user_content(&blocks);

        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(user_content(&[ContentBlock::text("plain")]), "plain");
    }

    #[test]
    fn an_assistant_turn_with_nothing_in_it_is_dropped() {
        let empty = AssistantMessage {
            content: vec![ContentBlock::Thinking {
                thinking: "hmm".into(),
                signature: None,
            }],
            provider: "openrouter".into(),
            model: "gpt-5".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        };
        assert!(assistant_message(&empty).is_none());
    }

    #[test]
    fn text_deltas_accumulate_into_one_block() {
        let events = drain(&[text_chunk("he"), text_chunk("llo"), stop_chunk("stop")]);

        assert_eq!(events[0], StreamEvent::Start);
        assert_eq!(events[1], StreamEvent::TextStart { index: 0 });
        assert!(matches!(events[2], StreamEvent::TextDelta { index: 0, .. }));

        let message = done(&events);
        assert_eq!(message.text(), "hello");
        assert_eq!(message.stop_reason, StopReason::Stop);
    }

    #[test]
    fn tool_arguments_are_reassembled_from_fragments() {
        let events = drain(&[
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "id": "call_1", "type": "function",
                  "function": { "name": "read", "arguments": "" } }
            ] } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "function": { "arguments": "{\"pa" } }
            ] } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "function": { "arguments": "th\":\"a.txt\"}" } }
            ] } }] }),
            stop_chunk("tool_calls"),
        ]);

        assert_eq!(
            events[1],
            StreamEvent::ToolCallStart {
                index: 0,
                id: "call_1".into(),
                name: "read".into(),
            }
        );

        let message = done(&events);
        assert_eq!(
            message.tool_calls(),
            vec![("call_1", "read", &json!({ "path": "a.txt" }))]
        );
        assert_eq!(message.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn parallel_tool_calls_stay_apart_and_keep_their_order() {
        let events = drain(&[
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "id": "call_1", "function": { "name": "read", "arguments": "{\"a\"" } },
                { "index": 1, "id": "call_2", "function": { "name": "list", "arguments": "{\"b\"" } }
            ] } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 1, "function": { "arguments": ":2}" } },
                { "index": 0, "function": { "arguments": ":1}" } }
            ] } }] }),
            stop_chunk("tool_calls"),
        ]);

        let message = done(&events);
        assert_eq!(
            message.tool_calls(),
            vec![
                ("call_1", "read", &json!({ "a": 1 })),
                ("call_2", "list", &json!({ "b": 2 })),
            ]
        );
    }

    #[test]
    fn reasoning_text_and_tool_calls_get_distinct_indices() {
        let events = drain(&[
            json!({ "choices": [{ "delta": { "reasoning_content": "thinking" } }] }),
            text_chunk("answer"),
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "id": "call_1", "function": { "name": "read", "arguments": "{}" } }
            ] } }] }),
            stop_chunk("tool_calls"),
        ]);

        assert_eq!(events[1], StreamEvent::ThinkingStart { index: 0 });
        assert_eq!(events[3], StreamEvent::TextStart { index: 1 });
        assert!(matches!(
            events[5],
            StreamEvent::ToolCallStart { index: 2, .. }
        ));

        let message = done(&events);
        assert!(matches!(
            message.content.as_slice(),
            [
                ContentBlock::Thinking { .. },
                ContentBlock::Text { .. },
                ContentBlock::ToolCall { .. }
            ]
        ));
    }

    #[test]
    fn a_gateway_that_reports_stop_alongside_tool_calls_still_means_tool_use() {
        let events = drain(&[
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "id": "call_1", "function": { "name": "read", "arguments": "{}" } }
            ] } }] }),
            stop_chunk("stop"),
        ]);
        assert_eq!(done(&events).stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn finish_reasons_map_onto_stop_reasons() {
        assert_eq!(map_finish_reason("stop"), StopReason::Stop);
        assert_eq!(map_finish_reason("length"), StopReason::Length);
        assert_eq!(map_finish_reason("tool_calls"), StopReason::ToolUse);
        assert_eq!(map_finish_reason("function_call"), StopReason::ToolUse);
        assert_eq!(map_finish_reason("content_filter"), StopReason::Error);
        assert_eq!(map_finish_reason("unheard-of"), StopReason::Stop);
    }

    #[test]
    fn a_stream_cut_short_is_reported_as_an_error() {
        let events = drain(&[text_chunk("partial")]);
        let message = done(&events);

        assert_eq!(message.text(), "partial");
        assert_eq!(message.stop_reason, StopReason::Error);
    }

    #[test]
    fn cached_tokens_are_excluded_from_the_billable_input() {
        let events = drain(&[
            text_chunk("hi"),
            stop_chunk("stop"),
            json!({ "choices": [], "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 20,
                "prompt_tokens_details": { "cached_tokens": 80 }
            } }),
        ]);
        let usage = done(&events).usage;

        assert_eq!(usage.input, 20);
        assert_eq!(usage.cache_read, 80);
        assert_eq!(usage.output, 20);
    }

    #[test]
    fn the_done_sentinel_and_unparseable_chunks_are_ignored() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut state = Accumulator::new("openrouter", &model());
        for data in ["[DONE]", "not json", ""] {
            state.handle(
                SseEvent {
                    event: None,
                    data: data.into(),
                },
                &sender,
            );
        }
        drop(sender);

        assert!(receiver.try_recv().is_err());
        assert!(!state.started);
    }

    #[test]
    fn an_error_chunk_ends_the_stream_with_its_status() {
        let events = drain(&[json!({ "error": { "message": "rate limited", "code": 429 } })]);

        assert_eq!(
            events.as_slice(),
            [StreamEvent::Error {
                message: "openrouter returned 429: rate limited".into(),
            }]
        );
    }

    #[test]
    fn an_error_chunk_without_a_status_is_still_reported() {
        let events = drain(&[json!({ "error": { "message": "upstream is down" } })]);
        assert_eq!(
            events.as_slice(),
            [StreamEvent::Error {
                message: "openrouter stream error: upstream is down".into(),
            }]
        );
        assert!(stream_error("openrouter", &json!({ "error": null })).is_none());
    }

    #[test]
    fn copilot_describes_the_editor_it_serves() {
        let headers: BTreeMap<_, _> = COPILOT.headers.iter().copied().collect();

        assert_eq!(headers["copilot-integration-id"], "vscode-chat");
        assert_eq!(headers["editor-version"], "vscode/1.107.0");
        assert_eq!(headers["editor-plugin-version"], "copilot-chat/0.35.0");
        assert_eq!(headers["user-agent"], "GitHubCopilotChat/0.35.0");
        assert_eq!(headers["openai-intent"], "conversation-edits");
    }

    #[test]
    fn openrouter_identifies_the_app_making_the_request() {
        let headers: BTreeMap<_, _> = OPENROUTER.headers.iter().copied().collect();

        assert_eq!(headers["x-title"], "micro");
        assert!(headers["http-referer"].starts_with("https://"));
    }
}
