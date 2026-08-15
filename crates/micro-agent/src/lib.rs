//! The agent loop: stream a response, run the tools it asks for, repeat until the
//! model stops asking for tools.

mod summarizer;

pub use summarizer::ProviderSummarizer;

use micro_context::CompactionConfig;
use micro_context::Compactor;
use micro_context::Summarizer;
use micro_provider::ApiKey;
use micro_provider::Provider;
use micro_tools::Tool;
use micro_types::content_hash;
use micro_types::now_ms;
use micro_types::AgentEvent;
use micro_types::AssistantMessage;
use micro_types::ContentBlock;
use micro_types::Context;
use micro_types::EventSource;
use micro_types::LedgerEvent;
use micro_types::Message;
use micro_types::Model;
use micro_types::PrefixSpan;
use micro_types::StopReason;
use micro_types::StreamEvent;
use micro_types::ThinkingLevel;
use micro_types::ToolDefinition;
use micro_types::ToolExecutionMode;
use micro_types::Usage;
use serde_json::Value;
use std::collections::HashSet;
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
    pub api_key: ApiKey,
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

/// Something a run produced that belongs in the session log.
///
/// Almost everything the model says is a message. Compaction is not: it does not add to
/// the conversation, it changes where the conversation is read from, and a session that
/// recorded only messages would summarize the same stretch again every time it reopened.
/// Everything else a run does — what it asked a provider for, what it was billed, what it
/// was not allowed to do — is a ledger event, which is a fact about the run rather than
/// part of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Message(Message),
    /// A stretch replaced by a summary, with how many of the most recent messages are
    /// still part of the conversation.
    Compacted {
        summary: String,
        kept: usize,
    },
    /// A fact about the run, and the content it names by hash.
    ///
    /// An event refers to a system prompt or a set of tool definitions rather than
    /// carrying one, so a long prompt is not written into the log once a turn. The content
    /// travels with the event the first time that hash is used and never again; the
    /// session files it under the same name and every later event points at it.
    Event {
        event: LedgerEvent,
        blobs: Vec<(String, Vec<u8>)>,
    },
}

/// A way to reach a run that is already going.
///
/// A run holds the agent, so nothing else can call into it while it lasts. This is what
/// a caller keeps hold of instead: messages left here are picked up by the loop at the
/// next point it can take them, without interrupting what it is doing.
#[derive(Clone, Default)]
pub struct Steering {
    queues: Arc<std::sync::Mutex<Queues>>,
}

#[derive(Default)]
struct Queues {
    /// Taken at the start of the next turn, so it reaches the model as soon as the one
    /// in flight is done rather than after everything the model asks for.
    steering: Vec<Message>,
    /// Taken when the run would otherwise end, which continues it rather than starting
    /// a second one.
    follow_up: Vec<Message>,
}

impl Steering {
    /// Say something to the model at the next turn boundary.
    pub fn steer(&self, message: Message) {
        self.lock().steering.push(message);
    }

    /// Say something once the run would otherwise be over.
    pub fn follow_up(&self, message: Message) {
        self.lock().follow_up.push(message);
    }

    /// Whether anything is waiting, for a caller deciding whether to start a new run.
    pub fn is_empty(&self) -> bool {
        self.waiting() == 0
    }

    /// How many messages are waiting to be said.
    pub fn waiting(&self) -> usize {
        let held = self.lock();
        held.steering.len() + held.follow_up.len()
    }

    /// Forget everything waiting, for a run that was abandoned: what was queued behind
    /// it was queued behind the thing that is now gone.
    pub fn take_all(&self) -> Vec<Message> {
        let mut held = self.lock();
        let mut all = std::mem::take(&mut held.steering);
        all.extend(std::mem::take(&mut held.follow_up));
        all
    }

    fn take_steering(&self) -> Vec<Message> {
        std::mem::take(&mut self.lock().steering)
    }

