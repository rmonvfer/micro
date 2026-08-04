//! The ChatGPT Codex backend — OpenAI's Responses API as the ChatGPT app reaches it.
//!
//! This is not the OpenAI platform API. It is `chatgpt.com/backend-api/codex/responses`,
//! authenticated with the JWT a ChatGPT subscription login issues, and it speaks the
//! Responses shape rather than chat completions: the conversation is a list of typed
//! `input` items, and the answer arrives as `response.*` events rather than as choices
//! and deltas.
//!
//! The account the request is billed to is not a header the caller supplies; it is a claim
//! inside the token, so it is read from there and sent back as `chatgpt-account-id`.

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
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

pub(crate) const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// Where the account id lives inside the token, as OpenAI namespaces its claims.
const AUTH_CLAIM: &str = "https://api.openai.com/auth";

/// How the answer comes back.
///
/// The backend offers the same conversation over a socket as well as over SSE. Streaming
/// is what micro uses; the other values are recorded so a configuration written for one is
/// not silently ignored by the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// Server-sent events, which is what this speaks.
    #[default]
    Sse,
    /// Let the provider decide. It decides on SSE.
    Auto,
}

impl Transport {
    /// The value a user wrote, or nothing when it names a transport this cannot speak.
    pub fn named(name: &str) -> Option<Transport> {
        match name.trim().to_ascii_lowercase().as_str() {
            "sse" => Some(Transport::Sse),
            "auto" => Some(Transport::Auto),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Transport::Sse => "sse",
            Transport::Auto => "auto",
        }
    }
}

#[derive(Clone)]
pub struct Codex {
    client: reqwest::Client,
    transport: Transport,
}

impl Default for Codex {
    fn default() -> Self {
        Codex::new()
    }
}

impl Codex {
    pub fn new() -> Self {
        Codex {
            client: crate::http_client(),
            transport: Transport::default(),
        }
    }

    /// How the answer should come back. Both values stream; the choice is recorded so the
    /// setting means the same thing here as it does where it is written down.
    pub fn with_transport(mut self, transport: Transport) -> Self {
        self.transport = transport;
        self
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }
}

impl Provider for Codex {
    fn name(&self) -> &str {
        "openai-codex"
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

async fn run(
    client: reqwest::Client,
    model: Model,
    context: Context,
    api_key: String,
    sender: &UnboundedSender<StreamEvent>,
) -> Result<(), String> {
    let account = account_id(&api_key)?;
    // One id for the request, sent as both the session and the client request id, which is
    // what lets the backend tie a retry to the attempt it repeats.
    let request_id = format!("micro-{}", now_ms());

    let request = client
        .post(endpoint(&model.base_url))
        .header("authorization", format!("Bearer {api_key}"))
        .header("chatgpt-account-id", account)
        .header("originator", "micro")
        .header("user-agent", user_agent())
        .header("openai-beta", "responses=experimental")
        .header("session-id", &request_id)
        .header("x-client-request-id", &request_id)
        .header("accept", "text/event-stream")
        .header("content-type", "application/json");
    let response = crate::with_carried_headers(request, &context)
        .json(&build_payload(&model, &context))
        .send()
        .await
        .map_err(|error| format!("Codex request failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Codex returned {}: {}",
            status.as_u16(),
            body.trim()
        ));
    }

    let mut state = Accumulator::new(&model);
    read_sse(response, |event| state.handle(event, sender))
        .await
        .map_err(|error| format!("Codex stream failed: {error}"))?;

    match state.failure.take() {
        Some(message) => Err(message),
        None => {
            if !state.finished {
                state.finish(sender);
            }
            Ok(())
        }
    }
}

fn endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let trimmed = match trimmed.is_empty() {
        true => CODEX_BASE_URL,
        false => trimmed,
    };
    if trimmed.ends_with("/codex/responses") {
        return trimmed.to_string();
    }
    if trimmed.ends_with("/codex") {
        return format!("{trimmed}/responses");
    }
    format!("{trimmed}/codex/responses")
}

