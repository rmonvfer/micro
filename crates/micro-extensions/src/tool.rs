//! An extension's tool, as the model sees it.
//!
//! Nothing about it is special once it is registered: it goes through the same policy as
//! every built-in tool, it is described to the model the same way, and it returns text.
//! The difference is only where it runs — in the host process, on the other end of a pipe.

use crate::host::Host;
use micro_tools::Tool;
use micro_types::ToolDefinition;
use serde_json::Value;
use std::sync::Arc;

/// A tool an extension registered.
pub struct ExtensionTool {
    definition: ToolDefinition,
    host: Arc<Host>,
}

impl ExtensionTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
        host: Arc<Host>,
    ) -> Self {
        ExtensionTool {
            definition: definition_for(name, description, parameters),
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
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        parameters: match parameters.is_object() {
            true => parameters,
            false => serde_json::json!({ "type": "object", "properties": {} }),
        },
    }
}

#[async_trait::async_trait]
impl Tool for ExtensionTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        self.host.call_tool(&self.definition.name, arguments).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_without_a_schema_still_describes_an_object() {
        let definition = definition_for("greet", "say hello", Value::Null);
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
        let definition = definition_for("greet", "say hello", schema.clone());
        assert_eq!(definition.parameters, schema);
    }
}
