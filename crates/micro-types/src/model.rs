use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;

/// How much reasoning effort to request from a model that supports extended thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh", alias = "x_high")]
    XHigh,
    Max,
}

impl ThinkingLevel {
    /// Every level, from no reasoning to the most a model can be asked for.
    pub const ALL: [ThinkingLevel; 7] = [
        ThinkingLevel::Off,
        ThinkingLevel::Minimal,
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
        ThinkingLevel::XHigh,
        ThinkingLevel::Max,
    ];

    /// The level a wire name stands for.
    pub fn named(name: &str) -> Option<ThinkingLevel> {
        ThinkingLevel::ALL
            .into_iter()
            .find(|level| level.as_str() == name)
    }

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

/// The levels a model can be asked for, from least to most reasoning.
///
/// A model that does not reason offers only `off`. One that does offers every level its thinking
/// map does not mark unsupported with `null`, except `xhigh` and `max`, which it offers only when
/// the map names them.
pub fn supported_thinking_levels(
    reasoning: bool,
    map: &BTreeMap<String, Option<String>>,
) -> Vec<ThinkingLevel> {
    if !reasoning {
        return vec![ThinkingLevel::Off];
    }
    ThinkingLevel::ALL
        .into_iter()
        .filter(|level| match map.get(level.as_str()) {
            Some(None) => false,
            Some(Some(_)) => true,
            None => !matches!(level, ThinkingLevel::XHigh | ThinkingLevel::Max),
        })
        .collect()
}

/// The level to use when `level` is asked of a model offering `supported`: the level itself when
/// offered, else the nearest offered level above it, else the nearest below.
pub fn clamp_thinking_level(supported: &[ThinkingLevel], level: ThinkingLevel) -> ThinkingLevel {
    if supported.contains(&level) {
        return level;
    }
    let above = ThinkingLevel::ALL
        .into_iter()
        .filter(|candidate| *candidate > level)
        .find(|candidate| supported.contains(candidate));
    let below = ThinkingLevel::ALL
        .into_iter()
        .rev()
        .filter(|candidate| *candidate < level)
        .find(|candidate| supported.contains(candidate));
    above
        .or(below)
        .or(supported.first().copied())
        .unwrap_or(ThinkingLevel::Off)
}

/// The level after `level` among the offered ones, wrapping at the top. A level that is not offered
/// moves to the first one.
pub fn next_thinking_level(supported: &[ThinkingLevel], level: ThinkingLevel) -> ThinkingLevel {
    let next = supported
        .iter()
        .position(|offered| *offered == level)
        .map_or(0, |index| (index + 1) % supported.len());
    supported.get(next).copied().unwrap_or(level)
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
    /// Whether asking this model to reason means anything.
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

    /// The levels this model can be asked for.
    pub fn thinking_levels(&self) -> Vec<ThinkingLevel> {
        supported_thinking_levels(self.reasoning, &self.compat.thinking)
    }

    /// The level this model uses when `level` is asked for.
    pub fn clamp_thinking(&self, level: ThinkingLevel) -> ThinkingLevel {
        clamp_thinking_level(&self.thinking_levels(), level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        entries
            .iter()
            .map(|(level, value)| (level.to_string(), value.map(str::to_string)))
            .collect()
    }

    #[test]
    fn a_model_that_does_not_reason_offers_only_off() {
        let levels = supported_thinking_levels(false, &map(&[("xhigh", Some("xhigh"))]));
        assert_eq!(levels, vec![ThinkingLevel::Off]);
    }

    #[test]
    fn xhigh_and_max_are_offered_only_when_the_map_names_them() {
        use ThinkingLevel::*;
        assert_eq!(
            supported_thinking_levels(true, &BTreeMap::new()),
            vec![Off, Minimal, Low, Medium, High]
        );
        assert_eq!(
            supported_thinking_levels(
                true,
                &map(&[
                    ("off", None),
                    ("xhigh", Some("xhigh")),
                    ("max", Some("max"))
                ])
            ),
            vec![Minimal, Low, Medium, High, XHigh, Max]
        );
    }

    #[test]
    fn a_level_marked_null_is_not_offered() {
        use ThinkingLevel::*;
        let levels = supported_thinking_levels(
            true,
            &map(&[("minimal", None), ("low", Some("LOW")), ("medium", None)]),
        );
        assert_eq!(levels, vec![Off, Low, High]);
    }

    #[test]
    fn an_unoffered_level_clamps_up_first_then_down() {
        use ThinkingLevel::*;
        let offered = [Low, High];
        assert_eq!(clamp_thinking_level(&offered, Medium), High);
        assert_eq!(clamp_thinking_level(&offered, Off), Low);
        assert_eq!(clamp_thinking_level(&offered, Max), High);
        assert_eq!(clamp_thinking_level(&offered, Low), Low);
        assert_eq!(clamp_thinking_level(&[], High), Off);
    }

    #[test]
    fn cycling_steps_through_the_offered_levels_only() {
        use ThinkingLevel::*;
        let offered = [Off, Low, High, XHigh];
        assert_eq!(next_thinking_level(&offered, Low), High);
        assert_eq!(next_thinking_level(&offered, XHigh), Off);
        assert_eq!(next_thinking_level(&offered, Medium), Off);
        assert_eq!(next_thinking_level(&[], Medium), Medium);
    }

    #[test]
    fn a_model_clamps_what_it_is_asked_for() {
        let mut model = Model::anthropic("claude-sonnet-4-5");
        assert_eq!(
            model.clamp_thinking(ThinkingLevel::High),
            ThinkingLevel::Off
        );
        model.reasoning = true;
        assert_eq!(
            model.clamp_thinking(ThinkingLevel::XHigh),
            ThinkingLevel::High
        );
        model
            .compat
            .thinking
            .insert("xhigh".into(), Some("xhigh".into()));
        assert_eq!(
            model.clamp_thinking(ThinkingLevel::XHigh),
            ThinkingLevel::XHigh
        );
    }

    #[test]
    fn every_level_is_found_by_its_wire_name() {
        for level in ThinkingLevel::ALL {
            assert_eq!(ThinkingLevel::named(level.as_str()), Some(level));
            assert_eq!(
                serde_json::to_value(level).unwrap(),
                serde_json::json!(level.as_str())
            );
        }
        assert_eq!(ThinkingLevel::named("none"), None);
    }
}
