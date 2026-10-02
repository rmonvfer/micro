//! An extension's tool, as the model sees it.

use crate::host::Host;
use crate::host::RegisteredTool;
use micro_tools::Progress;
use micro_tools::Tool;
use micro_tools::ToolContext;
use micro_tools::ToolOutput;
use micro_types::ConstrainedSampling;
use micro_types::ContentBlock;
use micro_types::ToolAnnotations;
use micro_types::ToolDefinition;
use micro_types::ToolExecutionMode;
use micro_types::ToolExposure;
use micro_types::ToolNamespace;
use serde_json::Value;
use std::sync::Arc;

/// A tool an extension registered.
pub struct ExtensionTool {
    definition: ToolDefinition,
    /// How this tool's calls are scheduled against the rest of a turn's tool calls.
    execution_mode: Option<ToolExecutionMode>,
    exposure: ToolExposure,
    namespace: Option<ToolNamespace>,
    annotations: Option<ToolAnnotations>,
    output_schema: Option<Value>,
    host: Arc<Host>,
}

impl ExtensionTool {
    /// The tool as the host described its registration on the wire.
    pub fn new(registered: RegisteredTool, host: Arc<Host>) -> Self {
        ExtensionTool {
            definition: definition_for(
                registered.name,
                registered.description,
                registered.parameters,
                registered.constrained_sampling,
            ),
            execution_mode: ToolExecutionMode::from_wire(registered.execution_mode.as_deref()),
            exposure: registered
                .exposure
                .as_deref()
                .and_then(ToolExposure::parse)
                .unwrap_or_default(),
            namespace: namespace_from_wire(registered.namespace),
            annotations: ToolAnnotations::from_wire(registered.annotations.as_ref()),
            output_schema: registered
                .output_schema
                .filter(|schema| schema.is_object() || schema.is_boolean()),
            host,
        }
    }
}

/// A namespace as an extension wrote it, when it has a name.
fn namespace_from_wire(value: Option<Value>) -> Option<ToolNamespace> {
    let value = value?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    Some(ToolNamespace {
        name: text("name")?,
        description: text("description"),
        instructions: text("instructions"),
    })
}

/// How a registration is described to the model.
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

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn namespace(&self) -> Option<ToolNamespace> {
        self.namespace.clone()
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        self.annotations
    }

    fn output_schema(&self) -> Option<Value> {
        self.output_schema.clone()
    }

    /// A caller with nowhere to send progress still needs a result.
    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        self.execute_reporting(arguments, &Progress::default())
            .await
    }

    async fn execute_reporting(
        &self,
        arguments: &Value,
        progress: &Progress,
    ) -> Result<String, String> {
        let content = self.execute_content(arguments, progress).await?;
        Ok(content.iter().map(ContentBlock::as_text).collect())
    }

    async fn execute_content(
        &self,
        arguments: &Value,
        progress: &Progress,
    ) -> Result<Vec<ContentBlock>, String> {
        self.host
            .call_tool(&self.definition.name, arguments, progress)
            .await
    }

    async fn call(&self, arguments: &Value, context: &ToolContext<'_>) -> ToolOutput {
        self.host
            .run_tool(&self.definition.name, arguments, context)
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

    /// `false` is documented as equivalent to leaving `constrainedSampling` undefined, so both must
    /// produce the same definition.
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

    #[test]
    fn a_namespace_needs_a_name() {
        let named = namespace_from_wire(Some(serde_json::json!({
            "name": "deploys",
            "description": "Ship things",
            "instructions": "  ",
        })))
        .unwrap();
        assert_eq!(named.name, "deploys");
        assert_eq!(named.description.as_deref(), Some("Ship things"));
        assert_eq!(named.instructions, None);
        assert_eq!(
            namespace_from_wire(Some(serde_json::json!({ "description": "x" }))),
            None
        );
        assert_eq!(namespace_from_wire(None), None);
    }

    /// `ExtensionTool::new` reads `execution_mode` through [`ToolExecutionMode::from_wire`] with
    /// nothing in between.
    #[test]
    fn execution_mode_is_read_through_the_same_conversion_the_wire_uses() {
        assert_eq!(ToolExecutionMode::from_wire(None), None);
        assert_eq!(
            ToolExecutionMode::from_wire(Some("sequential")),
            Some(ToolExecutionMode::Sequential)
        );

        assert_eq!(ToolExecutionMode::from_wire(Some("eventually")), None);
    }
}
