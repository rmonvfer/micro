//! An extension's tool, as the model sees it.
//!
//! Nothing about it is special once it is registered: it goes through the same policy as
//! every built-in tool, and it is described to the model the same way. The difference is
//! only where it runs — in the host process, on the other end of a pipe — which is why
//! every path here ends in [`Host::call_tool`] rather than in a computation of its own.

use crate::host::Host;
use micro_tools::Progress;
use micro_tools::Tool;
use micro_types::ConstrainedSampling;
use micro_types::ContentBlock;
use micro_types::ToolDefinition;
use micro_types::ToolExecutionMode;
use serde_json::Value;
use std::sync::Arc;

/// A tool an extension registered.
pub struct ExtensionTool {
    definition: ToolDefinition,
    /// How this tool's calls are scheduled against the rest of a turn's tool calls. Kept
    /// alongside the definition rather than inside it: this is the agent loop's concern,
    /// not something a provider is ever told.
    execution_mode: Option<ToolExecutionMode>,
    host: Arc<Host>,
}

impl ExtensionTool {
    /// `constrained_sampling` and `execution_mode` arrive as the host describes them on the
    /// wire — a raw JSON value and a raw string, both `None`/absent when a tool said
    /// nothing — and are read into their typed forms here.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
        constrained_sampling: Option<Value>,
        execution_mode: Option<String>,
        host: Arc<Host>,
    ) -> Self {
        ExtensionTool {
            definition: definition_for(name, description, parameters, constrained_sampling),
            execution_mode: ToolExecutionMode::from_wire(execution_mode.as_deref()),
            host,
        }
    }
}

/// How a registration is described to the model.
///
/// A tool that describes no parameters still has to describe an object: a provider that
/// meets anything else rejects the whole request rather than the one tool.
fn definition_for(
    name: impl Into<String>,
    description: impl Into<String>,
    parameters: Value,
    constrained_sampling: Option<Value>,
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        parameters: match parameters.is_object() {
            true => parameters,
            false => serde_json::json!({ "type": "object", "properties": {} }),
        },
        constrained_sampling: ConstrainedSampling::from_wire(constrained_sampling),
    }
}

#[async_trait::async_trait]
impl Tool for ExtensionTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        self.execution_mode
    }

    /// A caller with nowhere to send progress still needs a result, so this asks for one
    /// with nothing wired to hear about it along the way — [`Progress::default`] drops
    /// whatever the extension reports rather than holding a call open for a listener that
    /// was never there.
    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        self.execute_reporting(arguments, &Progress::default()).await
    }

    async fn execute_reporting(&self, arguments: &Value, progress: &Progress) -> Result<String, String> {
        let content = self.execute_content(arguments, progress).await?;
        Ok(content.iter().map(ContentBlock::as_text).collect())
    }

    /// Where the real work happens: an image an extension hands back arrives here as an
    /// actual [`ContentBlock::Image`] rather than as text describing one, the same way a
    /// built-in tool like `read` returns a screenshot.
    async fn execute_content(&self, arguments: &Value, progress: &Progress) -> Result<Vec<ContentBlock>, String> {
        self.host
            .call_tool(&self.definition.name, arguments, progress)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_without_a_schema_still_describes_an_object() {
        let definition = definition_for("greet", "say hello", Value::Null, None);
        assert_eq!(definition.name, "greet");
        assert_eq!(definition.description, "say hello");
        assert_eq!(definition.parameters["type"], "object");
        assert!(definition.parameters["properties"].is_object());
    }

    #[test]
    fn a_schema_an_extension_wrote_is_passed_through_as_it_stands() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "who": { "type": "string" } },
            "required": ["who"],
        });
        let definition = definition_for("greet", "say hello", schema.clone(), None);
        assert_eq!(definition.parameters, schema);
    }

    /// `false` is documented as equivalent to leaving `constrainedSampling` undefined, so
    /// both must produce the same definition.
    #[test]
    fn constrained_sampling_false_is_read_the_same_as_absent() {
        let omitted = definition_for("greet", "say hello", Value::Null, None);
        let disabled = definition_for("greet", "say hello", Value::Null, Some(Value::Bool(false)));
        assert_eq!(omitted.constrained_sampling, None);
        assert_eq!(disabled.constrained_sampling, None);
    }

    #[test]
    fn a_constrained_sampling_config_reaches_the_definition() {
        let config = serde_json::json!({ "type": "json_schema", "strict": "prefer" });
        let definition = definition_for("greet", "say hello", Value::Null, Some(config));
        assert_eq!(
            definition.constrained_sampling,
            Some(ConstrainedSampling::JsonSchema {
                strict: micro_types::JsonSchemaStrictness::Prefer
            })
        );
    }

    /// `ExtensionTool::new` reads `execution_mode` through [`ToolExecutionMode::from_wire`]
    /// with nothing in between, so what that conversion does is exercised directly rather
    /// than through a live `Host` — starting one means an actual `bun` process, which a
    /// unit test has no business depending on.
    #[test]
    fn execution_mode_is_read_through_the_same_conversion_the_wire_uses() {
        assert_eq!(ToolExecutionMode::from_wire(None), None);
        assert_eq!(
            ToolExecutionMode::from_wire(Some("sequential")),
            Some(ToolExecutionMode::Sequential)
        );
        // A name pi does not define is read the same as the field being absent, rather
        // than failing registration over it.
        assert_eq!(ToolExecutionMode::from_wire(Some("eventually")), None);
    }
}
