//! Keeping a provider's prompt cache alive while the run that wrote it is busy elsewhere.
//!
//! A cached prompt prefix expires a few minutes after it was last used. When a tool runs for
//! longer than that, the next request pays to write the whole prefix again. The warmer replays the
//! last request with a one-token output cap shortly before the cache would expire, but only when
//! the replay is expected to save more than it costs.

use crate::Hooks;
use crate::Record;
use micro_models::ModelCost;
use micro_models::TokenUsage;
use micro_provider::ApiKey;
use micro_provider::Provider;
use micro_types::Context;
use micro_types::LedgerEvent;
use micro_types::Model;
use micro_types::StreamEvent;
use micro_types::ThinkingLevel;
use micro_types::Usage;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;

/// Warming during a run never continues past this long after the request that started it.
const MAX_WARMING_AGE: Duration = Duration::from_secs(60 * 60);
/// Warming between runs stops sooner, because whether anyone comes back gets less likely.
const MAX_IDLE_WARMING_AGE: Duration = Duration::from_secs(30 * 60);
/// A refresh is sent only when it is expected to save at least this many dollars.
const MINIMUM_EXPECTED_SAVINGS: f64 = 0.05;
/// The chance that a real request arrives before the cache expires while nothing is running.
const IDLE_CONTINUATION_PROBABILITY: f64 = 0.15;
/// How long Anthropic keeps an ephemeral cache entry.
const ANTHROPIC_CACHE_LIFETIME: Duration = Duration::from_secs(300);

/// When the warmer may send refreshes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheWarmingMode {
    /// Never.
    Off,
    /// While a run is going, such as during a long tool call.
    #[default]
    Streaming,
    /// While a run is going, and between runs too.
    Idle,
}

/// How the warmer is set up: when it may run, and how long each model's cache lives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheWarming {
    pub mode: CacheWarmingMode,
    /// Cache lifetimes keyed by `provider/model` or by provider, over the built-in ones.
    pub lifetimes: BTreeMap<String, Duration>,
}

impl CacheWarming {
    /// How long a request's cache entry lives with this model, when that is known.
    pub fn lifetime(&self, model: &Model) -> Option<Duration> {
        let qualified = format!("{}/{}", model.provider, model.id);
        self.lifetimes
            .get(&qualified)
            .or_else(|| self.lifetimes.get(&model.provider))
            .copied()
            .or_else(|| (model.provider == "anthropic").then_some(ANTHROPIC_CACHE_LIFETIME))
    }
}

/// When a refresh goes out: at 90% of the lifetime, leaving at least ten seconds of margin.
pub fn warming_delay(lifetime: Duration) -> Option<Duration> {
    let millis = lifetime.as_millis() as u64;
    if millis <= 10_000 {
        return None;
    }
    let delay = (millis * 9 / 10).min(millis - 10_000).max(1);
    Some(Duration::from_millis(delay))
}

/// Whether replaying a request with a one-token cap leaves its cache entry untouched. Anthropic
/// derives a thinking budget from the output cap, and keys its cache on that budget.
pub fn is_replayable(model: &Model) -> bool {
    !(model.provider == "anthropic" && model.reasoning && model.thinking != ThinkingLevel::Off)
}

/// What a warming decision comes to: send the refresh, or let the cache entry expire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmingAction {
    Warm,
    /// Stop warming until the next real request.
    Stop,
}

impl WarmingAction {
    pub fn as_str(self) -> &'static str {
        match self {
            WarmingAction::Warm => "warm",
            WarmingAction::Stop => "stop",
        }
    }
}

/// Whether the run that sent the warmed request is still going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmingPhase {
    Streaming,
    Idle,
}

/// Whether to send a refresh, and why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarmingDecision {
    pub phase: WarmingPhase,
    /// What the refresh costs: a cache read of the prompt and one output token.
    pub warm_cost: f64,
    /// What the next real request costs on top if the cache entry is lost.
    pub miss_cost: f64,
    pub continuation_probability: f64,
    /// `continuation_probability * miss_cost - warm_cost`.
    pub expected_savings: f64,
    /// False when the prompt size or the model's prices are unknown.
    pub economics_available: bool,
    /// Whether the refresh is worth sending: it is when the expected savings reach $0.05.
    pub warm: bool,
}

impl WarmingDecision {
    pub fn action(&self) -> WarmingAction {
        match self.warm {
            true => WarmingAction::Warm,
            false => WarmingAction::Stop,
        }
    }
}

