//! Amazon Bedrock, over the Converse Stream shape.

use crate::eventstream::Decoder;
use crate::json::parse_arguments;
use crate::sigv4;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
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

/// The provider id Bedrock is listed under.
pub const PROVIDER: &str = "amazon-bedrock";
/// What AWS calls this service when signing for it.
const SERVICE: &str = "bedrock";

const DEFAULT_REGION: &str = "us-east-1";

/// Lets a Claude model that thinks to a budget keep thinking between tool calls.
const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";
/// Tokens always left for the answer when a thinking budget shares the response ceiling.
const MIN_ANSWER_TOKENS: u32 = 1_024;

/// Environment variables Bedrock reads, in the order AWS reads them.
const BEARER_TOKEN_ENV: &str = "AWS_BEARER_TOKEN_BEDROCK";
const ACCESS_KEY_ENV: &str = "AWS_ACCESS_KEY_ID";
const SECRET_KEY_ENV: &str = "AWS_SECRET_ACCESS_KEY";
const SESSION_TOKEN_ENV: &str = "AWS_SESSION_TOKEN";
const REGION_ENV: &str = "AWS_REGION";
const DEFAULT_REGION_ENV: &str = "AWS_DEFAULT_REGION";

#[derive(Clone, Default)]
pub struct Bedrock {
    client: reqwest::Client,
}

impl Bedrock {
    pub fn new() -> Self {
        Bedrock {
            client: crate::http_client(),
        }
    }
}

impl crate::Provider for Bedrock {
    fn name(&self) -> &str {
        PROVIDER
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

        tokio::spawn(async move {
            if let Err(message) = run(client, model, context, api_key, payload, &sender).await {
                let _ = sender.send(StreamEvent::Error { message });
            }
        });

        receiver
    }

    fn payload(&self, model: &Model, context: &Context) -> Value {
        build_payload(model, context).unwrap_or(Value::Null)
    }

    fn request_payload(
        &self,
        model: &Model,
        context: &Context,
        _api_key: &str,
    ) -> Result<Value, String> {
        build_payload(model, context)
    }
}

/// How this account proves who it is.
enum Authentication {
    Bearer(String),
    Signed(sigv4::Credentials),
}

fn authentication(api_key: &str) -> Result<Authentication, String> {
    let stored = api_key.trim();
    if !stored.is_empty() {
        return Ok(Authentication::Bearer(stored.to_string()));
    }
    if let Ok(token) = std::env::var(BEARER_TOKEN_ENV) {
        if !token.trim().is_empty() {
            return Ok(Authentication::Bearer(token));
        }
    }

    let access_key_id = std::env::var(ACCESS_KEY_ENV).unwrap_or_default();
    let secret_access_key = std::env::var(SECRET_KEY_ENV).unwrap_or_default();
    if access_key_id.trim().is_empty() || secret_access_key.trim().is_empty() {
        return Err(format!(
            "no Bedrock credentials: set {BEARER_TOKEN_ENV}, or {ACCESS_KEY_ENV} and \
             {SECRET_KEY_ENV}"
        ));
    }

    Ok(Authentication::Signed(sigv4::Credentials {
        access_key_id,
        secret_access_key,
        session_token: std::env::var(SESSION_TOKEN_ENV)
            .ok()
            .filter(|token| !token.trim().is_empty()),
    }))
}

/// Which region serves this account.
pub(crate) fn region(base_url: &str) -> String {
    if let Some(found) = region_in(base_url) {
        return found;
    }
    for name in [REGION_ENV, DEFAULT_REGION_ENV] {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return value.trim().to_string();
            }
        }
    }
    DEFAULT_REGION.to_string()
}

/// The region named in a Bedrock host, which is its second label.
fn region_in(base_url: &str) -> Option<String> {
    let host = base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()?;
    let mut labels = host.split('.');
    let first = labels.next()?;
    if !first.starts_with("bedrock") {
        return None;
    }
    let region = labels.next()?;
    (!region.is_empty() && region != "amazonaws").then(|| region.to_string())
}

pub(crate) fn endpoint(base_url: &str, region: &str, model_id: &str) -> String {
    let root = match base_url.trim().trim_end_matches('/') {
        "" => format!("https://bedrock-runtime.{region}.amazonaws.com"),
        given => given.to_string(),
    };

    let encoded = model_id.replace('/', "%2F");
    format!("{root}/model/{encoded}/converse-stream")
}