    fn take_follow_up(&self) -> Vec<Message> {
        std::mem::take(&mut self.lock().follow_up)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Queues> {
        self.queues
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Vec<Arc<dyn Tool>>,
    model: Model,
    api_key: ApiKey,
    system_prompt: Option<String>,
    messages: Vec<Message>,
    recorder: Option<UnboundedSender<Record>>,
    /// Anything else watching the events this run produces.
    observer: Option<UnboundedSender<AgentEvent>>,
    /// Anything allowed to change what the run does.
    hooks: Option<Arc<dyn Hooks>>,
    /// What this conversation is called, for a provider that caches against it.
    cache_key: Option<String>,
    summarizer: Arc<dyn Summarizer>,
    compaction: Option<CompactionConfig>,
    context_window: usize,
    /// What has been said to the run while it was running.
    steering: Steering,
    /// Which tools the model is told about, when something has narrowed them.
    ///
    /// An extension may choose a subset for the turns that follow, so this is read each
    /// time the model is told what exists rather than settled when the agent is built.
    /// `None` inside means nobody has narrowed anything and every tool is offered.
    offered: Option<Arc<std::sync::RwLock<Option<Vec<String>>>>>,
    /// How many requests this agent has issued, which is what numbers a turn in the
    /// ledger. Counted here because this is the only thing that issues one.
    turn: u64,
    /// Where each stretch of the system prompt came from, as whoever assembled it said.
    ///
    /// The agent is handed a prompt as one string and cannot tell a project's
    /// instructions from a skill's description by looking at it, so what it is told about
    /// the parts is what it records.
    prefix_spans: Vec<PrefixSpan>,
    /// Content already handed to the recorder, by hash. A system prompt that stands
    /// unchanged for a hundred turns crosses this channel once.
    stored_blobs: HashSet<String>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Vec<Arc<dyn Tool>>,
        model: Model,
        api_key: impl Into<ApiKey>,
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
            cache_key: None,
            summarizer,
            compaction: Some(CompactionConfig::default()),
            context_window: DEFAULT_CONTEXT_WINDOW,
            steering: Steering::default(),
            offered: None,
            turn: 0,
            prefix_spans: Vec::new(),
            stored_blobs: HashSet::new(),
        }
    }

    /// A handle onto this agent's runs, for saying something to one while it lasts.
    ///
    /// Taken before the run starts, since the run borrows the agent for as long as it
    /// goes on.
    pub fn steering(&self) -> Steering {
        self.steering.clone()
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
    pub fn with_recorder(mut self, recorder: UnboundedSender<Record>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Name the conversation, so a provider that caches a prompt can recognise it again.
    pub fn with_cache_key(mut self, key: impl Into<String>) -> Self {
        self.cache_key = Some(key.into());
        self
    }

    /// Say what the system prompt was assembled from, so every request the agent records
    /// can be attributed span by span rather than only as one block of text.
    pub fn with_prefix_spans(mut self, spans: Vec<PrefixSpan>) -> Self {
        self.prefix_spans = spans;
        self
    }

    /// Read which tools to tell the model about from `offered`, rather than telling it
    /// about all of them.
    ///
    /// Shared rather than given once, because whoever narrows the list does it while the
    /// run is already built — an extension choosing the tools for the turns that follow.
    pub fn with_offered_tools(
        mut self,
        offered: Arc<std::sync::RwLock<Option<Vec<String>>>>,
    ) -> Self {
        self.offered = Some(offered);
        self
    }

    /// Let something decide what the run may do.
    ///
    /// The one place anything outside the agent is allowed to change what happens rather
    /// than watch it happen.
    pub fn with_hooks(mut self, hooks: Arc<dyn Hooks>) -> Self {
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
    /// The summary is returned so a caller can show it. Every message stays in the log
    /// verbatim; what is recorded alongside them is where the conversation now starts
    /// reading from, so reopening the session costs no summarizing.
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
        self.record_compaction(&compacted.messages);
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
            let _ = recorder.send(Record::Message(message.clone()));
        }
        self.messages.push(message);
    }

    /// What anything watching the run has decided about this call.
    async fn decide(&self, id: &str, name: &str, arguments: &Value) -> ToolDecision {
        match &self.hooks {
            Some(hooks) => hooks.before_tool(id, name, arguments).await,
            None => ToolDecision::Proceed,
        }
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

    /// Carry a prepared call to its tool result: what preflight already settled, or a
    /// tool's own run, rewritten by anything watching before the model sees it.
    ///
    /// Shared by both ways a batch is scheduled — one call at a time, or every runnable
    /// call together — so a result is put together identically either way; only when this
    /// runs relative to the rest of the batch differs.
    async fn finish_call(
        &self,
        id: String,
        name: String,
        arguments: Value,
        settled: Option<(String, bool)>,
        runnable: Option<Arc<dyn Tool>>,
        events: &Fan<'_>,
    ) -> Message {
        let (content, is_error) = match (settled, runnable) {
            (Some((text, is_error)), _) => (vec![ContentBlock::text(text)], is_error),
            (None, Some(tool)) => run_tool(tool, &id, &name, &arguments, events).await,
            // Neither answered nor runnable cannot happen: the preflight sets one or the
            // other for every call.
            (None, None) => (
                vec![ContentBlock::text(format!("tool not found: {name}"))],
                true,
            ),
        };

        // What ran can be rewritten before the model sees it, which is how a result is
        // redacted or replaced by something watching. Only the text is offered: a rewrite
        // is a decision about what the model should be told, so anything else the tool
        // returned goes with it.
        let said: String = content.iter().map(ContentBlock::as_text).collect();
        let (output, is_error) = self.rewritten(&id, &name, said.clone(), is_error).await;
        let content = match output == said {
            true => content,
            false => vec![ContentBlock::text(output.clone())],
        };

        // Reported the moment this call is done rather than when the whole batch is, so a
        // quick tool is not held behind a slow one.
        events.send(AgentEvent::ToolEnd {
            id: id.clone(),
            name: name.clone(),
            output,
            is_error,
        });

        Message::tool_result_content(id, name, content, is_error)
    }

    /// Both places an event goes, as one thing to send to.
    fn fan<'a>(&self, events: &'a UnboundedSender<AgentEvent>) -> Fan<'a> {
        Fan {
            primary: events,
            observer: self.observer.clone(),
        }
    }

