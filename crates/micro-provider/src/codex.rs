

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    
    #[default]
    Sse,
    /// Let the provider decide.
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

/// Which service answers the Responses protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// `chatgpt.com/backend-api/codex`, reached with a ChatGPT subscription token.
    ChatGpt,
    /// The OpenAI platform, reached with an API key.
    Platform,
    /// Azure's hosting of the same protocol.
    Azure,
}

#[derive(Clone)]
pub struct Codex {
    client: reqwest::Client,
    transport: Transport,
    backend: Backend,
    /// Which service is answering, for the headers it expects to be told about.
    provider: String,
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
            backend: Backend::ChatGpt,
            provider: micro_auth::OPENAI_CODEX.to_string(),
        }
    }

    
    pub fn platform() -> Self {
        Codex {
            client: crate::http_client(),
            transport: Transport::default(),
            backend: Backend::Platform,
            provider: micro_auth::OPENAI.to_string(),
        }
    }

    /// The same protocol as Azure hosts it.
    pub fn azure() -> Self {
        Codex {
            client: crate::http_client(),
            transport: Transport::default(),
            backend: Backend::Azure,
            provider: AZURE_PROVIDER.to_string(),
        }
    }

    /// The platform's protocol as a named service serves it.
    pub fn for_provider(provider: impl Into<String>) -> Self {
        Codex {
            client: crate::http_client(),
            transport: Transport::default(),
            backend: Backend::Platform,
            provider: provider.into(),
        }
    }

    /// How the answer should come back.
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
        &self.provider
    }

    fn stream(
        &self,
        model: Model,
        context: Context,
        api_key: String,
    ) -> UnboundedReceiver<StreamEvent> {
        let payload = match self.request_payload(&model, &context, &api_key) {
            Ok(payload) => payload,
            Err(error) => return crate::error_stream(error),
        };
        self.stream_prepared(model, context, api_key, payload)
    }

    fn stream_prepared(
        &self,
        model: Model,
        context: Context,
        api_key: String,
        payload: Value,
    ) -> UnboundedReceiver<StreamEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let client = self.client.clone();
        let backend = self.backend;
        let provider = self.provider.clone();

        tokio::spawn(async move {
            if let Err(message) =
                run(
                    client, backend, provider, model, context, api_key, payload, &sender,
                )
                .await
            {
                let _ = sender.send(StreamEvent::Error { message });
            }
        });

        receiver
    }

    fn payload(&self, model: &Model, context: &Context) -> Value {
        build_payload(self.backend, model, context).unwrap_or(Value::Null)
    }

    fn request_payload(
        &self,
        model: &Model,
        context: &Context,
        _api_key: &str,
    ) -> Result<Value, String> {
        build_payload(self.backend, model, context)
    }
}

