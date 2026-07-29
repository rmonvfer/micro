//! Google's native Gemini API — `streamGenerateContent`, not the OpenAI-compatible shim.
//!
//! Gemini streams a sequence of whole `GenerateContentResponse` chunks. Text arrives a
//! fragment at a time across chunks, but a function call always arrives complete inside
//! one `functionCall` part, so every one of those parts closes a tool call outright.

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
use micro_types::Usage;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

pub(crate) const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// JSON Schema keywords Gemini's function declarations reject. Anything left in the
/// schema after this filter is a keyword it understands.
const UNSUPPORTED_SCHEMA_FIELDS: &[&str] = &[
    "$schema",
    "$ref",
    "$defs",
    "examples",
    "default",
    "pattern",
    "patternProperties",
    "minItems",
    "maxItems",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "additionalProperties",
    "format",
    "title",
];

#[derive(Clone, Default)]
pub struct Gemini {
    client: reqwest::Client,
    /// Where the next synthesized call id continues from. Shared across every request this
    /// provider serves, which is what keeps ids unique for a whole conversation rather
    /// than only within one response.
    next_call: Arc<AtomicUsize>,
}

impl Gemini {
    pub fn new() -> Self {
        Gemini {
            client: crate::http_client(),
            next_call: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Provider for Gemini {
    fn name(&self) -> &str {
        "gemini"
    }

    fn stream(
        &self,
        model: Model,
        context: Context,
        api_key: String,
    ) -> UnboundedReceiver<StreamEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let client = self.client.clone();

        // Two things have to hold for an id to stay unique across a whole conversation, and
        // they cover different failures. Starting from the calls the history already holds
        // makes a resumed session continue past everything in its log rather than restart
        // at zero. Never moving backwards covers compaction, where the conversation in
        // memory gets shorter than the transcript on disk and a history count alone would
        // hand out an id that transcript already used.
        let counter = Arc::clone(&self.next_call);
        counter.fetch_max(tool_calls_so_far(&context.messages), Ordering::Relaxed);

        tokio::spawn(async move {
            if let Err(message) = run(client, model, context, api_key, counter, &sender).await {
                let _ = sender.send(StreamEvent::Error { message });
            }
        });

        receiver
    }
}

/// The reasoning signature a part carries, if any.
///
/// Gemini hangs this off the part rather than off the content inside it, so it appears on
/// a text part and on a `functionCall` part alike. Only the text case can be kept today:
/// [`ContentBlock::Thinking`] has somewhere to put a signature and
/// [`ContentBlock::ToolCall`] does not.
fn thought_signature(part: &Value) -> Option<String> {
    part.get("thoughtSignature")
        .and_then(Value::as_str)
        .filter(|signature| !signature.is_empty())
        .map(str::to_string)
}

/// How many tool calls a conversation already holds.
fn tool_calls_so_far(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::Assistant(assistant) => Some(assistant.content.as_slice()),
            _ => None,
        })
        .flatten()
        .filter(|block| matches!(block, ContentBlock::ToolCall { .. }))
        .count()
}

async fn run(
    client: reqwest::Client,
    model: Model,
    context: Context,
    api_key: String,
    next_call: Arc<AtomicUsize>,
    sender: &UnboundedSender<StreamEvent>,
) -> Result<(), String> {
    let payload = build_payload(&model, &context);
    let request = client
        .post(endpoint(&model.base_url, &model.id))
        .header("x-goog-api-key", api_key)
        .header("content-type", "application/json");
    let response = crate::with_carried_headers(request, &context)
        .json(&payload)
        .send()
        .await
        .map_err(|error| format!("gemini request failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "gemini returned {}: {}",
            status.as_u16(),
            body.trim()
        ));
    }

    let mut state = Accumulator::new(&model, next_call);
    read_sse(response, |event| state.handle(event, sender))
        .await
        .map_err(|error| format!("gemini stream failed: {error}"))?;

    if !state.finished {
        state.finish(sender);
    }
    Ok(())
}

/// `{base}/models/{id}:streamGenerateContent?alt=sse`. `alt=sse` is what makes Gemini
/// deliver chunks as they are produced rather than one JSON array at the end.
fn endpoint(base_url: &str, model_id: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let id = model_id.trim_start_matches("models/");
    format!("{base}/models/{id}:streamGenerateContent?alt=sse")
}

/// The block currently being assembled. Gemini interleaves reasoning, text, and calls, so
/// a part of a different kind closes whatever is open.
enum OpenBlock {
    Text {
        index: usize,
        text: String,
    },
    Thinking {
        index: usize,
        thinking: String,
        signature: Option<String>,
    },
}

struct Accumulator {
    provider: String,
    model_id: String,
    blocks: Vec<ContentBlock>,
    usage: Usage,
    stop_reason: StopReason,
    open: Option<OpenBlock>,
    next_index: usize,
    next_call: Arc<AtomicUsize>,
    started: bool,
    finished: bool,
}