    /// Append to the conversation, reporting the message to the recorder if one is set.
    /// Record that a stretch was summarized, so reopening the session reads the summary
    /// rather than paying to write it again.
    fn record_compaction(&self, messages: &[Message]) {
        let Some(recorder) = &self.recorder else {
            return;
        };
        let Some(summary) = messages.first().and_then(micro_context::summary_text) else {
            return;
        };
        let _ = recorder.send(Record::Compacted {
            summary: summary.to_string(),
            // Everything after the summary is what was kept.
            kept: messages.len().saturating_sub(1),
        });
    }

    /// Record a fact about the run that refers to nothing outside itself.
    fn record_event(&self, event: LedgerEvent) {
        if let Some(recorder) = &self.recorder {
            let _ = recorder.send(Record::Event {
                event,
                blobs: Vec::new(),
            });
        }
    }

    /// Record the request about to go out: what identifies it, and what it was built from.
    ///
    /// The body itself is not recorded — it is the hash that identifies it, and the
    /// prompt, the tools and the model that rebuild it. Those three are named by hash and
    /// carried along the first time each is seen, which is what keeps a hundred turns of
    /// the same prompt to one copy on disk.
    fn record_request(&mut self, context: &Context, attempt: u32) {
        if self.recorder.is_none() {
            return;
        }

        let prompt = context.system_prompt.clone().unwrap_or_default();
        let tools = serde_json::to_vec(&context.tools).unwrap_or_default();
        let described = serde_json::to_vec(&self.model).unwrap_or_default();
        let body = serde_json::to_vec(&self.provider.payload(&self.model, context))
            .unwrap_or_default();

        // The prefix is the part of a request a provider can cache: what the model is told
        // before the conversation, and the tools it may call. Hashing them together is
        // what makes two turns comparable at a glance — the same prefix hash means the
        // same cacheable head, and a different one is a cache miss waiting to happen.
        let mut prefix = prompt.into_bytes();
        prefix.extend_from_slice(&tools);

        let mut blobs = Vec::new();
        let system_prompt_blob = context
            .system_prompt
            .as_ref()
            .map(|prompt| self.blob(&mut blobs, prompt.as_bytes()));
        let tools_blob = self.blob(&mut blobs, &tools);
        let model_blob = self.blob(&mut blobs, &described);

        let event = LedgerEvent::TurnRequest {
            turn: self.turn,
            provider: self.model.provider.clone(),
            model: self.model.id.clone(),
            prefix_hash: content_hash(&prefix),
            request_hash: content_hash(&body),
            system_prompt_blob,
            tools_blob,
            model_blob,
            prefix_spans: self.prefix_spans.clone(),
            // Named by the session as it writes this, which is the only place the entries
            // the conversation stands at have names.
            message_entry_ids: Vec::new(),
            attempt,
        };
        if let Some(recorder) = &self.recorder {
            let _ = recorder.send(Record::Event { event, blobs });
        }
    }

