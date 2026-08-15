//! Turning a tool's [`ConstrainedSampling`] request into what a provider is actually sent.

use micro_types::ConstrainedSampling;
use micro_types::JsonSchemaStrictness;
use micro_types::ToolDefinition;
use serde_json::Value;

/// JSON Schema keywords a strict schema cannot express.
const UNSUPPORTED_STRICT_SCHEMA_KEYS: &[&str] = &[
    "$ref",
    "$defs",
    "definitions",
    "allOf",
    "oneOf",
    "patternProperties",
    "dependentSchemas",
    "dependencies",
    "unevaluatedProperties",
    "propertyNames",
    "contains",
    "prefixItems",
    "not",
    "if",
    "then",
    "else",
];

fn is_structured_schema(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    let names_type = |value: &Value| -> bool {
        match value {
            Value::String(name) => name == "object" || name == "array",
            Value::Array(items) => items.iter().any(|item| {
                item.as_str()
                    .is_some_and(|name| name == "object" || name == "array")
            }),
            _ => false,
        }
    };
    object.get("type").is_some_and(names_type)
        || object.contains_key("properties")
        || object.contains_key("items")
}

fn schema_allows_null(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    let type_says_null = match object.get("type") {
        Some(Value::String(name)) => name == "null",
        Some(Value::Array(items)) => items.iter().any(|item| item.as_str() == Some("null")),
        _ => false,
    };
    if type_says_null {
        return true;
    }
    if object.get("const") == Some(&Value::Null) {
        return true;
    }
    if let Some(Value::Array(variants)) = object.get("enum") {
        if variants.iter().any(Value::is_null) {
            return true;
        }
    }
    if let Some(Value::Array(variants)) = object.get("anyOf") {
        return variants.iter().any(schema_allows_null);
    }
    false
}


fn make_node_strict(schema: &mut Value) -> Result<(), String> {
    let Some(object) = schema.as_object_mut() else {
        return Err("boolean schemas are unsupported".to_string());
    };

    for key in UNSUPPORTED_STRICT_SCHEMA_KEYS {
        if object.contains_key(*key) {
            return Err(format!("{key} schemas are unsupported"));
        }
    }

    if let Some(any_of) = object.get_mut("anyOf") {
        let variants = any_of
            .as_array_mut()
            .ok_or_else(|| "anyOf must contain at least one schema".to_string())?;
        if variants.is_empty() {
            return Err("anyOf must contain at least one schema".to_string());
        }
        for variant in variants.iter_mut() {
            if is_structured_schema(variant) {
                return Err("object and array unions are unsupported".to_string());
            }
            make_node_strict(variant)?;
        }
    }

    if let Some(items) = object.get_mut("items") {
        if items.is_array() {
            return Err("tuple schemas are unsupported".to_string());
        }
        make_node_strict(items)?;
    }

    let is_object_schema = object.get("type").and_then(Value::as_str) == Some("object");
    if object.contains_key("properties") && !is_object_schema {
        return Err("properties require type object".to_string());
    }
    if !is_object_schema {
        return Ok(());
    }

    match object.get("additionalProperties") {
        None | Some(Value::Bool(false)) => {}
        Some(_) => {
            return Err("schema-valued or true additionalProperties is unsupported".to_string())
        }
    }
    if let Some(properties) = object.get("properties") {
        if !properties.is_object() {
            return Err("object properties must be a schema map".to_string());
        }
    }

    let required: Vec<String> = match object.get("required") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "object required must be a string array".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("object required must be a string array".to_string()),
    };

    let property_names: Vec<String> = object
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default();
    for name in &required {
        if !property_names.contains(name) {
            return Err("required contains an unknown property".to_string());
        }
    }

    if let Some(Value::Object(properties)) = object.get_mut("properties") {
        for (name, property) in properties.iter_mut() {
            make_node_strict(property)?;
            if !required.contains(name) && !schema_allows_null(property) {
                let original = property.clone();
                *property = serde_json::json!({ "anyOf": [original, { "type": "null" }] });
            }
        }
    }

    object.insert("required".to_string(), serde_json::json!(property_names));
    object.insert("additionalProperties".to_string(), Value::Bool(false));

    Ok(())
}