impl Accumulator {
    fn new(model: &Model, next_call: Arc<AtomicUsize>) -> Self {
        Accumulator {
            provider: model.provider.clone(),
            model_id: model.id.clone(),
            blocks: Vec::new(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            open: None,
            next_index: 0,
            next_call,
            started: false,
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
        if data.is_empty() {
            return;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return;
        };

        // A prompt Gemini refuses outright comes back with feedback and no candidates.
        if let Some(reason) = chunk
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
        {
            self.finished = true;
            let _ = sender.send(StreamEvent::Error {
                message: format!("gemini blocked the prompt: {reason}"),
            });
            return;
        }

        if !self.started {
            self.started = true;
            let _ = sender.send(StreamEvent::Start);
        }

        if let Some(metadata) = chunk.get("usageMetadata") {
            self.read_usage(metadata);
        }

        let Some(candidate) = chunk
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
        else {
            return;
        };

        if let Some(parts) = candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
        {
            for part in parts {
                self.handle_part(part, sender);
            }
        }

        if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
            self.stop_reason = map_finish_reason(reason);
        }
    }

    fn handle_part(&mut self, part: &Value, sender: &UnboundedSender<StreamEvent>) {
        if let Some(call) = part.get("functionCall").filter(|call| call.is_object()) {
            self.close_open(sender);
            // The signature rides on the part, not inside the call, and belongs to the
            // reasoning that produced this call.
            self.emit_tool_call(call, thought_signature(part), sender);
            return;
        }

        let Some(text) = part.get("text").and_then(Value::as_str) else {
            return;
        };
        if text.is_empty() {
            return;
        }

        // Reasoning parts are marked; the signature on them has to be replayed verbatim
        // for Gemini to accept the thought on a later turn.
        let is_thought = part
            .get("thought")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let signature = thought_signature(part);

        match (is_thought, self.open.as_mut()) {
            (
                true,
                Some(OpenBlock::Thinking {
                    index,
                    thinking,
                    signature: held,
                }),
            ) => {
                let index = *index;
                thinking.push_str(text);
                if signature.is_some() {
                    *held = signature;
                }
                let _ = sender.send(StreamEvent::ThinkingDelta {
                    index,
                    delta: text.to_string(),
                });
            }
            (
                false,
                Some(OpenBlock::Text {
                    index,
                    text: buffer,
                }),
            ) => {
                let index = *index;
                buffer.push_str(text);
                let _ = sender.send(StreamEvent::TextDelta {
                    index,
                    delta: text.to_string(),
                });
            }
            _ => {
                self.close_open(sender);
                let index = self.take_index();
                if is_thought {
                    let _ = sender.send(StreamEvent::ThinkingStart { index });
                    self.open = Some(OpenBlock::Thinking {
                        index,
                        thinking: text.to_string(),
                        signature,
                    });
                    let _ = sender.send(StreamEvent::ThinkingDelta {
                        index,
                        delta: text.to_string(),
                    });
                } else {
                    let _ = sender.send(StreamEvent::TextStart { index });
                    self.open = Some(OpenBlock::Text {
                        index,
                        text: text.to_string(),
                    });
                    let _ = sender.send(StreamEvent::TextDelta {
                        index,
                        delta: text.to_string(),
                    });
                }
            }
        }
    }

    /// Gemini sends a function call whole, so the call opens and closes in one step.
    fn emit_tool_call(
        &mut self,
        call: &Value,
        signature: Option<String>,
        sender: &UnboundedSender<StreamEvent>,
    ) {
        let Some(name) = call.get("name").and_then(Value::as_str) else {
            return;
        };

        // Gemini names its calls but rarely identifies them. A synthesized id has to stay
        // unique for the whole conversation, not just this response: the history is replayed
        // verbatim, and a provider that pairs by id rather than by name reads two calls
        // sharing one id as the same call.
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "gemini-call-{}",
                    self.next_call.fetch_add(1, Ordering::Relaxed)
                )
            });

        let arguments = match call.get("args") {
            Some(args) if !args.is_null() => args.clone(),
            _ => Value::Object(Map::new()),
        };

        let index = self.take_index();
        let _ = sender.send(StreamEvent::ToolCallStart {
            index,
            id: id.clone(),
            name: name.to_string(),
        });
        let _ = sender.send(StreamEvent::ToolCallDelta {
            index,
            delta: arguments.to_string(),
        });
        self.blocks.push(ContentBlock::ToolCall {
            id: id.clone(),
            name: name.to_string(),
            arguments: arguments.clone(),
            signature,
        });
        let _ = sender.send(StreamEvent::ToolCallEnd {
            index,
            id,
            name: name.to_string(),
            arguments,
        });
    }

    fn close_open(&mut self, sender: &UnboundedSender<StreamEvent>) {
        match self.open.take() {
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
                        signature,
                    });
                }
                let _ = sender.send(StreamEvent::ThinkingEnd { index, thinking });
            }
            None => {}
        }
    }

    fn take_index(&mut self) -> usize {
        let index = self.next_index;
        self.next_index += 1;
        index
    }

    /// `promptTokenCount` already includes the cached prefix, so the billable input is the
    /// prompt minus that; reasoning tokens are billed as output.
    fn read_usage(&mut self, metadata: &Value) {
        let count = |key: &str| metadata.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;

        let cached = count("cachedContentTokenCount");
        self.usage.cache_read = cached;
        self.usage.input = count("promptTokenCount").saturating_sub(cached);
        self.usage.output = count("candidatesTokenCount") + count("thoughtsTokenCount");
    }

    fn finish(&mut self, sender: &UnboundedSender<StreamEvent>) {
        self.finished = true;
        self.close_open(sender);

        // Gemini reports STOP alongside the function calls it wants run, so a turn that
        // produced calls is a tool-use turn whatever the finish reason said.
        let has_tool_calls = self
            .blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }));
        if has_tool_calls && self.stop_reason == StopReason::Stop {
            self.stop_reason = StopReason::ToolUse;
        }

        let _ = sender.send(StreamEvent::Done {
            message: self.build(),
        });
    }
}

