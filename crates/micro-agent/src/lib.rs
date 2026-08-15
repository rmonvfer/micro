//! The agent loop: stream a response, run the tools it asks for, repeat until the
//! model stops asking for tools.

mod summarizer;

pub use summarizer::ProviderSummarizer;

use micro_context::Compacted;
use micro_context::CompactionConfig;
use micro_context::Compactor;
use micro_context::Summarizer;
use micro_models::ModelCost;
use micro_models::TokenUsage;
use micro_provider::ApiKey;
use micro_provider::Provider;
use micro_tools::Tool;
use micro_types::content_hash;
use micro_types::now_ms;
use micro_types::AgentEvent;
use micro_types::AssistantMessage;
use micro_types::CompactionCost;
use micro_types::ContentBlock;
use micro_types::Context;
use micro_types::EventSource;
use micro_types::LedgerEvent;
use micro_types::Message;
use micro_types::Model;
use micro_types::Prefix;
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
    /// What this model's tokens are charged at, so a budget carries on meaning the same
    /// thing after a swap: what a run has spent is one number, and the model it was spent
    /// on changes underneath it.
    pub cost: ModelCost,
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

/// What a session may spend, what it has spent, and what its tokens are charged at.
///
/// Held by the agent because the agent is what issues the requests, and checked after each
/// one rather than before: what a request will cost is not knowable until the provider says
/// what it read. A session that reopens over its limit is not bricked by that — it opens,
/// it answers once more, and it stops again — because a limit that made a session
/// unreadable would be a limit that lost the record of why it was reached.
#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    /// The ceiling for the whole session, in US dollars.
    pub limit: f64,
    /// What the session had already spent when this run opened it, which is what makes the
    /// ceiling the session's rather than each run's.
    pub spent: f64,
    /// What the model in use charges. Replaced by [`Agent::set_model`], since the same
    /// budget goes on applying across a swap.
    pub cost: ModelCost,
}

impl Budget {
    /// A ceiling on a session that has spent nothing yet.
    pub fn new(limit: f64, cost: ModelCost) -> Budget {
        Budget {
            limit,
            spent: 0.0,
            cost,
        }
    }

    /// What a session had already spent before this run opened it.
    pub fn already_spent(mut self, spent: f64) -> Budget {
        self.spent = spent;
        self
    }

