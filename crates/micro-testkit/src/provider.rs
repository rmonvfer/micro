//! A [`Provider`] whose responses a test writes in advance.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use micro_provider::Provider;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::StreamEvent;
use micro_types::Usage;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

/// One scripted model response: the events it streams and the message it assembles to.
///
/// The terminal [`StreamEvent::Done`] is appended when the turn is streamed, so a test
/// describes the content and never has to keep the events and the assembled message in
/// agreement by hand.
#[derive(Debug, Clone, Default)]
pub struct Turn {
    events: Vec<StreamEvent>,
    content: Vec<ContentBlock>,
    stop_reason: Option<StopReason>,
    usage: Usage,
    /// Set for a turn that terminates with [`StreamEvent::Error`] instead of `Done`.
    failure: Option<String>,
    emit_start: bool,
    next_index: usize,
}

impl Turn {
    /// An empty turn: no content, stopping normally.
    pub fn new() -> Self {
        Turn {
            emit_start: true,
            ..Default::default()
        }
    }

    /// A turn whose whole response is one block of text.
    pub fn text(text: impl Into<String>) -> Self {
        Turn::new().with_text(text)
    }

    /// A turn whose text arrives as several deltas, the way a real stream delivers it.
    pub fn streamed_text<I, S>(chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Turn::new().with_streamed_text(chunks)
    }

    /// A turn that fails outright. Nothing is streamed before the error, which is what a
    /// request rejected at the HTTP layer looks like.
    pub fn error(message: impl Into<String>) -> Self {
        Turn::new().with_start(false).failing(message)
    }

    /// End this turn with [`StreamEvent::Error`] instead of `Done`, after whatever it has
    /// already streamed. Combined with content, this is a stream that died partway
    /// through — the case where retrying would duplicate text the user has already seen.
    pub fn failing(mut self, message: impl Into<String>) -> Self {
        self.failure = Some(message.into());
        self
    }

    /// Emit [`StreamEvent::Start`] before whatever else this turn streams. On by default
    /// for turns that carry content, off for [`Turn::error`].
    pub fn with_start(mut self, emit_start: bool) -> Self {
        self.emit_start = emit_start;
        self
    }

    /// Drop the incremental events, keeping the assembled message. Models a provider that
    /// answers with nothing but the terminal event.
    pub fn without_deltas(mut self) -> Self {
        self.events.clear();
        self.emit_start = false;
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        let index = self.take_index();
        self.events.push(StreamEvent::TextStart { index });
        self.events.push(StreamEvent::TextDelta {
            index,
            delta: text.clone(),
        });
        self.events.push(StreamEvent::TextEnd {
            index,
            text: text.clone(),
        });
        self.content.push(ContentBlock::text(text));
        self
    }

    pub fn with_streamed_text<I, S>(mut self, chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let index = self.take_index();
        self.events.push(StreamEvent::TextStart { index });
        let mut text = String::new();
        for chunk in chunks {
            let delta = chunk.into();
            text.push_str(&delta);
            self.events.push(StreamEvent::TextDelta { index, delta });
        }
        self.events.push(StreamEvent::TextEnd {
            index,
            text: text.clone(),
        });
        self.content.push(ContentBlock::text(text));
        self
    }

    pub fn with_thinking(mut self, thinking: impl Into<String>) -> Self {
        let thinking = thinking.into();
        let index = self.take_index();
        self.events.push(StreamEvent::ThinkingStart { index });
        self.events.push(StreamEvent::ThinkingDelta {
            index,
            delta: thinking.clone(),
        });
        self.events.push(StreamEvent::ThinkingEnd {
            index,
            thinking: thinking.clone(),
        });
        self.content.push(ContentBlock::Thinking {
            thinking,
            signature: None,
        });
        self
    }

    /// Ask for a tool. Unless a stop reason is set explicitly, a turn with any tool call
    /// stops with [`StopReason::ToolUse`].
    pub fn with_tool_call(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: Value,
    ) -> Self {
        let (id, name) = (id.into(), name.into());
        let index = self.take_index();
        self.events.push(StreamEvent::ToolCallStart {
            index,
            id: id.clone(),
            name: name.clone(),
        });
        self.events.push(StreamEvent::ToolCallDelta {
            index,
            delta: arguments.to_string(),
        });
        self.events.push(StreamEvent::ToolCallEnd {
            index,
            id: id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        });
        self.content.push(ContentBlock::ToolCall {
            id,
            name,
            arguments,
            signature: None,
        });
        self
    }

