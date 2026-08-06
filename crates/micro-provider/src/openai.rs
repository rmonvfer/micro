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
use micro_types::MaxTokensField;
use micro_types::OffLevel;
use micro_types::Model;
use micro_types::ThinkingFormat;
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

/// The last line of a chat-completions stream, which carries no JSON.
const DONE_SENTINEL: &str = "[DONE]";

#[derive(Clone)]
pub struct OpenAi {
    /// The service being spoken to. What it accepts travels with each model; this names
    /// it, for the handful of decisions that are the service's own and for its errors.
    provider: String,
    client: reqwest::Client,
}

impl OpenAi {
    /// OpenAI itself.
    pub fn new() -> Self {
        OpenAi::for_provider(micro_auth::OPENAI)
    }

    pub fn openrouter() -> Self {
        OpenAi::for_provider(micro_auth::OPENROUTER)
    }

    pub fn copilot() -> Self {
        OpenAi::for_provider(micro_auth::GITHUB_COPILOT)
    }

    /// A client for one service, whether or not micro has heard of it. Every service
    /// answering this protocol is reached the same way; the model says what it accepts.
    pub fn for_provider(provider: impl Into<String>) -> Self {
        OpenAi {
            provider: provider.into(),
            client: crate::http_client(),
        }
    }

    /// The endpoint OpenAI itself serves, for callers assembling a [`Model`].
    pub fn base_url(&self) -> &'static str {
        OPENAI_BASE_URL
    }
}

impl Default for OpenAi {
    fn default() -> Self {
        OpenAi::new()
    }
}

impl Provider for OpenAi {
    fn name(&self) -> &str {
        &self.provider
    }

    fn stream(
        &self,
        model: Model,
        context: Context,
        api_key: String,
    ) -> UnboundedReceiver<StreamEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let client = self.client.clone();
        let provider = self.provider.clone();

        tokio::spawn(async move {
            if let Err(message) = run(client, provider, model, context, api_key, &sender).await {
                let _ = sender.send(StreamEvent::Error { message });
            }
        });

        receiver
    }
}

async fn run(
    client: reqwest::Client,
    provider: String,
    model: Model,
    context: Context,
    api_key: String,
    sender: &UnboundedSender<StreamEvent>,
) -> Result<(), String> {
    let payload = build_payload(&model, &context);
    let mut request = client
        .post(endpoint(&model.base_url))
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream");
    // Whatever the service asks to be told about the client it is talking to, which the
    // catalog records per model.
    for (name, value) in &model.headers {
        request = request.header(name.as_str(), value.as_str());
    }
    // Copilot bills and rate-limits differently depending on who started the request, and
    // refuses an image unless the request says one is coming.
    if is_copilot(&provider, &model.base_url) {
        request = request
            .header("x-initiator", initiator(&context.messages))
            .header("openai-intent", "conversation-edits");
        if carries_images(&context.messages) {
            request = request.header("copilot-vision-request", "true");
        }
    }
    // Anything the caller added wins, which is how an extension changes a header the
    // provider would otherwise set for itself.
    for (name, value) in &context.headers {
        request = request.header(name.as_str(), value.as_str());
    }

    let response = request
        .json(&payload)
        .send()
        .await
        .map_err(|error| format!("{provider} request failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "{provider} returned {}: {}",
            status.as_u16(),
            body.trim()
        ));
    }

    let mut state = Accumulator::new(&provider, &model);
    read_sse(response, |event| state.handle(event, sender))
        .await
        .map_err(|error| format!("{provider} stream failed: {error}"))?;

    if !state.finished {
        state.finish(sender);
    }
    Ok(())
}