/// Safety, recitation, and malformed-call terminations become errors rather than a silent
/// stop, so the agent loop can tell that the generation did not complete.
fn map_finish_reason(reason: &str) -> StopReason {
    match reason.to_uppercase().as_str() {
        "STOP" => StopReason::Stop,
        "MAX_TOKENS" => StopReason::Length,
        "SAFETY"
        | "RECITATION"
        | "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "SPII"
        | "IMAGE_SAFETY"
        | "MALFORMED_FUNCTION_CALL"
        | "OTHER"
        | "FINISH_REASON_UNSPECIFIED" => StopReason::Error,
        _ => StopReason::Stop,
    }
}

pub(crate) fn build_payload(model: &Model, context: &Context) -> Value {
    let mut payload = Map::new();

    if let Some(system) = context
        .system_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
    {
        payload.insert(
            "system_instruction".into(),
            json!({ "parts": [{ "text": system }] }),
        );
    }

    payload.insert(
        "contents".into(),
        Value::Array(build_contents(&context.messages)),
    );

    if !context.tools.is_empty() {
        let declarations: Vec<Value> = context
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": sanitize_schema(&tool.parameters),
                })
            })
            .collect();
        payload.insert(
            "tools".into(),
            json!([{ "functionDeclarations": declarations }]),
        );
    }

    let mut generation = Map::new();
    generation.insert("maxOutputTokens".into(), json!(model.max_tokens));
    if let Some(budget) = model.thinking.budget_tokens() {
        generation.insert(
            "thinkingConfig".into(),
            json!({ "thinkingBudget": budget, "includeThoughts": true }),
        );
    }
    payload.insert("generationConfig".into(), Value::Object(generation));

    Value::Object(payload)
}

/// Build the `contents` array. Gemini has only two roles, so a tool result is a user
/// turn; consecutive turns of the same role are merged into one.
fn build_contents(messages: &[Message]) -> Vec<Value> {
    let mut contents: Vec<(&str, Vec<Value>)> = Vec::new();

    for message in messages {
        let (role, parts) = match message {
            Message::User { content, .. } => ("user", content_parts(content)),
            Message::Assistant(assistant) => ("model", assistant_parts(&assistant.content)),
            Message::ToolResult {
                tool_name,
                content,
                is_error,
                ..
            } => ("user", tool_result_parts(tool_name, content, *is_error)),
        };

        if parts.is_empty() {
            continue;
        }

        match contents.last_mut() {
            Some((last_role, last_parts)) if *last_role == role => last_parts.extend(parts),
            _ => contents.push((role, parts)),
        }
    }

    contents
        .into_iter()
        .map(|(role, parts)| json!({ "role": role, "parts": parts }))
        .collect()
}

fn content_parts(blocks: &[ContentBlock]) -> Vec<Value> {
    let mut parts = Vec::new();
    let text = join_text(blocks);
    if !text.is_empty() {
        parts.push(json!({ "text": text }));
    }
    parts.extend(image_parts(blocks));
    parts
}

/// Assistant turns replay their calls as `functionCall` parts so Gemini can pair each
/// `functionResponse` with the call that produced it. Reasoning is replayed only when it
/// carries the signature Gemini issued, which is what it validates the thought against.
fn assistant_parts(blocks: &[ContentBlock]) -> Vec<Value> {
    let mut parts = Vec::new();

    for block in blocks {
        if let ContentBlock::Thinking {
            thinking,
            signature: Some(signature),
        } = block
        {
            parts.push(json!({
                "text": thinking,
                "thought": true,
                "thoughtSignature": signature,
            }));
        }
    }

    let text = join_text(blocks);
    if !text.is_empty() {
        parts.push(json!({ "text": text }));
    }

    for block in blocks {
        if let ContentBlock::ToolCall {
            name,
            arguments,
            signature,
            ..
        } = block
        {
            let mut part = Map::new();
            part.insert(
                "functionCall".into(),
                json!({ "name": name, "args": arguments_object(arguments) }),
            );
            // The signature sits beside the call rather than inside it, which is where
            // Gemini issued it and the only place it validates against.
            if let Some(signature) = signature {
                part.insert("thoughtSignature".into(), json!(signature));
            }
            parts.push(Value::Object(part));
        }
    }

    parts.extend(image_parts(blocks));
    parts
}

/// Gemini has no error flag on a function response, so a failure is reported through the
/// response key rather than being read as a successful result.
fn tool_result_parts(tool_name: &str, blocks: &[ContentBlock], is_error: bool) -> Vec<Value> {
    let text = join_text(blocks);
    let response = if is_error {
        json!({ "error": text })
    } else {
        json!({ "output": text })
    };

    let mut parts = vec![json!({
        "functionResponse": { "name": tool_name, "response": response },
    })];
    parts.extend(image_parts(blocks));
    parts
}

fn image_parts(blocks: &[ContentBlock]) -> Vec<Value> {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Image { data, mime_type } => Some(json!({
                "inlineData": { "mimeType": mime_type, "data": data },
            })),
            _ => None,
        })
        .collect()
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