fn user_agent() -> String {
    format!("micro ({} {})", std::env::consts::OS, std::env::consts::ARCH)
}

/// The account a subscription token belongs to, read from the token's own claims.
///
/// The backend bills against this and rejects a request without it, so a token that does
/// not carry one is not a token this endpoint will accept.
fn account_id(token: &str) -> Result<String, String> {
    let claims = jwt_claims(token)
        .ok_or_else(|| "the Codex credential is not a token this endpoint accepts".to_string())?;
    claims
        .get(AUTH_CLAIM)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "the Codex credential carries no ChatGPT account".to_string())
}

/// The middle segment of a JWT, decoded. Nothing is verified: the signature is the
/// backend's business, and this only reads a claim the backend itself put there.
fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64_url_decode(payload)?;
    serde_json::from_slice(&decoded).ok()
}

/// Base64url without padding, which is how a JWT segment is written.
fn base64_url_decode(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut accumulator: u32 = 0;
    let mut bits = 0;

    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let value = ALPHABET.iter().position(|candidate| *candidate == byte)? as u32;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    Some(out)
}

/// The request, in the Responses shape the backend expects.
fn build_payload(model: &Model, context: &Context) -> Value {
    let mut payload = json!({
        "model": model.id,
        // The backend refuses anything else: a conversation it stored would be one micro
        // could not account for.
        "store": false,
        "stream": true,
        "instructions": match context.system_prompt.as_deref() {
            Some(prompt) if !prompt.trim().is_empty() => prompt,
            _ => "You are a helpful assistant.",
        },
        "input": input_items(&context.messages),
        "text": { "verbosity": "low" },
        "include": ["reasoning.encrypted_content"],
        "tool_choice": "auto",
        "parallel_tool_calls": true,
    });

    if !context.tools.is_empty() {
        payload["tools"] = Value::Array(
            context
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    })
                })
                .collect(),
        );
    }

    if let Some(effort) = reasoning_effort(model.thinking) {
        payload["reasoning"] = json!({ "effort": effort, "summary": "auto" });
    }

    payload
}

fn reasoning_effort(level: ThinkingLevel) -> Option<&'static str> {
    match level {
        ThinkingLevel::Off => None,
        ThinkingLevel::Low => Some("low"),
        ThinkingLevel::Medium => Some("medium"),
        ThinkingLevel::High => Some("high"),
    }
}

/// The conversation as typed input items.
///
/// A tool call and the result answering it are separate items joined by `call_id`, which
/// is why a call keeps the id it was given rather than being renumbered.
fn input_items(messages: &[Message]) -> Vec<Value> {
    let mut items = Vec::new();
    for message in messages {
        match message {
            Message::User { content, .. } => {
                let parts: Vec<Value> = content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => {
                            Some(json!({ "type": "input_text", "text": text }))
                        }
                        ContentBlock::Image { data, mime_type } => Some(json!({
                            "type": "input_image",
                            "detail": "auto",
                            "image_url": format!("data:{mime_type};base64,{data}"),
                        })),
                        _ => None,
                    })
                    .collect();
                if !parts.is_empty() {
                    items.push(json!({ "role": "user", "content": parts }));
                }
            }
            Message::Assistant(assistant) => {
                for block in &assistant.content {
                    match block {
                        ContentBlock::Text { text } if !text.is_empty() => items.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "status": "completed",
                            "content": [{ "type": "output_text", "text": text, "annotations": [] }],
                        })),
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                            ..
                        } => items.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": arguments.to_string(),
                        })),
                        _ => {}
                    }
                }
            }
            Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => items.push(json!({
                "type": "function_call_output",
                "call_id": tool_call_id,
                "output": content
                    .iter()
                    .map(ContentBlock::as_text)
                    .collect::<Vec<_>>()
                    .join("\n"),
            })),
        }
    }
    items
}

