//! Amazon Bedrock, over the Converse Stream shape.
//!
//! Two things make this unlike every other client here. The request is signed rather than
//! carrying a key — see [`crate::sigv4`] — unless the account uses a bearer token, which
//! Bedrock also accepts. And the answer arrives as AWS's binary event stream rather than
//! as server-sent events, so the frames are decoded by [`crate::eventstream`] before any
//! of this reads them.
//!
//! The conversation shape is Bedrock's own: `messages` with `content` arrays, the system
//! prompt in its own `system` field rather than as a message, and tools under
//! `toolConfig`.

use crate::eventstream::Decoder;
use crate::json::parse_arguments;
use crate::sigv4;
use micro_types::now_ms;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::StreamEvent;
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
/// Where Bedrock is served when the account says nothing else.
const DEFAULT_REGION: &str = "us-east-1";

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
///
/// Bedrock takes either a bearer token, which is sent as it is, or AWS credentials, which
/// sign the request. A stored credential is treated as a bearer token, since that is the
/// only kind micro can be handed directly; everything else comes from the environment the
/// way every other AWS tool reads it.
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
    // A base URL naming a region is the most specific thing there is.
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

/// Where a model's stream is asked for.
pub(crate) fn endpoint(base_url: &str, region: &str, model_id: &str) -> String {
    let root = match base_url.trim().trim_end_matches('/') {
        "" => format!("https://bedrock-runtime.{region}.amazonaws.com"),
        given => given.to_string(),
    };
    // The id travels in the path, so a slash inside it would read as another segment.
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

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Bedrock returned {}: {}",
            status.as_u16(),
            body.trim()
        ));
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
        let _ = sender.send(StreamEvent::Done {
            message: state.build(),
        });
    }
    Ok(())
}

/// Bedrock's own request shape.
pub(crate) fn build_payload(model: &Model, context: &Context) -> Result<Value, String> {
    let mut payload = json!({
        "messages": build_messages(&context.messages),
        "inferenceConfig": { "maxTokens": model.max_tokens },
    });

    // The system prompt is its own field here rather than a message with a role.
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
            // Unlike the completions and Responses shapes, Bedrock's toolSpec has no
            // "unresolved" spelling of `strict` — the field is only ever added, never
            // sent false or null, so a tool that did not resolve to strict is left
            // exactly as it always was: no such key at all.
            if strict == Some(true) {
                spec["strict"] = json!(true);
            }
            tools.push(json!({ "toolSpec": spec }));
        }
        payload["toolConfig"] = json!({ "tools": tools });
    }

    Ok(payload)
}

/// The conversation as Bedrock reads it.
///
/// A tool result is a user turn carrying a `toolResult` block, which is how Bedrock pairs
/// an answer with the call that asked for it.
fn build_messages(messages: &[Message]) -> Vec<Value> {
    let mut wire: Vec<Value> = Vec::new();

    for message in messages {
        match message {
            Message::User { content, .. } => {
                wire.push(json!({ "role": "user", "content": user_content(content) }));
            }
            Message::Assistant(assistant) => {
                let content = assistant_content(&assistant.content);
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

fn assistant_content(content: &[ContentBlock]) -> Vec<Value> {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } if !text.is_empty() => Some(json!({ "text": text })),
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

/// Builds the answer as the frames arrive.
struct Accumulator {
    provider: String,
    model_id: String,
    blocks: Vec<ContentBlock>,
    /// The tool call currently being streamed, with its arguments so far as text.
    open_tool: Option<(usize, String, String, String)>,
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
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            finished: false,
        }
    }

    fn handle(&mut self, event_type: &str, event: &Value, sender: &UnboundedSender<StreamEvent>) {
        match event_type {
            "contentBlockStart" => {
                if let Some(tool) = event.pointer("/start/toolUse") {
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
                } else if let Some(thinking) = event
                    .pointer("/delta/reasoningContent/text")
                    .and_then(Value::as_str)
                {
                    let _ = sender.send(StreamEvent::ThinkingDelta {
                        index: self.blocks.len(),
                        delta: thinking.to_string(),
                    });
                    self.push_thinking(thinking);
                }
            }
            "contentBlockStop" => self.close_tool(),
            "messageStop" => {
                self.close_tool();
                self.stop_reason = stop_reason(event.get("stopReason").and_then(Value::as_str));
            }
            "metadata" => {
                if let Some(usage) = event.get("usage") {
                    self.usage = read_usage(usage);
                }
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

    fn push_thinking(&mut self, text: &str) {
        match self.blocks.last_mut() {
            Some(ContentBlock::Thinking { thinking, .. }) => thinking.push_str(text),
            _ => self.blocks.push(ContentBlock::Thinking {
                thinking: text.to_string(),
                signature: None,
            }),
        }
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
            usage: self.usage.clone(),
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
        // An id with a slash is one segment, not two.
        assert!(endpoint("", "us-east-1", "vendor/model").contains("vendor%2Fmodel"));
    }

    /// The region comes from the address when it names one.
    #[test]
    fn the_address_can_name_the_region() {
        assert_eq!(
            region("https://bedrock-runtime.ap-southeast-2.amazonaws.com"),
            "ap-southeast-2"
        );
        // A custom endpoint says nothing about a region, so the environment decides.
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

    /// A failed tool says so, so the model reads it as a failure rather than as an answer.
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

    /// No model in the bundled catalog claims this yet — matching pi, which has no
    /// `generate-models.ts` rule for Bedrock the way it does for OpenAI-Responses and
    /// Anthropic-Messages — so the default here is what every real request sees today.
    /// A test states the flag explicitly to exercise the consumer regardless.
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

    /// The default state: unaffected by a tool merely preferring constrained sampling, no
    /// `strict` key at all, schema untouched — the same request Bedrock has always been
    /// sent.
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

    /// `"require"` on a service that has not claimed support fails the request rather than
    /// silently sending it under ordinary sampling.
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
}