    /// Whether what has been spent has reached what was allowed.
    fn reached(&self) -> bool {
        self.spent >= self.limit
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
    /// still part of the conversation, and what writing the summary cost.
    Compacted {
        summary: String,
        kept: usize,
        cost: CompactionCost,
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

/// A way to change what the model is told before the conversation, from outside the run.
///
/// The agent owns the prefix and nothing else may write it, because a prompt that changes
/// mid-request is a cache miss nobody recorded. What reaches here is a request for a
/// change: it is picked up at the next turn boundary, hashed, and written to the ledger
/// with the reason it was made. Between those boundaries the prefix stands still, which is
/// the whole point of it.
#[derive(Clone, Default)]
pub struct PrefixControl {
    asked: Arc<std::sync::Mutex<Asked>>,
}

#[derive(Default)]
struct Asked {
    /// What the model is told before the conversation — or is about to be, when a change
    /// is waiting below.
    prompt: String,
    spans: Vec<PrefixSpan>,
    /// Why the prompt above is not the one in force yet. `None` means nothing is waiting.
    reason: Option<String>,
}

impl PrefixControl {
    /// What the model is being told before the conversation, including a change that has
    /// been asked for and not yet taken effect.
    ///
    /// Whoever asked for that change has to read back what they asked for rather than what
    /// the last turn used, or every reader between the ask and the next turn would be told
    /// something that is already out of date.
    pub fn system_prompt(&self) -> String {
        self.lock().prompt.clone()
    }

    /// Tell the model this instead, from the next turn on, and say why.
    ///
    /// The reason is what a session is read back by: `reload`, `extension:deploy`. It ends
    /// up on the ledger event beside the two hashes, which is what turns "the cache missed"
    /// into "the cache missed because the project's instructions were re-read".
    pub fn change(
        &self,
        prompt: impl Into<String>,
        spans: Vec<PrefixSpan>,
        reason: impl Into<String>,
    ) {
        let mut asked = self.lock();
        asked.prompt = prompt.into();
        asked.spans = spans;
        asked.reason = Some(reason.into());
    }

    /// Say what the run was built with, so the first thing to read this is told what is
    /// actually in force rather than nothing at all.
    pub(crate) fn opened_with(&self, prompt: &str, spans: &[PrefixSpan]) {
        let mut asked = self.lock();
        asked.prompt = prompt.to_string();
        asked.spans = spans.to_vec();
    }

    /// The change that is waiting, if one is, taken so it is applied once.
    pub(crate) fn take(&self) -> Option<(String, Vec<PrefixSpan>, String)> {
        let mut asked = self.lock();
        let reason = asked.reason.take()?;
        Some((asked.prompt.clone(), asked.spans.clone(), reason))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Asked> {
        self.asked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Vec<Arc<dyn Tool>>,
    model: Model,
    api_key: ApiKey,
    /// What every request opens with, and what identifies it.
    ///
    /// Held as one hashed value rather than as a prompt and a tool list, so there is no way
    /// to change half of it without the hash moving and the change being recorded.
    prefix: Prefix,
    /// How anything outside the run asks for the prefix to change.
    prefix_control: PrefixControl,
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
    /// Results written to answer tool calls a conversation arrived with unanswered, waiting
    /// for a run to report them.
    ///
    /// The repair happens where the conversation is installed, which is before there is
    /// anything to report it to: no recorder is set yet on a fresh agent, and nobody is
    /// watching between runs.
    repairs: Vec<Message>,
    /// Content already handed to the recorder, by hash. A system prompt that stands
    /// unchanged for a hundred turns crosses this channel once.
    stored_blobs: HashSet<String>,
    /// What this session is allowed to spend, when anything limits it.
    budget: Option<Budget>,
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
            prefix: Prefix::default(),
            prefix_control: PrefixControl::default(),
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
            repairs: Vec::new(),
            stored_blobs: HashSet::new(),
            budget: None,
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
        let spans = self.prefix.spans().to_vec();
        self.prefix = self.prefix.with_system_prompt(prompt, spans);
        self.prefix_control.opened_with(
            self.prefix.system_prompt().unwrap_or_default(),
            self.prefix.spans(),
        );
        self
    }

    /// How anything outside the run asks for the prefix to change: `/reload` re-reading the
    /// project's instructions, an extension replacing the prompt.
    ///
    /// Taken before the run starts, the same way steering is, since a run borrows the agent
    /// for as long as it lasts.
    pub fn prefix_control(&self) -> PrefixControl {
        self.prefix_control.clone()
    }

    /// What every request this agent issues opens with.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
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
        // What has been spent stands; what the next turn will be charged at is the new
        // model's. A budget is a ceiling on the session, not on the model that was running
        // when it was set.
        if let Some(budget) = &mut self.budget {
            budget.cost = swap.cost;
        }
        self.summarizer = self.provider_summarizer();
    }

    /// A summarizer that asks whichever model the agent is running, on behalf of the
    /// conversation it is running: compaction is part of this session, not a request from
    /// nowhere, and a provider caching against a name has to be told the same name.
    fn provider_summarizer(&self) -> Arc<dyn Summarizer> {
        Arc::new(
            ProviderSummarizer::new(
                self.provider.clone(),
                self.model.clone(),
                self.api_key.clone(),
            )
            .for_conversation(self.cache_key.clone()),
        )
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
        self.summarizer = self.provider_summarizer();
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

    /// Stop this session once it has spent what `budget` allows.
    ///
    /// Nothing is refused before a request goes out — a request's price is not knowable
    /// until the provider reports what it read — so the ceiling is checked against what has
    /// been billed, and the run ends at the first turn boundary past it.
    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Name the conversation, so a provider that caches a prompt can recognise it again.
    ///
    /// The summarizer is rebuilt with it: its request is part of this conversation, and one
    /// sent unnamed lands wherever nothing is cached and pays for the transcript twice.
    pub fn with_cache_key(mut self, key: impl Into<String>) -> Self {
        self.cache_key = Some(key.into());
        self.summarizer = self.provider_summarizer();
        self
    }

    /// Say what the system prompt was assembled from, so every request the agent records
    /// can be attributed span by span rather than only as one block of text.
    pub fn with_prefix_spans(mut self, spans: Vec<PrefixSpan>) -> Self {
        self.prefix = Prefix::new(
            self.prefix.system_prompt().map(str::to_string),
            self.prefix.tools().to_vec(),
            spans,
        );
        self.prefix_control.opened_with(
            self.prefix.system_prompt().unwrap_or_default(),
            self.prefix.spans(),
        );
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

    /// Take tools away for good, for a run that outlived whoever provided them.
    ///
    /// Narrower than the offered list, which says only which tools the model is told about:
    /// a tool left out of that one is still there to be called, which is the point of it.
    /// A tool removed here is gone — its provider is no longer running, so a call to it
    /// could not be answered — and the model stops being told about it at the next turn,
    /// where the change is recorded like any other move of the cacheable prefix.
    pub fn remove_tools(&mut self, names: &[String]) {
        self.tools
            .retain(|tool| !names.contains(&tool.definition().name));
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
        self.set_messages(history);
        self
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
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
        self.record_compaction(&compacted);
        self.charge(compacted.cost.usage);
        self.messages = compacted.messages;
        Ok(summary)
    }

    /// Put the agent in a different conversation.
    ///
    /// Branching, resuming and clearing all change what has been said, not just what is on
    /// screen. Anything less than this leaves the model answering from messages the user
    /// can no longer see.
    ///
    /// A conversation that arrives from outside — read back from a log, or taken from an
    /// earlier point in the tree — can hold tool calls that were never answered, and a
    /// provider rejects a request carrying one. They are answered here, where the
    /// conversation arrives, and nowhere else: the repair inserts results in the middle of
    /// the history, and doing that between two turns of a live session would move bytes a
    /// provider had already cached, for no reason the session could name.
    pub fn set_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        // Replaced rather than added to: a repair belongs to the conversation it was made
        // in, and this one has just been put aside. Reporting it against what took its
        // place would describe a message that is not there.
        self.repairs = answer_abandoned_calls(&mut self.messages);
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
    /// rather than paying to write it again, and what writing it cost.
    fn record_compaction(&self, compacted: &Compacted) {
        let Some(recorder) = &self.recorder else {
            return;
        };
        let Some(summary) = compacted
            .messages
            .first()
            .and_then(micro_context::summary_text)
        else {
            return;
        };
        let _ = recorder.send(Record::Compacted {
            summary: summary.to_string(),
            // Everything after the summary is what was kept.
            kept: compacted.messages.len().saturating_sub(1),
            cost: compacted.cost.clone(),
        });
    }

    /// Take up whatever was asked of the prefix while the last turn ran.
    ///
    /// The one moment the head of a request is allowed to move. Everything that wants to
    /// change it — a reload, an extension, a narrowed tool list — has left its request
    /// somewhere for this, so a change lands between two turns rather than in the middle of
    /// one, and lands once with a reason attached instead of quietly on every request.
    fn settle_prefix(&mut self) {
        if let Some((prompt, spans, reason)) = self.prefix_control.take() {
            let asked = self.prefix.with_system_prompt(prompt, spans);
            self.adopt_prefix(asked, &reason);
        }
        // Narrowing is read here rather than each time the model is told what exists, so a
        // list changed mid-turn reaches the model at the next one and is recorded when it
        // does. What is offered is a fact about the request, not a live value.
        let offered = self.tool_definitions();
        if offered != self.prefix.tools() {
            let narrowed = self.prefix.with_tools(offered);
            self.adopt_prefix(narrowed, "tools");
        }
    }

    /// Run on this prefix from now on, recording that the cacheable head moved.
    ///
    /// Nothing is recorded before the first request goes out: a prefix that has never been
    /// sent cannot have broken a cache, and a session that opened by reporting a change
    /// from nothing to its own opening prompt would be noise in front of every real one.
    fn adopt_prefix(&mut self, prefix: Prefix, reason: &str) {
        let from_hash = self.prefix.hash().to_string();
        let moved = prefix.hash() != from_hash;
        self.prefix = prefix;
        if moved && self.turn > 0 {
            self.record_event(LedgerEvent::PrefixChanged {
                reason: reason.to_string(),
                from_hash,
                to_hash: self.prefix.hash().to_string(),
            });
        }
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

    /// Charge what a provider said a request cost against the session's budget.
    ///
    /// Every request the session makes goes through here, the summarizer's included: a
    /// summary is money spent on this session whether or not anyone asked for it, and a
    /// ceiling that ignored it would be a ceiling with a hole in it.
    fn charge(&mut self, usage: Usage) {
        let Some(budget) = &mut self.budget else {
            return;
        };
        budget.spent += budget
            .cost
            .price(
                TokenUsage::new(usage.input as u64, usage.output as u64)
                    .with_cache(usage.cache_read as u64, usage.cache_write as u64),
            )
            .total();
    }

    /// The ceiling and what has been spent against it, once the spending has reached it.
    ///
    /// Recorded on the way past, so the reason a run ended where it did is a fact about the
    /// session rather than a line that scrolled by in a terminal.
    fn budget_reached(&self) -> Option<(f64, f64)> {
        let budget = self.budget.as_ref().filter(|budget| budget.reached())?;
        let reached = (budget.limit, budget.spent);
        self.record_event(LedgerEvent::BudgetStop {
            limit: reached.0,
            spent: reached.1,
        });
        Some(reached)
    }

    /// Say why the run stopped, in the shape a turn that failed is already said in.
    ///
    /// Announced rather than committed: the conversation holds what was said, and a run
    /// that stopped for want of money is a fact about the run, which the ledger already
    /// holds. Committing it would leave an answer in the transcript the model never gave,
    /// and hand it back to a provider the next time the session was resumed.
    fn say_stopped(&self, events: &Fan<'_>, limit: f64, spent: f64) {
        let message = Message::Assistant(self.empty_assistant(
            StopReason::Error,
            Some(format!(
                "Stopped: this session has spent ${spent:.4} of its ${limit:.4} budget. Raise \
                 the budget to carry on, or set it to 0 to run without a ceiling."
            )),
        ));
        events.send(AgentEvent::MessageStart {
            message: message.clone(),
        });
        events.send(AgentEvent::MessageEnd { message });
    }

    /// Record the request about to go out: what identifies it, and what it was built from.
    ///
    /// The exact serialized body is retained beside the prompt, tools, and model that
    /// explain it. Every blob is named by its hash and carried only the first time it is
    /// seen, so repeated prefixes do not multiply storage.
    fn record_request(&mut self, context: &Context, attempt: u32) {
        if self.recorder.is_none() {
            return;
        }

        let tools = serde_json::to_vec(&context.tools).unwrap_or_default();
        let described = serde_json::to_vec(&self.model).unwrap_or_default();
        let body =
            serde_json::to_vec(&self.provider.payload(&self.model, context)).unwrap_or_default();

        // Hashed from the request as it is about to go out rather than from the prefix the
        // agent holds, because those are the same thing only as long as nothing rewrote the
        // context on its way past. When something did, the hash recorded here is the one
        // that was actually sent, and it belongs to no recorded change — which is exactly
        // what a reader trying to account for a cache miss needs to be able to see.
        let sent = Prefix::new(
            context.system_prompt.clone(),
            context.tools.clone(),
            self.prefix.spans().to_vec(),
        );

        let mut blobs = Vec::new();
        let system_prompt_blob = context
            .system_prompt
            .as_ref()
            .map(|prompt| self.blob(&mut blobs, prompt.as_bytes()));
        let tools_blob = self.blob(&mut blobs, &tools);
        let model_blob = self.blob(&mut blobs, &described);
        let request_body_blob = Some(self.blob(&mut blobs, &body));

        let event = LedgerEvent::TurnRequest {
            turn: self.turn,
            provider: self.model.provider.clone(),
            model: self.model.id.clone(),
            prefix_hash: sent.hash().to_string(),
            request_hash: content_hash(&body),
            request_body_blob,
            system_prompt_blob,
            tools_blob,
            model_blob,
            prefix_spans: sent.spans().to_vec(),
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

    /// Report messages the conversation already holds: written to the log, announced to
    /// whoever is watching, and counted as part of what this run produced.
    ///
    /// Unlike [`Agent::commit`] nothing is appended, because these are already in place —
    /// a repair belongs beside the call it answers, not at the end of the conversation.
    fn announce(&self, messages: Vec<Message>, events: &Fan<'_>, produced: &mut Vec<Message>) {
        for message in messages {
            if let Some(recorder) = &self.recorder {
                let _ = recorder.send(Record::Message(message.clone()));
            }
            events.send(AgentEvent::MessageStart {
                message: message.clone(),
            });
            events.send(AgentEvent::MessageEnd {
                message: message.clone(),
            });
            produced.push(message);
        }
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

        // Answering an abandoned tool call is what makes a conversation sendable again: a
        // provider rejects a request that contains one, so an unanswered call would poison
        // every later turn and make the log unresumable. A conversation that arrived
        // already broken was repaired where it arrived, and this is the first moment there
        // is anywhere to report that.
        let installed = std::mem::take(&mut self.repairs);
        self.announce(installed, events, &mut produced);

        // A turn abandoned partway through this process — Ctrl+C during a tool — leaves its
        // own last answer unanswered. Nothing else will fix that: the run that was
        // abandoned is gone, and this is the next one. It appends to the end of the
        // conversation rather than rewriting anything earlier in it.
        if ends_unanswered(&self.messages) {
            let repairs = answer_abandoned_calls(&mut self.messages);
            self.announce(repairs, events, &mut produced);
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
            // Between turns is where the head of a request is allowed to move, so it moves
            // here, before anything is assembled from it.
            self.settle_prefix();
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

            // What the turn cost is known now, and the next request has not been assembled
            // yet, so this is where a ceiling can be honoured without abandoning a request
            // already in flight.
            if let Some((limit, spent)) = self.budget_reached() {
                events.send(AgentEvent::TurnEnd {
                    messages: produced.clone(),
                });
                self.say_stopped(events, limit, spent);
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
                self.find_tool(name).is_some_and(|tool| {
                    tool.execution_mode() == Some(ToolExecutionMode::Sequential)
                })
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
        self.record_compaction(&compacted);
        self.charge(compacted.cost.usage);
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
        let context = self
            .prefix
            .ahead_of(self.messages.clone(), self.cache_key.clone());
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
                    self.charge(message.usage);
                    events.send(AgentEvent::MessageEnd {
                        message: Message::Assistant(message.clone()),
                    });
                    return message;
                }
                Err(error) => {
                    self.record_event(LedgerEvent::RequestAttemptFailed {
                        turn,
                        attempt,
                        error: error.clone(),
                        usage_unknown: true,
                    });
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

/// Whether the conversation ends on tool calls that nothing answered.
///
/// True only of the last answer in it, which is what a turn abandoned partway through
/// leaves behind. An unanswered call earlier than that came in with the conversation and
/// was dealt with where it arrived; answering one here would move the middle of a history
/// a provider may already be caching.
fn ends_unanswered(messages: &[Message]) -> bool {
    let Some(last) = messages
        .iter()
        .rposition(|message| matches!(message, Message::Assistant(_)))
    else {
        return false;
    };
    let Message::Assistant(assistant) = &messages[last] else {
        unreachable!("the position above is an assistant message");
    };

    let answered: Vec<&str> = messages[last + 1..]
        .iter()
        .filter_map(|message| match message {
            Message::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assistant
        .tool_calls()
        .iter()
        .any(|(id, ..)| !answered.contains(id))
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

    /// A tool that exists and does nothing, for a test about which tools an agent holds
    /// rather than about what any of them do.
    struct NamedTool(&'static str);

    #[async_trait::async_trait]
    impl Tool for NamedTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.0.to_string(),
                description: String::new(),
                parameters: serde_json::json!({ "type": "object" }),
                constrained_sampling: Default::default(),
            }
        }

        async fn execute(&self, _arguments: &serde_json::Value) -> Result<String, String> {
            Ok(String::new())
        }
    }

    /// A tool whose provider is gone is gone: the model stops being told about it, and it
    /// can no longer be found by name either — unlike a narrowed list, where a tool left
    /// out is still there to be called.
    #[test]
    fn a_removed_tool_is_neither_offered_nor_findable() {
        let mut agent = Agent::new(
            Arc::new(NoProvider),
            vec![
                Arc::new(NamedTool("read")),
                Arc::new(NamedTool("from-an-extension")),
            ],
            Model::anthropic("test-model"),
            "test-key",
        );
        assert_eq!(agent.tool_definitions().len(), 2);

        agent.remove_tools(&["from-an-extension".to_string()]);

        let offered: Vec<String> = agent
            .tool_definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        assert_eq!(offered, vec!["read".to_string()]);
        assert!(agent.find_tool("from-an-extension").is_none());
        assert!(agent.find_tool("read").is_some());
    }

    /// A provider that is never asked for anything, for a test about the agent's own
    /// bookkeeping rather than about a request.
    struct NoProvider;

    impl Provider for NoProvider {
        fn name(&self) -> &str {
            "none"
        }

        fn stream(
            &self,
            _model: Model,
            _context: Context,
            _api_key: String,
        ) -> tokio::sync::mpsc::UnboundedReceiver<micro_types::StreamEvent> {
            unreachable!("this test never issues a request")
        }

        fn payload(&self, _model: &Model, _context: &Context) -> serde_json::Value {
            serde_json::Value::Null
        }
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
