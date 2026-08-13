//! The Anthropic Messages API.

use crate::json::parse_arguments;
use crate::json::read_str;
use crate::json::read_u32;
use crate::sse::read_sse;
use crate::Provider;
use micro_types::now_ms;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::ToolDefinition;
use micro_types::StreamEvent;
use micro_types::Usage;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

const API_VERSION: &str = "2023-06-01";

/// What an Anthropic subscription credential looks like. A plan's own token is an OAuth
/// token, and it is sent as a bearer rather than as an API key.
const OAUTH_PREFIX: &str = "sk-ant-oat";
/// What every credential Anthropic issues starts with.
const ANTHROPIC_KEY_PREFIX: &str = "sk-ant-";

/// Lets a thinking model keep thinking between tool calls instead of starting over each
/// time. Without it the thinking blocks around a tool call are refused.
const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";
/// Streams a tool call's arguments as they are produced rather than in one piece.
const FINE_GRAINED_TOOL_STREAMING_BETA: &str = "fine-grained-tool-streaming-2025-05-14";
/// What a subscription credential is allowed to be used for, and by what.
const CLAUDE_CODE_BETA: &str = "claude-code-20250219";
const OAUTH_BETA: &str = "oauth-2025-04-20";
/// The client a subscription credential is issued to.
const CLAUDE_CODE_VERSION: &str = "2.1.75";
/// Anthropic allows at most four cache breakpoints per request.
const MAX_CACHE_BREAKPOINTS: usize = 4;

#[derive(Clone, Default)]
pub struct Anthropic {
    client: reqwest::Client,
}

impl Anthropic {
    pub fn new() -> Self {
        Anthropic {
            client: crate::http_client(),
        }
    }
}

impl Provider for Anthropic {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn stream(
        &self,
        model: Model,
        context: Context,
        api_key: String,
    ) -> UnboundedReceiver<StreamEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let client = self.client.clone();

        tokio::spawn(async move {
            if let Err(message) = run(client, model, context, api_key, &sender).await {
                let _ = sender.send(StreamEvent::Error { message });
            }
        });

        receiver
    }
}

/// The tools Claude Code declares, in the casing it declares them with.
///
/// A subscription credential is issued to that client, and a request made with one is
/// expected to look like one of its requests: a tool micro spells differently is spelled
/// this way on the way out, and answered under its own name on the way back.
///
/// Source: https://cchistory.mariozechner.at/data/prompts-2.1.11.md
const CLAUDE_CODE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

/// One tool's name as Claude Code spells it, or as it was given when that client has no
/// tool by that name.
fn claude_code_name(name: &str) -> String {
    CLAUDE_CODE_TOOLS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .map(|known| known.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// The name the caller knows a tool by, for a call that came back under another spelling.
fn declared_name(name: &str, tools: &[ToolDefinition]) -> String {
    tools
        .iter()
        .find(|tool| tool.name.eq_ignore_ascii_case(name))
        .map(|tool| tool.name.clone())
        .unwrap_or_else(|| name.to_string())
}

async fn run(
    client: reqwest::Client,
    model: Model,
    context: Context,
    api_key: String,
    sender: &UnboundedSender<StreamEvent>,
) -> Result<(), String> {
    let subscription = is_oauth(&api_key);
    let payload = build_payload(&model, &context, subscription);
    let request = client
        .post(endpoint(&model.base_url))
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("anthropic-dangerous-direct-browser-access", "true")
        .header("anthropic-beta", betas(&api_key, &model, &context).join(","));
    // A subscription credential is a bearer token issued to a named client, and is
    // refused when it is sent as an API key. A gateway's token is a bearer too, but
    // carries none of that client's identity.
    let request = match scheme_for(&api_key) {
        AuthScheme::Subscription => request
            .header("authorization", format!("Bearer {api_key}"))
            .header("user-agent", format!("claude-cli/{CLAUDE_CODE_VERSION}"))
            .header("x-app", "cli"),
        AuthScheme::Bearer => request.header("authorization", format!("Bearer {api_key}")),
        AuthScheme::ApiKey => request.header("x-api-key", api_key),
    };
    let response = crate::with_carried_headers(request, &context)
        .json(&payload)
        .send()
        .await
        .map_err(|error| format!("Anthropic request failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Anthropic returned {}: {}",
            status.as_u16(),
            body.trim()
        ));
    }

    let mut state = Accumulator::new(&model, &context.tools, subscription);
    read_sse(response, |event| state.handle(event, sender))
        .await
        .map_err(|error| format!("Anthropic stream failed: {error}"))?;

    // A body that ended without `message_stop` still carries usable content.
    if !state.finished {
        let _ = sender.send(StreamEvent::Done {
            message: state.build(),
        });
    }
    Ok(())
}

/// Where a Messages request goes.
///
/// The catalog records a service's root, the way Anthropic's own clients take it, and the
/// version belongs to the protocol rather than to the address: every Messages-compatible
/// service answers at `/v1/messages` under whatever root it is given.
fn endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/messages") {
        return trimmed.to_string();
    }
    if trimmed.ends_with("/v1") {
        return format!("{trimmed}/messages");
    }
    format!("{trimmed}/v1/messages")
}

