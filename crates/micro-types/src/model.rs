use serde::Deserialize;
use serde::Serialize;

/// How much reasoning effort to request from a model that supports extended thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl ThinkingLevel {
    /// Token budget to hand the provider, or `None` when thinking is off.
    pub fn budget_tokens(&self) -> Option<u32> {
        match self {
            ThinkingLevel::Off => None,
            ThinkingLevel::Low => Some(4_000),
            ThinkingLevel::Medium => Some(12_000),
            ThinkingLevel::High => Some(32_000),
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
}

impl Model {
    pub fn anthropic(id: impl Into<String>) -> Self {
        Model {
            id: id.into(),
            provider: "anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            max_tokens: 32_000,
            thinking: ThinkingLevel::Off,
        }
    }

    pub fn with_thinking(mut self, level: ThinkingLevel) -> Self {
        self.thinking = level;
        self
    }
}