/// `args` must be an object; a call recorded without arguments still has to send one.
fn arguments_object(arguments: &Value) -> Value {
    if arguments.is_object() {
        arguments.clone()
    } else {
        Value::Object(Map::new())
    }
}

/// Keys whose value maps names the tool's author chose onto schemas. Those names are data,
/// not keywords, so a parameter called `pattern` or `default` must survive untouched.
const SCHEMA_MAPS: &[&str] = &[
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
];

/// Keys whose value is itself a schema.
const SCHEMA_VALUES: &[&str] = &[
    "items",
    "additionalProperties",
    "additionalItems",
    "unevaluatedItems",
    "unevaluatedProperties",
    "contains",
    "propertyNames",
    "not",
    "if",
    "then",
    "else",
];

/// Keys whose value is a list of schemas.
const SCHEMA_LISTS: &[&str] = &["allOf", "anyOf", "oneOf", "prefixItems"];

/// Strip the JSON Schema keywords Gemini rejects, wherever a schema appears.
///
/// The filter has to know where it is standing. A key at a schema position is a keyword and
/// is checked against the unsupported list; a key inside `properties` is a parameter name the
/// tool's author picked and is never checked, or a tool with a parameter called `pattern`
/// would lose it while `required` went on naming it. Anything that is plain data — `enum`,
/// `const`, `required`, a description — is copied through without being walked at all.
fn sanitize_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| !UNSUPPORTED_SCHEMA_FIELDS.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), sanitize_below(key, value)))
                .collect(),
        ),
        // A schema is an object; a bare `true` or `false` is a valid schema too and carries
        // nothing to strip.
        other => other.clone(),
    }
}