/// Which block the stream is currently inside, plus the text accumulated for it.
enum OpenBlock {
    Text {
        index: usize,
        text: String,
    },
    Thinking {
        index: usize,
        thinking: String,
        signature: String,
    },
    ToolCall {
        index: usize,
        id: String,
        name: String,
        json: String,
    },
    Other,
}

struct Accumulator {
    /// The tools the caller declared, when their names were changed on the way out. A
    /// call is answered under the name the caller knows.
    tools: Vec<ToolDefinition>,
    provider: String,
    model_id: String,
    blocks: Vec<ContentBlock>,
    usage: Usage,
    stop_reason: StopReason,
    open: Option<OpenBlock>,
    finished: bool,
}

impl Accumulator {
    fn new(model: &Model, tools: &[ToolDefinition], subscription: bool) -> Self {
        Accumulator {
            tools: match subscription {
                true => tools.to_vec(),
                false => Vec::new(),
            },
            provider: model.provider.clone(),
            model_id: model.id.clone(),
            blocks: Vec::new(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            open: None,
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

    fn handle(&mut self, event: crate::SseEvent, sender: &UnboundedSender<StreamEvent>) {
        let Some(name) = event.event.as_deref() else {
            return;
        };
        let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
            return;
        };

        match name {
            "message_start" => {
                if let Some(usage) = data.pointer("/message/usage") {
                    self.usage.input += read_u32(usage, "input_tokens");
                    self.usage.output += read_u32(usage, "output_tokens");
                    self.usage.cache_read += read_u32(usage, "cache_read_input_tokens");
                    self.usage.cache_write += read_u32(usage, "cache_creation_input_tokens");
                }
                let _ = sender.send(StreamEvent::Start);
            }

            "content_block_start" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = data.get("content_block");
                let block_type = block.and_then(|b| b.get("type")).and_then(Value::as_str);

                match block_type {
                    Some("text") => {
                        self.open = Some(OpenBlock::Text {
                            index,
                            text: String::new(),
                        });
                        let _ = sender.send(StreamEvent::TextStart { index });
                    }
                    Some("thinking") => {
                        self.open = Some(OpenBlock::Thinking {
                            index,
                            thinking: String::new(),
                            signature: String::new(),
                        });
                        let _ = sender.send(StreamEvent::ThinkingStart { index });
                    }
                    Some("tool_use") => {
                        let id = read_str(block, "id");
                        let name = declared_name(&read_str(block, "name"), &self.tools);
                        self.open = Some(OpenBlock::ToolCall {
                            index,
                            id: id.clone(),
                            name: name.clone(),
                            json: String::new(),
                        });
                        let _ = sender.send(StreamEvent::ToolCallStart { index, id, name });
                    }
                    Some("redacted_thinking") => {
                        // Delivered whole, with no deltas. Preserved so it can be replayed.
                        let data = read_str(block, "data");
                        if !data.is_empty() {
                            self.blocks.push(ContentBlock::RedactedThinking { data });
                        }
                        self.open = Some(OpenBlock::Other);
                    }
                    _ => self.open = Some(OpenBlock::Other),
                }
            }

            "content_block_delta" => {
                let delta = data.get("delta");
                let delta_type = delta.and_then(|d| d.get("type")).and_then(Value::as_str);

                match (delta_type, self.open.as_mut()) {
                    (Some("text_delta"), Some(OpenBlock::Text { index, text })) => {
                        let fragment = read_str(delta, "text");
                        text.push_str(&fragment);
                        let _ = sender.send(StreamEvent::TextDelta {
                            index: *index,
                            delta: fragment,
                        });
                    }
                    (
                        Some("thinking_delta"),
                        Some(OpenBlock::Thinking {
                            index, thinking, ..
                        }),
                    ) => {
                        let fragment = read_str(delta, "thinking");
                        thinking.push_str(&fragment);
                        let _ = sender.send(StreamEvent::ThinkingDelta {
                            index: *index,
                            delta: fragment,
                        });
                    }
                    (Some("signature_delta"), Some(OpenBlock::Thinking { signature, .. })) => {
                        // Required to replay the thinking block on the next turn.
                        signature.push_str(&read_str(delta, "signature"));
                    }
                    (Some("input_json_delta"), Some(OpenBlock::ToolCall { index, json, .. })) => {
                        let fragment = read_str(delta, "partial_json");
                        json.push_str(&fragment);
                        let _ = sender.send(StreamEvent::ToolCallDelta {
                            index: *index,
                            delta: fragment,
                        });
                    }
                    _ => {}
                }
            }

            "content_block_stop" => match self.open.take() {
                Some(OpenBlock::Text { index, text }) => {
                    if !text.trim().is_empty() {
                        self.blocks.push(ContentBlock::Text { text: text.clone() });
                    }
                    let _ = sender.send(StreamEvent::TextEnd { index, text });
                }
                Some(OpenBlock::Thinking {
                    index,
                    thinking,
                    signature,
                }) => {
                    if !thinking.trim().is_empty() {
                        self.blocks.push(ContentBlock::Thinking {
                            thinking: thinking.clone(),
                            signature: (!signature.is_empty()).then_some(signature),
                        });
                    }
                    let _ = sender.send(StreamEvent::ThinkingEnd { index, thinking });
                }
                Some(OpenBlock::ToolCall {
                    index,
                    id,
                    name,
                    json,
                }) => {
                    let arguments = parse_arguments(&json);
                    self.blocks.push(ContentBlock::ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),

                        signature: None,
                    });
                    let _ = sender.send(StreamEvent::ToolCallEnd {
                        index,
                        id,
                        name,
                        arguments,
                    });
                }
                _ => {}
            },

            "message_delta" => {
                if let Some(reason) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = map_stop_reason(reason);
                }
                if let Some(usage) = data.get("usage") {
                    let output = read_u32(usage, "output_tokens");
                    if output > 0 {
                        self.usage.output = output;
                    }
                }
            }