async fn run(
    client: reqwest::Client,
    model: Model,
    context: Context,
    api_key: String,
    payload: Value,
    sender: &UnboundedSender<StreamEvent>,
) -> Result<(), String> {
    let region = region(&model.base_url);
    let address = endpoint(&model.base_url, &region, &model.id);
    let body = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;

    let host = address
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let path = address
        .trim_start_matches("https://")
        .find('/')
        .map(|index| address.trim_start_matches("https://")[index..].to_string())
        .unwrap_or_else(|| "/".to_string());

    let mut request = client
        .post(&address)
        .header("content-type", "application/json")
        .header("accept", "application/vnd.amazon.eventstream")
        .header("host", &host);

    match authentication(&api_key)? {
        Authentication::Bearer(token) => {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        Authentication::Signed(credentials) => {
            let signed = sigv4::sign(
                &sigv4::Request {
                    method: "POST",
                    path: &path,
                    query: "",
                    headers: vec![
                        ("host".to_string(), host.clone()),
                        ("content-type".to_string(), "application/json".to_string()),
                    ],
                    body: &body,
                },
                &credentials,
                &region,
                SERVICE,
                &sigv4::timestamp_now(),
            );
            for (name, value) in signed {
                request = request.header(name, value);
            }
        }
    }

    let response = crate::with_carried_headers(request, &context, &model.base_url)
        .body(body)
        .send()
        .await
        .map_err(|error| format!("Bedrock request failed: {error}"))?;

    if !response.status().is_success() {
        return Err(crate::retry::refusal("Bedrock", response).await);
    }

    let mut state = Accumulator::new(&model);
    let mut decoder = Decoder::new();
    let mut response = response;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Bedrock stream failed: {error}"))?
    {
        for frame in decoder.push(&chunk)? {
            if frame.message_type == "exception" {
                let text = String::from_utf8_lossy(&frame.payload);
                return Err(format!("Bedrock returned an exception: {}", text.trim()));
            }
            let event: Value = match serde_json::from_slice(&frame.payload) {
                Ok(event) => event,
                Err(_) => continue,
            };
            state.handle(&frame.event_type, &event, sender);
        }
    }

    if !state.finished {
        state.close_open_blocks(sender);
        let _ = sender.send(StreamEvent::Done {
            message: state.build(),
        });
    }
    Ok(())
}

/// Bedrock's own request shape.
pub(crate) fn build_payload(model: &Model, context: &Context) -> Result<Value, String> {
    let model = &model
        .clone()
        .with_thinking(model.clamp_thinking(model.thinking));
    let mut payload = json!({
        "messages": build_messages(&context.messages, is_anthropic_claude(model)),
        "inferenceConfig": { "maxTokens": model.max_tokens },
    });

    if let Some(fields) = thinking_fields(model) {
        payload["additionalModelRequestFields"] = fields;
    }

    if let Some(system) = context
        .system_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
    {
        payload["system"] = json!([{ "text": system }]);
    }

    if !context.tools.is_empty() {
        let mut tools: Vec<Value> = Vec::with_capacity(context.tools.len());
        for tool in &context.tools {
            let strict = crate::constrained_sampling::resolve_json_schema_strict_sampling(
                tool,
                model.compat.bedrock_supports_strict_tools,
            )?;
            let parameters =
                crate::constrained_sampling::json_schema_tool_parameters(tool, strict)?;
            let mut spec = json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": { "json": parameters },
            });

            if strict == Some(true) {
                spec["strict"] = json!(true);
            }
            tools.push(json!({ "toolSpec": spec }));
        }
        payload["toolConfig"] = json!({ "tools": tools });
    }

    Ok(payload)
}

/// Whether a model is one of Anthropic's Claude models, which think the way Anthropic's own API
/// does and sign what they think.
fn is_anthropic_claude(model: &Model) -> bool {
    let id = model.id.to_lowercase();
    id.contains("anthropic.claude") || id.contains("anthropic/claude")
}

/// Whether the request goes to AWS GovCloud, which does not take Claude's `thinking.display`.
fn is_gov_cloud(model: &Model) -> bool {
    let id = model.id.to_lowercase();
    region(&model.base_url)
        .to_lowercase()
        .starts_with("us-gov-")
        || id.starts_with("us-gov.")
        || id.starts_with("arn:aws-us-gov:")
}

/// The fields Bedrock hands the model untouched, which is where Claude's thinking is asked for.
/// Only Claude models are asked to think; every other model reasons as its own defaults say.
fn thinking_fields(model: &Model) -> Option<Value> {
    if !is_anthropic_claude(model) || model.thinking == ThinkingLevel::Off {
        return None;
    }

    let mut fields = if model.compat.force_adaptive_thinking {
        json!({
            "thinking": { "type": "adaptive" },
            "output_config": { "effort": crate::anthropic::effort_for(model) },
        })
    } else {
        let budget: u32 = match model.thinking {
            ThinkingLevel::Off | ThinkingLevel::Minimal => 1_024,
            ThinkingLevel::Low => 2_048,
            ThinkingLevel::Medium => 8_192,
            ThinkingLevel::High | ThinkingLevel::XHigh | ThinkingLevel::Max => 16_384,
        };
        let budget = budget.min(model.max_tokens.saturating_sub(MIN_ANSWER_TOKENS));
        json!({
            "thinking": { "type": "enabled", "budget_tokens": budget },
            "anthropic_beta": [INTERLEAVED_THINKING_BETA],
        })
    };

    if !is_gov_cloud(model) {
        fields["thinking"]["display"] = json!("summarized");
    }
    Some(fields)
}

