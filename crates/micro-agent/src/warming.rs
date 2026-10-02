//! Keeping a provider's prompt cache alive while the run that wrote it is busy elsewhere.
//!
//! A cached prompt prefix expires a few minutes after it was last used. When a tool runs for
//! longer than that, the next request pays to write the whole prefix again. The warmer replays the
//! last request with a one-token output cap shortly before the cache would expire, but only when
//! the replay is expected to save more than it costs.

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
use std::time::Duration;
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

/// Whether to send a refresh, and why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarmingDecision {
    /// What the refresh costs: a cache read of the prompt and one output token.
    pub warm_cost: f64,
    /// What the next real request costs on top if the cache entry is lost.
    pub miss_cost: f64,
    pub continuation_probability: f64,
    pub warm: bool,
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
    let continuation_probability = match idle {
        true => IDLE_CONTINUATION_PROBABILITY,
        false => 1.0,
    };
    let known = prompt_tokens > 0 && (hit > 0.0 || miss > 0.0);
    let savings = continuation_probability * miss_cost - warm_cost;
    WarmingDecision {
        warm_cost,
        miss_cost,
        continuation_probability,
        warm: known && savings >= MINIMUM_EXPECTED_SAVINGS,
    }
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
}

/// The warmer for one agent: at most one request is kept warm at a time.
#[derive(Default)]
pub(crate) struct Warmer {
    task: Option<tokio::task::JoinHandle<()>>,
    /// Whether no run is going, shared with the task so it knows which kind of warming it is
    /// doing however the run ended.
    idle: Arc<AtomicBool>,
}

/// Marks a run as going for as long as it is held, and as over once it is dropped, whether the
/// run finished or was abandoned.
pub(crate) struct RunGuard {
    idle: Arc<AtomicBool>,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.idle.store(true, Ordering::Relaxed);
    }
}

impl Warmer {
    /// Note that a run has started, until the guard is dropped.
    pub fn run_started(&self) -> RunGuard {
        self.idle.store(false, Ordering::Relaxed);
        RunGuard {
            idle: Arc::clone(&self.idle),
        }
    }

    /// Keep `request`'s cache warm in place of whatever was kept warm before.
    pub fn start(&mut self, settings: &CacheWarming, request: WarmRequest) {
        self.stop();
        if settings.mode == CacheWarmingMode::Off || !is_replayable(&request.model) {
            return;
        }
        let Some(timing) = settings.lifetime(&request.model).and_then(|lifetime| {
            warming_delay(lifetime).map(|delay| (delay, lifetime.saturating_sub(delay)))
        }) else {
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
        )));
    }

    pub fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
}

impl Drop for Warmer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Replay the request before each expiry for as long as it pays.
async fn keep_warm(
    request: WarmRequest,
    (delay, margin): (Duration, Duration),
    mode: CacheWarmingMode,
    idle: Arc<AtomicBool>,
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
            return;
        }
        tokio::time::sleep_until(due).await;

        // A timer that fired late, after a sleep of the machine say, would find the cache gone:
        // the replay would be a full-price write rather than a refresh.
        if Instant::now().duration_since(due) > margin / 2 {
            return;
        }
        let idling = idle.load(Ordering::Relaxed);
        if idling && mode != CacheWarmingMode::Idle {
            return;
        }
        if !decide(prompt_tokens, &request.cost, idling).warm {
            return;
        }

        let Ok(api_key) = request.api_key.current().await else {
            return;
        };
        let mut stream = request
            .provider
            .stream(replay.clone(), request.context.clone(), api_key);
        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Done { message } => {
                    if let Some(recorder) = &request.recorder {
                        let _ = recorder.send(Record::Event {
                            event: LedgerEvent::CacheWarm {
                                turn: request.turn,
                                usage: message.usage,
                                provider: message.provider,
                                model: message.model,
                            },
                            blobs: Vec::new(),
                        });
                    }
                    break;
                }
                StreamEvent::Error { .. } => return,
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
    fn anthropic_thinking_requests_are_not_replayed() {
        assert!(!is_replayable(&model("anthropic", ThinkingLevel::High)));
        assert!(is_replayable(&model("anthropic", ThinkingLevel::Off)));
        assert!(is_replayable(&model("openai", ThinkingLevel::High)));
    }
}