            "message_stop" => {
                self.finished = true;
                let _ = sender.send(StreamEvent::Done {
                    message: self.build(),
                });
            }

            "error" => {
                // Anthropic reports overload and rate limits mid-stream as an error event.
                self.finished = true;
                let message = data
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or(&event.data)
                    .to_string();
                let _ = sender.send(StreamEvent::Error {
                    message: format!("Anthropic stream error: {message}"),
                });
            }

            _ => {}
        }
    }
}

fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "max_tokens" => StopReason::Length,
        "tool_use" => StopReason::ToolUse,
        "refusal" => StopReason::Error,
        _ => StopReason::Stop,
    }
}

/// Anthropic tool-use ids must be alphanumeric with dashes or underscores.
fn normalize_tool_id(id: &str) -> String {
    let sanitized: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    sanitized.chars().take(40).collect()
}

/// Whether a credential is a subscription token rather than an API key.
fn is_oauth(api_key: &str) -> bool {
    api_key.starts_with(OAUTH_PREFIX)
}

/// How a credential is presented to the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthScheme {
    /// A subscription token issued to Claude Code, sent as a bearer under that client's
    /// name.
    Subscription,
    /// A bearer token that is nobody's client in particular, which is what an
    /// Anthropic-compatible gateway issues.
    Bearer,
    /// A platform API key.
    ApiKey,
}