    /// Name a piece of content by the hash of its bytes, carrying the content itself along
    /// the first time that name is used.
    fn blob(&mut self, carried: &mut Vec<(String, Vec<u8>)>, content: &[u8]) -> String {
        let hash = content_hash(content);
        if self.stored_blobs.insert(hash.clone()) {
            carried.push((hash.clone(), content.to_vec()));
        }
        hash
    }

    fn commit(&mut self, message: Message, produced: &mut Vec<Message>) {
        if let Some(recorder) = &self.recorder {
            let _ = recorder.send(Record::Message(message.clone()));
        }
        self.messages.push(message.clone());
        produced.push(message);
    }

    /// The tools the model is told about.
    ///
    /// A deferred tool is left out of this and still found by [`Agent::find_tool`], so the
    /// model can call one it learned of by searching rather than by being told up front.
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        // Narrowing says which tools the model hears about, not which ones exist: a name
        // that is not offered is still found by `find_tool`, the same way a deferred tool
        // is, so a call already in flight when the list changed still runs.
        let offered = self.offered.as_ref().and_then(|offered| {
            offered
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        });
        self.tools
            .iter()
            .filter(|tool| !tool.deferred())
            .map(|tool| tool.definition())
            .filter(|definition| match &offered {
                Some(names) => names.iter().any(|name| name == &definition.name),
                None => true,
            })
            .collect()
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
        // Armed for the rest of this call, and disarmed only once `AgentEnd`/
        // `AgentSettled` are actually sent below. A turn abandoned before then — Ctrl+C,
        // or a caller that simply stops polling — drops this future along with everything
        // it owns, this guard included, and a `Drop` runs on that the same as it would on
        // a normal return. See its own doc comment for why that is what a caller watching
        // from outside this run needs.
        let mut settle = SettleGuard::armed(events.clone_for_updates());
        let prompt = match &self.hooks {
            Some(hooks) => hooks.before_agent_start(&prompt).await.unwrap_or(prompt),
            None => prompt,
        };
        let mut produced = Vec::new();

