//! What a session records beyond the conversation.

use crate::StopReason;
use crate::Usage;
use serde::de::Error as _;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde_json::Value;

/// The version every ledger line and every session sidecar carries.
pub const SCHEMA_VERSION: u32 = 1;

/// The name a piece of content is filed under: the hex sha-256 of its bytes.
pub fn content_hash(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Something that happened in a session which is not part of the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerEvent {
    /// One request to a provider, described completely enough to rebuild it.
    TurnRequest {
        turn: u64,
        provider: String,
        model: String,
        /// The rates in force when this request was issued.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pricing: Option<ModelPricing>,
        /// The hash of the system prompt and the tool definitions together, which is the part of a
        /// request a provider can cache.
        prefix_hash: String,

        request_hash: String,
        /// The exact serialized provider request body.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_body_blob: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        system_prompt_blob: Option<String>,
        tools_blob: String,
        /// The model as it was configured for this request.
        model_blob: String,
        #[serde(default)]
        prefix_spans: Vec<PrefixSpan>,
        /// The entries the conversation stood at, in the order the model was shown them.
        #[serde(default)]
        message_entry_ids: Vec<String>,
        /// Which try this was.
        attempt: u32,
    },
    /// What a turn was billed for, as the provider reported it.
    TurnUsage {
        turn: u64,
        usage: Usage,
        stop_reason: StopReason,
        provider: String,
        model: String,
    },
    /// A provider attempt ended without usage.
    RequestAttemptFailed {
        turn: u64,
        attempt: u32,
        error: String,
        #[serde(default)]
        usage_unknown: bool,
    },
    /// A stretch of the conversation replaced by a summary of it.
    Compaction {
        summary_blob: String,
        kept: usize,
        /// The branch path that was summarized.
        #[serde(default)]
        message_entry_ids: Vec<String>,
        #[serde(default)]
        cost: CompactionCost,
    },
    /// The conversation now continues from a different entry.
    HeadMoved { entry_id: String },
    /// A tool call something watching the run would not let happen.
    ToolDenied {
        tool: String,
        reason: String,
        source: EventSource,
    },
    /// What the sandbox allowed or refused, and under which policy.
    SandboxDecision {
        policy: String,
        operation: String,
        path_or_host: String,
        allowed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
    },
    /// An extension asking the host for something, and what it was told.
    ExtensionCrossing {
        extension: String,
        kind: String,
        name: String,
        allowed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<Value>,
    },
    /// The cacheable part of the request changed, and why.
    PrefixChanged {
        reason: String,
        from_hash: String,
        to_hash: String,
    },
    /// A run stopped because it had spent what it was allowed to.
    BudgetStop { limit: f64, spent: f64 },
    /// Anything else worth recording, for a fact that has not earned a kind of its own.
    Marker {
        #[serde(default)]
        data: Value,
    },
    /// A kind this build has never heard of, read from a log a later one wrote.
    #[serde(other)]
    Unknown,
}

/// What writing a summary cost, as the provider that wrote it reported.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactionCost {
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// The rates in force when the summary was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
}

/// Model rates captured when work was billed, in US dollars per million tokens.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<ModelPricingTier>,
}

/// Rates that replace the standard prices past an input-token threshold.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelPricingTier {
    pub input_tokens_above: u64,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrefixSpan {
    pub source: EventSource,
    pub bytes: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventSource {
    User,
    Model,
    SystemPrompt,
    ProjectInstructions,
    Skill(String),
    Extension(String),
    Tool(String),
    Compaction,
    Sandbox,
    Subagent(String),
}

impl EventSource {
    /// The kind on its own, for a reader grouping sources without caring which one.
    pub fn kind(&self) -> &'static str {
        match self {
            EventSource::User => "user",
            EventSource::Model => "model",
            EventSource::SystemPrompt => "system_prompt",
            EventSource::ProjectInstructions => "project_instructions",
            EventSource::Skill(_) => "skill",
            EventSource::Extension(_) => "extension",
            EventSource::Tool(_) => "tool",
            EventSource::Compaction => "compaction",
            EventSource::Sandbox => "sandbox",
            EventSource::Subagent(_) => "subagent",
        }
    }

    /// What this source is named in a log line.
    pub fn as_str(&self) -> String {
        match self {
            EventSource::Skill(name)
            | EventSource::Extension(name)
            | EventSource::Tool(name)
            | EventSource::Subagent(name)
                if !name.is_empty() =>
            {
                format!("{}:{name}", self.kind())
            }
            _ => self.kind().to_string(),
        }
    }
}