/// The answer, assembled from the events that describe it.
///
/// The Responses stream numbers its own output slots, and micro numbers content blocks, so
/// the two are kept in step here rather than assumed equal.
struct Accumulator {
    provider: String,
    model: String,
    blocks: Vec<Slot>,
    usage: Usage,
    stop_reason: StopReason,
    finished: bool,
    failure: Option<String>,
}

/// One output slot, and what it is collecting.
struct Slot {
    output_index: u64,
    index: usize,
    kind: Kind,
    buffer: String,
    id: String,
    name: String,
}

#[derive(PartialEq)]
enum Kind {
    Text,
    Thinking,
    ToolCall,
}

impl Accumulator {
    fn new(model: &Model) -> Self {
        Accumulator {
            provider: model.provider.clone(),
            model: model.id.clone(),
            blocks: Vec::new(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            finished: false,
            failure: None,
        }
    }

    fn handle(&mut self, event: SseEvent, sender: &UnboundedSender<StreamEvent>) {
        if event.data.trim() == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
            return;
        };
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        match kind.as_str() {
            "response.created" => {
                let _ = sender.send(StreamEvent::Start);
            }
            "response.output_text.delta" => {
                self.delta(&value, Kind::Text, sender);
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                self.delta(&value, Kind::Thinking, sender);
            }
            "response.reasoning_summary_part.done" => {
                // A summary part is one paragraph of the reasoning; the next one starts
                // after a blank line rather than running into it.
                self.delta_text(&value, Kind::Thinking, "\n\n", sender);
            }
            "response.function_call_arguments.delta" => {
                self.tool_delta(&value, sender);
            }
            "response.output_item.added" => {
                self.open(&value, sender);
            }
            "response.output_item.done" => {
                self.close(&value, sender);
            }
            "response.completed" | "response.incomplete" => {
                self.complete(&value, sender);
            }
            "response.failed" => {
                self.failure = Some(failure_message(&value));
            }
            "error" => {
                let message = value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("the Codex backend reported an error");
                self.failure = Some(message.to_string());
            }
            _ => {}
        }
    }

    /// The slot an event belongs to, opening one when the event is the first sight of it.
    fn slot_for(
        &mut self,
        output_index: u64,
        kind: Kind,
        sender: &UnboundedSender<StreamEvent>,
    ) -> usize {
        if let Some(position) = self
            .blocks
            .iter()
            .position(|slot| slot.output_index == output_index && slot.kind == kind)
        {
            return position;
        }

        let index = self.blocks.len();
        let started = match kind {
            Kind::Text => StreamEvent::TextStart { index },
            Kind::Thinking => StreamEvent::ThinkingStart { index },
            Kind::ToolCall => StreamEvent::ToolCallStart {
                index,
                id: String::new(),
                name: String::new(),
            },
        };
        if !matches!(kind, Kind::ToolCall) {
            let _ = sender.send(started);
        }
        self.blocks.push(Slot {
            output_index,
            index,
            kind,
            buffer: String::new(),
            id: String::new(),
            name: String::new(),
        });
        index
    }

    fn delta(&mut self, value: &Value, kind: Kind, sender: &UnboundedSender<StreamEvent>) {
        let delta = value
            .get("delta")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if delta.is_empty() {
            return;
        }
        self.delta_text(value, kind, &delta, sender);
    }

    fn delta_text(
        &mut self,
        value: &Value,
        kind: Kind,
        delta: &str,
        sender: &UnboundedSender<StreamEvent>,
    ) {
        let output_index = value.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        let is_text = matches!(kind, Kind::Text);
        let position = self.slot_for(output_index, kind, sender);
        let slot = &mut self.blocks[position];
        slot.buffer.push_str(delta);

        let index = slot.index;
        let _ = sender.send(match is_text {
            true => StreamEvent::TextDelta {
                index,
                delta: delta.to_string(),
            },
            false => StreamEvent::ThinkingDelta {
                index,
                delta: delta.to_string(),
            },
        });
    }