    pub fn with_stop_reason(mut self, stop_reason: StopReason) -> Self {
        self.stop_reason = Some(stop_reason);
        self
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }

    fn take_index(&mut self) -> usize {
        let index = self.next_index;
        self.next_index += 1;
        index
    }

    fn stop_reason(&self) -> StopReason {
        self.stop_reason.unwrap_or({
            if self
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
            {
                StopReason::ToolUse
            } else {
                StopReason::Stop
            }
        })
    }

    /// The full event sequence, stamped with the provider and model the agent asked for.
    fn into_events(self, provider: &str, model: &str) -> Vec<StreamEvent> {
        let mut events = Vec::with_capacity(self.events.len() + 2);
        if self.emit_start {
            events.push(StreamEvent::Start);
        }
        events.extend(self.events.iter().cloned());

        events.push(match &self.failure {
            Some(message) => StreamEvent::Error {
                message: message.clone(),
            },
            None => StreamEvent::Done {
                message: AssistantMessage {
                    content: self.content.clone(),
                    provider: provider.to_string(),
                    model: model.to_string(),
                    usage: self.usage,
                    stop_reason: self.stop_reason(),
                    error: None,
                    timestamp: 0,
                },
            },
        });
        events
    }
}

/// One request the agent issued, captured as the provider received it.
#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub model: Model,
    pub api_key: String,
    pub context: Context,
}

impl RecordedCall {
    /// The tool definitions the agent advertised, in order.
    pub fn tool_names(&self) -> Vec<&str> {
        self.context
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect()
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.context.system_prompt.as_deref()
    }

    pub fn messages(&self) -> &[Message] {
        &self.context.messages
    }

    /// The shape of the conversation as `"user"` / `"assistant"` / `"tool_result"`, which
    /// is the readable way to assert on message ordering.
    pub fn message_roles(&self) -> Vec<&'static str> {
        self.context.messages.iter().map(role_of).collect()
    }

    /// Every tool result in the context: `(tool_call_id, tool_name, text, is_error)`.
    pub fn tool_results(&self) -> Vec<(&str, &str, String, bool)> {
        self.context
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::ToolResult {
                    tool_call_id,
                    tool_name,
                    content,
                    is_error,
                    ..
                } => Some((
                    tool_call_id.as_str(),
                    tool_name.as_str(),
                    text_of(content),
                    *is_error,
                )),
                _ => None,
            })
            .collect()
    }

    /// Tool result ids with no matching tool call earlier in the conversation. A
    /// well-formed context leaves this empty; providers reject the request otherwise.
    pub fn orphaned_tool_results(&self) -> Vec<&str> {
        let mut requested: Vec<&str> = Vec::new();
        let mut orphaned = Vec::new();
        for message in &self.context.messages {
            match message {
                Message::Assistant(assistant) => {
                    requested.extend(assistant.tool_calls().into_iter().map(|(id, ..)| id));
                }
                Message::ToolResult { tool_call_id, .. } => {
                    if !requested.contains(&tool_call_id.as_str()) {
                        orphaned.push(tool_call_id.as_str());
                    }
                }
                Message::User { .. } => {}
            }
        }
        orphaned
    }

    /// Tool call ids the conversation never answered. A context ending in an unanswered
    /// call is the other half of the pairing invariant.
    pub fn unanswered_tool_calls(&self) -> Vec<&str> {
        let answered: Vec<&str> = self
            .context
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
                _ => None,
            })
            .collect();

        self.context
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::Assistant(assistant) => Some(assistant),
                _ => None,
            })
            .flat_map(|assistant| assistant.tool_calls())
            .map(|(id, ..)| id)
            .filter(|id| !answered.contains(id))
            .collect()
    }
}

fn role_of(message: &Message) -> &'static str {
    match message {
        Message::User { .. } => "user",
        Message::Assistant(_) => "assistant",
        Message::ToolResult { .. } => "tool_result",
    }
}

fn text_of(content: &[ContentBlock]) -> String {
    content.iter().map(ContentBlock::as_text).collect()
}

/// A provider that replays scripted turns and records every request it was given.
///
/// Turns are consumed in order, one per request. Once the script runs out, further
/// requests fail with [`FakeProvider::EXHAUSTED`] rather than hanging, so a loop that
/// requests more turns than a test expected ends with a legible failure.
#[derive(Clone)]
pub struct FakeProvider {
    inner: Arc<Inner>,
}