/// Weigh a refresh of a prompt `prompt_tokens` long against letting its cache entry expire.
pub fn decide(prompt_tokens: u64, cost: &ModelCost, idle: bool) -> WarmingDecision {
    let price = |usage: TokenUsage| cost.price(usage).total();
    let hit = price(TokenUsage {
        cache_read: prompt_tokens,
        ..TokenUsage::default()
    });
    let miss = match cost.cache_write > 0.0 {
        true => price(TokenUsage {
            cache_write: prompt_tokens,
            ..TokenUsage::default()
        }),
        false => price(TokenUsage {
            input: prompt_tokens,
            ..TokenUsage::default()
        }),
    };
    let warm_cost = price(TokenUsage {
        cache_read: prompt_tokens,
        output: 1,
        ..TokenUsage::default()
    });
    let miss_cost = (miss - hit).max(0.0);
    let (phase, continuation_probability) = match idle {
        true => (WarmingPhase::Idle, IDLE_CONTINUATION_PROBABILITY),
        false => (WarmingPhase::Streaming, 1.0),
    };
    let economics_available = prompt_tokens > 0 && (hit > 0.0 || miss > 0.0);
    let expected_savings = continuation_probability * miss_cost - warm_cost;
    WarmingDecision {
        phase,
        warm_cost,
        miss_cost,
        continuation_probability,
        expected_savings,
        economics_available,
        warm: economics_available && expected_savings >= MINIMUM_EXPECTED_SAVINGS,
    }
}

/// What the warmer is doing about the cache entry of the last request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmingState {
    Inactive,
    /// A refresh is due at `next_warm_at`.
    Scheduled,
    /// A refresh is in flight.
    Refreshing,
}

/// The warmer's state, as `/session` shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct WarmingStatus {
    pub state: WarmingState,
    /// Why nothing is scheduled.
    pub reason: Option<String>,
    pub next_warm_at: Option<SystemTime>,
    /// The pending decision, or the one that stopped warming.
    pub decision: Option<WarmingDecision>,
    /// Whether an extension changed the decision.
    pub extension_override: bool,
}

impl WarmingStatus {
    fn inactive(reason: impl Into<String>) -> Self {
        WarmingStatus {
            state: WarmingState::Inactive,
            reason: Some(reason.into()),
            next_warm_at: None,
            decision: None,
            extension_override: false,
        }
    }
}

impl Default for WarmingStatus {
    fn default() -> Self {
        WarmingStatus::inactive("waiting for first request")
    }
}

/// A refresh that went out, as the transcript announces it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarmNotice {
    /// What the refresh cost, in dollars.
    pub cost: f64,
    /// Whether an extension asked for the refresh against micro's own decision.
    pub extension_override: bool,
}

type WarmListener = Arc<dyn Fn(WarmNotice) + Send + Sync>;

#[derive(Default)]
struct Watched {
    mode: CacheWarmingMode,
    status: WarmingStatus,
    /// What the pending decision is weighed on: the prompt's size and the model's rates.
    basis: Option<(u64, ModelCost)>,
    listener: Option<WarmListener>,
}

/// A handle onto what an agent's warmer is doing, for anything outside the agent to read.
#[derive(Clone, Default)]
pub struct WarmingWatch {
    watched: Arc<Mutex<Watched>>,
}

impl WarmingWatch {
    /// What the warmer is doing now.
    pub fn status(&self) -> WarmingStatus {
        let watched = self.lock();
        if watched.mode == CacheWarmingMode::Off {
            return WarmingStatus::inactive("cache warming disabled");
        }
        watched.status.clone()
    }

    pub fn mode(&self) -> CacheWarmingMode {
        self.lock().mode
    }

    /// Be told about every refresh that goes out.
    pub fn on_warmed(&self, listener: impl Fn(WarmNotice) + Send + Sync + 'static) {
        self.lock().listener = Some(Arc::new(listener));
    }

    fn set_mode(&self, mode: CacheWarmingMode) {
        self.lock().mode = mode;
    }

    fn scheduled(&self, basis: (u64, ModelCost), next_warm_at: SystemTime, idle: bool) {
        let mut watched = self.lock();
        let decision = decide(basis.0, &basis.1, idle);
        watched.basis = Some(basis);
        watched.status = WarmingStatus {
            state: WarmingState::Scheduled,
            reason: None,
            next_warm_at: Some(next_warm_at),
            decision: Some(decision),
            extension_override: false,
        };
    }

    fn refreshing(&self, decision: WarmingDecision, extension_override: bool) {
        let mut watched = self.lock();
        watched.status = WarmingStatus {
            state: WarmingState::Refreshing,
            reason: None,
            next_warm_at: None,
            decision: Some(decision),
            extension_override,
        };
    }

    fn stopped(&self, reason: &str) {
        self.stopped_on(reason, None, false);
    }