    fn tool_delta(&mut self, value: &Value, sender: &UnboundedSender<StreamEvent>) {
        let delta = value
            .get("delta")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if delta.is_empty() {
            return;
        }
        let output_index = value.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        let position = self.slot_for(output_index, Kind::ToolCall, sender);
        let slot = &mut self.blocks[position];
        slot.buffer.push_str(&delta);
        let index = slot.index;
        let _ = sender.send(StreamEvent::ToolCallDelta { index, delta });
    }

    /// A slot the stream has just opened. A tool call is announced here, because this is
    /// where its name first arrives.
    fn open(&mut self, value: &Value, sender: &UnboundedSender<StreamEvent>) {
        let Some(item) = value.get("item") else { return };
        let output_index = value.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();

        let kind = match item_type {
            "message" => Kind::Text,
            "reasoning" => Kind::Thinking,
            "function_call" => Kind::ToolCall,
            _ => return,
        };
        let announces = matches!(kind, Kind::ToolCall);
        let position = self.slot_for(output_index, kind, sender);

        if announces {
            let slot = &mut self.blocks[position];
            slot.id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            slot.name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let _ = sender.send(StreamEvent::ToolCallStart {
                index: slot.index,
                id: slot.id.clone(),
                name: slot.name.clone(),
            });
        }
    }

    /// A slot the stream has finished. The item carries the whole thing, which is what is
    /// kept: a delta stream can be lossy, an item is not.
    fn close(&mut self, value: &Value, sender: &UnboundedSender<StreamEvent>) {
        let Some(item) = value.get("item") else { return };
        let output_index = value.get("output_index").and_then(Value::as_u64).unwrap_or(0);
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();

        match item_type {
            "message" => {
                let position = self.slot_for(output_index, Kind::Text, sender);
                let text = item
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("")
                    })
                    .filter(|text| !text.is_empty());
                let slot = &mut self.blocks[position];
                if let Some(text) = text {
                    slot.buffer = text;
                }
                let _ = sender.send(StreamEvent::TextEnd {
                    index: slot.index,
                    text: slot.buffer.clone(),
                });
            }
            "reasoning" => {
                let position = self.slot_for(output_index, Kind::Thinking, sender);
                let summary = item
                    .get("summary")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .filter(|text| !text.is_empty());
                let slot = &mut self.blocks[position];
                if let Some(summary) = summary {
                    slot.buffer = summary;
                }
                let _ = sender.send(StreamEvent::ThinkingEnd {
                    index: slot.index,
                    thinking: slot.buffer.clone(),
                });
            }
            "function_call" => {
                let position = self.slot_for(output_index, Kind::ToolCall, sender);
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let slot = &mut self.blocks[position];
                if slot.id.is_empty() {
                    slot.id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                if slot.name.is_empty() {
                    slot.name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                if let Some(arguments) = arguments {
                    slot.buffer = arguments;
                }
                let parsed = serde_json::from_str::<Value>(&slot.buffer).unwrap_or(json!({}));
                self.stop_reason = StopReason::ToolUse;
                let _ = sender.send(StreamEvent::ToolCallEnd {
                    index: slot.index,
                    id: slot.id.clone(),
                    name: slot.name.clone(),
                    arguments: parsed,
                });
            }
            _ => {}
        }
    }

    fn complete(&mut self, value: &Value, sender: &UnboundedSender<StreamEvent>) {
        if let Some(response) = value.get("response") {
            if let Some(usage) = response.get("usage") {
                self.usage = read_usage(usage);
            }
            if let Some(status) = response.get("status").and_then(Value::as_str) {
                self.stop_reason = match (status, self.stop_reason) {
                    (_, StopReason::ToolUse) => StopReason::ToolUse,
                    ("incomplete", _) => StopReason::Length,
                    _ => StopReason::Stop,
                };
            }
        }
        self.finish(sender);
    }

