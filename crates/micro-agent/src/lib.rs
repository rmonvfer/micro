//! The agent loop: stream a response, run the tools it asks for, repeat until the
//! model stops asking for tools.

mod summarizer;

pub use summarizer::ProviderSummarizer;

use micro_context::CompactionConfig;
use micro_context::Compactor;
use micro_context::Summarizer;
use micro_provider::Provider;
use micro_tools::Tool;
use micro_types::now_ms;
use micro_types::AgentEvent;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
use micro_types::Context;
use micro_types::Message;
use micro_types::Model;
use micro_types::StopReason;
use micro_types::StreamEvent;
use micro_types::ThinkingLevel;
use micro_types::ToolDefinition;
use micro_types::Usage;
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const MAX_ATTEMPTS: u32 = 5;
const MAX_RETRY_DELAY_MS: u64 = 30_000;
/// Transient statuses worth re-issuing a request for.
const RETRYABLE_STATUSES: [u16; 8] = [408, 409, 425, 429, 500, 502, 503, 504];

/// The context window assumed when the caller does not supply the model's own.
///
/// Set to the smallest window among the models the agent is likely to run against, so an
/// unconfigured agent compacts early rather than never. A caller that knows the real
/// number passes it to [`Agent::with_context_window`].
pub const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

/// A model to run from now on, and everything needed to reach it.
///
/// Assembled by whoever owns the catalog and the credentials, and handed to the agent,
/// because the agent knows neither. This is what `/model` and `/provider` produce.
#[derive(Clone)]
pub struct ModelSwap {
    pub provider: Arc<dyn Provider>,
    pub model: Model,
    pub api_key: String,
    pub context_window: usize,
}

impl std::fmt::Debug for ModelSwap {
    /// The credential is deliberately absent: a swap ends up in log lines and error
    /// messages, and a key has no business in either.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelSwap")
            .field("provider", &self.provider.name())
            .field("model", &self.model.id)
            .field("context_window", &self.context_window)
            .finish()
    }
}

impl PartialEq for ModelSwap {
    /// Two swaps are the same when they name the same model on the same provider. The
    /// client is a live object rather than a value, so identity is taken from what it is
    /// pointed at.
    fn eq(&self, other: &Self) -> bool {
        self.provider.name() == other.provider.name()
            && self.model == other.model
            && self.context_window == other.context_window
    }
}

pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Vec<Arc<dyn Tool>>,
    model: Model,
    api_key: String,
    system_prompt: Option<String>,
    messages: Vec<Message>,
    recorder: Option<UnboundedSender<Message>>,
    /// Anything else watching the events this run produces.
    observer: Option<UnboundedSender<AgentEvent>>,
    /// Anything allowed to change what a tool call does.
    hooks: Option<Arc<dyn ToolHooks>>,
    summarizer: Arc<dyn Summarizer>,
    compaction: Option<CompactionConfig>,
    context_window: usize,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Vec<Arc<dyn Tool>>,
        model: Model,
        api_key: impl Into<String>,
    ) -> Self {
        let api_key = api_key.into();
        let summarizer = Arc::new(ProviderSummarizer::new(
            provider.clone(),
            model.clone(),
            api_key.clone(),
        ));

        Agent {
            provider,
            tools,
            model,
            api_key,
            system_prompt: None,
            messages: Vec::new(),
            recorder: None,
            observer: None,
            hooks: None,
            summarizer,
            compaction: Some(CompactionConfig::default()),
            context_window: DEFAULT_CONTEXT_WINDOW,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Point the agent at a different model, keeping the conversation.
    ///
    /// The summarizer is rebuilt with it, so compaction stops using the model that was
    /// swapped out, and the context window comes along because it belongs to the model
    /// rather than to the agent.
    pub fn set_model(&mut self, swap: ModelSwap) {
        self.provider = swap.provider;
        self.model = swap.model;
        self.api_key = swap.api_key;
        self.context_window = swap.context_window;
        self.summarizer = Arc::new(ProviderSummarizer::new(
            self.provider.clone(),
            self.model.clone(),
            self.api_key.clone(),
        ));
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Reason this hard from the next turn on.
    ///
    /// Unlike a model swap this needs nothing from outside — the level rides on the model
    /// the agent already holds — so the interface applies it without asking the host.
    pub fn set_thinking(&mut self, level: ThinkingLevel) {
        self.model.thinking = level;
    }

    /// The model's context window in tokens, which decides when compaction fires.
    pub fn with_context_window(mut self, tokens: usize) -> Self {
        self.context_window = tokens;
        self
    }

    pub fn with_compaction(mut self, config: CompactionConfig) -> Self {
        self.compaction = Some(config);
        self
    }

    /// Let the conversation grow unchecked, for a caller that manages the window itself.
    pub fn without_compaction(mut self) -> Self {
        self.compaction = None;
        self
    }

    /// Turn compaction on or off while the agent is running, which is what a headless
    /// caller does when it would rather manage the window itself.
    pub fn set_auto_compaction(&mut self, enabled: bool) {
        self.compaction = match enabled {
            true => Some(self.compaction.unwrap_or_default()),
            false => None,
        };
    }

    /// Run a different model through the provider already in hand.
    ///
    /// Only for a model the current provider serves: a model somewhere else needs a client
    /// and a credential, which is [`Agent::set_model`].
    pub fn set_runtime_model(&mut self, model: Model) {
        self.model = model;
        self.summarizer = Arc::new(ProviderSummarizer::new(
            self.provider.clone(),
            self.model.clone(),
            self.api_key.clone(),
        ));
    }

    /// Summarize with something other than the model the agent is running, which is how a
    /// caller routes summaries to a cheaper model.
    pub fn with_summarizer(mut self, summarizer: Arc<dyn Summarizer>) -> Self {
        self.summarizer = summarizer;
        self
    }

    /// Send every finalized message to `recorder` as it is produced, so a conversation is
    /// durable as it happens rather than only once the run returns.
    pub fn with_recorder(mut self, recorder: UnboundedSender<Message>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Let something decide what a tool call may do.
    ///
    /// A hook sits between the model asking for a tool and the tool running, and again
    /// between the tool answering and the model reading it. It is the one place an
    /// extension is allowed to change what happens rather than watch it happen.
    pub fn with_tool_hooks(mut self, hooks: Arc<dyn ToolHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Send every event to `observer` as well as to whoever asked for the turn.
    ///
    /// One turn has one caller — a terminal, a headless mode — and that caller owns the
    /// events. Anything else that needs to see them, extensions among them, watches from
    /// here rather than intercepting the caller's channel.
    pub fn with_observer(mut self, observer: UnboundedSender<AgentEvent>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Seed the conversation with prior history, for resuming a saved session.
    pub fn with_history(mut self, history: Vec<Message>) -> Self {
        self.messages = history;
        self
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Replace what the model is told before the conversation, which is how re-read
    /// instruction files and skills reach a run that is already going.
    pub fn set_system_prompt(&mut self, prompt: impl Into<String>) {
        self.system_prompt = Some(prompt.into());
    }

    /// Summarize the conversation now, whether or not it has grown enough to trigger on
    /// its own, and continue from the summary.
    ///
    /// The summary is returned so a caller can show it. Only the live conversation is
    /// rewritten: the recorder already has every message verbatim, so the session log
    /// stays a full transcript.
    pub async fn compact_now(&mut self) -> std::result::Result<Message, CompactionRefusal> {
        let config = self.compaction.unwrap_or_default();
        let compactor = Compactor::new(self.summarizer.clone(), config);
        let compacted = compactor
            .compact(&self.messages, self.context_window)
            .await
            .map_err(|error| match error {
                micro_context::ContextError::NothingToCompact => CompactionRefusal::TooSmall,
                error => CompactionRefusal::Failed(error.to_string()),
            })?;

        let summary = compacted.messages[0].clone();
        self.messages = compacted.messages;
        Ok(summary)
    }

    /// Put the agent in a different conversation.
    ///
    /// Branching, resuming and clearing all change what has been said, not just what is on
    /// screen. Anything less than this leaves the model answering from messages the user
    /// can no longer see.
    pub fn set_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// Put a message into the conversation without asking the model anything.
    ///
    /// A shell command the user ran themselves belongs in the context — the model should
    /// know what they just did — but running it is not a turn, so nothing is sent and no
    /// response is waited for. The message still reaches the recorder, so it lands in the
    /// session log and survives a resume.
    pub fn record(&mut self, message: Message) {
        if let Some(recorder) = &self.recorder {
            let _ = recorder.send(message.clone());
        }
        self.messages.push(message);
    }

    /// Whether something watching the run refuses this call, and why.
    async fn blocked(&self, id: &str, name: &str, arguments: &Value) -> Option<String> {
        self.hooks.as_ref()?.before_tool(id, name, arguments).await
    }

    /// The result as it should reach the model, after anything watching has had it.
    async fn rewritten(
        &self,
        id: &str,
        name: &str,
        output: String,
        is_error: bool,
    ) -> (String, bool) {
        match &self.hooks {
            Some(hooks) => hooks.after_tool(id, name, output, is_error).await,
            None => (output, is_error),
        }
    }

    /// Both places an event goes, as one thing to send to.
    fn fan<'a>(&self, events: &'a UnboundedSender<AgentEvent>) -> Fan<'a> {
        Fan {
            primary: events,
            observer: self.observer.clone(),
        }
    }

    /// Append to the conversation, reporting the message to the recorder if one is set.
    fn commit(&mut self, message: Message, produced: &mut Vec<Message>) {
        if let Some(recorder) = &self.recorder {
            let _ = recorder.send(message.clone());
        }
        self.messages.push(message.clone());
        produced.push(message);
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|tool| tool.definition()).collect()
    }

    fn find_tool(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|tool| tool.definition().name == name)
    }

    /// Run one exchange to completion. Every step is reported on `events`; the returned
    /// vector is the messages this run added to the conversation.
    pub async fn run(
        &mut self,
        prompt: Message,
        events: &UnboundedSender<AgentEvent>,
    ) -> Vec<Message> {
        let events = &self.fan(events);
        let mut produced = Vec::new();

        // A turn abandoned partway — Ctrl+C during a tool, or a crash — leaves an assistant
        // message whose tool calls were never answered. Providers reject a request that
        // contains one, so the conversation would stay unusable for the rest of its life and
        // its log would be unresumable. Answering the abandoned calls first makes the
        // conversation valid again, whether it was interrupted moments ago in this process
        // or a week ago in another one.
        for repair in answer_abandoned_calls(&mut self.messages) {
            if let Some(recorder) = &self.recorder {
                let _ = recorder.send(repair.clone());
            }
            events.send(AgentEvent::MessageStart {
                message: repair.clone(),
            });
            events.send(AgentEvent::MessageEnd {
                message: repair.clone(),
            });
            produced.push(repair);
        }

        events.send(AgentEvent::AgentStart);
        events.send(AgentEvent::MessageStart {
            message: prompt.clone(),
        });
        events.send(AgentEvent::MessageEnd {
            message: prompt.clone(),
        });
        self.commit(prompt, &mut produced);

        loop {
            events.send(AgentEvent::TurnStart);
            self.compact_if_needed(events).await;

            let assistant = self.stream_once(events).await;
            self.commit(Message::Assistant(assistant.clone()), &mut produced);

            if matches!(
                assistant.stop_reason,
                StopReason::Error | StopReason::Aborted
            ) {
                break;
            }

            let calls: Vec<(String, String, serde_json::Value)> = assistant
                .tool_calls()
                .into_iter()
                .map(|(id, name, arguments)| (id.to_string(), name.to_string(), arguments.clone()))
                .collect();

            if calls.is_empty() {
                break;
            }

            // A response cut off by the token limit can yield tool calls whose streamed
            // arguments happen to parse but are silently incomplete. None are safe to run.
            let truncated = assistant.stop_reason == StopReason::Length;

            for (id, name, arguments) in calls {
                events.send(AgentEvent::ToolStart {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                });

                let (output, is_error) = if truncated {
                    (
                        format!(
                            "Tool call \"{name}\" was not executed: the response hit the output \
                             token limit, so its arguments may be truncated. Re-issue the call \
                             with complete arguments."
                        ),
                        true,
                    )
                } else if let Some(refusal) = self.blocked(&id, &name, &arguments).await {
                    // Something watching the run refused the call. The model is told why,
                    // in the same shape a tool's own failure takes.
                    (refusal, true)
                } else {
                    match self.find_tool(&name) {
                        Some(tool) => match tool.execute(&arguments).await {
                            Ok(output) => (output, false),
                            Err(error) => (error, true),
                        },
                        None => (format!("tool not found: {name}"), true),
                    }
                };

                // What ran can be rewritten before the model sees it, which is how a
                // result is redacted or replaced by something watching.
                let (output, is_error) = self.rewritten(&id, &name, output, is_error).await;

                events.send(AgentEvent::ToolEnd {
                    id: id.clone(),
                    name: name.clone(),
                    output: output.clone(),
                    is_error,
                });

                let result = Message::tool_result(id, name, output, is_error);
                events.send(AgentEvent::MessageStart {
                    message: result.clone(),
                });
                events.send(AgentEvent::MessageEnd {
                    message: result.clone(),
                });
                self.commit(result, &mut produced);
            }
        }

        events.send(AgentEvent::AgentEnd {
            messages: produced.clone(),
        });
        produced
    }

    /// Replace the older part of the conversation with a summary once it approaches the
    /// context window.
    ///
    /// Only the live conversation is rewritten. The run's own output and the recorder keep
    /// every message verbatim, so what gets persisted stays a full transcript and
    /// compaction stays a property of this process's context rather than of the session.
    async fn compact_if_needed(&mut self, events: &Fan<'_>) {
        let Some(config) = self.compaction else {
            return;
        };

        let compactor = Compactor::new(self.summarizer.clone(), config);
        // Compaction is best effort. A summary the model declined to write leaves the
        // conversation as it stands, which still goes through whenever the character
        // estimate was more pessimistic than the model's own accounting.
        let Ok(Some(compacted)) = compactor
            .compact_if_needed(&self.messages, self.context_window)
            .await
        else {
            return;
        };

        let summary = compacted.messages[0].clone();
        self.messages = compacted.messages;

        // The summary joins the conversation like any other message, so it is announced
        // like one; `micro_context::is_summary` tells a renderer to draw it as a
        // compaction marker rather than as something the user typed.
        events.send(AgentEvent::MessageStart {
            message: summary.clone(),
        });
        events.send(AgentEvent::MessageEnd { message: summary });
    }

    /// Issue one model request, forwarding stream events and retrying transient failures
    /// that happen before any content is shown.
    async fn stream_once(&self, events: &Fan<'_>) -> AssistantMessage {
        let context = Context {
            system_prompt: self.system_prompt.clone(),
            messages: self.messages.clone(),
            tools: self.tool_definitions(),
        };

        let mut attempt = 0;
        // Tracked across attempts: a retry continues the same assistant message, so a
        // second `MessageStart` would leave a consumer with two bubbles for one response.
        let mut started = false;

        loop {
            attempt += 1;
            let mut stream =
                self.provider
                    .stream(self.model.clone(), context.clone(), self.api_key.clone());

            let mut emitted_content = false;
            let mut outcome: Option<Result<AssistantMessage, String>> = None;

            while let Some(event) = stream.recv().await {
                match event {
                    StreamEvent::Done { message } => {
                        outcome = Some(Ok(message));
                        break;
                    }
                    StreamEvent::Error { message } => {
                        outcome = Some(Err(message));
                        break;
                    }
                    other => {
                        if !started {
                            started = true;
                            events.send(AgentEvent::MessageStart {
                                message: Message::Assistant(
                                    self.empty_assistant(StopReason::Stop, None),
                                ),
                            });
                        }
                        if matches!(
                            other,
                            StreamEvent::TextDelta { .. } | StreamEvent::ThinkingDelta { .. }
                        ) {
                            emitted_content = true;
                        }
                        events.send(AgentEvent::MessageDelta { event: other });
                    }
                }
            }

            let result = outcome
                .unwrap_or_else(|| Err("provider closed the stream without a result".to_string()));

            match result {
                Ok(message) => {
                    events.send(AgentEvent::MessageEnd {
                        message: Message::Assistant(message.clone()),
                    });
                    return message;
                }
                Err(error) => {
                    let retryable =
                        !emitted_content && attempt < MAX_ATTEMPTS && is_retryable(&error);

                    if retryable {
                        let delay_ms = retry_delay_ms(attempt);
                        events.send(AgentEvent::Retry {
                            attempt,
                            max_attempts: MAX_ATTEMPTS,
                            delay_ms,
                        });
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }

                    let message = self.empty_assistant(StopReason::Error, Some(error));
                    events.send(AgentEvent::MessageEnd {
                        message: Message::Assistant(message.clone()),
                    });
                    return message;
                }
            }
        }
    }

    fn empty_assistant(&self, stop_reason: StopReason, error: Option<String>) -> AssistantMessage {
        AssistantMessage {
            content: Vec::new(),
            provider: self.model.provider.clone(),
            model: self.model.id.clone(),
            usage: Usage::default(),
            stop_reason,
            error,
            timestamp: now_ms(),
        }
    }
}

/// Accumulate streamed deltas into the text shown so far. Consumers that render a
/// response as it arrives keep one of these per assistant message.
#[derive(Debug, Default, Clone)]
pub struct PartialResponse {
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<(String, String)>,
}

impl PartialResponse {
    pub fn apply(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::TextDelta { delta, .. } => self.text.push_str(delta),
            StreamEvent::ThinkingDelta { delta, .. } => self.thinking.push_str(delta),
            StreamEvent::ToolCallStart { id, name, .. } => {
                self.tool_calls.push((id.clone(), name.clone()))
            }
            _ => {}
        }
    }

    pub fn blocks(&self) -> Vec<ContentBlock> {
        let mut blocks = Vec::new();
        if !self.thinking.is_empty() {
            blocks.push(ContentBlock::Thinking {
                thinking: self.thinking.clone(),
                signature: None,
            });
        }
        if !self.text.is_empty() {
            blocks.push(ContentBlock::text(&self.text));
        }
        blocks
    }
}

/// What an abandoned tool call is told, so the model understands why it has no output.
const ABANDONED_CALL: &str =
    "This tool call was interrupted and never ran, so it produced no result. Request it \
     again if the work still needs doing.";

/// Give every tool call that was never answered a failed result, in place.
///
/// A provider requires each tool call in an assistant message to be answered before the
/// conversation moves on, so an unanswered one poisons every later request. The results a
/// call already has sit immediately after it; anything missing is inserted at the end of
/// that group, preserving the order a provider expects. Returns the results it created, in
/// the order they were inserted, so a caller can persist and announce them.
fn answer_abandoned_calls(messages: &mut Vec<Message>) -> Vec<Message> {
    let mut repairs = Vec::new();
    let mut index = 0;

    while index < messages.len() {
        let calls: Vec<(String, String)> = match &messages[index] {
            Message::Assistant(assistant) => assistant
                .tool_calls()
                .into_iter()
                .map(|(id, name, _)| (id.to_string(), name.to_string()))
                .collect(),
            _ => Vec::new(),
        };

        if calls.is_empty() {
            index += 1;
            continue;
        }

        // The results answering this message run consecutively after it.
        let mut end = index + 1;
        let mut answered = Vec::new();
        while let Some(Message::ToolResult { tool_call_id, .. }) = messages.get(end) {
            answered.push(tool_call_id.clone());
            end += 1;
        }

        let missing: Vec<Message> = calls
            .into_iter()
            .filter(|(id, _)| !answered.contains(id))
            .map(|(id, name)| Message::tool_result(id, name, ABANDONED_CALL, true))
            .collect();

        let inserted = missing.len();
        for (offset, repair) in missing.into_iter().enumerate() {
            messages.insert(end + offset, repair.clone());
            repairs.push(repair);
        }
        index = end + inserted;
    }

    repairs
}

/// True when an error message carries a transient HTTP status.
fn is_retryable(error: &str) -> bool {
    let Some(rest) = error.split("returned ").nth(1) else {
        return false;
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse::<u16>()
        .map(|status| RETRYABLE_STATUSES.contains(&status))
        .unwrap_or(false)
}

/// Exponential backoff: 1s, 2s, 4s, … capped at 30s.
fn retry_delay_ms(attempt: u32) -> u64 {
    (1000u64 << (attempt.saturating_sub(1)).min(20)).min(MAX_RETRY_DELAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_calling(calls: &[(&str, &str)]) -> Message {
        Message::Assistant(AssistantMessage {
            content: calls
                .iter()
                .map(|(id, name)| ContentBlock::ToolCall {
                    id: (*id).into(),
                    name: (*name).into(),
                    arguments: serde_json::Value::Null,

                    signature: None,
                })
                .collect(),
            provider: "test".into(),
            model: "test".into(),
            usage: Usage::default(),
            stop_reason: StopReason::ToolUse,
            error: None,
            timestamp: 0,
        })
    }

    fn answered_ids(messages: &[Message]) -> Vec<&str> {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_abandoned_call_is_answered_so_the_conversation_stays_valid() {
        let mut messages = vec![Message::user("go"), assistant_calling(&[("a", "bash")])];

        let repairs = answer_abandoned_calls(&mut messages);

        assert_eq!(repairs.len(), 1);
        assert_eq!(answered_ids(&messages), vec!["a"]);
        assert!(matches!(
            messages.last(),
            Some(Message::ToolResult { is_error: true, .. })
        ));
    }

    #[test]
    fn a_partly_answered_turn_only_gains_the_missing_results() {
        let mut messages = vec![
            Message::user("go"),
            assistant_calling(&[("a", "read"), ("b", "bash")]),
            Message::tool_result("a", "read", "contents", false),
        ];

        let repairs = answer_abandoned_calls(&mut messages);

        assert_eq!(repairs.len(), 1);
        assert_eq!(answered_ids(&messages), vec!["a", "b"]);
    }

    #[test]
    fn a_fully_answered_conversation_is_left_alone() {
        let mut messages = vec![
            Message::user("go"),
            assistant_calling(&[("a", "read")]),
            Message::tool_result("a", "read", "contents", false),
        ];
        let before = messages.clone();

        assert!(answer_abandoned_calls(&mut messages).is_empty());
        assert_eq!(messages, before);
    }

    #[test]
    fn results_are_inserted_next_to_the_call_they_answer() {
        let mut messages = vec![
            assistant_calling(&[("a", "bash")]),
            Message::user("never mind, do this instead"),
            assistant_calling(&[("b", "read")]),
            Message::tool_result("b", "read", "contents", false),
        ];

        answer_abandoned_calls(&mut messages);

        // The repair belongs directly after the call it answers, not at the end.
        assert!(matches!(messages[1], Message::ToolResult { .. }));
        assert_eq!(answered_ids(&messages), vec!["a", "b"]);
    }

    #[test]
    fn repairing_twice_changes_nothing_the_second_time() {
        let mut messages = vec![Message::user("go"), assistant_calling(&[("a", "bash")])];

        answer_abandoned_calls(&mut messages);
        let once = messages.clone();

        assert!(answer_abandoned_calls(&mut messages).is_empty());
        assert_eq!(messages, once);
    }

    #[test]
    fn transient_statuses_are_retryable() {
        assert!(is_retryable("Anthropic returned 429: slow down"));
        assert!(is_retryable("Anthropic returned 503: overloaded"));
    }

    #[test]
    fn client_errors_and_unparsed_failures_are_not_retryable() {
        assert!(!is_retryable("Anthropic returned 400: bad request"));
        assert!(!is_retryable("Anthropic returned 401: no key"));
        assert!(!is_retryable("connection reset"));
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(retry_delay_ms(1), 1_000);
        assert_eq!(retry_delay_ms(2), 2_000);
        assert_eq!(retry_delay_ms(3), 4_000);
        assert_eq!(retry_delay_ms(20), MAX_RETRY_DELAY_MS);
    }

    #[test]
    fn a_recorded_message_joins_the_conversation_and_the_log() {
        let (recorder, mut written) = tokio::sync::mpsc::unbounded_channel();
        let mut agent = Agent::new(
            Arc::new(micro_provider::Anthropic::new()),
            Vec::new(),
            Model::anthropic("claude-opus-5"),
            "key",
        )
        .with_recorder(recorder);

        agent.record(Message::user("<bash command=\"ls\">a.txt</bash>"));

        assert_eq!(agent.messages().len(), 1);
        assert_eq!(written.try_recv().unwrap(), agent.messages()[0]);
    }

    #[test]
    fn partial_response_accumulates_deltas_in_order() {
        let mut partial = PartialResponse::default();
        partial.apply(&StreamEvent::TextDelta {
            index: 0,
            delta: "he".into(),
        });
        partial.apply(&StreamEvent::TextDelta {
            index: 0,
            delta: "llo".into(),
        });
        partial.apply(&StreamEvent::ToolCallStart {
            index: 1,
            id: "a".into(),
            name: "read".into(),
        });

        assert_eq!(partial.text, "hello");
        assert_eq!(partial.tool_calls, vec![("a".into(), "read".into())]);
    }
}

/// Why a conversation was not summarized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionRefusal {
    /// There is not enough behind the recent messages to be worth giving up.
    TooSmall,
    /// The model would not write a summary.
    Failed(String),
}

impl fmt::Display for CompactionRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompactionRefusal::TooSmall => {
                formatter.write_str("Nothing to compact (session too small)")
            }
            CompactionRefusal::Failed(message) => write!(formatter, "Compaction failed: {message}"),
        }
    }
}

/// Where an event goes: to whoever asked for the turn, and to anything watching.
///
/// A watcher that has gone away is not an error — the turn is the caller's, and it carries
/// on whether or not anybody else is still listening.
struct Fan<'a> {
    primary: &'a UnboundedSender<AgentEvent>,
    observer: Option<UnboundedSender<AgentEvent>>,
}

impl Fan<'_> {
    fn send(&self, event: AgentEvent) {
        if let Some(observer) = &self.observer {
            let _ = observer.send(event.clone());
        }
        let _ = self.primary.send(event);
    }
}

/// Something allowed to change what a tool call does.
#[async_trait::async_trait]
pub trait ToolHooks: Send + Sync {
    /// Called before a tool runs. `Some(reason)` refuses the call, and the reason is what
    /// the model is told instead of the tool's output.
    async fn before_tool(&self, id: &str, name: &str, arguments: &Value) -> Option<String>;

    /// Called once a tool has answered, before the model reads it. Returns the output and
    /// whether it should be read as a failure.
    async fn after_tool(
        &self,
        id: &str,
        name: &str,
        output: String,
        is_error: bool,
    ) -> (String, bool);
}