pub fn make_strict_json_schema(schema: &Value) -> Result<Value, String> {
    let mut cloned = schema.clone();
    if !cloned.is_object() {
        return Err("root schema must have type object".to_string());
    }
    make_node_strict(&mut cloned)?;
    if cloned.get("type").and_then(Value::as_str) != Some("object") {
        return Err("root schema must have type object".to_string());
    }
    Ok(cloned)
}

/// Whether a tool's call should be requested under strict JSON-schema sampling, and what to do when
/// it cannot be.
pub fn resolve_json_schema_strict_sampling(
    tool: &ToolDefinition,
    supports_strict_mode: bool,
) -> Result<Option<bool>, String> {
    let Some(ConstrainedSampling::JsonSchema { strict }) = &tool.constrained_sampling else {
        return Ok(None);
    };

    if supports_strict_mode {
        return match make_strict_json_schema(&tool.parameters) {
            Ok(_) => Ok(Some(true)),
            Err(message) => match strict {
                JsonSchemaStrictness::Prefer => Ok(None),
                JsonSchemaStrictness::Require => Err(format!(
                    "Tool \"{}\" requires JSON-schema constrained sampling, but {message}.",
                    tool.name
                )),
            },
        };
    }

    match strict {
        JsonSchemaStrictness::Prefer => Ok(None),
        JsonSchemaStrictness::Require => Err(format!(
            "Tool \"{}\" requires JSON-schema constrained sampling, but strict tools are unsupported.",
            tool.name
        )),
    }
}


