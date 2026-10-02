use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// A provider-side sampling directive for a tool's arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConstrainedSampling {
    JsonSchema { strict: JsonSchemaStrictness },

    Grammar { variants: GrammarVariants },
}

impl ConstrainedSampling {
    /// What a tool's `constrainedSampling` value means, given how it crosses the boundary from an
    /// extension.
    pub fn from_wire(value: Option<Value>) -> Option<Self> {
        match value {
            None | Some(Value::Bool(false)) => None,
            Some(other) => serde_json::from_value(other).ok(),
        }
    }
}

/// How firmly [`ConstrainedSampling::JsonSchema`] is meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonSchemaStrictness {
    Prefer,

    Require,
}

/// A grammar constraint written in one or more provider-specific encodings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GrammarVariants {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_lark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_regex: Option<String>,
}

/// How a tool's call is scheduled against the other tool calls in the same turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

impl ToolExecutionMode {
    /// What a tool's `executionMode` string means, given how it crosses the boundary from an
    /// extension.
    pub fn from_wire(value: Option<&str>) -> Option<Self> {
        match value {
            Some("sequential") => Some(ToolExecutionMode::Sequential),
            Some("parallel") => Some(ToolExecutionMode::Parallel),
            _ => None,
        }
    }
}

/// How the model reaches a tool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolExposure {
    /// Declared to the model, and callable from other tools.
    #[default]
    Direct,
    /// Declared to the model, never callable from other tools: for tools that orchestrate other
    /// tools or ask the user.
    ModelOnly,
    /// Callable from other tools and listed by the `codemode` tool, but not declared to the model.
    Codemode,
    /// Like `Codemode`, but left out of the `codemode` listing: found by searching.
    Deferred,
    /// Registered but unreachable.
    Hidden,
}

impl ToolExposure {
    pub fn name(self) -> &'static str {
        match self {
            ToolExposure::Direct => "direct",
            ToolExposure::ModelOnly => "model-only",
            ToolExposure::Codemode => "codemode",
            ToolExposure::Deferred => "deferred",
            ToolExposure::Hidden => "hidden",
        }
    }

    /// What an exposure name means. `codemode-deferred` is another name for `codemode`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(ToolExposure::Direct),
            "model-only" => Some(ToolExposure::ModelOnly),
            "codemode" | "codemode-deferred" => Some(ToolExposure::Codemode),
            "deferred" => Some(ToolExposure::Deferred),
            "hidden" => Some(ToolExposure::Hidden),
            _ => None,
        }
    }

    /// Whether the model is given the tool's declaration while the tool is offered.
    pub fn is_declared(self) -> bool {
        matches!(self, ToolExposure::Direct | ToolExposure::ModelOnly)
    }

    /// Whether another tool, such as a `codemode` script, may call it.
    pub fn is_callable_from_tools(self) -> bool {
        matches!(
            self,
            ToolExposure::Direct | ToolExposure::Codemode | ToolExposure::Deferred
        )
    }

    /// Whether a tool is reached by searching rather than by its declaration.
    pub fn is_searchable(self) -> bool {
        matches!(self, ToolExposure::Codemode | ToolExposure::Deferred)
    }
}

/// A group of related tools, such as the tools of one MCP server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolNamespace {
    pub name: String,
    /// What the group offers, in a sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Longer guidance on using the group, read on request rather than listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Hints about what a tool does, with the meaning of MCP tool annotations. They are not verified.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

impl ToolAnnotations {
    /// Read the hints out of an object such as an MCP tool's `annotations`, ignoring anything
    /// that is not a boolean.
    pub fn from_wire(value: Option<&Value>) -> Option<Self> {
        let object = value?.as_object()?;
        let hint = |key: &str| object.get(key).and_then(Value::as_bool);
        let annotations = ToolAnnotations {
            read_only_hint: hint("readOnlyHint"),
            destructive_hint: hint("destructiveHint"),
            idempotent_hint: hint("idempotentHint"),
            open_world_hint: hint("openWorldHint"),
        };
        (annotations != ToolAnnotations::default()).then_some(annotations)
    }
}

/// How a call one tool made on behalf of another ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NestedCallStatus {
    Ok,
    Error,
    /// Still running when the calling tool finished, so it was cancelled.
    Unfinished,
}