/// Whether this is Copilot, which wants to be told who started a request.
fn is_copilot(provider: &str, base_url: &str) -> bool {
    provider == micro_auth::GITHUB_COPILOT || base_url.contains("githubcopilot.com")
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
    /// The service, for the errors it reports mid-stream.
    label: String,
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
    fn new(label: impl Into<String>, model: &Model) -> Self {
        Accumulator {
            label: label.into(),
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
        if let Some(message) = stream_error(&self.label, &value) {
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

/// Ask for reasoning effort in the shape this service accepts.
///
/// Every service that answers this protocol has its own spelling: a top-level field, a
/// nested object, a switch, or nothing at all. The level a model does not offer is left
/// out rather than sent under a name the service would reject.
fn apply_thinking(payload: &mut Map<String, Value>, model: &Model) {
    let compat = &model.compat;
    if !model.reasoning {
        return;
    }

    let asked = !matches!(model.thinking, ThinkingLevel::Off);
    let effort = compat.level(model.thinking);
    let off = compat.off();

    match compat.thinking_format {
        ThinkingFormat::Zai => {
            payload.insert(
                "thinking".into(),
                match asked {
                    true => json!({ "type": "enabled", "clear_thinking": false }),
                    false => json!({ "type": "disabled" }),
                },
            );
            if asked && compat.supports_reasoning_effort {
                if let Some(effort) = effort {
                    payload.insert("reasoning_effort".into(), json!(effort));
                }
            }
        }
        ThinkingFormat::Qwen => {
            payload.insert("enable_thinking".into(), json!(asked));
        }
        ThinkingFormat::QwenChatTemplate | ThinkingFormat::ChatTemplate => {
            payload.insert(
                "chat_template_kwargs".into(),
                json!({ "enable_thinking": asked, "preserve_thinking": true }),
            );
        }
        ThinkingFormat::Deepseek => {
            match asked {
                true => {
                    payload.insert("thinking".into(), json!({ "type": "enabled" }));
                }
                false => {
                    if off != OffLevel::Unsupported {
                        payload.insert("thinking".into(), json!({ "type": "disabled" }));
                    }
                }
            }
            if asked && compat.supports_reasoning_effort {
                if let Some(effort) = effort {
                    payload.insert("reasoning_effort".into(), json!(effort));
                }
            }
        }
        ThinkingFormat::Openrouter => {
            // The gateway spells thinking being off as an effort of its own, so it is
            // told even then.
            let level = match asked {
                true => effort,
                false => off.or("none"),
            };
            if let Some(level) = level {
                payload.insert("reasoning".into(), json!({ "effort": level }));
            }
        }
        ThinkingFormat::AntLing => {
            if asked {
                if let Some(effort) = effort {
                    payload.insert("reasoning".into(), json!({ "effort": effort }));
                }
            }
        }
        ThinkingFormat::Together => {
            payload.insert("reasoning".into(), json!({ "enabled": asked }));
            if asked && compat.supports_reasoning_effort {
                if let Some(effort) = effort {
                    payload.insert("reasoning_effort".into(), json!(effort));
                }
            }
        }
        ThinkingFormat::StringThinking => {
            let level = match asked {
                true => effort,
                false => off.or("none"),
            };
            if let Some(level) = level {
                payload.insert("thinking".into(), json!(level));
            }
        }
        ThinkingFormat::Openai => {
            if !compat.supports_reasoning_effort {
                return;
            }
            // There is no name for off here: a model that has one says so, and one that
            // does not is simply not asked to reason.
            let level = match asked {
                true => effort,
                false => match off {
                    OffLevel::Named(named) => Some(named),
                    _ => None,
                },
            };
            if let Some(level) = level {
                payload.insert("reasoning_effort".into(), json!(level));
            }
        }
    }
}


/// The longest a prompt cache key may be. A longer one is refused rather than truncated
/// by the provider, so it is cut here.
const PROMPT_CACHE_KEY_MAX: usize = 64;

pub(crate) fn build_payload(model: &Model, context: &Context) -> Value {
    let compat = &model.compat;
    let mut payload = Map::new();
    payload.insert("model".into(), json!(model.id));
    payload.insert("messages".into(), Value::Array(build_messages(context)));
    payload.insert("stream".into(), json!(true));
    payload.insert(
        match compat.max_tokens_field {
            MaxTokensField::MaxTokens => "max_tokens",
            MaxTokensField::MaxCompletionTokens => "max_completion_tokens",
        }
        .into(),
        json!(model.max_tokens),
    );

    if compat.supports_usage_in_streaming {
        payload.insert("stream_options".into(), json!({ "include_usage": true }));
    }

    apply_thinking(&mut payload, model);

    // Naming the conversation is what lets a prompt be cached against it and hit next
    // turn. Clamped, because the field has a limit and a longer name is simply refused.
    if model.base_url.contains("api.openai.com") {
        if let Some(key) = &context.cache_key {
            let clamped: String = key.chars().take(PROMPT_CACHE_KEY_MAX).collect();
            payload.insert("prompt_cache_key".into(), json!(clamped));
        }
    }

    // Nothing is stored on the provider's side: a conversation micro cannot account for
    // is one it should not be leaving behind.
    if compat.supports_store {
        payload.insert("store".into(), json!(false));
    }

    if !context.tools.is_empty() {
        let tools: Vec<Value> = context
            .tools
            .iter()
            .map(|tool| {
                let mut function = json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                });
                // Some services reject a tool definition carrying fields they do not know.
                if compat.supports_strict_mode {
                    function["strict"] = json!(false);
                }
                json!({ "type": "function", "function": function })
            })
            .collect();
        payload.insert("tools".into(), Value::Array(tools));
        if compat.zai_tool_stream {
            payload.insert("tool_stream".into(), json!(true));
        }
    } else if has_tool_history(&context.messages) {
        // A conversation holding tool calls is refused by some endpoints unless the
        // request still declares tools, even when there are none left to offer.
        payload.insert("tools".into(), Value::Array(Vec::new()));
    }

    Value::Object(payload)
}

/// Who the request is on behalf of: the person, or the agent carrying on by itself.
///
/// Anything other than the user having just spoken is the agent continuing, which is what
/// Copilot means by an agent-initiated request.
fn initiator(messages: &[Message]) -> &'static str {
    match messages.last() {
        Some(Message::User { .. }) => "user",
        _ => "agent",
    }
}

/// Whether the conversation carries an image, in a prompt or in a tool's result.
fn carries_images(messages: &[Message]) -> bool {
    let has_image = |content: &[micro_types::ContentBlock]| {
        content
            .iter()
            .any(|block| matches!(block, micro_types::ContentBlock::Image { .. }))
    };
    messages.iter().any(|message| match message {
        Message::User { content, .. } => has_image(content),
        Message::ToolResult { content, .. } => has_image(content),
        Message::Assistant(_) => false,
    })
}

/// Whether anything in the conversation was a tool call or its result.
fn has_tool_history(messages: &[Message]) -> bool {
    messages.iter().any(|message| match message {
        Message::ToolResult { .. } => true,
        Message::Assistant(assistant) => !assistant.tool_calls().is_empty(),
        Message::User { .. } => false,
    })
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

    /// A model as the catalog would hand it over, so each test speaks to the service it
    /// names rather than to a hand-written approximation of one.
    fn served_by(provider: &str, id: &str) -> Model {
        let catalog = micro_models::Catalog::bundled();
        let model = catalog
            .by_provider(provider)
            .find(|model| model.id == id)
            .unwrap_or_else(|| panic!("{provider} serves {id}"));
        let mut runtime = model.to_runtime(ThinkingLevel::Off);
        runtime.max_tokens = 4_096;
        runtime
    }

    fn model() -> Model {
        served_by("openrouter", "openai/gpt-5.6-terra")
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
        let payload = build_payload(&served_by("openai", "gpt-5.6-terra"), &context);

        assert_eq!(payload["tools"][0]["function"]["parameters"], parameters);
    }

    #[test]
    fn endpoint_is_not_doubled_up() {
        assert_eq!(
            endpoint("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            endpoint("https://openrouter.ai/api/v1/chat/completions/"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }

    /// Which field carries the limit is the service's own business.
    #[test]
    fn each_service_spells_its_output_limit_its_own_way() {
        let payload = build_payload(&served_by("together", "openai/gpt-oss-120b"), &Context::default());
        assert_eq!(payload["max_tokens"], 4_096);
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["stream_options"]["include_usage"], true);

        let payload = build_payload(&served_by("openai", "gpt-5.6-terra"), &Context::default());
        assert_eq!(payload["max_completion_tokens"], 4_096);
        assert!(payload.get("max_tokens").is_none());
    }

    /// Thinking being off is not a level every service has a name for. A model that
    /// spells it out is sent that spelling; one that says nothing is sent nothing.
    #[test]
    fn a_service_is_only_told_thinking_is_off_when_it_has_a_word_for_it() {
        let unsaid = served_by("openai", "o1");
        assert!(build_payload(&unsaid, &Context::default())
            .get("reasoning_effort")
            .is_none());

        // This one names it, and is told.
        let named = served_by("openai", "gpt-5.6-terra");
        assert_eq!(named.compat.off(), OffLevel::Named("none".into()));
        assert_eq!(
            build_payload(&named, &Context::default())["reasoning_effort"],
            "none"
        );

        // A gateway has a word for it of its own, so it is told either way.
        assert_eq!(
            build_payload(&model(), &Context::default())["reasoning"]["effort"],
            "none"
        );
    }

    /// OpenRouter normalizes reasoning into an object of its own; OpenAI takes a field.
    #[test]
    fn reasoning_is_asked_for_in_the_shape_the_service_accepts() {
        let openrouter = model().with_thinking(ThinkingLevel::High);
        assert_eq!(
            build_payload(&openrouter, &Context::default())["reasoning"]["effort"],
            "high"
        );

        let openai = served_by("openai", "gpt-5.6-terra").with_thinking(ThinkingLevel::High);
        assert_eq!(
            build_payload(&openai, &Context::default())["reasoning_effort"],
            "high"
        );

        // A service that offers no reasoning control is not asked for one.
        let copilot = served_by("github-copilot", "gpt-4.1").with_thinking(ThinkingLevel::High);
        assert!(build_payload(&copilot, &Context::default())
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
        let payload = build_payload(&model(), &context);

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
            headers: Vec::new(),
            cache_key: None,
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

    /// Copilot serves whichever editor its headers describe, and the catalog is where
    /// that description lives.
    #[test]
    fn copilot_describes_the_editor_it_serves() {
        let headers = served_by("github-copilot", "gpt-4.1").headers;

        assert_eq!(headers["Copilot-Integration-Id"], "vscode-chat");
        assert_eq!(headers["Editor-Version"], "vscode/1.107.0");
        assert_eq!(headers["Editor-Plugin-Version"], "copilot-chat/0.35.0");
        assert_eq!(headers["User-Agent"], "GitHubCopilotChat/0.35.0");
    }

    /// Only Copilot is told who started a request, and it is told on every request.
    #[test]
    fn copilot_is_recognised_by_name_and_by_address() {
        assert!(is_copilot("github-copilot", "https://example.test"));
        assert!(is_copilot(
            "elsewhere",
            "https://api.individual.githubcopilot.com"
        ));
        assert!(!is_copilot("openrouter", "https://openrouter.ai/api/v1"));
    }

    /// Naming the conversation is what makes a cached prompt hit on the next turn.
    #[test]
    fn a_conversation_is_named_so_its_prompt_can_be_cached() {
        let context = Context {
            cache_key: Some("session-1786".into()),
            ..Context::default()
        };

        let cached = build_payload(&served_by("openai", "gpt-5.6-terra"), &context);
        assert_eq!(cached["prompt_cache_key"], "session-1786");
        // Nothing is left behind on the provider's side.
        assert_eq!(cached["store"], false);

        // A gateway keeps a prompt only for the few minutes it takes to answer, so
        // naming the conversation buys nothing and is left out.
        let elsewhere = build_payload(&model(), &context);
        assert!(elsewhere.get("prompt_cache_key").is_none());
        assert_eq!(elsewhere["store"], false);

        // A service that does not understand being told not to store is not told.
        let reimplementation = build_payload(&served_by("cerebras", "gpt-oss-120b"), &context);
        assert!(reimplementation.get("store").is_none());
    }

    /// The field has a limit, and a longer name is refused rather than truncated, so it is
    /// cut before it is sent.
    #[test]
    fn a_name_too_long_to_send_is_cut() {
        let context = Context {
            cache_key: Some("x".repeat(200)),
            ..Context::default()
        };
        let payload = build_payload(&served_by("openai", "gpt-5.6-terra"), &context);
        assert_eq!(
            payload["prompt_cache_key"].as_str().unwrap().len(),
            PROMPT_CACHE_KEY_MAX
        );
    }

    /// A conversation holding tool calls still declares tools, even when there are none
    /// left to offer: some endpoints refuse it otherwise.
    #[test]
    fn a_conversation_with_tool_history_still_declares_tools() {
        let context = Context {
            messages: vec![
                Message::user("read it"),
                Message::tool_result("call_1", "read", "done", false),
            ],
            ..Context::default()
        };
        let payload = build_payload(&served_by("openai", "gpt-5.6-terra"), &context);
        assert_eq!(payload["tools"], serde_json::json!([]));

        // A conversation with no tools in it at all says nothing about them.
        let plain = build_payload(&served_by("openai", "gpt-5.6-terra"), &Context::default());
        assert!(plain.get("tools").is_none());
    }

    /// Copilot bills a request the user started differently from one the agent continued.
    #[test]
    fn copilot_is_told_who_started_the_request() {
        assert_eq!(initiator(&[Message::user("hi")]), "user");
        assert_eq!(
            initiator(&[
                Message::user("hi"),
                Message::tool_result("call_1", "read", "done", false)
            ]),
            "agent"
        );
        assert_eq!(initiator(&[]), "agent");
    }

    #[test]
    fn an_image_anywhere_in_the_conversation_is_declared() {
        let image = micro_types::ContentBlock::Image {
            data: "AAAA".into(),
            mime_type: "image/png".into(),
        };

        assert!(!carries_images(&[Message::user("no pictures")]));
        assert!(carries_images(&[Message::User {
            content: vec![image.clone()],
            timestamp: 0,
        }]));
        // A tool that returned one counts too.
        assert!(carries_images(&[Message::ToolResult {
            tool_call_id: "call_1".into(),
            tool_name: "read".into(),
            content: vec![image],
            is_error: false,
            timestamp: 0,
        }]));
    }
}
