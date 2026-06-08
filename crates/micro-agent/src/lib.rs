//! The agent loop: stream a response, run the tools it asks for, repeat until the model stops
//! asking for tools.

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

const RETRYABLE_STATUSES: [u16; 8] = [408, 409, 425, 429, 500, 502, 503, 504];

/// The context window assumed when the caller does not supply the model's own.
pub const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

/// A model to run from now on, and everything needed to reach it.
#[derive(Clone)]
pub struct ModelSwap {
    pub provider: Arc<dyn Provider>,
    pub model: Model,
    pub api_key: ApiKey,
    pub context_window: usize,
    /// What this model's tokens are charged at.
    pub cost: ModelCost,
}

impl std::fmt::Debug for ModelSwap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelSwap")
            .field("provider", &self.provider.name())
            .field("model", &self.model.id)
            .field("context_window", &self.context_window)
            .finish()
    }
}

impl PartialEq for ModelSwap {
    /// Two swaps are the same when they name the same model on the same provider.
    fn eq(&self, other: &Self) -> bool {
        self.provider.name() == other.provider.name()
            && self.model == other.model
            && self.context_window == other.context_window
    }
}

/// What a session may spend, what it has spent, and what its tokens are charged at.
#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    /// The ceiling for the whole session, in US dollars.
    pub limit: f64,

    pub spent: f64,
    /// What the model in use charges.
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
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Message(Message),

    Compacted {
        summary: String,
        kept: usize,
        cost: CompactionCost,
    },
    /// A fact about the run, and the content it names by hash.
    Event {
        event: LedgerEvent,
        blobs: Vec<(String, Vec<u8>)>,
    },
}

/// A way to reach a run that is already going.
#[derive(Clone, Default)]
pub struct Steering {
    queues: Arc<std::sync::Mutex<Queues>>,
}

#[derive(Default)]
struct Queues {
    /// Taken at the start of the next turn.
    steering: Vec<Message>,

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

    /// Forget everything waiting, for a run that was abandoned: what was queued behind it was
    /// queued behind the thing that is now gone.
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
#[derive(Clone, Default)]
pub struct PrefixControl {
    asked: Arc<std::sync::Mutex<Asked>>,
}

#[derive(Default)]
struct Asked {
    prompt: String,
    spans: Vec<PrefixSpan>,
    /// Why the prompt above is not the one in force yet.
    reason: Option<String>,
    /// A prompt standing in for the one above, for a single run only.
    overriding: Option<(String, Vec<PrefixSpan>, String)>,
}

impl PrefixControl {
    /// What the model is being told before the conversation, including a change that has been asked
    /// for and not yet taken effect, but never a run-scoped override.
    pub fn system_prompt(&self) -> String {
        self.lock().prompt.clone()
    }

    /// Tell the model this instead, from the next turn on, and say why.
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

    /// Tell the model this for the run about to start, and say why. The prompt settled with
    /// [`Self::change`] returns once that run ends.
    pub fn override_run(
        &self,
        prompt: impl Into<String>,
        spans: Vec<PrefixSpan>,
        reason: impl Into<String>,
    ) {
        let mut asked = self.lock();
        asked.overriding = Some((prompt.into(), spans, reason.into()));
    }

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

    /// The stand-in for the coming run, if one was asked for, taken so it is applied once.
    pub(crate) fn take_override(&self) -> Option<(String, Vec<PrefixSpan>, String)> {
        self.lock().overriding.take()
    }