    fn finish(&mut self, sender: &UnboundedSender<StreamEvent>) {
        if self.finished {
            return;
        }
        self.finished = true;

        let mut content = Vec::new();
        for slot in &self.blocks {
            match slot.kind {
                Kind::Text if !slot.buffer.is_empty() => {
                    content.push(ContentBlock::text(slot.buffer.clone()))
                }
                Kind::Thinking if !slot.buffer.is_empty() => content.push(ContentBlock::Thinking {
                    thinking: slot.buffer.clone(),
                    signature: None,
                }),
                Kind::ToolCall if !slot.id.is_empty() => content.push(ContentBlock::ToolCall {
                    id: slot.id.clone(),
                    name: slot.name.clone(),
                    arguments: serde_json::from_str(&slot.buffer).unwrap_or(json!({})),
                    signature: None,
                }),
                _ => {}
            }
        }

        let _ = sender.send(StreamEvent::Done {
            message: AssistantMessage {
                content,
                provider: self.provider.clone(),
                model: self.model.clone(),
                usage: self.usage,
                stop_reason: self.stop_reason,
                error: None,
                timestamp: now_ms(),
            },
        });
    }
}

/// Token counts, as the Responses API reports them. Cached input is reported inside the
/// input count rather than beside it, so it is taken back out to leave the two disjoint.
fn read_usage(usage: &Value) -> Usage {
    let number = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0) as u32;
    let cached = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;

    Usage {
        input: number("input_tokens").saturating_sub(cached),
        output: number("output_tokens"),
        cache_read: cached,
        cache_write: 0,
    }
}

