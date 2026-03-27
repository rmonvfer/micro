//! What a tool can ask for beyond its name, description and schema: a provider-side
//! sampling directive, and how its calls are scheduled against the rest of a turn.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// A provider-side sampling directive for a tool's arguments.
///
/// Absent by default: most tools take whatever shape the model sends and let a provider's
/// ordinary sampling produce it. A tool asks for this to have a provider hold its call
/// closer to the schema than ordinary sampling guarantees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConstrainedSampling {
    /// Ask a provider that supports it to keep a tool call's arguments to the tool's JSON
    /// Schema, rather than to whatever shape ordinary sampling happens to produce.
    JsonSchema { strict: JsonSchemaStrictness },
    /// Ask a provider that supports it to keep one string argument inside a grammar, in
    /// whichever provider-specific encoding was supplied.
    Grammar { variants: GrammarVariants },
}

impl ConstrainedSampling {
    /// What a tool's `constrainedSampling` value means, given how it crosses the boundary
    /// from an extension.
    ///
    /// The value there is `false | ConstrainedSamplingConfig`, and `false` is documented as
    /// "equivalent to leaving it undefined" — a tool that explicitly opts out reads the
    /// same as one that never mentioned the field. Both collapse to `None` here. A shape
    /// that is present but does not parse as either variant is also read as `None`: a
    /// malformed extra field a tool sent is not reason to fail the tool's registration, and
    /// there is nothing sensible to constrain sampling with in that case anyway.
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
    /// Use strict sampling where the provider and the schema allow it; say nothing where
    /// they do not, and let the tool run under ordinary sampling instead.
    Prefer,
    /// Use strict sampling, or refuse to offer the tool: the caller is telling a provider
    /// this must hold rather than merely asking for it.
    Require,
}

/// A grammar constraint written in one or more provider-specific encodings.
///
/// Empty until a caller fills in the formats it targets — there is no single grammar
/// language every provider that offers this speaks, so a tool supplies whichever encodings
/// it has and a provider takes the one it understands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GrammarVariants {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_lark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_regex: Option<String>,
}

/// How a tool's call is scheduled against the other tool calls in the same turn.
///
/// - `Sequential`: this tool must run alone — nothing else from the same turn starts until
///   it finishes.
/// - `Parallel`: this tool may run alongside other tool calls from the same turn.
///
/// A tool with no opinion asks for neither; the turn's own default decides, which is
/// `Parallel` unless something running it says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

impl ToolExecutionMode {
    /// What a tool's `executionMode` string means, given how it crosses the boundary from
    /// an extension. Anything other than the two names pi defines — including the field
    /// being absent — is read as no opinion, so the turn's own default applies.
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

    /// A shape neither variant recognizes is dropped rather than failing tool registration
    /// over it.
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