/// The conversation as Bedrock reads it. Only Claude takes a signature on replayed reasoning.
fn build_messages(messages: &[Message], signs_reasoning: bool) -> Vec<Value> {
    let mut wire: Vec<Value> = Vec::new();

    for message in messages {
        match message {
            Message::User { content, .. } => {
                wire.push(json!({ "role": "user", "content": user_content(content) }));
            }
            Message::Assistant(assistant) => {
                let content = assistant_content(&assistant.content, signs_reasoning);
                if !content.is_empty() {
                    wire.push(json!({ "role": "assistant", "content": content }));
                }
            }
            Message::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } => {
                let text: String = content.iter().map(ContentBlock::as_text).collect();
                wire.push(json!({
                    "role": "user",
                    "content": [{
                        "toolResult": {
                            "toolUseId": tool_call_id,
                            "content": [{ "text": text }],
                            "status": match is_error {
                                true => "error",
                                false => "success",
                            },
                        }
                    }],
                }));
            }
        }
    }

    wire
}

fn user_content(content: &[ContentBlock]) -> Vec<Value> {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(json!({ "text": text })),
            ContentBlock::Image { data, mime_type } => Some(json!({
                "image": {
                    "format": mime_type.rsplit('/').next().unwrap_or("png"),
                    "source": { "bytes": data },
                }
            })),
            _ => None,
        })
        .collect()
}

fn assistant_content(content: &[ContentBlock], signs_reasoning: bool) -> Vec<Value> {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } if !text.is_empty() => Some(json!({ "text": text })),
            ContentBlock::Thinking {
                thinking,
                signature,
            } => replayed_thinking(thinking, signature.as_deref(), signs_reasoning),
            ContentBlock::RedactedThinking { data } => replayed_redacted_thinking(data),
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some(json!({
                "toolUse": { "toolUseId": id, "name": name, "input": arguments },
            })),
            _ => None,
        })
        .collect()
}

/// Reasoning as it goes back to the model. Claude rejects reasoning without the signature it
/// issued, so unsigned Claude reasoning goes back as plain text; other models reject a signature.
fn replayed_thinking(
    thinking: &str,
    signature: Option<&str>,
    signs_reasoning: bool,
) -> Option<Value> {
    if thinking.trim().is_empty() {
        return None;
    }
    let signature = signature.filter(|signature| !signature.trim().is_empty());
    let replayed = match (signs_reasoning, signature) {
        (true, Some(signature)) => json!({
            "reasoningContent": {
                "reasoningText": { "text": thinking, "signature": signature },
            }
        }),
        (true, None) => json!({ "text": thinking }),
        (false, _) => json!({
            "reasoningContent": { "reasoningText": { "text": thinking } }
        }),
    };
    Some(replayed)
}

/// Encrypted reasoning goes back exactly as it arrived. A payload that is not base64 cannot have
/// come from Bedrock, so it is left out rather than failing the request.
fn replayed_redacted_thinking(data: &str) -> Option<Value> {
    let bytes = BASE64.decode(data).ok().filter(|bytes| !bytes.is_empty())?;
    Some(json!({ "reasoningContent": { "redactedContent": BASE64.encode(bytes) } }))
}

/// A reasoning block still streaming.
struct OpenReasoning {
    /// Where Bedrock numbers this block among the response's content.
    content_index: u64,
    /// Where this block sits in the answer being built.
    position: usize,
    /// Encrypted reasoning so far, which replaces any readable reasoning once the block closes.
    redacted: Vec<u8>,
}

/// Builds the answer as the frames arrive.
struct Accumulator {
    provider: String,
    model_id: String,
    blocks: Vec<ContentBlock>,
    /// The tool call currently being streamed, with its arguments so far as text.
    open_tool: Option<(usize, String, String, String)>,
    open_reasoning: Option<OpenReasoning>,
    usage: Usage,
    stop_reason: StopReason,
    finished: bool,
}

impl Accumulator {
    fn new(model: &Model) -> Self {
        Accumulator {
            provider: model.provider.clone(),
            model_id: model.id.clone(),
            blocks: Vec::new(),
            open_tool: None,
            open_reasoning: None,
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            finished: false,
        }
    }