struct Inner {
    name: String,
    turns: Mutex<VecDeque<Turn>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeProvider {
    /// The error a request gets once the scripted turns are used up. Not retryable, so
    /// the agent loop stops instead of spinning.
    pub const EXHAUSTED: &'static str = "fake provider: no turns left in the script";

    pub fn builder() -> FakeProviderBuilder {
        FakeProviderBuilder::default()
    }

    /// A provider that answers one request and no more.
    pub fn once(turn: Turn) -> Self {
        FakeProvider::builder().turn(turn).build()
    }

    /// Every request the agent issued, oldest first.
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.inner.calls.lock().expect("calls lock").clone()
    }

    pub fn call_count(&self) -> usize {
        self.inner.calls.lock().expect("calls lock").len()
    }

    /// The request at `index`, panicking with a legible message when there wasn't one.
    pub fn call(&self, index: usize) -> RecordedCall {
        let calls = self.calls();
        assert!(
            index < calls.len(),
            "expected at least {} request(s), the agent issued {}",
            index + 1,
            calls.len()
        );
        calls[index].clone()
    }

    /// Turns still waiting to be streamed.
    pub fn remaining_turns(&self) -> usize {
        self.inner.turns.lock().expect("turns lock").len()
    }
}

impl Provider for FakeProvider {
    fn name(&self) -> &str {
        &self.inner.name
    }

    fn stream(
        &self,
        model: Model,
        context: Context,
        api_key: String,
    ) -> UnboundedReceiver<StreamEvent> {
        self.inner
            .calls
            .lock()
            .expect("calls lock")
            .push(RecordedCall {
                model: model.clone(),
                api_key,
                context,
            });

        let turn = self.inner.turns.lock().expect("turns lock").pop_front();
        let events = match turn {
            Some(turn) => turn.into_events(&self.inner.name, &model.id),
            None => vec![StreamEvent::Error {
                message: FakeProvider::EXHAUSTED.to_string(),
            }],
        };

        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        for event in events {
            let _ = sender.send(event);
        }
        receiver
    }
}

#[derive(Default)]
pub struct FakeProviderBuilder {
    name: Option<String>,
    turns: VecDeque<Turn>,
}