fn failure_message(value: &Value) -> String {
    let response = value.get("response");
    if let Some(error) = response.and_then(|response| response.get("error")) {
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("no message");
        return format!("{code}: {message}");
    }
    if let Some(reason) = response
        .and_then(|response| response.get("incomplete_details"))
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str)
    {
        return format!("incomplete: {reason}");
    }
    "the Codex backend refused the request".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::ToolDefinition;

    /// A token with the claim the backend puts there, signed by nobody: only the claim is
    /// read, and the signature is the backend's to check.
    fn token(claims: Value) -> String {
        let encode = |bytes: &[u8]| {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let mut buffer = [0_u8; 3];
                buffer[..chunk.len()].copy_from_slice(chunk);
                let value = ((buffer[0] as u32) << 16) | ((buffer[1] as u32) << 8) | buffer[2] as u32;
                let characters = chunk.len() + 1;
                for position in 0..characters {
                    let shift = 18 - position * 6;
                    out.push(ALPHABET[((value >> shift) & 0x3f) as usize] as char);
                }
            }
            out
        };
        format!(
            "header.{}.signature",
            encode(claims.to_string().as_bytes())
        )
    }

    fn model() -> Model {
        Model {
            id: "gpt-5-codex".into(),
            provider: "openai-codex".into(),
            base_url: CODEX_BASE_URL.into(),
            max_tokens: 8192,
            thinking: ThinkingLevel::Off,
        }
    }

    #[test]
    fn the_account_comes_out_of_the_token() {
        let token = token(json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_123" }
        }));
        assert_eq!(account_id(&token).unwrap(), "acct_123");
    }

    #[test]
    fn a_credential_without_an_account_is_refused_before_it_is_spent() {
        let token = token(json!({ "sub": "someone" }));
        let error = account_id(&token).expect_err("no account");
        assert!(error.contains("no ChatGPT account"), "{error}");

        let error = account_id("not-a-token").expect_err("not a token");
        assert!(error.contains("not a token this endpoint accepts"), "{error}");
    }

    #[test]
    fn the_endpoint_is_built_from_whatever_base_it_is_given() {
        assert_eq!(
            endpoint(CODEX_BASE_URL),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            endpoint("https://chatgpt.com/backend-api/codex"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            endpoint("https://chatgpt.com/backend-api/codex/responses"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            endpoint(""),
            "https://chatgpt.com/backend-api/codex/responses"
        );
    }

    /// The backend refuses a stored conversation, and takes the system prompt as
    /// `instructions` rather than as a message.
    #[test]
    fn the_request_is_shaped_the_way_the_backend_takes_it() {
        let context = Context {
            system_prompt: Some("be brief".into()),
            messages: vec![Message::user("what does this do")],
            tools: vec![ToolDefinition {
                name: "read".into(),
                description: "read a file".into(),
                parameters: json!({ "type": "object" }),
            }],
            headers: Vec::new(),
            cache_key: None,
        };
        let payload = build_payload(&model(), &context);

        assert_eq!(payload["store"], false);
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["instructions"], "be brief");
        assert_eq!(payload["input"][0]["role"], "user");
        assert_eq!(payload["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(payload["tools"][0]["type"], "function");
        assert_eq!(payload["tools"][0]["name"], "read");
        assert!(payload.get("reasoning").is_none(), "effort was off");
    }

    #[test]
    fn reasoning_effort_rides_with_the_request_when_it_is_asked_for() {
        let mut model = model();
        model.thinking = ThinkingLevel::High;
        let payload = build_payload(
            &model,
            &Context {
                system_prompt: None,
                messages: vec![Message::user("think about it")],
                tools: Vec::new(),
                headers: Vec::new(),
                cache_key: None,
            },
        );
        assert_eq!(payload["reasoning"]["effort"], "high");
        assert_eq!(payload["reasoning"]["summary"], "auto");
        assert_eq!(payload["instructions"], "You are a helpful assistant.");
    }

    /// A call and the result answering it are joined by the id the call was given.
    #[test]
    fn a_tool_exchange_keeps_the_id_that_joins_it() {
        let messages = vec![
            Message::user("read it"),
            Message::Assistant(AssistantMessage {
                content: vec![ContentBlock::ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: json!({ "path": "src/main.rs" }),
                    signature: None,
                }],
                provider: "openai-codex".into(),
                model: "gpt-5-codex".into(),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error: None,
                timestamp: 0,
            }),
            Message::tool_result("call_1", "read", "fn main() {}", false),
        ];

        let items = input_items(&messages);
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["call_id"], "call_1");
        assert_eq!(items[2]["type"], "function_call_output");
        assert_eq!(items[2]["call_id"], "call_1");
        assert_eq!(items[2]["output"], "fn main() {}");
    }

    fn drain(events: Vec<Value>) -> Vec<StreamEvent> {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut state = Accumulator::new(&model());
        for event in events {
            state.handle(
                SseEvent {
                    event: None,
                    data: event.to_string(),
                },
                &sender,
            );
        }
        if !state.finished {
            state.finish(&sender);
        }
        drop(sender);

        let mut out = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            out.push(event);
        }
        out
    }

    #[test]
    fn an_answer_arrives_as_text_and_ends_as_one_message() {
        let events = drain(vec![
            json!({ "type": "response.created" }),
            json!({ "type": "response.output_item.added", "output_index": 0,
                    "item": { "type": "message" } }),
            json!({ "type": "response.output_text.delta", "output_index": 0, "delta": "half " }),
            json!({ "type": "response.output_text.delta", "output_index": 0, "delta": "an answer" }),
            json!({ "type": "response.output_item.done", "output_index": 0,
                    "item": { "type": "message",
                              "content": [{ "type": "output_text", "text": "half an answer" }] } }),
            json!({ "type": "response.completed",
                    "response": { "status": "completed",
                                  "usage": { "input_tokens": 130, "output_tokens": 20,
                                             "input_tokens_details": { "cached_tokens": 30 } } } }),
        ]);

        assert!(matches!(events.first(), Some(StreamEvent::Start)));
        let Some(StreamEvent::Done { message }) = events.last() else {
            panic!("the last event is the message");
        };
        assert_eq!(message.text(), "half an answer");
        assert_eq!(message.usage.input, 100, "cached tokens are counted apart");
        assert_eq!(message.usage.cache_read, 30);
        assert_eq!(message.usage.output, 20);
        assert_eq!(message.stop_reason, StopReason::Stop);
    }

    #[test]
    fn reasoning_arrives_as_thinking_and_paragraphs_stay_apart() {
        let events = drain(vec![
            json!({ "type": "response.output_item.added", "output_index": 0,
                    "item": { "type": "reasoning" } }),
            json!({ "type": "response.reasoning_summary_text.delta", "output_index": 0,
                    "delta": "first thought" }),
            json!({ "type": "response.reasoning_summary_part.done", "output_index": 0 }),
            json!({ "type": "response.reasoning_summary_text.delta", "output_index": 0,
                    "delta": "second thought" }),
            json!({ "type": "response.completed", "response": { "status": "completed" } }),
        ]);

        let Some(StreamEvent::Done { message }) = events.last() else {
            panic!("the last event is the message");
        };
        let ContentBlock::Thinking { thinking, .. } = &message.content[0] else {
            panic!("reasoning is thinking");
        };
        assert_eq!(thinking, "first thought\n\nsecond thought");
    }

    #[test]
    fn a_tool_call_is_announced_streamed_and_closed() {
        let events = drain(vec![
            json!({ "type": "response.output_item.added", "output_index": 0,
                    "item": { "type": "function_call", "call_id": "call_1", "name": "read" } }),
            json!({ "type": "response.function_call_arguments.delta", "output_index": 0,
                    "delta": "{\"path\":" }),
            json!({ "type": "response.function_call_arguments.delta", "output_index": 0,
                    "delta": "\"src/main.rs\"}" }),
            json!({ "type": "response.output_item.done", "output_index": 0,
                    "item": { "type": "function_call", "call_id": "call_1", "name": "read",
                              "arguments": "{\"path\":\"src/main.rs\"}" } }),
            json!({ "type": "response.completed", "response": { "status": "completed" } }),
        ]);

        assert!(events.iter().any(|event| matches!(
            event,
            StreamEvent::ToolCallStart { id, name, .. } if id == "call_1" && name == "read"
        )));
        let Some(StreamEvent::Done { message }) = events.last() else {
            panic!("the last event is the message");
        };
        assert_eq!(message.stop_reason, StopReason::ToolUse);
        let ContentBlock::ToolCall { arguments, .. } = &message.content[0] else {
            panic!("a call was made");
        };
        assert_eq!(arguments["path"], "src/main.rs");
    }

    /// Two answers in one response keep their own blocks rather than running together.
    #[test]
    fn separate_output_items_stay_separate_blocks() {
        let events = drain(vec![
            json!({ "type": "response.output_text.delta", "output_index": 0, "delta": "first" }),
            json!({ "type": "response.output_text.delta", "output_index": 1, "delta": "second" }),
            json!({ "type": "response.completed", "response": { "status": "completed" } }),
        ]);
        let Some(StreamEvent::Done { message }) = events.last() else {
            panic!("the last event is the message");
        };
        assert_eq!(message.content.len(), 2);
        assert_eq!(message.text(), "firstsecond");
    }

    #[test]
    fn a_refusal_is_reported_rather_than_finished() {
        let (sender, _receiver) = mpsc::unbounded_channel();
        let mut state = Accumulator::new(&model());
        state.handle(
            SseEvent {
                event: None,
                data: json!({ "type": "response.failed",
                              "response": { "error": { "code": "rate_limit",
                                                       "message": "slow down" } } })
                .to_string(),
            },
            &sender,
        );
        assert_eq!(state.failure.as_deref(), Some("rate_limit: slow down"));
    }

    #[test]
    fn only_the_transports_this_speaks_are_accepted() {
        assert_eq!(Transport::named("sse"), Some(Transport::Sse));
        assert_eq!(Transport::named("AUTO"), Some(Transport::Auto));
        assert_eq!(Transport::named("websocket"), None);
        assert_eq!(Codex::new().transport(), Transport::Sse);
        assert_eq!(
            Codex::new().with_transport(Transport::Auto).transport(),
            Transport::Auto
        );
    }
}