    fn handle(&mut self, event_type: &str, event: &Value, sender: &UnboundedSender<StreamEvent>) {
        crate::observe::parsed(
            &self.provider,
            "bedrock-converse-stream",
            &self.model_id,
            &serde_json::json!({ event_type: event }),
        );
        match event_type {
            "contentBlockStart" => {
                if let Some(tool) = event.pointer("/start/toolUse") {
                    self.close_reasoning(sender);
                    let index = event
                        .get("contentBlockIndex")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize;
                    self.open_tool = Some((
                        index,
                        tool.get("toolUseId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        tool.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        String::new(),
                    ));
                }
            }
            "contentBlockDelta" => {
                if let Some(text) = event.pointer("/delta/text").and_then(Value::as_str) {
                    self.close_reasoning(sender);
                    let _ = sender.send(StreamEvent::TextDelta {
                        index: self.blocks.len(),
                        delta: text.to_string(),
                    });
                    self.push_text(text);
                } else if let Some(partial) = event
                    .pointer("/delta/toolUse/input")
                    .and_then(Value::as_str)
                {
                    if let Some((_, _, _, arguments)) = self.open_tool.as_mut() {
                        arguments.push_str(partial);
                    }
                } else if let Some(reasoning) = event.pointer("/delta/reasoningContent") {
                    let content_index = event
                        .get("contentBlockIndex")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    self.push_reasoning(content_index, reasoning, sender);
                }
            }
            "contentBlockStop" => self.close_open_blocks(sender),
            "messageStop" => {
                self.close_open_blocks(sender);
                self.stop_reason = stop_reason(event.get("stopReason").and_then(Value::as_str));
            }
            "metadata" => {
                if let Some(usage) = event.get("usage") {
                    self.usage = read_usage(usage);
                }
                self.close_open_blocks(sender);
                self.finished = true;
                let _ = sender.send(StreamEvent::Done {
                    message: self.build(),
                });
            }
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        match self.blocks.last_mut() {
            Some(ContentBlock::Text { text: existing }) => existing.push_str(text),
            _ => self.blocks.push(ContentBlock::text(text)),
        }
    }

    /// Adds one reasoning delta: readable text, a piece of its signature, or encrypted reasoning.
    fn push_reasoning(
        &mut self,
        content_index: u64,
        reasoning: &Value,
        sender: &UnboundedSender<StreamEvent>,
    ) {
        let continues = self
            .open_reasoning
            .as_ref()
            .is_some_and(|open| open.content_index == content_index);
        if !continues {
            self.close_open_blocks(sender);
            let position = self.blocks.len();
            self.blocks.push(ContentBlock::Thinking {
                thinking: String::new(),
                signature: None,
            });
            self.open_reasoning = Some(OpenReasoning {
                content_index,
                position,
                redacted: Vec::new(),
            });
            let _ = sender.send(StreamEvent::ThinkingStart { index: position });
        }

        let Some(open) = self.open_reasoning.as_mut() else {
            return;
        };
        let Some(ContentBlock::Thinking {
            thinking,
            signature,
        }) = self.blocks.get_mut(open.position)
        else {
            return;
        };

        if let Some(text) = reasoning.get("text").and_then(Value::as_str) {
            if !text.is_empty() {
                thinking.push_str(text);
                let _ = sender.send(StreamEvent::ThinkingDelta {
                    index: open.position,
                    delta: text.to_string(),
                });
            }
        }
        if let Some(piece) = reasoning.get("signature").and_then(Value::as_str) {
            signature.get_or_insert_with(String::new).push_str(piece);
        }
        if let Some(encoded) = reasoning.get("redactedContent").and_then(Value::as_str) {
            if let Ok(bytes) = BASE64.decode(encoded) {
                open.redacted.extend(bytes);
            }
        }
    }

    /// Finish whatever reasoning was streaming. Encrypted reasoning takes the block's place, since
    /// it is what the model needs back and a signature cannot travel with it.
    fn close_reasoning(&mut self, sender: &UnboundedSender<StreamEvent>) {
        let Some(open) = self.open_reasoning.take() else {
            return;
        };
        let Some(block) = self.blocks.get_mut(open.position) else {
            return;
        };
        let thinking = match block {
            ContentBlock::Thinking { thinking, .. } => thinking.clone(),
            _ => String::new(),
        };
        if !open.redacted.is_empty() {
            *block = ContentBlock::RedactedThinking {
                data: BASE64.encode(&open.redacted),
            };
        }
        let _ = sender.send(StreamEvent::ThinkingEnd {
            index: open.position,
            thinking,
        });
    }

    /// Finish every block still streaming.
    fn close_open_blocks(&mut self, sender: &UnboundedSender<StreamEvent>) {
        self.close_reasoning(sender);
        self.close_tool();
    }

    /// Finish whatever tool call was streaming, if one was.
    fn close_tool(&mut self) {
        let Some((_, id, name, arguments)) = self.open_tool.take() else {
            return;
        };
        self.blocks.push(ContentBlock::ToolCall {
            id,
            name,
            arguments: parse_arguments(&arguments),
            signature: None,
        });
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
}

fn stop_reason(reason: Option<&str>) -> StopReason {
    match reason {
        Some("end_turn") | Some("stop_sequence") => StopReason::Stop,
        Some("max_tokens") | Some("model_context_window_exceeded") => StopReason::Length,
        Some("tool_use") => StopReason::ToolUse,
        _ => StopReason::Stop,
    }
}

fn read_usage(usage: &Value) -> Usage {
    let count = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0) as u32;
    Usage {
        input: count("inputTokens"),
        output: count("outputTokens"),
        cache_read: count("cacheReadInputTokens"),
        cache_write: count("cacheWriteInputTokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::ToolDefinition;

    fn model() -> Model {
        Model {
            id: "anthropic.claude-opus-4-v1:0".into(),
            provider: PROVIDER.into(),
            base_url: String::new(),
            max_tokens: 4096,
            thinking: micro_types::ThinkingLevel::Off,
            reasoning: false,
            compat: Default::default(),
            headers: Default::default(),
        }
    }

    /// A model id carries characters that would otherwise read as more path.
    #[test]
    fn the_model_travels_in_the_path_intact() {
        let address = endpoint("", "eu-west-1", "anthropic.claude-opus-4-v1:0");
        assert_eq!(
            address,
            "https://bedrock-runtime.eu-west-1.amazonaws.com/model/anthropic.claude-opus-4-v1:0/converse-stream"
        );

        assert!(endpoint("", "us-east-1", "vendor/model").contains("vendor%2Fmodel"));
    }

    /// The region comes from the address when it names one.
    #[test]
    fn the_address_can_name_the_region() {
        assert_eq!(
            region("https://bedrock-runtime.ap-southeast-2.amazonaws.com"),
            "ap-southeast-2"
        );

        assert!(region_in("https://my-proxy.example.com").is_none());
    }

    /// The system prompt is its own field here, not a message with a role.
    #[test]
    fn the_system_prompt_is_not_a_message() {
        let context = Context {
            system_prompt: Some("be brief".into()),
            messages: vec![Message::user("hello")],
            ..Default::default()
        };
        let payload = build_payload(&model(), &context).unwrap();

        assert_eq!(payload["system"][0]["text"], "be brief");
        assert_eq!(payload["messages"].as_array().unwrap().len(), 1);
        assert_eq!(payload["messages"][0]["role"], "user");
    }

    /// A tool result is a user turn carrying the id of the call it answers.
    #[test]
    fn a_tool_result_names_the_call_it_answers() {
        let context = Context {
            messages: vec![Message::tool_result(
                "call-1",
                "read",
                "file contents",
                false,
            )],
            ..Default::default()
        };
        let payload = build_payload(&model(), &context).unwrap();
        let result = &payload["messages"][0]["content"][0]["toolResult"];

        assert_eq!(payload["messages"][0]["role"], "user");
        assert_eq!(result["toolUseId"], "call-1");
        assert_eq!(result["content"][0]["text"], "file contents");
        assert_eq!(result["status"], "success");
    }

    #[test]
    fn a_failed_tool_is_marked_as_one() {
        let context = Context {
            messages: vec![Message::tool_result("call-1", "read", "no such file", true)],
            ..Default::default()
        };
        let payload = build_payload(&model(), &context).unwrap();
        assert_eq!(
            payload["messages"][0]["content"][0]["toolResult"]["status"],
            "error"
        );
    }

    #[test]
    fn tools_are_declared_the_way_bedrock_reads_them() {
        let context = Context {
            messages: vec![Message::user("go")],
            tools: vec![ToolDefinition {
                name: "read".into(),
                description: "read a file".into(),
                parameters: json!({ "type": "object" }),
                constrained_sampling: None,
            }],
            ..Default::default()
        };
        let payload = build_payload(&model(), &context).unwrap();
        let spec = &payload["toolConfig"]["tools"][0]["toolSpec"];

        assert_eq!(spec["name"], "read");
        assert_eq!(spec["description"], "read a file");
        assert_eq!(spec["inputSchema"]["json"]["type"], "object");
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

    /// No model in the bundled catalog claims this yet.
    #[test]
    fn a_tool_preferring_strict_sampling_gets_it_when_the_service_claims_support() {
        let mut model = model();
        model.compat.bedrock_supports_strict_tools = true;
        let context = Context {
            tools: vec![tool_asking_for_json_schema_sampling(
                micro_types::JsonSchemaStrictness::Prefer,
            )],
            ..Default::default()
        };
        let payload = build_payload(&model, &context).unwrap();
        let spec = &payload["toolConfig"]["tools"][0]["toolSpec"];

        assert_eq!(spec["strict"], true);
        assert_eq!(spec["inputSchema"]["json"]["additionalProperties"], false);
    }

    /// The default state: unaffected by a tool merely preferring constrained sampling, no `strict`
    /// key at all, schema untouched.
    #[test]
    fn a_service_that_has_not_claimed_support_is_unaffected_by_a_tool_preferring_strict_sampling() {
        let original_parameters = json!({
            "type": "object",
            "properties": { "pattern": { "type": "string" } },
            "required": ["pattern"],
        });
        let model = model();
        assert!(
            !model.compat.bedrock_supports_strict_tools,
            "the default this test relies on"
        );
        let context = Context {
            tools: vec![tool_asking_for_json_schema_sampling(
                micro_types::JsonSchemaStrictness::Prefer,
            )],
            ..Default::default()
        };
        let payload = build_payload(&model, &context).unwrap();
        let spec = &payload["toolConfig"]["tools"][0]["toolSpec"];

        assert!(spec.get("strict").is_none());
        assert_eq!(spec["inputSchema"]["json"], original_parameters);
    }

    #[test]
    fn requiring_strict_sampling_on_a_service_that_does_not_support_it_fails_the_request() {
        let context = Context {
            tools: vec![tool_asking_for_json_schema_sampling(
                micro_types::JsonSchemaStrictness::Require,
            )],
            ..Default::default()
        };
        let error = build_payload(&model(), &context).unwrap_err();
        assert!(error.contains("\"grep\""), "got {error:?}");
    }

    /// Text arrives in pieces and is joined into one block.
    #[test]
    fn streamed_text_is_gathered_into_one_block() {
        let (sender, mut received) = mpsc::unbounded_channel();
        let mut state = Accumulator::new(&model());

        for piece in ["Hel", "lo, ", "world"] {
            state.handle(
                "contentBlockDelta",
                &json!({ "delta": { "text": piece } }),
                &sender,
            );
        }
        state.handle("messageStop", &json!({ "stopReason": "end_turn" }), &sender);

        let built = state.build();
        assert_eq!(built.content.len(), 1);
        assert_eq!(built.content[0].as_text(), "Hello, world");
        assert_eq!(built.stop_reason, StopReason::Stop);

        let deltas: Vec<String> = std::iter::from_fn(|| received.try_recv().ok())
            .filter_map(|event| match event {
                StreamEvent::TextDelta { delta, .. } => Some(delta),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["Hel", "lo, ", "world"]);
    }

    /// A tool call's arguments stream as text and are parsed once they are whole.
    #[test]
    fn a_streamed_tool_call_is_put_back_together() {
        let (sender, _received) = mpsc::unbounded_channel();
        let mut state = Accumulator::new(&model());

        state.handle(
            "contentBlockStart",
            &json!({
                "contentBlockIndex": 0,
                "start": { "toolUse": { "toolUseId": "call-9", "name": "read" } }
            }),
            &sender,
        );
        for piece in [r#"{"pa"#, r#"th":"a."#, r#"txt"}"#] {
            state.handle(
                "contentBlockDelta",
                &json!({ "delta": { "toolUse": { "input": piece } } }),
                &sender,
            );
        }
        state.handle("contentBlockStop", &json!({}), &sender);

        let built = state.build();
        match &built.content[0] {
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
                ..
            } => {
                assert_eq!(id, "call-9");
                assert_eq!(name, "read");
                assert_eq!(arguments["path"], "a.txt");
            }
            other => panic!("expected a tool call, got {other:?}"),
        }
    }

    #[test]
    fn a_stop_reason_is_read_the_way_bedrock_writes_it() {
        assert_eq!(stop_reason(Some("end_turn")), StopReason::Stop);
        assert_eq!(stop_reason(Some("max_tokens")), StopReason::Length);
        assert_eq!(stop_reason(Some("tool_use")), StopReason::ToolUse);
    }

    #[test]
    fn usage_is_read_from_the_metadata_frame() {
        let usage = read_usage(&json!({
            "inputTokens": 120,
            "outputTokens": 45,
            "cacheReadInputTokens": 12,
            "cacheWriteInputTokens": 3,
        }));
        assert_eq!(usage.input, 120);
        assert_eq!(usage.output, 45);
        assert_eq!(usage.cache_read, 12);
        assert_eq!(usage.cache_write, 3);
    }

    /// A model as the catalog would hand it over, reasoning at `level`.
    fn served(id: &str, level: ThinkingLevel) -> Model {
        let catalog = micro_models::Catalog::bundled();
        let model = catalog
            .by_provider(PROVIDER)
            .find(|model| model.id == id)
            .unwrap_or_else(|| panic!("Bedrock serves {id}"))
            .to_runtime(level);
        model
    }

    fn fields_for(model: &Model) -> Value {
        let context = Context {
            messages: vec![Message::user("hi")],
            ..Default::default()
        };
        build_payload(model, &context).unwrap()["additionalModelRequestFields"].clone()
    }

    #[test]
    fn a_claude_model_with_thinking_off_is_not_asked_to_think() {
        let model = served(
            "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
            ThinkingLevel::Off,
        );
        assert!(fields_for(&model).is_null());
    }

    #[test]
    fn a_budget_claude_model_is_given_a_budget_and_interleaved_thinking() {
        let model = served(
            "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
            ThinkingLevel::Medium,
        );
        let fields = fields_for(&model);

        assert_eq!(fields["thinking"]["type"], "enabled");
        assert_eq!(fields["thinking"]["budget_tokens"], 8_192);
        assert_eq!(fields["thinking"]["display"], "summarized");
        assert_eq!(
            fields["anthropic_beta"],
            json!(["interleaved-thinking-2025-05-14"])
        );
        assert!(fields.get("output_config").is_none());
    }

    #[test]
    fn each_budget_level_asks_for_its_own_budget() {
        for (level, budget) in [
            (ThinkingLevel::Minimal, 1_024),
            (ThinkingLevel::Low, 2_048),
            (ThinkingLevel::Medium, 8_192),
            (ThinkingLevel::High, 16_384),
        ] {
            let model = served("anthropic.claude-opus-4-5-20251101-v1:0", level);
            assert_eq!(fields_for(&model)["thinking"]["budget_tokens"], budget);
        }
    }

    /// A level the model does not offer is asked for as the nearest one it does.
    #[test]
    fn a_budget_model_asked_beyond_high_thinks_at_high() {
        let mut model = served(
            "anthropic.claude-opus-4-5-20251101-v1:0",
            ThinkingLevel::High,
        );
        model.thinking = ThinkingLevel::Max;
        assert_eq!(fields_for(&model)["thinking"]["budget_tokens"], 16_384);
    }

    /// The budget always leaves room for an answer under the response ceiling.
    #[test]
    fn the_budget_leaves_room_for_the_answer() {
        let mut model = served(
            "anthropic.claude-opus-4-5-20251101-v1:0",
            ThinkingLevel::High,
        );
        model.max_tokens = 8_000;
        let payload = build_payload(&model, &Context::default()).unwrap();

        assert_eq!(payload["inferenceConfig"]["maxTokens"], 8_000);
        assert_eq!(
            payload["additionalModelRequestFields"]["thinking"]["budget_tokens"],
            8_000 - 1_024
        );
    }

    #[test]
    fn an_adaptive_claude_model_is_asked_for_an_effort() {
        let model = served("global.anthropic.claude-opus-4-6-v1", ThinkingLevel::Low);
        let fields = fields_for(&model);

        assert_eq!(fields["thinking"]["type"], "adaptive");
        assert_eq!(fields["thinking"]["display"], "summarized");
        assert_eq!(fields["output_config"]["effort"], "low");
        assert!(fields["thinking"].get("budget_tokens").is_none());
        assert!(
            fields.get("anthropic_beta").is_none(),
            "adaptive thinking interleaves on its own"
        );
    }

    #[test]
    fn an_adaptive_model_is_asked_for_the_efforts_its_thinking_map_names() {
        for (id, level, effort) in [
            (
                "us.anthropic.claude-opus-4-7",
                ThinkingLevel::XHigh,
                "xhigh",
            ),
            ("us.anthropic.claude-opus-4-7", ThinkingLevel::Max, "max"),
            ("us.anthropic.claude-sonnet-4-6", ThinkingLevel::Max, "max"),
            (
                "us.anthropic.claude-sonnet-4-6",
                ThinkingLevel::Minimal,
                "low",
            ),
            (
                "us.anthropic.claude-sonnet-4-6",
                ThinkingLevel::High,
                "high",
            ),
        ] {
            let model = served(id, level);
            assert_eq!(
                fields_for(&model)["output_config"]["effort"],
                effort,
                "{id} at {level:?}"
            );
        }
    }

    /// A model whose thinking cannot be turned off thinks at its lowest level instead.
    #[test]
    fn a_model_that_always_thinks_is_never_asked_for_nothing() {
        let model = served("us.anthropic.claude-fable-5", ThinkingLevel::Off);
        assert_eq!(fields_for(&model)["output_config"]["effort"], "low");
    }

    #[test]
    fn govcloud_is_not_sent_the_display_field() {
        let mut model = served(
            "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
            ThinkingLevel::High,
        );
        model.base_url = "https://bedrock-runtime.us-gov-west-1.amazonaws.com".into();
        let fields = fields_for(&model);

        assert_eq!(fields["thinking"]["type"], "enabled");
        assert!(fields["thinking"].get("display").is_none());
    }

    /// Only Claude is asked to think; other models reason as their defaults say.
    #[test]
    fn a_model_that_is_not_claude_is_sent_no_thinking_fields() {
        let model = served("openai.gpt-5.5", ThinkingLevel::High);
        assert!(fields_for(&model).is_null());
    }

    fn stream(frames: &[Value]) -> (AssistantMessage, Vec<StreamEvent>) {
        let (sender, mut received) = mpsc::unbounded_channel();
        let mut state = Accumulator::new(&model());
        for frame in frames {
            let (event_type, event) = frame.as_object().unwrap().iter().next().unwrap();
            state.handle(event_type, event, &sender);
        }
        let events = std::iter::from_fn(|| received.try_recv().ok()).collect();
        (state.build(), events)
    }

    #[test]
    fn signed_reasoning_is_gathered_into_one_thinking_block() {
        let (message, events) = stream(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "Let me " } } } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "think." } } } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "reasoningContent": { "signature": "sig-" } } } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "reasoningContent": { "signature": "abc" } } } }),
            json!({ "contentBlockStop": { "contentBlockIndex": 0 } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 1, "delta": { "text": "Answer" } } }),
            json!({ "contentBlockStop": { "contentBlockIndex": 1 } }),
            json!({ "messageStop": { "stopReason": "end_turn" } }),
        ]);

        assert_eq!(
            message.content,
            vec![
                ContentBlock::Thinking {
                    thinking: "Let me think.".into(),
                    signature: Some("sig-abc".into()),
                },
                ContentBlock::text("Answer"),
            ]
        );
        assert_eq!(events[0], StreamEvent::ThinkingStart { index: 0 });
        assert!(events.contains(&StreamEvent::ThinkingEnd {
            index: 0,
            thinking: "Let me think.".into(),
        }));
    }

    /// Encrypted reasoning arrives as base64 pieces and is kept as one payload.
    #[test]
    fn redacted_reasoning_is_kept_whole() {
        let (message, _) = stream(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "reasoningContent": { "redactedContent": BASE64.encode(b"secret ") } } } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "reasoningContent": { "redactedContent": BASE64.encode(b"thoughts") } } } }),
            json!({ "contentBlockStop": { "contentBlockIndex": 0 } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 1, "delta": { "text": "Done" } } }),
            json!({ "messageStop": { "stopReason": "end_turn" } }),
        ]);

        assert_eq!(
            message.content,
            vec![
                ContentBlock::RedactedThinking {
                    data: BASE64.encode(b"secret thoughts"),
                },
                ContentBlock::text("Done"),
            ]
        );
    }

    /// What a Claude model thought goes back to it as it came, signature and all.
    #[test]
    fn claude_reasoning_is_replayed_with_its_signature() {
        let (answer, _) = stream(&[
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "reasoningContent": { "text": "Plan." } } } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 0, "delta": { "reasoningContent": { "signature": "sig" } } } }),
            json!({ "contentBlockStop": { "contentBlockIndex": 0 } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 1, "delta": { "reasoningContent": { "redactedContent": BASE64.encode(b"hidden") } } } }),
            json!({ "contentBlockStop": { "contentBlockIndex": 1 } }),
            json!({ "contentBlockDelta": { "contentBlockIndex": 2, "delta": { "text": "Hi" } } }),
            json!({ "messageStop": { "stopReason": "end_turn" } }),
        ]);
        let context = Context {
            messages: vec![
                Message::user("hi"),
                Message::Assistant(answer),
                Message::user("again"),
            ],
            ..Default::default()
        };
        let model = served(
            "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
            ThinkingLevel::High,
        );
        let payload = build_payload(&model, &context).unwrap();

        assert_eq!(
            payload["messages"][1]["content"],
            json!([
                { "reasoningContent": { "reasoningText": { "text": "Plan.", "signature": "sig" } } },
                { "reasoningContent": { "redactedContent": BASE64.encode(b"hidden") } },
                { "text": "Hi" },
            ])
        );
    }

    /// Claude rejects unsigned reasoning, so it goes back as plain text.
    #[test]
    fn unsigned_claude_reasoning_is_replayed_as_text() {
        let content = assistant_content(
            &[ContentBlock::Thinking {
                thinking: "Plan.".into(),
                signature: None,
            }],
            true,
        );
        assert_eq!(content, vec![json!({ "text": "Plan." })]);
    }

    /// Other models reject a signature on replayed reasoning.
    #[test]
    fn reasoning_goes_back_to_other_models_without_a_signature() {
        let context = Context {
            messages: vec![Message::Assistant(AssistantMessage {
                content: vec![
                    ContentBlock::Thinking {
                        thinking: "Plan.".into(),
                        signature: Some("sig".into()),
                    },
                    ContentBlock::Thinking {
                        thinking: "  ".into(),
                        signature: None,
                    },
                    ContentBlock::RedactedThinking {
                        data: "not base64!".into(),
                    },
                ],
                provider: PROVIDER.into(),
                model: "openai.gpt-5.5".into(),
                usage: Usage::default(),
                stop_reason: StopReason::Stop,
                error: None,
                timestamp: 0,
            })],
            ..Default::default()
        };
        let payload =
            build_payload(&served("openai.gpt-5.5", ThinkingLevel::High), &context).unwrap();

        assert_eq!(
            payload["messages"][0]["content"],
            json!([{ "reasoningContent": { "reasoningText": { "text": "Plan." } } }])
        );
    }
}