/// Sanitizes the value under `key`, according to what that key holds.
fn sanitize_below(key: &str, value: &Value) -> Value {
    if SCHEMA_MAPS.contains(&key) {
        return match value {
            Value::Object(named) => Value::Object(
                named
                    .iter()
                    .map(|(name, schema)| (name.clone(), sanitize_schema(schema)))
                    .collect(),
            ),
            other => other.clone(),
        };
    }

    if SCHEMA_LISTS.contains(&key) {
        return match value {
            Value::Array(schemas) => Value::Array(schemas.iter().map(sanitize_schema).collect()),
            other => other.clone(),
        };
    }

    if SCHEMA_VALUES.contains(&key) {
        return match value {
            // Older drafts allow a list of schemas where later ones take a single schema.
            Value::Array(schemas) => Value::Array(schemas.iter().map(sanitize_schema).collect()),
            other => sanitize_schema(other),
        };
    }

    // Not a schema position: `enum` values, a `const`, the `required` list, a description.
    value.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::ThinkingLevel;
    use micro_types::ToolDefinition;

    fn model() -> Model {
        Model {
            id: "gemini-2.5-pro".into(),
            provider: "gemini".into(),
            base_url: GEMINI_BASE_URL.into(),
            max_tokens: 8_192,
            thinking: ThinkingLevel::Off,
        }
    }

    fn drain(chunks: &[Value]) -> Vec<StreamEvent> {
        drain_with(&Arc::new(AtomicUsize::new(0)), chunks)
    }

    /// Drain one response against a counter the caller owns, which is how a test plays
    /// several requests through a single provider.
    fn drain_with(next_call: &Arc<AtomicUsize>, chunks: &[Value]) -> Vec<StreamEvent> {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut state = Accumulator::new(&model(), Arc::clone(next_call));
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

    fn chunk(parts: Value) -> Value {
        json!({ "candidates": [{ "content": { "parts": parts } }] })
    }

    fn finish(reason: &str) -> Value {
        json!({ "candidates": [{ "content": { "parts": [] }, "finishReason": reason }] })
    }

    fn tool_calls(message: &AssistantMessage) -> Vec<(String, String, Value)> {
        message
            .tool_calls()
            .into_iter()
            .map(|(id, name, arguments)| (id.to_string(), name.to_string(), arguments.clone()))
            .collect()
    }

    #[test]
    fn the_endpoint_names_the_model_and_asks_for_server_sent_events() {
        assert_eq!(
            endpoint(GEMINI_BASE_URL, "gemini-2.5-pro"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn a_catalog_id_that_already_says_models_is_not_repeated() {
        assert_eq!(
            endpoint("https://generativelanguage.googleapis.com/v1beta/", "models/gemini-2.5-flash"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn the_system_prompt_travels_in_its_own_field() {
        let context = Context {
            system_prompt: Some("be brief".into()),
            ..Context::default()
        };
        let payload = build_payload(&model(), &context);

        assert_eq!(
            payload["system_instruction"]["parts"][0]["text"],
            "be brief"
        );
        assert_eq!(payload["generationConfig"]["maxOutputTokens"], 8_192);
        assert!(payload["generationConfig"].get("thinkingConfig").is_none());

        let blank = build_payload(&model(), &Context::default());
        assert!(blank.get("system_instruction").is_none());
    }

    #[test]
    fn a_thinking_budget_asks_for_the_thoughts_as_well() {
        let payload = build_payload(
            &model().with_thinking(ThinkingLevel::Medium),
            &Context::default(),
        );
        let config = &payload["generationConfig"]["thinkingConfig"];

        assert_eq!(config["thinkingBudget"], 12_000);
        assert_eq!(config["includeThoughts"], true);
    }

    #[test]
    fn tools_are_declared_under_one_function_declarations_entry() {
        let context = Context {
            tools: vec![ToolDefinition {
                name: "read".into(),
                description: "read a file".into(),
                parameters: json!({ "type": "object" }),
            }],
            ..Context::default()
        };
        let payload = build_payload(&model(), &context);

        assert_eq!(payload["tools"].as_array().unwrap().len(), 1);
        assert_eq!(
            payload["tools"][0]["functionDeclarations"][0]["name"],
            "read"
        );
    }

    /// The failure this guards was a live 400 from Gemini: `required[0]: property is not
    /// defined`. `grep` declares a parameter named `pattern`, the filter matched that name
    /// against the JSON Schema keyword of the same name and deleted the property, and the
    /// `required` entry naming it was left pointing at nothing.
    #[test]
    fn a_parameter_named_after_a_keyword_survives() {
        let grep = micro_tools::builtin_tools("/work")
            .into_iter()
            .find(|tool| tool.definition().name == "grep")
            .expect("grep is a builtin tool");
        let sanitized = sanitize_schema(&grep.definition().parameters);

        assert!(
            sanitized["properties"].get("pattern").is_some(),
            "the parameter named `pattern` was deleted: {sanitized}"
        );
        assert_eq!(sanitized["properties"]["pattern"]["type"], "string");
        assert_eq!(sanitized["required"][0], "pattern");
    }

    #[test]
    fn other_parameters_named_after_keywords_survive_too() {
        let schema = json!({
            "type": "object",
            "properties": {
                "default": { "type": "string" },
                "enum": { "type": "string" },
                "items": { "type": "string" },
                "format": { "type": "string" },
                "title": { "type": "string" },
                "examples": { "type": "array" },
            },
            "required": ["default", "enum", "items", "format", "title", "examples"],
        });
        let sanitized = sanitize_schema(&schema);

        for name in ["default", "enum", "items", "format", "title", "examples"] {
            assert!(
                sanitized["properties"].get(name).is_some(),
                "the parameter named `{name}` was deleted"
            );
        }
    }

    #[test]
    fn a_keyword_at_a_keyword_position_is_still_stripped() {
        let schema = json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "pattern": "^a",
                    "format": "uri",
                    "minLength": 1,
                    "description": "kept",
                },
            },
        });
        let sanitized = sanitize_schema(&schema);

        // The property survives; the constraints inside it do not.
        let property = &sanitized["properties"]["pattern"];
        assert_eq!(property["description"], "kept");
        assert!(property.get("pattern").is_none(), "the keyword survived");
        assert!(property.get("format").is_none());
        assert!(property.get("minLength").is_none());
    }

    #[test]
    fn data_that_merely_looks_like_a_schema_is_left_alone() {
        let schema = json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["pattern", "default", "title"],
                    "const": { "pattern": "not a schema" },
                },
            },
            "required": ["mode"],
        });
        let sanitized = sanitize_schema(&schema);

        assert_eq!(
            sanitized["properties"]["mode"]["enum"],
            json!(["pattern", "default", "title"])
        );
        // A `const` is a value, so its keys are not keywords whatever they are named.
        assert_eq!(
            sanitized["properties"]["mode"]["const"]["pattern"],
            "not a schema"
        );
    }

    #[test]
    fn nested_schemas_are_still_reached() {
        let schema = json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "pattern": { "type": "string", "minLength": 1 } },
                        "required": ["pattern"],
                    },
                },
            },
        });
        let sanitized = sanitize_schema(&schema);
        let items = &sanitized["properties"]["edits"]["items"];

        assert!(items["properties"].get("pattern").is_some());
        assert!(items["properties"]["pattern"].get("minLength").is_none());
        assert_eq!(items["required"][0], "pattern");
    }

    /// The one that generalizes: whatever a tool declares, a sanitized schema may never
    /// name something in `required` that it does not also define in `properties`. Gemini
    /// rejects the request outright when it does.
    #[test]
    fn no_builtin_tool_requires_a_property_it_does_not_define() {
        fn check(schema: &Value, path: &str) {
            let Some(object) = schema.as_object() else {
                return;
            };

            if let Some(required) = object.get("required").and_then(Value::as_array) {
                let properties = object.get("properties").and_then(Value::as_object);
                for name in required.iter().filter_map(Value::as_str) {
                    assert!(
                        properties.is_some_and(|properties| properties.contains_key(name)),
                        "{path} requires `{name}` but does not define it: {schema}"
                    );
                }
            }

            // Every nested schema has to hold the same property.
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for (name, nested) in properties {
                    check(nested, &format!("{path}.{name}"));
                }
            }
            if let Some(items) = object.get("items") {
                check(items, &format!("{path}[]"));
            }
        }

        let tools = micro_tools::builtin_tools("/work");
        assert!(!tools.is_empty(), "there are builtin tools to check");
        for tool in tools {
            let definition = tool.definition();
            check(&sanitize_schema(&definition.parameters), &definition.name);
        }
    }

    #[test]
    fn unsupported_schema_keywords_are_stripped_at_every_depth() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "path": { "type": "string", "minLength": 1, "format": "uri", "description": "kept" },
                "modes": { "type": "array", "items": { "type": "string", "pattern": "^a" } }
            },
            "required": ["path"],
            "additionalProperties": false
        });
        let sanitized = sanitize_schema(&schema);

        assert!(sanitized.get("$schema").is_none());
        assert!(sanitized.get("additionalProperties").is_none());
        assert!(sanitized["properties"]["path"].get("minLength").is_none());
        assert!(sanitized["properties"]["path"].get("format").is_none());
        assert!(sanitized["properties"]["modes"]["items"]
            .get("pattern")
            .is_none());
        assert_eq!(sanitized["properties"]["path"]["description"], "kept");
        assert_eq!(sanitized["required"][0], "path");
    }

    #[test]
    fn assistant_calls_and_their_results_pair_up_by_name() {
        let context = Context {
            messages: vec![
                Message::user("go"),
                Message::Assistant(AssistantMessage {
                    content: vec![ContentBlock::ToolCall {
                        id: "gemini-call-0".into(),
                        name: "read".into(),
                        arguments: json!({ "path": "a.txt" }),

                        signature: None,
                    }],
                    provider: "gemini".into(),
                    model: "gemini-2.5-pro".into(),
                    usage: Usage::default(),
                    stop_reason: StopReason::ToolUse,
                    error: None,
                    timestamp: 0,
                }),
                Message::tool_result("gemini-call-0", "read", "contents", false),
            ],
            ..Context::default()
        };
        let contents = build_contents(&context.messages);

        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["functionCall"]["name"], "read");
        assert_eq!(
            contents[1]["parts"][0]["functionCall"]["args"]["path"],
            "a.txt"
        );
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "read");
        assert_eq!(
            contents[2]["parts"][0]["functionResponse"]["response"]["output"],
            "contents"
        );
    }

    #[test]
    fn a_failed_tool_reports_through_the_error_key() {
        let parts = tool_result_parts("read", &[ContentBlock::text("no such file")], true);
        assert_eq!(
            parts[0]["functionResponse"]["response"]["error"],
            "no such file"
        );
    }

    #[test]
    fn consecutive_user_turns_merge_into_one() {
        let messages = vec![
            Message::tool_result("a", "read", "first", false),
            Message::tool_result("b", "read", "second", false),
            Message::user("carry on"),
        ];
        let contents = build_contents(&messages);

        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn reasoning_is_replayed_only_when_it_carries_its_signature() {
        let signed = assistant_parts(&[
            ContentBlock::Thinking {
                thinking: "signed".into(),
                signature: Some("sig".into()),
            },
            ContentBlock::text("answer"),
        ]);
        assert_eq!(signed[0]["thought"], true);
        assert_eq!(signed[0]["thoughtSignature"], "sig");
        assert_eq!(signed[1]["text"], "answer");

        let unsigned = assistant_parts(&[
            ContentBlock::Thinking {
                thinking: "unsigned".into(),
                signature: None,
            },
            ContentBlock::text("answer"),
        ]);
        assert_eq!(unsigned.len(), 1);
        assert_eq!(unsigned[0]["text"], "answer");
    }

    #[test]
    fn images_ride_along_as_inline_data() {
        let parts = content_parts(&[
            ContentBlock::text("look"),
            ContentBlock::Image {
                data: "AAAA".into(),
                mime_type: "image/png".into(),
            },
        ]);
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/png");
        assert_eq!(parts[1]["inlineData"]["data"], "AAAA");
    }

    #[test]
    fn text_fragments_accumulate_across_chunks() {
        let events = drain(&[
            chunk(json!([{ "text": "he" }])),
            chunk(json!([{ "text": "llo" }])),
            finish("STOP"),
        ]);

        assert_eq!(events[0], StreamEvent::Start);
        assert_eq!(events[1], StreamEvent::TextStart { index: 0 });
        assert_eq!(done(&events).text(), "hello");
        assert_eq!(done(&events).stop_reason, StopReason::Stop);
    }

    #[test]
    fn every_function_call_part_closes_a_tool_call() {
        let events = drain(&[
            chunk(json!([
                { "functionCall": { "name": "read", "args": { "path": "a.txt" } } },
                { "functionCall": { "name": "list", "args": { "path": "." } } }
            ])),
            chunk(json!([{ "functionCall": { "name": "grep", "args": { "q": "x" } } }])),
            finish("STOP"),
        ]);

        let ends: Vec<_> = events
            .iter()
            .filter(|event| matches!(event, StreamEvent::ToolCallEnd { .. }))
            .collect();
        assert_eq!(ends.len(), 3);

        let message = done(&events);
        assert_eq!(
            tool_calls(&message),
            vec![
                (
                    "gemini-call-0".into(),
                    "read".into(),
                    json!({ "path": "a.txt" })
                ),
                (
                    "gemini-call-1".into(),
                    "list".into(),
                    json!({ "path": "." })
                ),
                ("gemini-call-2".into(), "grep".into(), json!({ "q": "x" })),
            ]
        );
        assert_eq!(message.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn a_call_without_arguments_still_arrives_as_an_object() {
        let events = drain(&[
            chunk(json!([{ "functionCall": { "name": "ls" } }])),
            finish("STOP"),
        ]);
        assert_eq!(
            tool_calls(&done(&events)),
            vec![("gemini-call-0".into(), "ls".into(), json!({}))]
        );
    }

    #[test]
    fn two_requests_through_one_provider_do_not_reuse_an_id() {
        let counter = Arc::new(AtomicUsize::new(0));

        let first = drain_with(
            &counter,
            &[
                chunk(json!([
                    { "functionCall": { "name": "read", "args": {} } },
                    { "functionCall": { "name": "list", "args": {} } }
                ])),
                finish("STOP"),
            ],
        );
        let second = drain_with(
            &counter,
            &[
                chunk(json!([{ "functionCall": { "name": "grep", "args": {} } }])),
                finish("STOP"),
            ],
        );

        let ids: Vec<String> = [first, second]
            .iter()
            .flat_map(|events| tool_calls(&done(events)))
            .map(|(id, ..)| id)
            .collect();
        assert_eq!(
            ids,
            vec!["gemini-call-0", "gemini-call-1", "gemini-call-2"],
            "a second request must not restart the numbering"
        );
    }

    #[test]
    fn ids_continue_from_the_calls_the_conversation_already_holds() {
        let history = vec![
            Message::user("go"),
            Message::Assistant(AssistantMessage {
                content: vec![
                    ContentBlock::text("working"),
                    ContentBlock::ToolCall {
                        id: "gemini-call-0".into(),
                        name: "read".into(),
                        arguments: json!({}),

                        signature: None,
                    },
                    ContentBlock::ToolCall {
                        id: "gemini-call-1".into(),
                        name: "list".into(),
                        arguments: json!({}),

                        signature: None,
                    },
                ],
                provider: "gemini".into(),
                model: "gemini-2.5-pro".into(),
                usage: Usage::default(),
                stop_reason: StopReason::ToolUse,
                error: None,
                timestamp: 0,
            }),
            Message::tool_result("gemini-call-0", "read", "contents", false),
        ];
        assert_eq!(tool_calls_so_far(&history), 2);

        // A resumed session seeds the counter from its own log, so the next id lands past
        // everything the log already holds.
        let counter = Arc::new(AtomicUsize::new(0));
        counter.fetch_max(tool_calls_so_far(&history), Ordering::Relaxed);
        let events = drain_with(
            &counter,
            &[
                chunk(json!([{ "functionCall": { "name": "grep", "args": {} } }])),
                finish("STOP"),
            ],
        );
        assert_eq!(done(&events).tool_calls()[0].0, "gemini-call-2");
    }

    #[test]
    fn a_shrinking_history_does_not_pull_the_numbering_back() {
        // Compaction leaves fewer calls in the conversation than the transcript on disk
        // holds. Seeding must never lower the counter, or the next id would collide with
        // one the transcript already used.
        let counter = Arc::new(AtomicUsize::new(0));
        counter.fetch_max(9, Ordering::Relaxed);
        counter.fetch_max(2, Ordering::Relaxed);

        let events = drain_with(
            &counter,
            &[
                chunk(json!([{ "functionCall": { "name": "grep", "args": {} } }])),
                finish("STOP"),
            ],
        );
        assert_eq!(done(&events).tool_calls()[0].0, "gemini-call-9");
    }

    #[test]
    fn only_a_synthesized_id_consumes_a_number() {
        let counter = Arc::new(AtomicUsize::new(0));
        let events = drain_with(
            &counter,
            &[
                chunk(json!([
                    { "functionCall": { "id": "fc_7", "name": "read", "args": {} } },
                    { "functionCall": { "name": "list", "args": {} } }
                ])),
                finish("STOP"),
            ],
        );

        let ids: Vec<String> = tool_calls(&done(&events))
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        assert_eq!(ids, vec!["fc_7", "gemini-call-0"]);
    }

    #[test]
    fn a_signature_is_read_off_whichever_part_carries_it() {
        assert_eq!(
            thought_signature(&json!({ "text": "hm", "thoughtSignature": "sig" })),
            Some("sig".to_string())
        );
        assert_eq!(
            thought_signature(
                &json!({ "functionCall": { "name": "read" }, "thoughtSignature": "sig" })
            ),
            Some("sig".to_string())
        );
        assert_eq!(thought_signature(&json!({ "text": "hm" })), None);
        assert_eq!(
            thought_signature(&json!({ "text": "hm", "thoughtSignature": "" })),
            None
        );
    }

    #[test]
    fn a_signature_on_a_function_call_is_kept_with_the_call() {
        let events = drain(&[
            chunk(json!([{
                "functionCall": { "name": "read", "args": {} },
                "thoughtSignature": "sig",
            }])),
            finish("STOP"),
        ]);

        let message = done(&events);
        assert_eq!(
            message.content,
            vec![ContentBlock::ToolCall {
                id: "gemini-call-0".into(),
                name: "read".into(),
                arguments: json!({}),
                signature: Some("sig".into()),
            }]
        );
    }

    #[test]
    fn a_call_without_a_signature_carries_none() {
        let events = drain(&[
            chunk(json!([{ "functionCall": { "name": "read", "args": {} } }])),
            finish("STOP"),
        ]);
        assert!(matches!(
            &done(&events).content[..],
            [ContentBlock::ToolCall {
                signature: None,
                ..
            }]
        ));
    }

    #[test]
    fn a_signature_is_replayed_beside_the_call_it_belongs_to() {
        let parts = assistant_parts(&[ContentBlock::ToolCall {
            id: "gemini-call-0".into(),
            name: "read".into(),
            arguments: json!({ "path": "a.txt" }),
            signature: Some("sig".into()),
        }]);

        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["functionCall"]["name"], "read");
        assert_eq!(parts[0]["functionCall"]["args"]["path"], "a.txt");
        assert_eq!(parts[0]["thoughtSignature"], "sig");
        // The signature is a sibling of the call, not a field inside it.
        assert!(parts[0]["functionCall"].get("thoughtSignature").is_none());
    }

    #[test]
    fn a_call_with_no_signature_replays_without_the_key() {
        let parts = assistant_parts(&[ContentBlock::ToolCall {
            id: "gemini-call-0".into(),
            name: "read".into(),
            arguments: json!({}),
            signature: None,
        }]);

        assert_eq!(parts[0]["functionCall"]["name"], "read");
        assert!(parts[0].get("thoughtSignature").is_none());
    }

    #[test]
    fn a_signature_survives_the_round_trip_through_a_session_log() {
        let events = drain(&[
            chunk(json!([{
                "functionCall": { "name": "read", "args": { "path": "a.txt" } },
                "thoughtSignature": "sig",
            }])),
            finish("STOP"),
        ]);
        let message = Message::Assistant(done(&events));

        let encoded = serde_json::to_string(&message).unwrap();
        let decoded: Message = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, message);

        // What comes back out of the log replays exactly as what went in.
        let Message::Assistant(decoded) = &decoded else {
            panic!("expected an assistant message");
        };
        assert_eq!(
            assistant_parts(&decoded.content)[0]["thoughtSignature"],
            "sig"
        );
    }

    #[test]
    fn a_call_that_names_itself_keeps_its_own_id() {
        let events = drain(&[
            chunk(json!([{ "functionCall": { "id": "fc_7", "name": "ls", "args": {} } }])),
            finish("STOP"),
        ]);
        assert_eq!(done(&events).tool_calls()[0].0, "fc_7");
    }

    #[test]
    fn text_is_closed_before_the_tool_call_that_follows_it() {
        let events = drain(&[
            chunk(json!([{ "text": "I will read it." }])),
            chunk(json!([{ "functionCall": { "name": "read", "args": {} } }])),
            finish("STOP"),
        ]);

        let order: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::TextEnd { index, .. } => Some(("text-end", *index)),
                StreamEvent::ToolCallStart { index, .. } => Some(("call-start", *index)),
                _ => None,
            })
            .collect();
        assert_eq!(order, vec![("text-end", 0), ("call-start", 1)]);

        assert!(matches!(
            done(&events).content.as_slice(),
            [ContentBlock::Text { .. }, ContentBlock::ToolCall { .. }]
        ));
    }

    #[test]
    fn thoughts_become_a_thinking_block_that_keeps_its_signature() {
        let events = drain(&[
            chunk(json!([{ "text": "weighing it", "thought": true }])),
            chunk(json!([{ "text": " up", "thought": true, "thoughtSignature": "sig" }])),
            chunk(json!([{ "text": "done" }])),
            finish("STOP"),
        ]);

        assert_eq!(events[1], StreamEvent::ThinkingStart { index: 0 });
        assert_eq!(
            done(&events).content[0],
            ContentBlock::Thinking {
                thinking: "weighing it up".into(),
                signature: Some("sig".into()),
            }
        );
    }

    #[test]
    fn reasoning_tokens_are_billed_as_output_and_cached_ones_leave_the_input() {
        let events = drain(&[
            chunk(json!([{ "text": "hi" }])),
            json!({
                "candidates": [{ "content": { "parts": [] }, "finishReason": "STOP" }],
                "usageMetadata": {
                    "promptTokenCount": 100,
                    "cachedContentTokenCount": 40,
                    "candidatesTokenCount": 10,
                    "thoughtsTokenCount": 5
                }
            }),
        ]);
        let usage = done(&events).usage;

        assert_eq!(usage.input, 60);
        assert_eq!(usage.cache_read, 40);
        assert_eq!(usage.output, 15);
    }

    #[test]
    fn abnormal_terminations_are_errors_rather_than_a_silent_stop() {
        assert_eq!(map_finish_reason("STOP"), StopReason::Stop);
        assert_eq!(map_finish_reason("MAX_TOKENS"), StopReason::Length);
        assert_eq!(map_finish_reason("SAFETY"), StopReason::Error);
        assert_eq!(
            map_finish_reason("MALFORMED_FUNCTION_CALL"),
            StopReason::Error
        );
        assert_eq!(map_finish_reason("stop"), StopReason::Stop);
        assert_eq!(map_finish_reason("SOMETHING_NEW"), StopReason::Stop);
    }

    #[test]
    fn a_safety_stop_is_not_upgraded_to_tool_use() {
        let events = drain(&[
            chunk(json!([{ "functionCall": { "name": "read", "args": {} } }])),
            finish("SAFETY"),
        ]);
        assert_eq!(done(&events).stop_reason, StopReason::Error);
    }

    #[test]
    fn a_blocked_prompt_ends_the_stream_with_an_error() {
        let events = drain(&[json!({ "promptFeedback": { "blockReason": "SAFETY" } })]);
        assert_eq!(
            events.as_slice(),
            [StreamEvent::Error {
                message: "gemini blocked the prompt: SAFETY".into(),
            }]
        );
    }

    #[test]
    fn unparseable_chunks_are_ignored() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut state = Accumulator::new(&model(), Arc::new(AtomicUsize::new(0)));
        state.handle(
            SseEvent {
                event: None,
                data: "not json".into(),
            },
            &sender,
        );
        drop(sender);

        assert!(receiver.try_recv().is_err());
        assert!(!state.started);
    }
}