pub fn json_schema_tool_parameters(
    tool: &ToolDefinition,
    strict: Option<bool>,
) -> Result<Value, String> {
    match strict {
        Some(true) => make_strict_json_schema(&tool.parameters),
        _ => Ok(tool.parameters.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::GrammarVariants;
    use serde_json::json;

    fn tool(
        parameters: Value,
        constrained_sampling: Option<ConstrainedSampling>,
    ) -> ToolDefinition {
        ToolDefinition {
            name: "grep".to_string(),
            description: "search".to_string(),
            parameters,
            constrained_sampling,
        }
    }

    #[test]
    fn a_simple_object_schema_becomes_strict() {
        let schema = json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "limit": { "type": "number" },
            },
            "required": ["pattern"],
        });

        let strict = make_strict_json_schema(&schema).unwrap();

        assert_eq!(strict["additionalProperties"], json!(false));
        let required: std::collections::BTreeSet<&str> = strict["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert_eq!(
            required,
            std::collections::BTreeSet::from(["pattern", "limit"]),
            "every property becomes required, in whatever order the schema holds them"
        );
        
        assert_eq!(
            strict["properties"]["limit"],
            json!({ "anyOf": [{ "type": "number" }, { "type": "null" }] })
        );
        
        assert_eq!(strict["properties"]["pattern"], json!({ "type": "string" }));
    }

    #[test]
    fn a_property_that_already_allows_null_is_not_wrapped_again() {
        let schema = json!({
            "type": "object",
            "properties": { "note": { "type": ["string", "null"] } },
            "required": [],
        });

        let strict = make_strict_json_schema(&schema).unwrap();

        assert_eq!(
            strict["properties"]["note"],
            json!({ "type": ["string", "null"] })
        );
    }

    #[test]
    fn nested_object_schemas_are_made_strict_recursively() {
        let schema = json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                },
            },
            "required": ["target"],
        });

        let strict = make_strict_json_schema(&schema).unwrap();

        assert_eq!(
            strict["properties"]["target"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn a_ref_makes_a_schema_impossible_to_strictify() {
        let schema = json!({
            "type": "object",
            "properties": { "target": { "$ref": "#/$defs/target" } },
            "required": ["target"],
        });

        let error = make_strict_json_schema(&schema).unwrap_err();
        assert!(error.contains("$ref"), "got {error:?}");
    }

    #[test]
    fn a_root_schema_that_is_not_an_object_is_rejected() {
        let error = make_strict_json_schema(&json!({ "type": "string" })).unwrap_err();
        assert!(error.contains("root schema"), "got {error:?}");
    }

    #[test]
    fn no_config_leaves_sampling_untouched() {
        let schema = json!({ "type": "object", "properties": {} });
        let plain = tool(schema, None);
        assert_eq!(resolve_json_schema_strict_sampling(&plain, true), Ok(None));
    }

    /// A grammar request is a different mechanism than JSON-schema strict sampling, and is resolved
    /// by nothing in this crate.
    #[test]
    fn a_grammar_config_leaves_json_schema_sampling_untouched() {
        let schema = json!({ "type": "object", "properties": {} });
        let grammar = tool(
            schema,
            Some(ConstrainedSampling::Grammar {
                variants: GrammarVariants {
                    openai_lark: Some("start: WORD".to_string()),
                    openai_regex: None,
                },
            }),
        );
        assert_eq!(
            resolve_json_schema_strict_sampling(&grammar, true),
            Ok(None)
        );
    }

    #[test]
    fn preferring_strict_sampling_resolves_true_when_the_schema_and_provider_allow_it() {
        let schema = json!({ "type": "object", "properties": { "n": { "type": "number" } } });
        let prefer = tool(
            schema,
            Some(ConstrainedSampling::JsonSchema {
                strict: JsonSchemaStrictness::Prefer,
            }),
        );
        assert_eq!(
            resolve_json_schema_strict_sampling(&prefer, true),
            Ok(Some(true))
        );
    }

    #[test]
    fn preferring_strict_sampling_falls_back_silently_when_the_schema_cannot_be_strict() {
        let schema = json!({
            "type": "object",
            "properties": { "target": { "$ref": "#/$defs/target" } },
        });
        let prefer = tool(
            schema,
            Some(ConstrainedSampling::JsonSchema {
                strict: JsonSchemaStrictness::Prefer,
            }),
        );
        assert_eq!(resolve_json_schema_strict_sampling(&prefer, true), Ok(None));
    }

    #[test]
    fn preferring_strict_sampling_falls_back_silently_when_the_provider_does_not_support_it() {
        let schema = json!({ "type": "object", "properties": {} });
        let prefer = tool(
            schema,
            Some(ConstrainedSampling::JsonSchema {
                strict: JsonSchemaStrictness::Prefer,
            }),
        );
        assert_eq!(
            resolve_json_schema_strict_sampling(&prefer, false),
            Ok(None)
        );
    }

    #[test]
    fn requiring_strict_sampling_fails_when_the_schema_cannot_be_strict() {
        let schema = json!({
            "type": "object",
            "properties": { "target": { "$ref": "#/$defs/target" } },
        });
        let require = tool(
            schema,
            Some(ConstrainedSampling::JsonSchema {
                strict: JsonSchemaStrictness::Require,
            }),
        );
        let error = resolve_json_schema_strict_sampling(&require, true).unwrap_err();
        assert!(error.contains("\"grep\""), "got {error:?}");
        assert!(error.contains("$ref"), "got {error:?}");
    }

    #[test]
    fn requiring_strict_sampling_fails_when_the_provider_does_not_support_it() {
        let schema = json!({ "type": "object", "properties": {} });
        let require = tool(
            schema,
            Some(ConstrainedSampling::JsonSchema {
                strict: JsonSchemaStrictness::Require,
            }),
        );
        let error = resolve_json_schema_strict_sampling(&require, false).unwrap_err();
        assert!(error.contains("unsupported"), "got {error:?}");
    }

    #[test]
    fn parameters_are_only_rewritten_when_strict_resolved_to_true() {
        let schema = json!({
            "type": "object",
            "properties": { "n": { "type": "number" } },
        });
        let plain = tool(schema.clone(), None);
        assert_eq!(
            json_schema_tool_parameters(&plain, None).unwrap(),
            schema,
            "no strict resolution leaves the schema exactly as the tool wrote it"
        );
        assert_eq!(
            json_schema_tool_parameters(&plain, Some(false)).unwrap()["additionalProperties"],
            Value::Null,
            "strict resolving to false is not the same as it resolving to true"
        );
        let strict = json_schema_tool_parameters(&plain, Some(true)).unwrap();
        assert_eq!(strict["additionalProperties"], json!(false));
    }
}