/// Which scheme a credential is for, read from the credential itself.
///
/// Anthropic's own credentials are prefixed: `sk-ant-oat` for a subscription token and
/// `sk-ant-` for a platform key. A value with neither prefix did not come from Anthropic,
/// so it belongs to a gateway answering the same protocol, and those take a plain bearer
/// token with none of Claude Code's identity attached to it.
fn scheme_for(api_key: &str) -> AuthScheme {
    if is_oauth(api_key) {
        AuthScheme::Subscription
    } else if api_key.starts_with(ANTHROPIC_KEY_PREFIX) {
        AuthScheme::ApiKey
    } else {
        AuthScheme::Bearer
    }
}

/// The beta features this request needs, in the order ohm sends them.
///
/// A subscription credential names what it is being used for; everything else is asked for
/// only when the request would otherwise be answered differently.
fn betas(api_key: &str, model: &Model, context: &Context) -> Vec<&'static str> {
    let mut betas = Vec::new();
    if is_oauth(api_key) {
        betas.push(CLAUDE_CODE_BETA);
        betas.push(OAUTH_BETA);
    }
    if !context.tools.is_empty() {
        if !model.compat.supports_eager_tool_input_streaming {
            betas.push(FINE_GRAINED_TOOL_STREAMING_BETA);
        }
        // Only worth asking for when there is thinking to interleave with the calls.
        if model.thinking.budget_tokens().is_some() {
            betas.push(INTERLEAVED_THINKING_BETA);
        }
    }
    betas
}

/// A tool's name as it should be sent.
fn name_for(name: &str, subscription: bool) -> String {
    match subscription {
        true => claude_code_name(name),
        false => name.to_string(),
    }
}

/// What a thinking level is called when the model is asked for an effort rather than a
/// budget. `Off` never reaches here — a model told not to think is sent `disabled`.
fn effort_for(level: micro_types::ThinkingLevel) -> &'static str {
    match level {
        micro_types::ThinkingLevel::Off => "low",
        micro_types::ThinkingLevel::Low => "low",
        micro_types::ThinkingLevel::Medium => "medium",
        micro_types::ThinkingLevel::High => "high",
    }
}

pub(crate) fn build_payload(model: &Model, context: &Context, subscription: bool) -> Value {
    let mut payload = Map::new();
    payload.insert("model".into(), json!(model.id));
    payload.insert("max_tokens".into(), json!(model.max_tokens));
    payload.insert("stream".into(), json!(true));

    match model.thinking.budget_tokens() {
        // A model that decides its own thinking is asked for an effort instead of a
        // budget, and spends it as it sees fit. Sending it a budget asks for a shape it
        // does not use.
        Some(_) if model.compat.force_adaptive_thinking => {
            payload.insert(
                "thinking".into(),
                json!({ "type": "adaptive", "display": "summarized" }),
            );
            payload.insert(
                "output_config".into(),
                json!({ "effort": effort_for(model.thinking) }),
            );
        }
        Some(budget) => {
            payload.insert(
                "thinking".into(),
                json!({ "type": "enabled", "budget_tokens": budget }),
            );
        }
        // Said outright rather than left out: a model that thinks by default keeps
        // thinking when nothing tells it not to, and bills for it.
        None => {
            payload.insert("thinking".into(), json!({ "type": "disabled" }));
        }
    }

    if let Some(system) = &context.system_prompt {
        payload.insert("system".into(), json!(system));
    }

    payload.insert(
        "messages".into(),
        Value::Array(build_messages(&context.messages, subscription)),
    );

    if !context.tools.is_empty() {
        let tools: Vec<Value> = context
            .tools
            .iter()
            .map(|tool| {
                let mut described = json!({
                    "name": name_for(&tool.name, subscription),
                    "description": tool.description,
                    "input_schema": tool.parameters,
                });
                // A service that streams a tool's arguments as they are decided is told
                // to; one that does not is asked for the same thing as a beta instead.
                if model.compat.supports_eager_tool_input_streaming {
                    described["eager_input_streaming"] = json!(true);
                }
                described
            })
            .collect();
        payload.insert("tools".into(), Value::Array(tools));
    }

    let mut payload = Value::Object(payload);
    apply_cache_breakpoints(&mut payload, model.compat.supports_cache_control_on_tools);
    payload
}