impl std::fmt::Display for EventSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.pad(&self.as_str())
    }
}

impl std::str::FromStr for EventSource {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (kind, name) = match text.split_once(':') {
            Some((kind, name)) => (kind, name.to_string()),
            None => (text, String::new()),
        };
        match kind {
            "user" => Ok(EventSource::User),
            "model" => Ok(EventSource::Model),
            "system_prompt" => Ok(EventSource::SystemPrompt),
            "project_instructions" => Ok(EventSource::ProjectInstructions),
            "skill" => Ok(EventSource::Skill(name)),
            "extension" => Ok(EventSource::Extension(name)),
            "tool" => Ok(EventSource::Tool(name)),
            "compaction" => Ok(EventSource::Compaction),
            "sandbox" => Ok(EventSource::Sandbox),
            "subagent" => Ok(EventSource::Subagent(name)),
            _ => Err(format!("unknown source: {text}")),
        }
    }
}

impl Serialize for EventSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for EventSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_is_written_as_its_kind_and_its_name() {
        let cases = [
            (EventSource::User, "user"),
            (EventSource::SystemPrompt, "system_prompt"),
            (EventSource::ProjectInstructions, "project_instructions"),
            (EventSource::Skill("review".into()), "skill:review"),
            (EventSource::Extension("deploy".into()), "extension:deploy"),
            (EventSource::Tool("bash".into()), "tool:bash"),
            (EventSource::Subagent("scout".into()), "subagent:scout"),
        ];
        for (source, written) in cases {
            let encoded = serde_json::to_string(&source).unwrap();
            assert_eq!(encoded, format!("\"{written}\""));
            assert_eq!(source, serde_json::from_str(&encoded).unwrap());
        }
    }

    #[test]
    fn a_source_with_no_name_is_written_as_its_kind_alone() {
        let source = EventSource::Skill(String::new());
        assert_eq!(serde_json::to_string(&source).unwrap(), "\"skill\"");
        assert_eq!(
            serde_json::from_str::<EventSource>("\"skill\"").unwrap(),
            source
        );
    }

    #[test]
    fn a_source_nobody_has_heard_of_is_refused() {
        assert!(serde_json::from_str::<EventSource>("\"telepathy\"").is_err());
    }

    #[test]
    fn a_turn_request_round_trips_through_its_tag() {
        let event = LedgerEvent::TurnRequest {
            turn: 2,
            provider: "openai".into(),
            model: "test-model".into(),
            pricing: Some(ModelPricing {
                input: 1.25,
                output: 5.0,
                ..Default::default()
            }),
            prefix_hash: "aa".into(),
            request_hash: "bb".into(),
            request_body_blob: Some("body".into()),
            system_prompt_blob: Some("cc".into()),
            tools_blob: "dd".into(),
            model_blob: "ee".into(),
            prefix_spans: vec![PrefixSpan {
                source: EventSource::ProjectInstructions,
                bytes: 12,
                hash: "ff".into(),
            }],
            message_entry_ids: vec!["1".into(), "2".into()],
            attempt: 1,
        };
        let encoded = serde_json::to_string(&event).unwrap();
        assert!(encoded.contains("\"type\":\"turn_request\""));
        assert!(encoded.contains("\"pricing\""));
        assert!(encoded.contains("\"source\":\"project_instructions\""));
        assert_eq!(event, serde_json::from_str(&encoded).unwrap());
    }

    #[test]
    fn ledger_pricing_is_optional_for_sessions_written_before_it_was_recorded() {
        let event: LedgerEvent = serde_json::from_str(
            r#"{
                "type": "turn_request",
                "turn": 1,
                "provider": "anthropic",
                "model": "claude",
                "prefix_hash": "aa",
                "request_hash": "bb",
                "tools_blob": "cc",
                "model_blob": "dd",
                "attempt": 1
            }"#,
        )
        .unwrap();
        let LedgerEvent::TurnRequest { pricing, .. } = event else {
            panic!("expected a turn request");
        };
        assert_eq!(pricing, None);

        let cost: CompactionCost = serde_json::from_str(
            r#"{
                "usage": { "input": 10, "output": 2, "cache_read": 0, "cache_write": 0 },
                "provider": "anthropic",
                "model": "claude"
            }"#,
        )
        .unwrap();
        assert_eq!(cost.pricing, None);
    }

    #[test]
    fn every_event_round_trips_through_its_tag() {
        let events = vec![
            LedgerEvent::TurnUsage {
                turn: 1,
                usage: Usage {
                    input: 10,
                    output: 20,
                    cache_read: 30,
                    cache_write: 40,
                },
                stop_reason: StopReason::ToolUse,
                provider: "anthropic".into(),
                model: "claude-opus-5".into(),
            },
            LedgerEvent::Compaction {
                summary_blob: "aa".into(),
                kept: 4,
                message_entry_ids: Vec::new(),
                cost: CompactionCost {
                    usage: Usage {
                        input: 900,
                        output: 120,
                        cache_read: 0,
                        cache_write: 0,
                    },
                    provider: "anthropic".into(),
                    model: "claude-opus-5".into(),
                    pricing: None,
                },
            },
            LedgerEvent::HeadMoved {
                entry_id: "3".into(),
            },
            LedgerEvent::ToolDenied {
                tool: "bash".into(),
                reason: "not while the deploy is running".into(),
                source: EventSource::Extension("guard".into()),
            },
            LedgerEvent::SandboxDecision {
                policy: "workspace-write".into(),
                operation: "write".into(),
                path_or_host: "/etc/hosts".into(),
                allowed: false,
                tool_call_id: Some("call_1".into()),
            },
            LedgerEvent::ExtensionCrossing {
                extension: "deploy".into(),
                kind: "exec".into(),
                name: "git".into(),
                allowed: true,
                detail: None,
            },
            LedgerEvent::PrefixChanged {
                reason: "reload".into(),
                from_hash: "aa".into(),
                to_hash: "bb".into(),
            },
            LedgerEvent::BudgetStop {
                limit: 5.0,
                spent: 5.2,
            },
            LedgerEvent::Marker {
                data: serde_json::json!({ "note": "sandbox off" }),
            },
        ];

        for event in events {
            let encoded = serde_json::to_string(&event).unwrap();
            assert_eq!(
                event,
                serde_json::from_str(&encoded).unwrap(),
                "round trip of {encoded}"
            );
        }
    }

    /// A compaction recorded before anyone priced one says nothing about what it cost.
    #[test]
    fn a_compaction_recorded_without_a_price_still_reads() {
        let line = r#"{"type":"compaction","summary_blob":"aa","kept":4}"#;
        assert_eq!(
            serde_json::from_str::<LedgerEvent>(line).unwrap(),
            LedgerEvent::Compaction {
                summary_blob: "aa".into(),
                kept: 4,
                message_entry_ids: Vec::new(),
                cost: CompactionCost::default(),
            }
        );
    }

    /// A log written by a later build carries kinds this one has never seen.
    #[test]
    fn an_event_kind_from_a_later_build_still_reads() {
        let line = r#"{"type":"quantum_entanglement","spooky":true}"#;
        assert_eq!(
            serde_json::from_str::<LedgerEvent>(line).unwrap(),
            LedgerEvent::Unknown
        );
    }
}