    fn stopped_on(
        &self,
        reason: &str,
        decision: Option<WarmingDecision>,
        extension_override: bool,
    ) {
        let mut watched = self.lock();
        watched.basis = None;
        watched.status = WarmingStatus {
            decision,
            extension_override,
            ..WarmingStatus::inactive(reason)
        };
    }

    /// Reweigh the pending decision once the run that sent the request is over.
    fn settled(&self) {
        let mut watched = self.lock();
        if watched.status.state != WarmingState::Scheduled {
            return;
        }
        if watched.mode != CacheWarmingMode::Idle {
            watched.basis = None;
            watched.status = WarmingStatus::inactive("agent run settled");
            return;
        }
        if let Some((prompt_tokens, cost)) = &watched.basis {
            watched.status.decision = Some(decide(*prompt_tokens, cost, true));
        }
    }

    fn notify(&self, notice: WarmNotice) {
        let listener = self.lock().listener.clone();
        if let Some(listener) = listener {
            listener(notice);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Watched> {
        self.watched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn dollars(value: f64) -> String {
    match value < 0.0 {
        true => format!("-${:.3}", value.abs()),
        false => format!("${value:.3}"),
    }
}

fn economics(decision: &WarmingDecision) -> String {
    if !decision.economics_available {
        return "cache economics unavailable".to_string();
    }
    let probability = (decision.continuation_probability * 100.0).round();
    let while_running = match decision.phase {
        WarmingPhase::Streaming => " while agent is running",
        WarmingPhase::Idle => "",
    };
    let comparison = match decision.warm {
        true => ">=",
        false => "<",
    };
    format!(
        "{probability}% continuation probability{while_running}, expected savings {} \
         {comparison} ${MINIMUM_EXPECTED_SAVINGS:.3}",
        dollars(decision.expected_savings)
    )
}

fn decision_time(next_warm_at: Option<SystemTime>, now: SystemTime) -> String {
    let remaining = next_warm_at
        .and_then(|at| at.duration_since(now).ok())
        .filter(|remaining| !remaining.is_zero());
    let Some(remaining) = remaining else {
        return "Decision now".to_string();
    };
    let mut seconds = (remaining.as_millis() as u64).div_ceil(1000);
    let hours = seconds / 3600;
    seconds %= 3600;
    let minutes = seconds / 60;
    seconds %= 60;
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds}s"));
    }
    format!("Decision in {}", parts.join(" "))
}

/// The warmer's status on one line, as `/session` shows it.
pub fn format_warming_status(status: &WarmingStatus, now: SystemTime) -> String {
    // A decision is attached once micro or an extension acted on it; an inactive warmer without
    // one never got that far.
    let decision = match &status.decision {
        Some(decision)
            if status.state != WarmingState::Inactive
                || decision.economics_available
                || status.extension_override =>
        {
            decision
        }
        _ => {
            return format!(
                "Inactive ({})",
                status.reason.as_deref().unwrap_or("unknown reason")
            )
        }
    };
    let details = match status.extension_override {
        true => format!("extension override, {}", economics(decision)),
        false => format!("{} -> {}", economics(decision), decision.action().as_str()),
    };
    match status.state {
        WarmingState::Inactive => format!("Stopped ({details})"),
        WarmingState::Refreshing => format!("Warming cache ({details})"),
        WarmingState::Scheduled => {
            format!("{} ({details})", decision_time(status.next_warm_at, now))
        }
    }
}

/// The transcript's line for a refresh that went out: its cost to at least three decimals.
pub fn format_warm_notice(notice: &WarmNotice) -> String {
    let note = match notice.extension_override {
        true => " (extension override)",
        false => "",
    };
    let cost = format!("{:.6}", notice.cost);
    let (whole, fraction) = cost.split_once('.').unwrap_or((cost.as_str(), "000000"));
    let significant = fraction.trim_end_matches('0').len().max(3);
    format!("Cache warmed{note}: ${whole}.{}", &fraction[..significant])
}

/// The request whose cache entry is to be kept warm, as it was sent.
pub(crate) struct WarmRequest {
    pub provider: Arc<dyn Provider>,
    pub model: Model,
    pub context: Context,
    pub api_key: ApiKey,
    pub turn: u64,
    pub usage: Usage,
    pub cost: ModelCost,
    pub recorder: Option<UnboundedSender<Record>>,
    /// What may overrule each decision.
    pub hooks: Option<Arc<dyn Hooks>>,
}

/// The warmer for one agent: at most one request is kept warm at a time.
#[derive(Default)]
pub(crate) struct Warmer {
    task: Option<tokio::task::JoinHandle<()>>,
    /// Whether no run is going, shared with the task so it knows which kind of warming it is
    /// doing however the run ended.
    idle: Arc<AtomicBool>,
    watch: WarmingWatch,
}

/// Marks a run as going for as long as it is held, and as over once it is dropped, whether the
/// run finished or was abandoned.
pub(crate) struct RunGuard {
    idle: Arc<AtomicBool>,
    watch: WarmingWatch,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.idle.store(true, Ordering::Relaxed);
        self.watch.settled();
    }
}

impl Warmer {
    /// Note that a run has started, until the guard is dropped.
    pub fn run_started(&self) -> RunGuard {
        self.idle.store(false, Ordering::Relaxed);
        RunGuard {
            idle: Arc::clone(&self.idle),
            watch: self.watch.clone(),
        }
    }

