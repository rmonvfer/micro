

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
