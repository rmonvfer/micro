use serde::Deserialize;
use serde::Serialize;

/// How much reasoning effort to request from a model that supports extended thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ThinkingLevel {
    /// What this level is called on the wire.
    pub fn as_str(&self) -> &'static str {
        match self {
            ThinkingLevel::Off => "off",
            ThinkingLevel::Minimal => "minimal",
            ThinkingLevel::Low => "low",
            ThinkingLevel::Medium => "medium",
            ThinkingLevel::High => "high",
            ThinkingLevel::XHigh => "xhigh",
            ThinkingLevel::Max => "max",
        }
    }

    /// Token budget to hand the provider, or `None` when thinking is off.
    pub fn budget_tokens(&self) -> Option<u32> {
        match self {
            ThinkingLevel::Off => None,
            ThinkingLevel::Minimal => Some(2_000),
            ThinkingLevel::Low => Some(4_000),
            ThinkingLevel::Medium => Some(12_000),
            ThinkingLevel::High => Some(32_000),
            ThinkingLevel::XHigh => Some(64_000),
            ThinkingLevel::Max => Some(128_000),
        }
    }
}

/// A model plus the endpoint that serves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub provider: String,
    pub base_url: String,
    pub max_tokens: u32,
    #[serde(default)]
    pub thinking: ThinkingLevel,
    /// Whether asking this model to reason means anything. A model that does not reason
    /// is never sent a reasoning parameter, whatever level is selected.
    #[serde(default)]
    pub reasoning: bool,
    /// What the service serving this model accepts, on top of the protocol it speaks.
    #[serde(default)]
    pub compat: crate::Compat,
    /// Headers this model's service asks every request to carry.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub headers: std::collections::BTreeMap<String, String>,
}

impl Model {
    pub fn anthropic(id: impl Into<String>) -> Self {
        Model {
            id: id.into(),
            provider: "anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            max_tokens: 32_000,
            thinking: ThinkingLevel::Off,
            compat: Default::default(),
            headers: Default::default(),
            reasoning: Default::default(),
        }
    }

    pub fn with_thinking(mut self, level: ThinkingLevel) -> Self {
        self.thinking = level;
        self
    }
}