        // A turn abandoned partway — Ctrl+C during a tool, or a crash — leaves an assistant
        // message whose tool calls were never answered. Providers reject a request that
        // contains one, so the conversation would stay unusable for the rest of its life and
        // its log would be unresumable. Answering the abandoned calls first makes the
        // conversation valid again, whether it was interrupted moments ago in this process
        // or a week ago in another one.
        for repair in answer_abandoned_calls(&mut self.messages) {
            if let Some(recorder) = &self.recorder {
                let _ = recorder.send(Record::Message(repair.clone()));
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
            // Anything said while the last turn ran goes in before this one is sent, so
            // it reaches the model at the first moment it can rather than after
            // everything the model went on to ask for.
            for said in self.steering.take_steering() {
                events.send(AgentEvent::MessageStart {
                    message: said.clone(),
                });
                events.send(AgentEvent::MessageEnd {
                    message: said.clone(),
                });
                self.commit(said, &mut produced);
            }

            events.send(AgentEvent::TurnStart);
            self.compact_if_needed(events).await;

            let assistant = self.stream_once(events).await;
            if let Some(hooks) = &self.hooks {
                hooks.after_response(&assistant).await;
            }
            self.commit(Message::Assistant(assistant.clone()), &mut produced);

            if matches!(
                assistant.stop_reason,
                StopReason::Error | StopReason::Aborted
            ) {
                // A turn that failed is still a turn that ended. Anything pairing a start
                // with an end would be left waiting for one that never came.
                events.send(AgentEvent::TurnEnd {
                    messages: produced.clone(),
                });
                break;
            }

            let calls: Vec<(String, String, serde_json::Value)> = assistant
                .tool_calls()
                .into_iter()
                .map(|(id, name, arguments)| (id.to_string(), name.to_string(), arguments.clone()))
                .collect();

            if calls.is_empty() {
                // The exchange is over whether or not the model asked for anything, so it
                // is closed on the way out as well as at the end of a round of tools.
                events.send(AgentEvent::TurnEnd {
                    messages: produced.clone(),
                });

                // Something queued for after the answer continues this run rather than
                // starting another, which is what keeps it one conversation.
                let queued = self.steering.take_follow_up();
                if queued.is_empty() {
                    break;
                }
                for said in queued {
                    events.send(AgentEvent::MessageStart {
                        message: said.clone(),
                    });
                    events.send(AgentEvent::MessageEnd {
                        message: said.clone(),
                    });
                    self.commit(said, &mut produced);
                }
                continue;
            }

            // A response cut off by the token limit can yield tool calls whose streamed
            // arguments happen to parse but are silently incomplete. None are safe to run.
            let truncated = assistant.stop_reason == StopReason::Length;

            // A tool asking to run alone forces the whole batch to run one call at a time:
            // there would be nothing for "alone" to mean if everything else in the same
            // turn kept going around it. Looked up by name against the registered tools
            // rather than against what preflight below decides, so a call that ends up
            // refused or unresolved still counts — the model asked for that tool by name,
            // and that is what the batch is scheduled around.
            let sequential = calls.iter().any(|(_, name, _)| {
                self.find_tool(name)
                    .is_some_and(|tool| tool.execution_mode() == Some(ToolExecutionMode::Sequential))
            });

            // Every call is vetted and then announced, in the order the model asked for it,
            // so whatever is watching sees a stable order and what it is shown is the call
            // that actually runs.
            let mut prepared = Vec::with_capacity(calls.len());
            for (id, name, arguments) in calls {
                // Either the call is already answered — refused, or never runnable — or a
                // tool is held ready to run it.
                let mut arguments = arguments;
                let mut settled = None;
                let mut runnable = None;
                if truncated {
                    settled = Some((
                        format!(
                            "Tool call \"{name}\" was not executed: the response hit the output \
                             token limit, so its arguments may be truncated. Re-issue the call \
                             with complete arguments."
                        ),
                        true,
                    ));
                } else {
                    match self.decide(&id, &name, &arguments).await {
                        // Something watching the run refused the call. The model is told
                        // why, in the same shape a tool's own failure takes — and the
                        // ledger says it was a refusal, which a failed result cannot.
                        ToolDecision::Refuse(reason) => {
                            self.record_event(LedgerEvent::ToolDenied {
                                tool: name.clone(),
                                reason: reason.clone(),
                                // The agent knows the decision came from what is watching
                                // the run, and watching is what an extension does here.
                                source: EventSource::Extension(String::new()),
                            });
                            settled = Some((reason, true));
                        }
                        decision => {
                            // Rewritten arguments are the ones that run, so they are also
                            // the ones announced, recorded, and handed to the tool.
                            if let ToolDecision::Rewrite(replacement) = decision {
                                arguments = replacement;
                            }
                            match self.find_tool(&name) {
                                Some(tool) => runnable = Some(Arc::clone(tool)),
                                None => settled = Some((format!("tool not found: {name}"), true)),
                            }
                        }
                    }
                }

                events.send(AgentEvent::ToolStart {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                });

                prepared.push((id, name, arguments, settled, runnable));
            }

            if sequential {
                // A tool in this batch must run alone, so every call in it does: each is
                // executed, rewritten, reported and committed before the next one starts,
                // rather than the batch racing to whichever finishes first.
                for (id, name, arguments, settled, runnable) in prepared {
                    let result = self
                        .finish_call(id, name, arguments, settled, runnable, events)
                        .await;
                    events.send(AgentEvent::MessageStart {
                        message: result.clone(),
                    });
                    events.send(AgentEvent::MessageEnd {
                        message: result.clone(),
                    });
                    self.commit(result, &mut produced);
                }
            } else {
                // The calls the model asked for in one answer do not depend on each other,
                // so they run together: a turn asking for several files takes as long as
                // the slowest read rather than the sum of all of them.
                // Shared for the length of the batch: every call reads the same hooks, and
                // nothing is committed until they have all answered.
                let agent = &*self;
                let ran = futures::future::join_all(prepared.into_iter().map(
                    |(id, name, arguments, settled, runnable)| {
                        agent.finish_call(id, name, arguments, settled, runnable, events)
                    },
                ))
                .await;

                // Committed in the order the model asked, whatever order they finished
                // in, so the conversation reads the same every time.
                for result in ran {
                    events.send(AgentEvent::MessageStart {
                        message: result.clone(),
                    });
                    events.send(AgentEvent::MessageEnd {
                        message: result.clone(),
                    });
                    self.commit(result, &mut produced);
                }
            }

            // One exchange is over: the model answered and every tool it asked for has
            // run. Another begins only if it asked for more.
            events.send(AgentEvent::TurnEnd {
                messages: produced.clone(),
            });
        }

        events.send(AgentEvent::AgentEnd {
            messages: produced.clone(),
        });
        // Nothing is left to do, which is a different thing from the run being over: a
        // caller with more queued starts another run without ever settling.
        events.send(AgentEvent::AgentSettled);
        settle.disarm();
        produced
    }