/// One call a tool made while it ran, as recorded on the calling tool's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedToolCall {
    pub id: String,
    pub name: String,
    pub status: NestedCallStatus,
    /// Left out when they were too large to keep; `arguments_bytes` then says how large.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The start of the error text, for a call that failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The calls a tool made while it ran. `complete` is false when calls or arguments were left out
/// to keep the record bounded, or a call never finished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedToolCalls {
    pub calls: Vec<NestedToolCall>,
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicitly_disabling_constrained_sampling_is_the_same_as_omitting_it() {
        assert_eq!(ConstrainedSampling::from_wire(None), None);
        assert_eq!(
            ConstrainedSampling::from_wire(Some(Value::Bool(false))),
            None
        );
    }

    #[test]
    fn a_json_schema_config_parses_off_the_wire() {
        let value = serde_json::json!({ "type": "json_schema", "strict": "require" });
        assert_eq!(
            ConstrainedSampling::from_wire(Some(value)),
            Some(ConstrainedSampling::JsonSchema {
                strict: JsonSchemaStrictness::Require
            })
        );
    }

    #[test]
    fn a_grammar_config_parses_off_the_wire() {
        let value = serde_json::json!({
            "type": "grammar",
            "variants": { "openai_lark": "start: WORD" },
        });
        assert_eq!(
            ConstrainedSampling::from_wire(Some(value)),
            Some(ConstrainedSampling::Grammar {
                variants: GrammarVariants {
                    openai_lark: Some("start: WORD".to_string()),
                    openai_regex: None,
                }
            })
        );
    }

    #[test]
    fn an_unrecognized_shape_is_read_as_absent() {
        let value = serde_json::json!({ "type": "something_else" });
        assert_eq!(ConstrainedSampling::from_wire(Some(value)), None);
    }

    #[test]
    fn exposure_names_read_back_and_codemode_deferred_is_codemode() {
        for exposure in [
            ToolExposure::Direct,
            ToolExposure::ModelOnly,
            ToolExposure::Codemode,
            ToolExposure::Deferred,
            ToolExposure::Hidden,
        ] {
            assert_eq!(ToolExposure::parse(exposure.name()), Some(exposure));
            assert_eq!(
                serde_json::to_value(exposure).unwrap(),
                Value::String(exposure.name().to_string())
            );
        }
        assert_eq!(
            ToolExposure::parse("codemode-deferred"),
            Some(ToolExposure::Codemode)
        );
        assert_eq!(ToolExposure::parse("sometimes"), None);
    }

    #[test]
    fn only_direct_and_model_only_tools_are_declared() {
        assert!(ToolExposure::Direct.is_declared());
        assert!(ToolExposure::ModelOnly.is_declared());
        assert!(!ToolExposure::Codemode.is_declared());
        assert!(!ToolExposure::ModelOnly.is_callable_from_tools());
        assert!(!ToolExposure::Hidden.is_callable_from_tools());
        assert!(ToolExposure::Deferred.is_callable_from_tools());
    }

    #[test]
    fn annotations_read_only_the_boolean_hints() {
        let value =
            serde_json::json!({ "readOnlyHint": true, "openWorldHint": "yes", "title": "x" });
        assert_eq!(
            ToolAnnotations::from_wire(Some(&value)),
            Some(ToolAnnotations {
                read_only_hint: Some(true),
                ..ToolAnnotations::default()
            })
        );
        assert_eq!(
            ToolAnnotations::from_wire(Some(&serde_json::json!({}))),
            None
        );
        assert_eq!(ToolAnnotations::from_wire(None), None);
    }

    #[test]
    fn execution_mode_reads_its_two_names_and_nothing_else() {
        assert_eq!(
            ToolExecutionMode::from_wire(Some("sequential")),
            Some(ToolExecutionMode::Sequential)
        );
        assert_eq!(
            ToolExecutionMode::from_wire(Some("parallel")),
            Some(ToolExecutionMode::Parallel)
        );
        assert_eq!(ToolExecutionMode::from_wire(Some("concurrent")), None);
        assert_eq!(ToolExecutionMode::from_wire(None), None);
    }
}