    /// The settled prompt and its spans, unaffected by any run-scoped override.
    pub(crate) fn settled(&self) -> (String, Vec<PrefixSpan>) {
        let asked = self.lock();
        (asked.prompt.clone(), asked.spans.clone())
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
    prefix: Prefix,
    /// How anything outside the run asks for the prefix to change.
    prefix_control: PrefixControl,
    /// Whether a run-scoped override is standing in for the settled prompt.
    overridden: bool,
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
    /// Which tools the model may see and the agent may execute, when something has narrowed them.
    offered: Option<Arc<std::sync::RwLock<Option<Vec<String>>>>>,

    turn: u64,
    /// Results written to answer tool calls a conversation arrived with unanswered, waiting for a
    /// run to report them.
    repairs: Vec<Message>,
    /// Content already handed to the recorder, by hash.
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
            overridden: false,
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

    pub fn prefix_control(&self) -> PrefixControl {
        self.prefix_control.clone()
    }

    /// What every request this agent issues opens with.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    /// Point the agent at a different model, keeping the conversation.
    pub fn set_model(&mut self, swap: ModelSwap) {
        self.provider = swap.provider;
        self.model = swap.model;
        self.api_key = swap.api_key;
        self.context_window = swap.context_window;

        if let Some(budget) = &mut self.budget {
            budget.cost = swap.cost;
        }
        self.summarizer = self.provider_summarizer();
    }

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

    /// Turn compaction on or off while the agent is running.
    pub fn set_auto_compaction(&mut self, enabled: bool) {
        self.compaction = match enabled {
            true => Some(self.compaction.unwrap_or_default()),
            false => None,
        };
    }

    /// Run a different model through the provider already in hand.
    pub fn set_runtime_model(&mut self, model: Model) {
        self.model = model;
        self.summarizer = self.provider_summarizer();
    }

    /// Summarize with something other than the model the agent is running, which is how a caller
    /// routes summaries to a cheaper model.
    pub fn with_summarizer(mut self, summarizer: Arc<dyn Summarizer>) -> Self {
        self.summarizer = summarizer;
        self
    }

    /// Send every finalized message to `recorder` as it is produced.
    pub fn with_recorder(mut self, recorder: UnboundedSender<Record>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Stop this session once it has spent what `budget` allows.
    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Name the conversation, so a provider that caches a prompt can recognise it again.
    pub fn with_cache_key(mut self, key: impl Into<String>) -> Self {
        self.cache_key = Some(key.into());
        self.summarizer = self.provider_summarizer();
        self
    }

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

    pub fn with_offered_tools(
        mut self,
        offered: Arc<std::sync::RwLock<Option<Vec<String>>>>,
    ) -> Self {
        self.offered = Some(offered);
        self
    }

    /// Take tools away for good, for a run that outlived whoever provided them.
    pub fn remove_tools(&mut self, names: &[String]) {
        self.tools
            .retain(|tool| !names.contains(&tool.definition().name));
    }

    /// Let something decide what the run may do.
    pub fn with_hooks(mut self, hooks: Arc<dyn Hooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Send every event to `observer` as well as to whoever asked for the turn.
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

    /// Summarize the conversation now, whether or not it has grown enough to trigger on its own,
    /// and continue from the summary.
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
    pub fn set_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;

        self.repairs = answer_abandoned_calls(&mut self.messages);
    }

    /// Put a message into the conversation without asking the model anything.
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

            (None, None) => (
                vec![ContentBlock::text(format!("tool not found: {name}"))],
                true,
            ),
        };

        let said: String = content.iter().map(ContentBlock::as_text).collect();
        let (output, is_error) = self.rewritten(&id, &name, said.clone(), is_error).await;
        let content = match output == said {
            true => content,
            false => vec![ContentBlock::text(output.clone())],
        };

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

            kept: compacted.messages.len().saturating_sub(1),
            cost: compacted.cost.clone(),
        });
    }

    /// Take up whatever was asked of the prefix while the last turn ran.
    fn settle_prefix(&mut self) {
        if let Some((prompt, spans, reason)) = self.prefix_control.take() {
            let asked = self.prefix.with_system_prompt(prompt, spans);
            self.adopt_prefix(asked, &reason);
            self.overridden = false;
        }
        if let Some((prompt, spans, reason)) = self.prefix_control.take_override() {
            let asked = self.prefix.with_system_prompt(prompt, spans);
            self.adopt_prefix(asked, &reason);
            self.overridden = true;
        }

        let offered = self.tool_definitions();
        if offered != self.prefix.tools() {
            let narrowed = self.prefix.with_tools(offered);
            self.adopt_prefix(narrowed, "tools");
        }
    }

    /// Run on this prefix from now on, recording that the cacheable head moved.
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

    fn record_request(&mut self, context: &Context, attempt: u32, payload: &serde_json::Value) {
        if self.recorder.is_none() {
            return;
        }

        let tools = serde_json::to_vec(&context.tools).unwrap_or_default();
        let described = serde_json::to_vec(&self.model).unwrap_or_default();
        let body = serde_json::to_vec(payload).unwrap_or_default();

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

            message_entry_ids: Vec::new(),
            attempt,
        };
        if let Some(recorder) = &self.recorder {
            let _ = recorder.send(Record::Event { event, blobs });
        }
    }

    /// Name a piece of content by the hash of its bytes, carrying the content itself along the
    /// first time that name is used.
    fn blob(&mut self, carried: &mut Vec<(String, Vec<u8>)>, content: &[u8]) -> String {
        let hash = content_hash(content);
        if self.stored_blobs.insert(hash.clone()) {
            carried.push((hash.clone(), content.to_vec()));
        }
        hash
    }

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
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
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
        let offered = self.offered.as_ref().and_then(|offered| {
            offered
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        });
        if offered.is_some_and(|names| !names.iter().any(|offered| offered == name)) {
            return None;
        }
        self.tools
            .iter()
            .find(|tool| tool.definition().name == name)
    }

    /// Run one exchange to completion.
    pub async fn run(
        &mut self,
        prompt: Message,
        events: &UnboundedSender<AgentEvent>,
    ) -> Vec<Message> {
        let events = &self.fan(events);

        // A prompt overridden for one run does not outlive it.
        if self.overridden {
            let (prompt, spans) = self.prefix_control.settled();
            let restored = self.prefix.with_system_prompt(prompt, spans);
            self.adopt_prefix(restored, "override ended");
            self.overridden = false;
        }

        let mut settle = SettleGuard::armed(events.clone_for_updates());
        let prompt = match &self.hooks {
            Some(hooks) => hooks.before_agent_start(&prompt).await.unwrap_or(prompt),
            None => prompt,
        };
        let mut produced = Vec::new();

        let installed = std::mem::take(&mut self.repairs);
        self.announce(installed, events, &mut produced);

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

            self.settle_prefix();
            if let Err(error) = self.compact_if_needed(events).await {
                let message = Message::Assistant(self.empty_assistant(
                    StopReason::Error,
                    Some(format!("Automatic compaction failed: {error}")),
                ));
                events.send(AgentEvent::MessageStart {
                    message: message.clone(),
                });
                events.send(AgentEvent::MessageEnd {
                    message: message.clone(),
                });
                self.commit(message, &mut produced);
                events.send(AgentEvent::TurnEnd {
                    messages: produced.clone(),
                });
                break;
            }

            let assistant = self.stream_once(events).await;
            if let Some(hooks) = &self.hooks {
                hooks.after_response(&assistant).await;
            }
            self.commit(Message::Assistant(assistant.clone()), &mut produced);

            if matches!(
                assistant.stop_reason,
                StopReason::Error | StopReason::Aborted
            ) {
                events.send(AgentEvent::TurnEnd {
                    messages: produced.clone(),
                });
                break;
            }

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
                events.send(AgentEvent::TurnEnd {
                    messages: produced.clone(),
                });

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

            let truncated = assistant.stop_reason == StopReason::Length;

            let sequential = calls.iter().any(|(_, name, _)| {
                self.find_tool(name).is_some_and(|tool| {
                    tool.execution_mode() == Some(ToolExecutionMode::Sequential)
                })
            });

            let mut prepared = Vec::with_capacity(calls.len());
            for (id, name, arguments) in calls {
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
                        ToolDecision::Refuse(reason) => {
                            self.record_event(LedgerEvent::ToolDenied {
                                tool: name.clone(),
                                reason: reason.clone(),

                                source: EventSource::Extension(String::new()),
                            });
                            settled = Some((reason, true));
                        }
                        decision => {
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
                let agent = &*self;
                let ran = futures::future::join_all(prepared.into_iter().map(
                    |(id, name, arguments, settled, runnable)| {
                        agent.finish_call(id, name, arguments, settled, runnable, events)
                    },
                ))
                .await;

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

            events.send(AgentEvent::TurnEnd {
                messages: produced.clone(),
            });
        }

        events.send(AgentEvent::AgentEnd {
            messages: produced.clone(),
        });

        events.send(AgentEvent::AgentSettled);
        settle.disarm();
        produced
    }

    /// Replace the older part of the conversation with a summary once it approaches the context
    /// window.
    async fn compact_if_needed(&mut self, events: &Fan<'_>) -> std::result::Result<(), String> {
        let Some(config) = self.compaction else {
            return Ok(());
        };

        let compactor = Compactor::new(self.summarizer.clone(), config);
        let Some(compacted) = compactor
            .compact_if_needed(&self.messages, self.context_window)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(());
        };

        let summary = compacted.messages[0].clone();
        self.record_compaction(&compacted);
        self.charge(compacted.cost.usage);
        self.messages = compacted.messages;

        events.send(AgentEvent::MessageStart {
            message: summary.clone(),
        });
        events.send(AgentEvent::MessageEnd { message: summary });
        Ok(())
    }

    /// Issue one model request, forwarding stream events and retrying transient failures that
    /// happen before any content is shown.
    async fn stream_once(&mut self, events: &Fan<'_>) -> AssistantMessage {
        let context = self
            .prefix
            .ahead_of(self.messages.clone(), self.cache_key.clone());

        let context = match &self.hooks {
            Some(hooks) => hooks.before_request(context).await,
            None => context,
        };

        self.turn += 1;
        let turn = self.turn;

        let mut attempt = 0;

        events.send(AgentEvent::MessageStart {
            message: Message::Assistant(self.empty_assistant(StopReason::Stop, None)),
        });

        loop {
            attempt += 1;

            // A request with nothing to authenticate it is not worth sending: the provider can only
            // answer that the header is malformed, which says nothing about the credential that is
            // missing behind it.
            let prepared = match self.api_key.current().await {
                Ok(api_key) => self
                    .provider
                    .request_payload(&self.model, &context, &api_key)
                    .map(|payload| (api_key, payload)),
                Err(error) => Err(error),
            };
            let (api_key, payload) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.record_event(LedgerEvent::RequestAttemptFailed {
                        turn,
                        attempt,
                        error: error.clone(),
                        usage_unknown: false,
                    });
                    let message = self.empty_assistant(StopReason::Error, Some(error));
                    events.send(AgentEvent::MessageEnd {
                        message: Message::Assistant(message.clone()),
                    });
                    return message;
                }
            };

            self.record_request(&context, attempt, &payload);
            let mut stream = self.provider.stream_prepared(
                self.model.clone(),
                context.clone(),
                api_key,
                payload,
            );

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

/// Accumulate streamed deltas into the text shown so far.
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

    /// A tool whose provider is gone is gone: the model stops being told about it, and it can no
    /// longer be found by name either.
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

    let _ = forwarding.await;

    match ran {
        Ok(content) => (content, false),
        Err(error) => (vec![ContentBlock::text(error)], true),
    }
}

/// Where an event goes: to whoever asked for the turn, and to anything watching.
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
#[derive(Debug, Clone, PartialEq)]
pub enum ToolDecision {
    /// Run the call as the model wrote it.
    Proceed,
    /// Run it with these arguments in place of the ones the model wrote.
    Rewrite(Value),
    /// Do not run it.
    Refuse(String),
}

#[async_trait::async_trait]
pub trait Hooks: Send + Sync {
    /// Called before a tool runs, with the chance to change the call or refuse it.
    async fn before_tool(&self, id: &str, name: &str, arguments: &Value) -> ToolDecision {
        let _ = (id, name, arguments);
        ToolDecision::Proceed
    }

    /// Called once a tool has answered, before the model reads it.
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

    /// Called with the prompt a run is about to start on.
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

    /// Say the run's own `AgentEnd`/`AgentSettled` already went out, so this reports nothing when
    /// it is dropped.
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