    /// Replace the older part of the conversation with a summary once it approaches the
    /// context window.
    ///
    /// Nothing is deleted: the log keeps every message verbatim. Recorded alongside them
    /// is where the conversation now starts reading from, which makes compaction a fact
    /// about the session rather than about the process that happened to do it — a session
    /// reopened later reads the summary instead of writing the same one again.
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
        self.record_compaction(&compacted.messages);
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
    ///
    /// Every request the agent makes is assembled here and nowhere else, which is what
    /// makes it the one place a turn can be recorded completely: after anything watching
    /// the run has had its say about the context, and before the provider is handed it.
    async fn stream_once(&mut self, events: &Fan<'_>) -> AssistantMessage {
        let context = Context {
            system_prompt: self.system_prompt.clone(),
            messages: self.messages.clone(),
            tools: self.tool_definitions(),
            headers: Vec::new(),
            cache_key: self.cache_key.clone(),
        };
        // Whatever is watching gets the conversation before the provider does, and may
        // change it: this is where a summary is swapped in, or a file is added.
        let context = match &self.hooks {
            Some(hooks) => hooks.before_request(context).await,
            None => context,
        };

        self.turn += 1;
        let turn = self.turn;

        let mut attempt = 0;
        // Tracked across attempts: a retry continues the same assistant message, so a
        // second `MessageStart` would leave a consumer with two bubbles for one response.
        let mut started = false;

        loop {
            attempt += 1;
            // Recorded once per attempt rather than once per turn: a request that was
            // issued is a request that was issued, whether or not the one before it came
            // back, and a reader counting what a session cost has to see both.
            self.record_request(&context, attempt);
            // Asked for once per attempt rather than held: a credential that expires is
            // exchanged by whoever owns it, and a request made an hour into a session
            // must carry the token that is current then, not the one it started with.
            let api_key = self.api_key.current().await;
            let mut stream = self
                .provider
                .stream(self.model.clone(), context.clone(), api_key);

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
                    // What the provider says the turn cost, as it says it. Recorded as its
                    // own fact rather than left inside the answer, so a session that is
                    // later summarized still knows what it was billed for.
                    self.record_event(LedgerEvent::TurnUsage {
                        turn,
                        usage: message.usage,
                        stop_reason: message.stop_reason,
                        provider: message.provider.clone(),
                        model: message.model.clone(),
                    });
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
        assert_eq!(
            written.try_recv().unwrap(),
            Record::Message(agent.messages()[0].clone())
        );
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

/// Run one tool, forwarding what it says while it works.
///
/// A long command is watchable rather than silent: whatever it prints is reported as it
/// prints it, and the reporting ends when the call does.
async fn run_tool(
    tool: Arc<dyn Tool>,
    id: &str,
    name: &str,
    arguments: &Value,
    events: &Fan<'_>,
) -> (Vec<ContentBlock>, bool) {
    let (reporting, mut reported) = tokio::sync::mpsc::unbounded_channel::<String>();
    let forwarding = {
        let events = events.clone_for_updates();
        let id = id.to_string();
        let name = name.to_string();
        tokio::spawn(async move {
            while let Some(output) = reported.recv().await {
                events.send(AgentEvent::ToolUpdate {
                    id: id.clone(),
                    name: name.clone(),
                    output,
                });
            }
        })
    };

    let ran = tool
        .execute_content(arguments, &micro_tools::Progress::new(reporting))
        .await;
    // The sender is gone with the call, so the forwarder ends.
    let _ = forwarding.await;

    match ran {
        Ok(content) => (content, false),
        Err(error) => (vec![ContentBlock::text(error)], true),
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
    /// A sender that can be moved into a task, for events produced while a tool runs.
    fn clone_for_updates(&self) -> Updates {
        Updates {
            primary: self.primary.clone(),
            observer: self.observer.clone(),
        }
    }

    fn send(&self, event: AgentEvent) {
        if let Some(observer) = &self.observer {
            let _ = observer.send(event.clone());
        }
        let _ = self.primary.send(event);
    }
}

/// What something watching the run decided about a tool call.
///
/// Rewriting rather than only refusing is what lets a hook fix a call instead of ending
/// it: a path made absolute, a flag added, a secret taken out of a command before it runs.
/// The model is never told the call changed, because the call it asked for is the one it
/// meant; what changed is how it was carried out.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolDecision {
    /// Run the call as the model wrote it.
    Proceed,
    /// Run it with these arguments in place of the ones the model wrote.
    Rewrite(Value),
    /// Do not run it. The reason takes the place of the output the tool would have given.
    Refuse(String),
}

/// Something allowed to change what a run does, rather than only to watch it.
///
/// Every point sits between two things the agent would otherwise do directly: between the
/// model asking for a tool and the tool running, between the conversation being assembled
/// and being sent, between an answer arriving and being read. Each has a default that
/// changes nothing, so an implementation takes only the ones it cares about.
#[async_trait::async_trait]
pub trait Hooks: Send + Sync {
    /// Called before a tool runs, with the chance to change the call or refuse it.
    async fn before_tool(&self, id: &str, name: &str, arguments: &Value) -> ToolDecision {
        let _ = (id, name, arguments);
        ToolDecision::Proceed
    }

    /// Called once a tool has answered, before the model reads it. Returns the output and
    /// whether it should be read as a failure.
    async fn after_tool(
        &self,
        id: &str,
        name: &str,
        output: String,
        is_error: bool,
    ) -> (String, bool) {
        let _ = (id, name);
        (output, is_error)
    }

    /// Called with the prompt a run is about to start on. Returning a message replaces it.
    async fn before_agent_start(&self, prompt: &Message) -> Option<Message> {
        let _ = prompt;
        None
    }

    /// Called with everything about to be sent to the model, which may be changed.
    async fn before_request(&self, context: Context) -> Context {
        context
    }

    /// Called with the answer, once it is complete.
    async fn after_response(&self, message: &AssistantMessage) {
        let _ = message;
    }
}

/// The same two places an event goes, owned rather than borrowed, so a task that outlives
/// the call site can still report.
struct Updates {
    primary: UnboundedSender<AgentEvent>,
    observer: Option<UnboundedSender<AgentEvent>>,
}

impl Updates {
    fn send(&self, event: AgentEvent) {
        if let Some(observer) = &self.observer {
            let _ = observer.send(event.clone());
        }
        let _ = self.primary.send(event);
    }
}

/// Reports that a run ended even when `run()` never says so itself.
///
/// `run()` sends `AgentEnd` and `AgentSettled` as its last two lines, reached only by
/// returning normally. An interrupted turn does not return normally: the caller stops
/// polling the future and drops it, which is how micro-tui's `run_turn` and micro-rpc's
/// `turn()` both carry out an abort. Dropping a future drops its live locals exactly the
/// way leaving a block drops the locals declared in it, so a guard that is one of those
/// locals has its own `Drop` run on the same path — reported empty, since nothing here
/// reconstructs what the abandoned turn had produced by that point. A listener told
/// nothing at all could not tell an interrupted run from one still in progress; this way
/// it can.
struct SettleGuard {
    events: Updates,
    armed: bool,
}

impl SettleGuard {
    fn armed(events: Updates) -> Self {
        SettleGuard {
            events,
            armed: true,
        }
    }

    /// Say the run's own `AgentEnd`/`AgentSettled` already went out, so this reports
    /// nothing when it is dropped.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SettleGuard {
    fn drop(&mut self) {
        if self.armed {
            self.events.send(AgentEvent::AgentEnd {
                messages: Vec::new(),
            });
            self.events.send(AgentEvent::AgentSettled);
        }
    }
}