impl FakeProviderBuilder {
    /// The name the provider reports, which also stamps the assembled messages.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn turn(mut self, turn: Turn) -> Self {
        self.turns.push_back(turn);
        self
    }

    pub fn turns(mut self, turns: impl IntoIterator<Item = Turn>) -> Self {
        self.turns.extend(turns);
        self
    }

    pub fn build(self) -> FakeProvider {
        FakeProvider {
            inner: Arc::new(Inner {
                name: self.name.unwrap_or_else(|| "fake".to_string()),
                turns: Mutex::new(self.turns),
                calls: Mutex::new(Vec::new()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::ThinkingLevel;
    use serde_json::json;

    fn model() -> Model {
        Model {
            id: "test-model".into(),
            provider: "fake".into(),
            base_url: "https://example.invalid".into(),
            max_tokens: 1024,
            thinking: ThinkingLevel::Off,
            compat: Default::default(),
            headers: Default::default(),
            reasoning: Default::default(),
        }
    }

    async fn drain(mut receiver: UnboundedReceiver<StreamEvent>) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn a_text_turn_streams_deltas_then_done() {
        let provider = FakeProvider::once(Turn::streamed_text(["he", "llo"]));
        let events = drain(provider.stream(model(), Context::default(), "key".into())).await;

        assert!(matches!(events.first(), Some(StreamEvent::Start)));
        let Some(StreamEvent::Done { message }) = events.last() else {
            panic!("a text turn ends with Done, got {:?}", events.last());
        };
        assert_eq!(message.text(), "hello");
        assert_eq!(message.stop_reason, StopReason::Stop);
        assert_eq!(message.provider, "fake");
        assert_eq!(message.model, "test-model");
    }

    #[tokio::test]
    async fn a_tool_call_turn_stops_for_tool_use() {
        let provider =
            FakeProvider::once(Turn::new().with_tool_call("c1", "read", json!({"path": "a"})));
        let events = drain(provider.stream(model(), Context::default(), "key".into())).await;

        let Some(StreamEvent::Done { message }) = events.last() else {
            panic!("expected Done");
        };
        assert_eq!(message.stop_reason, StopReason::ToolUse);
        assert_eq!(
            message.tool_calls(),
            vec![("c1", "read", &json!({"path": "a"}))]
        );
    }

    #[tokio::test]
    async fn an_explicit_stop_reason_wins_over_the_inferred_one() {
        let provider = FakeProvider::once(
            Turn::new()
                .with_tool_call("c1", "read", json!({}))
                .with_stop_reason(StopReason::Length),
        );
        let events = drain(provider.stream(model(), Context::default(), "key".into())).await;

        let Some(StreamEvent::Done { message }) = events.last() else {
            panic!("expected Done");
        };
        assert_eq!(message.stop_reason, StopReason::Length);
    }

    #[tokio::test]
    async fn an_error_turn_streams_nothing_before_failing() {
        let provider = FakeProvider::once(Turn::error("boom"));
        let events = drain(provider.stream(model(), Context::default(), "key".into())).await;

        assert_eq!(
            events,
            vec![StreamEvent::Error {
                message: "boom".into()
            }]
        );
    }

    #[tokio::test]
    async fn an_error_turn_can_be_preceded_by_a_start() {
        let provider = FakeProvider::once(Turn::error("boom").with_start(true));
        let events = drain(provider.stream(model(), Context::default(), "key".into())).await;

        assert_eq!(events.len(), 2);
        assert!(matches!(events.first(), Some(StreamEvent::Start)));
    }

    #[tokio::test]
    async fn turns_are_replayed_in_order_then_exhaust() {
        let provider = FakeProvider::builder()
            .turns([Turn::text("first"), Turn::text("second")])
            .build();

        for expected in ["first", "second"] {
            let events = drain(provider.stream(model(), Context::default(), "key".into())).await;
            let Some(StreamEvent::Done { message }) = events.last() else {
                panic!("expected Done");
            };
            assert_eq!(message.text(), expected);
        }
        assert_eq!(provider.remaining_turns(), 0);

        let events = drain(provider.stream(model(), Context::default(), "key".into())).await;
        assert_eq!(
            events,
            vec![StreamEvent::Error {
                message: FakeProvider::EXHAUSTED.into()
            }]
        );
    }

    #[tokio::test]
    async fn every_request_is_recorded() {
        let provider = FakeProvider::builder()
            .turns([Turn::new(), Turn::new()])
            .build();
        let context = Context {
            system_prompt: Some("be brief".into()),
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            headers: Vec::new(),
            cache_key: None,
        };

        drain(provider.stream(model(), context.clone(), "secret".into())).await;
        drain(provider.stream(model(), context, "secret".into())).await;

        assert_eq!(provider.call_count(), 2);
        assert_eq!(provider.call(0).api_key, "secret");
        assert_eq!(provider.call(0).system_prompt(), Some("be brief"));
        assert_eq!(provider.call(0).message_roles(), vec!["user"]);
    }

    #[test]
    fn tool_call_pairing_is_reported_from_a_recorded_context() {
        let assistant = AssistantMessage {
            content: vec![ContentBlock::ToolCall {
                id: "c1".into(),
                name: "read".into(),
                arguments: json!({}),

                signature: None,
            }],
            provider: "fake".into(),
            model: "test-model".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error: None,
            timestamp: 0,
        };

        let paired = RecordedCall {
            model: model(),
            api_key: String::new(),
            context: Context {
                system_prompt: None,
                messages: vec![
                    Message::user("hi"),
                    Message::Assistant(assistant.clone()),
                    Message::tool_result("c1", "read", "ok", false),
                ],
                tools: Vec::new(),
                headers: Vec::new(),
                cache_key: None,
            },
        };
        assert!(paired.orphaned_tool_results().is_empty());
        assert!(paired.unanswered_tool_calls().is_empty());
        assert_eq!(
            paired.tool_results(),
            vec![("c1", "read", "ok".to_string(), false)]
        );

        let unanswered = RecordedCall {
            context: Context {
                messages: vec![Message::user("hi"), Message::Assistant(assistant)],
                ..Default::default()
            },
            ..paired.clone()
        };
        assert_eq!(unanswered.unanswered_tool_calls(), vec!["c1"]);

        let orphaned = RecordedCall {
            context: Context {
                messages: vec![Message::tool_result("stray", "read", "ok", false)],
                ..Default::default()
            },
            ..paired
        };
        assert_eq!(orphaned.orphaned_tool_results(), vec!["stray"]);
    }
}