async fn run(
    client: reqwest::Client,
    backend: Backend,
    provider: String,
    model: Model,
    context: Context,
    api_key: String,
    payload: Value,
    sender: &UnboundedSender<StreamEvent>,
) -> Result<(), String> {
    let service = provider.clone();

    let request = client
        .post(endpoint_for(backend, &model.base_url))
        .header("accept", "text/event-stream")
        .header("content-type", "application/json");
    
    let request = match backend {
        Backend::Azure => request.header("api-key", &api_key),
        _ => request.header("authorization", format!("Bearer {api_key}")),
    };

    
    let request = match backend {
        Backend::ChatGpt => {
            
            let request_id = session_name(context.cache_key.as_deref());
            request
                .header("chatgpt-account-id", account_id(&api_key)?)
                .header("originator", "micro")
                .header("user-agent", user_agent())
                .header("openai-beta", "responses=experimental")
                .header("session-id", &request_id)
                .header("x-client-request-id", request_id)
        }
        
        Backend::Platform | Backend::Azure => request,
    };

    
    let mut request = request;
    for (name, value) in &model.headers {
        request = request.header(name.as_str(), value.as_str());
    }
    
    if crate::openai::is_copilot(&provider, &model.base_url) {
        request = request
            .header("x-initiator", crate::openai::initiator(&context.messages))
            .header("openai-intent", "conversation-edits");
        if crate::openai::carries_images(&context.messages) {
            request = request.header("copilot-vision-request", "true");
        }
    }

    let response = crate::with_carried_headers(request, &context, &model.base_url)
        .json(&payload)
        .send()
        .await
        .map_err(|error| format!("{service} request failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "{service} returned {}: {}",
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

/// Where a Responses request goes, which depends on which service is answering.
fn endpoint_for(backend: Backend, base_url: &str) -> String {
    match backend {
        Backend::ChatGpt => endpoint(base_url),
        Backend::Platform => {
            let trimmed = base_url.trim_end_matches('/');
            match trimmed.ends_with("/responses") {
                true => trimmed.to_string(),
                false => format!("{trimmed}/responses"),
            }
        }
        Backend::Azure => azure_endpoint(base_url),
    }
}

/// Where an Azure resource answers.
fn azure_endpoint(base_url: &str) -> String {
    
    let named = std::env::var(AZURE_RESOURCE_ENV)
        .ok()
        .map(|resource| resource.trim().to_string())
        .filter(|resource| !resource.is_empty())
        .map(|resource| format!("https://{resource}.openai.azure.com/openai/v1"));
    let base_url = named.as_deref().unwrap_or(base_url);
    let trimmed = base_url.trim().trim_end_matches('/');
    let (address, _) = trimmed.split_once('?').unwrap_or((trimmed, ""));
    let address = address.trim_end_matches('/');

    let root = if address.ends_with("/openai/v1") {
        address.to_string()
    } else if address.ends_with("/openai") {
        format!("{address}/v1")
    } else if address.ends_with("/responses") {
        address
            .trim_end_matches("/responses")
            .trim_end_matches('/')
            .to_string()
    } else {
        format!("{address}/openai/v1")
    };

    let version = std::env::var(AZURE_API_VERSION_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| AZURE_API_VERSION.to_string());
    format!("{root}/responses?api-version={version}")
}

/// What the resource calls the deployment serving a model.
fn azure_deployment(model_id: &str) -> String {
    let Ok(map) = std::env::var(AZURE_DEPLOYMENT_MAP_ENV) else {
        return model_id.to_string();
    };
    for pair in map.split(',') {
        let Some((id, deployment)) = pair.trim().split_once('=') else {
            continue;
        };
        if id.trim() == model_id {
            return deployment.trim().to_string();
        }
    }
    model_id.to_string()
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
    format!(
        "micro ({} {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

/// What this conversation is called on the wire.
fn session_name(cache_key: Option<&str>) -> String {
    static PROCESS: std::sync::OnceLock<String> = std::sync::OnceLock::new();

    let named = match cache_key.filter(|key| !key.is_empty()) {
        Some(key) => key.to_string(),
        None => {
            return PROCESS
                .get_or_init(|| format!("micro-{}", now_ms()))
                .clone()
        }
    };
    let safe: String = named
        .chars()
        .map(
            |character| match character.is_ascii_alphanumeric() || character == '_' {
                true => character,
                false => '-',
            },
        )
        .collect();
    format!("micro-{safe}")
}

/// The account a subscription token belongs to, read from the token's own claims.
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

/// The middle segment of a JWT, decoded.
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
const MIN_OUTPUT_TOKENS: u32 = 16;

/// The provider id Azure's hosting is listed under.
pub const AZURE_PROVIDER: &str = "azure-openai-responses";
/// Which version of the protocol Azure is asked for, when nothing says otherwise.
const AZURE_API_VERSION: &str = "v1";
/// Maps a model id to what the resource calls its deployment, as `id=deployment` pairs.
const AZURE_DEPLOYMENT_MAP_ENV: &str = "AZURE_OPENAI_DEPLOYMENT_NAME_MAP";
const AZURE_API_VERSION_ENV: &str = "AZURE_OPENAI_API_VERSION";

const AZURE_RESOURCE_ENV: &str = "AZURE_OPENAI_RESOURCE_NAME";

fn build_payload(backend: Backend, model: &Model, context: &Context) -> Result<Value, String> {
    let mut payload = json!({
        "model": model.id,
        
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
        let mut tools: Vec<Value> = Vec::with_capacity(context.tools.len());
        for tool in &context.tools {
            let strict = crate::constrained_sampling::resolve_json_schema_strict_sampling(
                tool,
                model.compat.supports_strict_mode,
            )?;
            let parameters =
                crate::constrained_sampling::json_schema_tool_parameters(tool, strict)?;
            let mut described = json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": parameters,
            });
            
            if model.compat.supports_strict_mode {
                described["strict"] = match strict {
                    Some(true) => Value::Bool(true),
                    _ => Value::Null,
                };
            }
            tools.push(described);
        }
        payload["tools"] = Value::Array(tools);
    }

    if let Some(effort) = reasoning_effort(model.thinking) {
        payload["reasoning"] = json!({ "effort": effort, "summary": "auto" });
    }

    
    if matches!(backend, Backend::Platform | Backend::Azure) {
        payload["max_output_tokens"] = json!(model.max_tokens.max(MIN_OUTPUT_TOKENS));
    }
    
    if backend == Backend::Azure {
        payload["model"] = json!(azure_deployment(&model.id));
    }

    Ok(payload)
}

fn reasoning_effort(level: ThinkingLevel) -> Option<&'static str> {
    match level {
        ThinkingLevel::Off => None,
        ThinkingLevel::Minimal => Some("low"),
        ThinkingLevel::Low => Some("low"),
        ThinkingLevel::Medium => Some("medium"),
        ThinkingLevel::High => Some("high"),
        ThinkingLevel::XHigh => Some("high"),
        ThinkingLevel::Max => Some("high"),
    }
}

/// The conversation as typed input items.
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
        let output_index = value
            .get("output_index")
            .and_then(Value::as_u64)
            .unwrap_or(0);
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
        let output_index = value
            .get("output_index")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let position = self.slot_for(output_index, Kind::ToolCall, sender);
        let slot = &mut self.blocks[position];
        slot.buffer.push_str(&delta);
        let index = slot.index;
        let _ = sender.send(StreamEvent::ToolCallDelta { index, delta });
    }

    /// A slot the stream has just opened.
    fn open(&mut self, value: &Value, sender: &UnboundedSender<StreamEvent>) {
        let Some(item) = value.get("item") else {
            return;
        };
        let output_index = value
            .get("output_index")
            .and_then(Value::as_u64)
            .unwrap_or(0);
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

    /// A slot the stream has finished.
    fn close(&mut self, value: &Value, sender: &UnboundedSender<StreamEvent>) {
        let Some(item) = value.get("item") else {
            return;
        };
        let output_index = value
            .get("output_index")
            .and_then(Value::as_u64)
            .unwrap_or(0);
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

/// Token counts, as the Responses API reports them.
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

    
    fn token(claims: Value) -> String {
        let encode = |bytes: &[u8]| {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let mut buffer = [0_u8; 3];
                buffer[..chunk.len()].copy_from_slice(chunk);
                let value =
                    ((buffer[0] as u32) << 16) | ((buffer[1] as u32) << 8) | buffer[2] as u32;
                let characters = chunk.len() + 1;
                for position in 0..characters {
                    let shift = 18 - position * 6;
                    out.push(ALPHABET[((value >> shift) & 0x3f) as usize] as char);
                }
            }
            out
        };
        format!("header.{}.signature", encode(claims.to_string().as_bytes()))
    }

    fn model() -> Model {
        Model {
            id: "gpt-5-codex".into(),
            provider: "openai-codex".into(),
            base_url: CODEX_BASE_URL.into(),
            max_tokens: 8192,
            thinking: ThinkingLevel::Off,
            compat: Default::default(),
            headers: Default::default(),
            reasoning: Default::default(),
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
        assert!(
            error.contains("not a token this endpoint accepts"),
            "{error}"
        );
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

    
    #[test]
    fn the_request_is_shaped_the_way_the_backend_takes_it() {
        let context = Context {
            system_prompt: Some("be brief".into()),
            messages: vec![Message::user("what does this do")],
            tools: vec![ToolDefinition {
                name: "read".into(),
                description: "read a file".into(),
                parameters: json!({ "type": "object" }),
                constrained_sampling: None,
            }],
            headers: Vec::new(),
            cache_key: None,
        };
        let payload = build_payload(Backend::ChatGpt, &model(), &context).unwrap();

        assert_eq!(payload["store"], false);
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["instructions"], "be brief");
        assert_eq!(payload["input"][0]["role"], "user");
        assert_eq!(payload["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(payload["tools"][0]["type"], "function");
        assert_eq!(payload["tools"][0]["name"], "read");
        
        assert_eq!(payload["tools"][0]["strict"], Value::Null);
        assert!(payload.get("reasoning").is_none(), "effort was off");
    }

    fn tool_asking_for_json_schema_sampling(
        strict: micro_types::JsonSchemaStrictness,
    ) -> ToolDefinition {
        ToolDefinition {
            name: "grep".into(),
            description: "search".into(),
            parameters: json!({
                "type": "object",
                "properties": { "pattern": { "type": "string" } },
                "required": ["pattern"],
            }),
            constrained_sampling: Some(micro_types::ConstrainedSampling::JsonSchema { strict }),
        }
    }

    /// This backend shares the same strict-mode gate and default as plain OpenAI: assumed supported
    /// unless the catalog says otherwise.
    #[test]
    fn a_tool_preferring_strict_sampling_gets_it_by_default() {
        let context = Context {
            tools: vec![tool_asking_for_json_schema_sampling(
                micro_types::JsonSchemaStrictness::Prefer,
            )],
            ..Context::default()
        };
        let payload = build_payload(Backend::ChatGpt, &model(), &context).unwrap();

        assert_eq!(payload["tools"][0]["strict"], true);
        assert_eq!(
            payload["tools"][0]["parameters"]["additionalProperties"],
            false
        );
    }

    
    #[test]
    fn a_schema_that_cannot_be_strict_falls_back_to_null_rather_than_false() {
        let unstrictifiable = ToolDefinition {
            name: "grep".into(),
            description: "search".into(),
            parameters: json!({
                "type": "object",
                "properties": { "target": { "$ref": "#/$defs/target" } },
            }),
            constrained_sampling: Some(micro_types::ConstrainedSampling::JsonSchema {
                strict: micro_types::JsonSchemaStrictness::Prefer,
            }),
        };
        let context = Context {
            tools: vec![unstrictifiable],
            ..Context::default()
        };
        let payload = build_payload(Backend::ChatGpt, &model(), &context).unwrap();

        assert_eq!(payload["tools"][0]["strict"], Value::Null);
    }

    
    #[test]
    fn an_unsupported_service_is_unaffected_by_a_tool_preferring_strict_sampling() {
        let original_parameters = json!({
            "type": "object",
            "properties": { "pattern": { "type": "string" } },
            "required": ["pattern"],
        });
        let mut unsupported = model();
        unsupported.compat.supports_strict_mode = false;
        let context = Context {
            tools: vec![tool_asking_for_json_schema_sampling(
                micro_types::JsonSchemaStrictness::Prefer,
            )],
            ..Context::default()
        };
        let payload = build_payload(Backend::ChatGpt, &unsupported, &context).unwrap();

        assert!(payload["tools"][0].get("strict").is_none());
        assert_eq!(payload["tools"][0]["parameters"], original_parameters);
    }

    
    #[test]
    fn requiring_strict_sampling_on_an_unsupported_service_fails_the_request() {
        let mut unsupported = model();
        unsupported.compat.supports_strict_mode = false;
        let context = Context {
            tools: vec![tool_asking_for_json_schema_sampling(
                micro_types::JsonSchemaStrictness::Require,
            )],
            ..Context::default()
        };
        let error = build_payload(Backend::ChatGpt, &unsupported, &context).unwrap_err();
        assert!(error.contains("\"grep\""), "got {error:?}");
    }

    #[test]
    fn reasoning_effort_rides_with_the_request_when_it_is_asked_for() {
        let mut model = model();
        model.thinking = ThinkingLevel::High;
        let payload = build_payload(
            Backend::ChatGpt,
            &model,
            &Context {
                system_prompt: None,
                messages: vec![Message::user("think about it")],
                tools: Vec::new(),
                headers: Vec::new(),
                cache_key: None,
            },
        )
        .unwrap();
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

#[cfg(test)]
mod platform {
    use super::*;

    fn platform_model() -> Model {
        Model {
            id: "gpt-5.5".into(),
            provider: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            max_tokens: 32_000,
            thinking: ThinkingLevel::Off,
            reasoning: true,
            compat: Default::default(),
            headers: Default::default(),
        }
    }

    /// The platform answers at `/responses`, not at the ChatGPT backend's path.
    #[test]
    fn the_platform_endpoint_is_responses() {
        assert_eq!(
            endpoint_for(Backend::Platform, "https://api.openai.com/v1"),
            "https://api.openai.com/v1/responses"
        );
        
        assert_eq!(
            endpoint_for(Backend::Platform, "https://api.openai.com/v1/responses"),
            "https://api.openai.com/v1/responses"
        );
        
        assert!(
            endpoint_for(Backend::ChatGpt, "https://chatgpt.com/backend-api")
                .ends_with("/codex/responses")
        );
    }

    /// The platform takes an output limit and refuses one below its floor.
    #[test]
    fn the_platform_is_given_an_output_limit() {
        let payload = build_payload(
            Backend::Platform,
            &platform_model(),
            &Context {
                system_prompt: None,
                messages: vec![Message::user("hi")],
                tools: Vec::new(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(payload["max_output_tokens"], 32_000);

        let mut tiny = platform_model();
        tiny.max_tokens = 1;
        let payload = build_payload(
            Backend::Platform,
            &tiny,
            &Context {
                system_prompt: None,
                messages: vec![Message::user("hi")],
                tools: Vec::new(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            payload["max_output_tokens"], MIN_OUTPUT_TOKENS,
            "raised to what the service will accept",
        );
    }

    
    #[test]
    fn reasoning_is_replayed_on_both_backends() {
        for backend in [Backend::ChatGpt, Backend::Platform] {
            let payload = build_payload(
                backend,
                &platform_model(),
                &Context {
                    system_prompt: None,
                    messages: vec![Message::user("hi")],
                    tools: Vec::new(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(payload["include"][0], "reasoning.encrypted_content");
            assert_eq!(payload["store"], false, "nothing is stored either way");
        }
    }

    /// The client says which service it speaks for, so an error names the right one.
    #[test]
    fn each_backend_names_itself() {
        assert_eq!(Codex::new().name(), "openai-codex");
        assert_eq!(Codex::platform().name(), "openai");
    }
}

#[cfg(test)]
mod azure {
    use super::*;

    /// A resource is named by its host; the protocol lives under `/openai/v1` on it, and the
    /// version is asked for in the query.
    #[test]
    fn the_endpoint_is_completed_from_the_resource() {
        assert_eq!(
            azure_endpoint("https://my-resource.openai.azure.com"),
            "https://my-resource.openai.azure.com/openai/v1/responses?api-version=v1"
        );
        
        assert_eq!(
            azure_endpoint("https://my-resource.openai.azure.com/openai/v1"),
            "https://my-resource.openai.azure.com/openai/v1/responses?api-version=v1"
        );
        assert_eq!(
            azure_endpoint("https://my-resource.openai.azure.com/openai"),
            "https://my-resource.openai.azure.com/openai/v1/responses?api-version=v1"
        );
    }

    /// A model reaches a deployment, and a resource may call its deployment anything.
    #[test]
    fn a_model_is_addressed_by_its_deployment() {
        
        assert_eq!(azure_deployment("gpt-5.5"), "gpt-5.5");
    }

    
    #[test]
    fn azure_is_told_apart_from_the_platform() {
        assert_eq!(Codex::azure().name(), AZURE_PROVIDER);
        assert_ne!(Codex::azure().name(), Codex::platform().name());
    }
}