/// Convert the conversation to Anthropic's wire shape.
///
/// Consecutive tool results are merged into a single user message because Anthropic
/// expects every `tool_use` in an assistant turn to be answered by `tool_result` blocks
/// grouped in the user turn that follows it.
fn build_messages(messages: &[Message], subscription: bool) -> Vec<Value> {
    let mut wire: Vec<Value> = Vec::new();
    let mut pending_results: Vec<Value> = Vec::new();

    let flush = |pending: &mut Vec<Value>, wire: &mut Vec<Value>| {
        if !pending.is_empty() {
            wire.push(json!({ "role": "user", "content": std::mem::take(pending) }));
        }
    };

    for message in messages {
        match message {
            Message::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } => {
                let text = content
                    .iter()
                    .map(ContentBlock::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut block = Map::new();
                block.insert("type".into(), json!("tool_result"));
                block.insert("tool_use_id".into(), json!(normalize_tool_id(tool_call_id)));
                block.insert("content".into(), json!(text));
                if *is_error {
                    block.insert("is_error".into(), json!(true));
                }
                pending_results.push(Value::Object(block));
            }
            Message::User { content, .. } => {
                flush(&mut pending_results, &mut wire);
                wire.push(json!({ "role": "user", "content": encode_blocks(content, subscription) }));
            }
            Message::Assistant(assistant) => {
                flush(&mut pending_results, &mut wire);
                let content = encode_blocks(&assistant.content, subscription);
                if !content.is_empty() {
                    wire.push(json!({ "role": "assistant", "content": content }));
                }
            }
        }
    }

    flush(&mut pending_results, &mut wire);
    wire
}

fn encode_blocks(blocks: &[ContentBlock], subscription: bool) -> Vec<Value> {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
            ContentBlock::Thinking {
                thinking,
                signature,
            } => {
                let mut object = Map::new();
                object.insert("type".into(), json!("thinking"));
                object.insert("thinking".into(), json!(thinking));
                // Anthropic rejects a thinking block replayed without its signature.
                match signature {
                    Some(signature) => {
                        object.insert("signature".into(), json!(signature));
                        Some(Value::Object(object))
                    }
                    None => None,
                }
            }
            ContentBlock::RedactedThinking { data } => {
                Some(json!({ "type": "redacted_thinking", "data": data }))
            }
            ContentBlock::Image { data, mime_type } => Some(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": mime_type, "data": data },
            })),
            // A reasoning signature on a call is Gemini's, and Anthropic has nowhere to put
            // one, so it is left behind rather than replayed into a format that would
            // reject it.
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some(json!({
                "type": "tool_use",
                "id": normalize_tool_id(id),
                "name": name_for(name, subscription),
                "input": arguments,
            })),
        })
        .collect()
}