    pub fn watch(&self) -> WarmingWatch {
        self.watch.clone()
    }

    /// Note the mode warming runs in, for the status to say so before anything was warmed.
    pub fn set_mode(&self, mode: CacheWarmingMode) {
        self.watch.set_mode(mode);
    }

    /// Keep `request`'s cache warm in place of whatever was kept warm before.
    pub fn start(&mut self, settings: &CacheWarming, request: WarmRequest) {
        self.stop("conversation context changed");
        self.watch.set_mode(settings.mode);
        if settings.mode == CacheWarmingMode::Off {
            return;
        }
        if !is_replayable(&request.model) {
            self.watch.stopped("request cannot be replayed safely");
            return;
        }
        let Some(timing) = settings.lifetime(&request.model).and_then(|lifetime| {
            warming_delay(lifetime).map(|delay| (delay, lifetime.saturating_sub(delay)))
        }) else {
            self.watch.stopped("cache lifetime unavailable");
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        self.task = Some(runtime.spawn(keep_warm(
            request,
            timing,
            settings.mode,
            Arc::clone(&self.idle),
            self.watch.clone(),
        )));
    }

    /// Stop keeping anything warm, for `reason`.
    pub fn stop(&mut self, reason: &str) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.watch.stopped(reason);
    }

    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
}

impl Drop for Warmer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Replay the request before each expiry for as long as it pays.
async fn keep_warm(
    request: WarmRequest,
    (delay, margin): (Duration, Duration),
    mode: CacheWarmingMode,
    idle: Arc<AtomicBool>,
    watch: WarmingWatch,
) {
    let started = Instant::now();
    let prompt_tokens = u64::from(request.usage.input)
        + u64::from(request.usage.cache_read)
        + u64::from(request.usage.cache_write);
    let mut replay = request.model.clone();
    replay.max_tokens = 1;

    loop {
        let due = Instant::now() + delay;
        let idling = idle.load(Ordering::Relaxed);
        let horizon = match idling {
            true => MAX_IDLE_WARMING_AGE,
            false => MAX_WARMING_AGE,
        };
        if due.duration_since(started) > horizon {
            watch.stopped(match idling {
                true => "30-minute idle safety limit reached",
                false => "one-hour safety limit reached",
            });
            return;
        }
        watch.scheduled(
            (prompt_tokens, request.cost.clone()),
            SystemTime::now() + delay,
            idling,
        );
        tokio::time::sleep_until(due).await;

        // A timer that fired late, after a sleep of the machine say, would find the cache gone:
        // the replay would be a full-price write rather than a refresh.
        if Instant::now().duration_since(due) > margin / 2 {
            watch.stopped("cache refresh deadline missed");
            return;
        }
        let idling = idle.load(Ordering::Relaxed);
        if idling && mode != CacheWarmingMode::Idle {
            watch.stopped("agent run settled");
            return;
        }
        let decision = decide(prompt_tokens, &request.cost, idling);
        let action = match &request.hooks {
            Some(hooks) => hooks
                .cache_warming_decision(&decision)
                .await
                .unwrap_or(decision.action()),
            None => decision.action(),
        };
        let extension_override = action != decision.action();
        if action == WarmingAction::Stop {
            let reason = match (extension_override, decision.economics_available) {
                (true, _) => "stopped by extension",
                (false, true) => "expected savings below threshold",
                (false, false) => "cache economics unavailable",
            };
            watch.stopped_on(reason, Some(decision), extension_override);
            return;
        }
        if Instant::now().duration_since(due) > margin / 2 {
            watch.stopped("cache refresh deadline missed");
            return;
        }

        watch.refreshing(decision, extension_override);
        let Ok(api_key) = request.api_key.current().await else {
            watch.stopped("no credential for the refresh");
            return;
        };
        let mut stream = request
            .provider
            .stream(replay.clone(), request.context.clone(), api_key);
        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Done { message } => {
                    let usage = message.usage;
                    if let Some(recorder) = &request.recorder {
                        let _ = recorder.send(Record::Event {
                            event: LedgerEvent::CacheWarm {
                                turn: request.turn,
                                usage,
                                provider: message.provider,
                                model: message.model,
                            },
                            blobs: Vec::new(),
                        });
                    }
                    watch.notify(WarmNotice {
                        cost: request
                            .cost
                            .price(
                                TokenUsage::new(u64::from(usage.input), u64::from(usage.output))
                                    .with_cache(
                                        u64::from(usage.cache_read),
                                        u64::from(usage.cache_write),
                                    ),
                            )
                            .total(),
                        extension_override,
                    });
                    break;
                }
                StreamEvent::Error { .. } => {
                    watch.stopped("the refresh failed");
                    return;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: &str, thinking: ThinkingLevel) -> Model {
        Model {
            id: "m".into(),
            provider: provider.into(),
            base_url: "https://example.invalid".into(),
            max_tokens: 1_000,
            thinking,
            reasoning: true,
            compat: Default::default(),
            headers: Default::default(),
        }
    }

    fn cost() -> ModelCost {
        ModelCost {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
            ..ModelCost::default()
        }
    }

    #[test]
    fn a_refresh_goes_out_well_before_expiry() {
        assert_eq!(
            warming_delay(Duration::from_secs(300)),
            Some(Duration::from_secs(270))
        );
        assert_eq!(
            warming_delay(Duration::from_secs(60)),
            Some(Duration::from_secs(50))
        );
        assert_eq!(warming_delay(Duration::from_secs(10)), None);
    }

    #[test]
    fn a_large_prompt_is_worth_warming_while_a_run_is_going() {
        let decision = decide(100_000, &cost(), false);
        assert!(decision.warm, "{decision:?}");
        assert!((decision.miss_cost - (0.375 - 0.03)).abs() < 1e-9);
    }

    #[test]
    fn a_small_or_idle_prompt_is_not() {
        assert!(!decide(5_000, &cost(), false).warm);
        assert!(!decide(100_000, &cost(), true).warm);
        assert!(decide(1_000_000, &cost(), true).warm);
        assert!(!decide(100_000, &ModelCost::default(), false).warm);
    }

    #[test]
    fn lifetimes_come_from_settings_before_the_built_in_ones() {
        let settings = CacheWarming {
            mode: CacheWarmingMode::Streaming,
            lifetimes: [
                ("openai".to_string(), Duration::from_secs(600)),
                ("anthropic/m".to_string(), Duration::from_secs(3600)),
            ]
            .into(),
        };
        let off = ThinkingLevel::Off;
        assert_eq!(
            settings.lifetime(&model("anthropic", off)),
            Some(Duration::from_secs(3600))
        );
        assert_eq!(
            settings.lifetime(&model("openai", off)),
            Some(Duration::from_secs(600))
        );
        assert_eq!(
            CacheWarming::default().lifetime(&model("anthropic", off)),
            Some(ANTHROPIC_CACHE_LIFETIME)
        );
        assert_eq!(
            CacheWarming::default().lifetime(&model("gemini", off)),
            None
        );
    }

    #[test]
    fn a_refresh_reads_as_its_cost() {
        let notice = |cost, extension_override| WarmNotice {
            cost,
            extension_override,
        };
        assert_eq!(
            format_warm_notice(&notice(0.03, false)),
            "Cache warmed: $0.030"
        );
        assert_eq!(
            format_warm_notice(&notice(0.012345, true)),
            "Cache warmed (extension override): $0.012345"
        );
    }

    #[test]
    fn an_idle_decision_is_shown_with_its_economics() {
        let status = WarmingStatus {
            state: WarmingState::Scheduled,
            reason: None,
            next_warm_at: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(3_725)),
            decision: Some(decide(1_000_000, &cost(), true)),
            extension_override: false,
        };
        let line = format_warming_status(&status, SystemTime::UNIX_EPOCH);
        assert!(
            line.starts_with(
                "Decision in 1h 2m 5s (15% continuation probability, expected savings $"
            ),
            "{line}"
        );
        assert!(line.ends_with(">= $0.050 -> warm)"), "{line}");
    }

    #[test]
    fn anthropic_thinking_requests_are_not_replayed() {
        assert!(!is_replayable(&model("anthropic", ThinkingLevel::High)));
        assert!(is_replayable(&model("anthropic", ThinkingLevel::Off)));
        assert!(is_replayable(&model("openai", ThinkingLevel::High)));
    }
}