/// Mark up to four cache breakpoints: the last tool definition, the system prompt, and
/// the final two user turns. Everything before a breakpoint is served from cache on the
/// next request that shares the same prefix.
pub(crate) fn apply_cache_breakpoints(payload: &mut Value, on_tools: bool) {
    let cache_control = json!({ "type": "ephemeral" });
    let mut remaining = MAX_CACHE_BREAKPOINTS;

    // Some services answering this protocol reject a marker on a tool definition, so
    // there the breakpoints start with the system prompt instead.
    if on_tools {
        if let Some(tools) = payload.get_mut("tools").and_then(Value::as_array_mut) {
            if let Some(last) = tools.last_mut().and_then(Value::as_object_mut) {
                last.insert("cache_control".into(), cache_control.clone());
                remaining -= 1;
            }
        }
    }

    if remaining > 0 {
        if let Some(system) = payload.get_mut("system") {
            if let Some(text) = system.as_str() {
                *system = json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": cache_control.clone(),
                }]);
                remaining -= 1;
            }
        }
    }

    let Some(messages) = payload.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.get("role").and_then(Value::as_str) == Some("user"))
        .map(|(index, _)| index)
        .collect();

    // Oldest first, so that when only one breakpoint is left it lands on the final turn.
    let targets = user_indices.iter().rev().take(2).rev().copied();
    for index in targets {
        if remaining == 0 {
            break;
        }
        if let Some(content) = messages[index]
            .get_mut("content")
            .and_then(Value::as_array_mut)
        {
            if let Some(last) = content.last_mut().and_then(Value::as_object_mut) {
                last.insert("cache_control".into(), cache_control.clone());
                remaining -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::ToolDefinition;

    fn context_with(messages: Vec<Message>) -> Context {
        Context {
            system_prompt: Some("be brief".into()),
            messages,
            tools: vec![ToolDefinition {
                name: "read".into(),
                description: "read a file".into(),
                parameters: json!({ "type": "object" }),
            }],
            headers: Vec::new(),
            cache_key: None,
        }
    }

    /// Anthropic takes JSON Schema as it is written, so nothing here may filter it. A
    /// parameter named after a schema keyword is the case that catches a filter creeping in.
    #[test]
    fn tool_schemas_reach_anthropic_exactly_as_written() {
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
        let payload = build_payload(&Model::anthropic("claude-opus-5"), &context, false);

        assert_eq!(payload["tools"][0]["input_schema"], parameters);
    }

    #[test]
    fn endpoint_is_not_doubled_up() {
        assert_eq!(
            endpoint("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint("https://api.anthropic.com/v1/messages/"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    /// A service's root is recorded without the protocol version, which is added here.
    #[test]
    fn a_service_root_gains_the_version_the_protocol_lives_under() {
        assert_eq!(
            endpoint("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint("https://api.kimi.com/coding"),
            "https://api.kimi.com/coding/v1/messages"
        );
    }

    #[test]
    fn consecutive_tool_results_merge_into_one_user_turn() {
        let messages = vec![
            Message::user("go"),
            Message::Assistant(AssistantMessage {
                content: vec![ContentBlock::ToolCall {
                    id: "a".into(),
                    name: "read".into(),
                    arguments: json!({}),

                    signature: None,
                }],
                provider: "anthropic".into(),
                model: "claude-opus-5".into(),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error: None,
                timestamp: 0,
            }),
            Message::tool_result("a", "read", "first", false),
            Message::tool_result("b", "read", "second", false),
        ];
        let wire = build_messages(&messages, false);

        assert_eq!(wire.len(), 3);
        assert_eq!(wire[2]["role"], "user");
        assert_eq!(wire[2]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn thinking_without_a_signature_is_dropped() {
        let blocks = vec![
            ContentBlock::Thinking {
                thinking: "unsigned".into(),
                signature: None,
            },
            ContentBlock::text("kept"),
        ];
        let encoded = encode_blocks(&blocks, false);
        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0]["type"], "text");
    }

    #[test]
    fn cache_breakpoints_land_on_tools_system_and_last_user_turns() {
        let payload = build_payload(
            &Model::anthropic("claude-opus-5"),
            &context_with(vec![Message::user("one"), Message::user("two")]),
            false,
        );

        assert_eq!(payload["tools"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(payload["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            payload["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(
            payload["messages"][1]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn tool_ids_are_sanitized_and_bounded() {
        assert_eq!(normalize_tool_id("call/with:punct"), "call_with_punct");
        assert_eq!(normalize_tool_id(&"x".repeat(60)).len(), 40);
    }

    #[test]
    fn truncated_tool_arguments_fall_back_to_an_empty_object() {
        assert_eq!(parse_arguments("{\"a\":"), json!({}));
        assert_eq!(parse_arguments(""), json!({}));
        assert_eq!(parse_arguments("{\"a\":1}"), json!({ "a": 1 }));
    }

    #[test]
    fn thinking_budget_is_sent_only_when_enabled() {
        let plain = build_payload(&Model::anthropic("claude-opus-5"), &Context::default(), false);
        // Turned off outright rather than left unsaid, so a model that thinks by default
        // does not keep thinking and billing for it.
        assert_eq!(plain["thinking"]["type"], "disabled");

        let thinking = build_payload(
            &Model::anthropic("claude-opus-5").with_thinking(micro_types::ThinkingLevel::High),
            &Context::default(),
            false,
        );
        assert_eq!(thinking["thinking"]["budget_tokens"], 32_000);
    }

    /// A subscription credential is a bearer token issued to a named client, and says so.
    #[test]
    fn a_subscription_credential_asks_for_what_it_is_allowed() {
        let model = Model::anthropic("claude-opus-5");
        let context = context_with(vec![Message::user("hi")]);

        let asked = betas("sk-ant-oat01-abc", &model, &context);
        assert!(asked.contains(&CLAUDE_CODE_BETA), "{asked:?}");
        assert!(asked.contains(&OAUTH_BETA), "{asked:?}");
        assert!(is_oauth("sk-ant-oat01-abc"));
    }

    #[test]
    fn an_api_key_asks_for_nothing_it_does_not_need() {
        let model = Model::anthropic("claude-opus-5");
        let without_tools = Context {
            tools: Vec::new(),
            ..context_with(vec![Message::user("hi")])
        };

        assert!(betas("sk-ant-api03-abc", &model, &without_tools).is_empty());
        assert!(!is_oauth("sk-ant-api03-abc"));
    }

    /// Keeping the thinking between tool calls is asked for only when there is thinking
    /// to keep. Streaming a call's arguments is asked for per tool where that is taken,
    /// and as a beta where it is not.
    #[test]
    fn tools_and_thinking_ask_for_what_they_need() {
        let context = context_with(vec![Message::user("hi")]);
        let model = Model::anthropic("claude-opus-5");

        let plain = betas("sk-ant-api03-abc", &model, &context);
        assert!(!plain.contains(&FINE_GRAINED_TOOL_STREAMING_BETA), "{plain:?}");
        assert!(!plain.contains(&INTERLEAVED_THINKING_BETA), "{plain:?}");
        assert_eq!(
            build_payload(&model, &context, false)["tools"][0]["eager_input_streaming"],
            true
        );

        let mut legacy = model.clone();
        legacy.compat.supports_eager_tool_input_streaming = false;
        let asked = betas("sk-ant-api03-abc", &legacy, &context);
        assert!(asked.contains(&FINE_GRAINED_TOOL_STREAMING_BETA), "{asked:?}");
        assert!(build_payload(&legacy, &context, false)["tools"][0]
            .get("eager_input_streaming")
            .is_none());

        let thinking = model.with_thinking(micro_types::ThinkingLevel::High);
        let asked = betas("sk-ant-api03-abc", &thinking, &context);
        assert!(asked.contains(&INTERLEAVED_THINKING_BETA), "{asked:?}");
    }

    /// A subscription credential is issued to Claude Code, so a request made with one
    /// names its tools the way that client names them, and a call comes back under the
    /// name the caller declared.
    #[test]
    fn a_subscription_request_names_tools_the_way_its_client_does() {
        let context = context_with(vec![Message::user("hi")]);
        let model = Model::anthropic("claude-opus-5");

        let sent = build_payload(&model, &context, true);
        assert_eq!(sent["tools"][0]["name"], "Read");
        assert_eq!(
            build_payload(&model, &context, false)["tools"][0]["name"],
            "read"
        );

        assert_eq!(declared_name("Read", &context.tools), "read");
        // A tool that client has never heard of is sent as it was given.
        assert_eq!(claude_code_name("compact"), "compact");
    }

    /// Thinking is turned off outright, because a model that thinks by default keeps
    /// thinking when nothing says otherwise.
    #[test]
    fn thinking_is_turned_off_rather_than_left_unsaid() {
        let payload = build_payload(
            &Model::anthropic("claude-opus-5"),
            &context_with(vec![Message::user("hi")]),
            false,
        );
        assert_eq!(payload["thinking"]["type"], "disabled");

        let thinking =
            Model::anthropic("claude-opus-5").with_thinking(micro_types::ThinkingLevel::Medium);
        let payload = build_payload(&thinking, &context_with(vec![Message::user("hi")]), false);
        assert_eq!(payload["thinking"]["type"], "enabled");
        assert!(payload["thinking"]["budget_tokens"].as_u64().unwrap() > 0);
    }

    /// A model that decides its own thinking is asked for an effort. The budget shape is
    /// what older models take, and sending it to a newer one asks for something it does
    /// not use.
    #[test]
    fn a_model_that_decides_its_own_thinking_is_asked_for_an_effort() {
        let mut model =
            Model::anthropic("claude-opus-5").with_thinking(micro_types::ThinkingLevel::High);
        model.compat.force_adaptive_thinking = true;

        let payload = build_payload(&model, &context_with(vec![Message::user("hi")]), false);
        assert_eq!(payload["thinking"]["type"], "adaptive");
        assert_eq!(payload["thinking"]["display"], "summarized");
        assert_eq!(payload["output_config"]["effort"], "high");
        assert!(
            payload["thinking"].get("budget_tokens").is_none(),
            "a budget is not what this model takes",
        );
    }

    /// Turning thinking off is the same for both shapes: said outright, with no effort
    /// alongside it to argue with.
    #[test]
    fn an_adaptive_model_still_turns_thinking_off_outright() {
        let mut model = Model::anthropic("claude-opus-5");
        model.compat.force_adaptive_thinking = true;

        let payload = build_payload(&model, &context_with(vec![Message::user("hi")]), false);
        assert_eq!(payload["thinking"]["type"], "disabled");
        assert!(payload.get("output_config").is_none());
    }
}

#[cfg(test)]
mod auth_scheme {
    use super::*;

    /// Each kind of credential is presented the way the service expects it.
    #[test]
    fn a_credential_is_presented_by_what_it_is() {
        assert_eq!(scheme_for("sk-ant-oat01-abc"), AuthScheme::Subscription);
        assert_eq!(scheme_for("sk-ant-api03-abc"), AuthScheme::ApiKey);
        // Neither prefix: a gateway answering the same protocol, which takes a plain
        // bearer token with none of Claude Code's identity on it.
        assert_eq!(scheme_for("glsa_abc123"), AuthScheme::Bearer);
        assert_eq!(scheme_for("eyJhbGciOi.payload.sig"), AuthScheme::Bearer);
    }

    /// Only a subscription credential carries the client's name and betas.
    #[test]
    fn only_a_subscription_names_the_client() {
        let model = Model::anthropic("claude-opus-5");
        let context = micro_types::Context {
            messages: vec![Message::user("hi")],
            ..Default::default()
        };
        assert!(betas("sk-ant-oat01-abc", &model, &context).contains(&CLAUDE_CODE_BETA));
        assert!(!betas("glsa_abc123", &model, &context).contains(&CLAUDE_CODE_BETA));
        assert!(!betas("sk-ant-api03-abc", &model, &context).contains(&CLAUDE_CODE_BETA));
    }
}
