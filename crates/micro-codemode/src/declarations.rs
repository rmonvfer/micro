//! TypeScript declarations of what a script can call, rendered from JSON Schemas.

use crate::identifier::identifier;
use crate::identifier::is_identifier;
use serde_json::Value;
use std::collections::BTreeSet;
use std::collections::HashSet;

const INDENT: &str = "  ";
/// The largest rendered input type, in characters, before it becomes `unknown`.
const INPUT_MAX_CHARS: usize = 16_000;
/// Local `$ref` expansions per rendered schema, so shared definitions cannot blow up the output.
const MAX_REF_EXPANSIONS: usize = 32;

/// TypeScript types for MCP results, from the MCP `CallToolResult` schema, so `CallToolResult<T>`
/// declarations can refer to them.
pub const MCP_TYPESCRIPT_PREAMBLE: &str = r#"type Role = "user" | "assistant";
type MetaObject = Record<string, unknown>;
type Annotations = {
  audience?: Role[];
  priority?: number;
  lastModified?: string;
};
type TextResourceContents = {
  uri: string;
  mimeType?: string;
  _meta?: MetaObject;
  text: string;
};
type BlobResourceContents = {
  uri: string;
  mimeType?: string;
  _meta?: MetaObject;
  blob: string;
};
type TextContent = {
  type: "text";
  text: string;
  annotations?: Annotations;
  _meta?: MetaObject;
};
type ImageContent = {
  type: "image";
  data: string;
  mimeType: string;
  annotations?: Annotations;
  _meta?: MetaObject;
};
type AudioContent = {
  type: "audio";
  data: string;
  mimeType: string;
  annotations?: Annotations;
  _meta?: MetaObject;
};
type ResourceLink = {
  name: string;
  title?: string;
  uri: string;
  description?: string;
  mimeType?: string;
  annotations?: Annotations;
  size?: number;
  _meta?: MetaObject;
  type: "resource_link";
};
type EmbeddedResource = {
  type: "resource";
  resource: TextResourceContents | BlobResourceContents;
  annotations?: Annotations;
  _meta?: MetaObject;
};
type ContentBlock =
  | TextContent
  | ImageContent
  | AudioContent
  | ResourceLink
  | EmbeddedResource;
type CallToolResult<TStructured = { [key: string]: unknown }> = {
  _meta?: MetaObject;
  content: ContentBlock[];
  isError?: boolean;
  structuredContent?: TStructured;
  [key: string]: unknown;
};"#;

/// What a script sees of a tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub description: String,
    pub input_schema: Option<Value>,
    /// The type of what a call resolves to; `unknown` when absent.
    pub output_schema: Option<Value>,
}

/// One tool as a member of the `tools` object: `name(args: T): Promise<R>;`, named by the
/// identifier scripts use.
pub fn render_signature(declaration: &Declaration) -> String {
    let input = match &declaration.input_schema {
        None => "unknown".to_string(),
        Some(schema) => schema_to_type(schema, Some(INPUT_MAX_CHARS)),
    };
    format!(
        "{}(args: {input}): Promise<{}>;",
        identifier(&declaration.name),
        render_output_type(declaration.output_schema.as_ref())
    )
}

/// A tool's description followed by its declaration, as tool listings and `ALL_TOOLS` show it.
pub fn render_sample(declaration: &Declaration) -> String {
    format!(
        "{}\n\ncodemode tool declaration:\n```ts\ndeclare const tools: {{ {} }};\n```",
        declaration.description.trim(),
        render_signature(declaration)
    )
}

/// The `structuredContent` schema of an MCP `CallToolResult` output schema (recognized by a
/// `content` array of objects, a boolean `isError`, and an object `_meta`), `true` when it
/// declares none, or `None` when the schema is not a `CallToolResult`.
pub fn mcp_structured_content_schema(schema: Option<&Value>) -> Option<Value> {
    let properties = schema?.get("properties")?.as_object()?;
    let content = properties.get("content")?;
    let is_array_of_objects = content.get("type")? == "array"
        && content.get("items").and_then(|items| items.get("type"))
            == Some(&Value::String("object".into()));
    let is_error = properties.get("isError")?.get("type")? == "boolean";
    let meta = properties.get("_meta")?.get("type")? == "object";
    if !(is_array_of_objects && is_error && meta) {
        return None;
    }
    Some(match properties.get("structuredContent") {
        Some(structured @ (Value::Object(_) | Value::Bool(_))) => structured.clone(),
        _ => Value::Bool(true),
    })
}

/// The type a call resolves to: `CallToolResult<T>` for MCP output schemas (which needs
/// [`MCP_TYPESCRIPT_PREAMBLE`]), the schema's type otherwise, `unknown` without a schema.
pub fn render_output_type(schema: Option<&Value>) -> String {
    if let Some(structured) = mcp_structured_content_schema(schema) {
        return match schema_to_type(&structured, None).as_str() {
            "unknown" => "CallToolResult".to_string(),
            rendered => format!("CallToolResult<{rendered}>"),
        };
    }
    match schema {
        None => "unknown".to_string(),
        Some(schema) => schema_to_type(schema, None),
    }
}

/// A JSON Schema as a TypeScript type expression. Objects render on one line with their
/// properties sorted by name, or one property per line with `//` comments when a property has a
/// description. Local references resolve against `schema`; recursive and remote ones render as
/// `unknown`, as does a result longer than `max_chars`.
pub fn schema_to_type(schema: &Value, max_chars: Option<usize>) -> String {
    let mut context = SchemaContext {
        root: schema,
        resolving: HashSet::new(),
        expansions: 0,
    };
    let rendered = to_type(schema, &mut context);
    match max_chars {
        Some(max) if rendered.chars().count() > max => "unknown".to_string(),
        _ => rendered,
    }
}

struct SchemaContext<'a> {
    root: &'a Value,
    /// References being expanded on the current path, to stop at recursive types.
    resolving: HashSet<String>,
    expansions: usize,
}

fn resolve_ref<'a>(reference: &str, root: &'a Value) -> Option<&'a Value> {
    if reference != "#" && !reference.starts_with("#/") {
        return None;
    }
    let mut current = root;
    for segment in reference
        .get(2..)
        .unwrap_or_default()
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        let key = percent_decode(segment)
            .replace("~1", "/")
            .replace("~0", "~");
        current = current.as_object()?.get(&key)?;
    }
    matches!(current, Value::Object(_) | Value::Bool(_)).then_some(current)
}

fn percent_decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Some(byte) = segment
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn json_literal(value: &Value) -> String {
    value.to_string()
}

fn union(types: Vec<String>) -> String {
    let mut unique: Vec<String> = Vec::new();
    for rendered in types {
        if !unique.contains(&rendered) {
            unique.push(rendered);
        }
    }
    if unique.iter().any(|rendered| rendered == "unknown") {
        return "unknown".to_string();
    }
    match unique.is_empty() {
        true => "never".to_string(),
        false => unique.join(" | "),
    }
}

fn to_type(schema: &Value, context: &mut SchemaContext<'_>) -> String {
    let object = match schema {
        Value::Bool(true) => return "unknown".to_string(),
        Value::Bool(false) => return "never".to_string(),
        Value::Object(object) => object,
        _ => return "unknown".to_string(),
    };

    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        if context.resolving.contains(reference) || context.expansions >= MAX_REF_EXPANSIONS {
            return "unknown".to_string();
        }
        let Some(target) = resolve_ref(reference, context.root) else {
            return "unknown".to_string();
        };
        context.expansions += 1;
        context.resolving.insert(reference.to_string());
        let rendered = to_type(target, context);
        context.resolving.remove(reference);
        return rendered;
    }

    if let Some(constant) = object.get("const") {
        return json_literal(constant);
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        return union(values.iter().map(json_literal).collect());
    }

    let variants = object
        .get("anyOf")
        .and_then(Value::as_array)
        .or_else(|| object.get("oneOf").and_then(Value::as_array));
    if let Some(variants) = variants {
        return union(
            variants
                .iter()
                .map(|variant| to_type(variant, context))
                .collect(),
        );
    }
    if let Some(parts) = object.get("allOf").and_then(Value::as_array) {
        let parts: Vec<String> = parts
            .iter()
            .map(|part| to_type(part, context))
            .filter(|part| part != "unknown")
            .collect();
        if parts.is_empty() {
            return "unknown".to_string();
        }
        return parts
            .iter()
            .map(|part| match part.contains(" | ") {
                true => format!("({part})"),
                false => part.clone(),
            })
            .collect::<Vec<_>>()
            .join(" & ");
    }

    match object.get("type") {
        Some(Value::Array(types)) => union(
            types
                .iter()
                .map(|entry| {
                    let mut single = object.clone();
                    single.insert("type".into(), entry.clone());
                    to_type(&Value::Object(single), context)
                })
                .collect(),
        ),
        Some(Value::String(kind)) => match kind.as_str() {
            "string" => "string".to_string(),
            "number" | "integer" => "number".to_string(),
            "boolean" => "boolean".to_string(),
            "null" => "null".to_string(),
            "array" => array_type(object, context),
            "object" => object_type(object, context),
            _ => "unknown".to_string(),
        },
        None => {
            if ["properties", "additionalProperties", "required"]
                .iter()
                .any(|key| object.contains_key(*key))
            {
                return object_type(object, context);
            }
            if object.contains_key("items") || object.contains_key("prefixItems") {
                return array_type(object, context);
            }
            "unknown".to_string()
        }
        Some(_) => "unknown".to_string(),
    }
}

fn array_type(schema: &serde_json::Map<String, Value>, context: &mut SchemaContext<'_>) -> String {
    if let Some(items @ (Value::Object(_) | Value::Bool(_))) = schema.get("items") {
        return format!("Array<{}>", to_type(items, context));
    }
    let tuple = schema
        .get("prefixItems")
        .and_then(Value::as_array)
        .or_else(|| schema.get("items").and_then(Value::as_array));
    match tuple {
        Some(items) if !items.is_empty() => format!(
            "[{}]",
            items
                .iter()
                .map(|item| to_type(item, context))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "unknown[]".to_string(),
    }
}

fn description_of(property: &Value) -> String {
    property
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn property_key(name: &str) -> String {
    match is_identifier(name) {
        true => name.to_string(),
        false => Value::String(name.to_string()).to_string(),
    }
}

fn object_type(schema: &serde_json::Map<String, Value>, context: &mut SchemaContext<'_>) -> String {
    let empty = serde_json::Map::new();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|required| required.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let names: BTreeSet<&String> = properties.keys().collect();
    let mut members: Vec<String> = names
        .iter()
        .map(|name| {
            let optional = if required.contains(name.as_str()) { "" } else { "?" };
            format!(
                "{}{optional}: {};",
                property_key(name),
                to_type(&properties[name.as_str()], context)
            )
        })
        .collect();
    match schema.get("additionalProperties") {
        Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => members.push("[key: string]: unknown;".to_string()),
        Some(additional) => {
            let rendered = to_type(additional, context);
            members.push(format!("[key: string]: {rendered};"));
        }
        None if names.is_empty() => members.push("[key: string]: unknown;".to_string()),
        None => {}
    }
    if members.is_empty() {
        return "{}".to_string();
    }
    if !names
        .iter()
        .any(|name| !description_of(&properties[name.as_str()]).is_empty())
    {
        return format!("{{ {} }}", members.join(" "));
    }

    let mut lines = vec!["{".to_string()];
    for (index, name) in names.iter().enumerate() {
        for line in description_of(&properties[name.as_str()]).lines() {
            if !line.trim().is_empty() {
                lines.push(format!("{INDENT}// {}", line.trim()));
            }
        }
        lines.push(format!(
            "{INDENT}{}",
            members[index].replace('\n', &format!("\n{INDENT}"))
        ));
    }
    for member in &members[names.len()..] {
        lines.push(format!("{INDENT}{member}"));
    }
    lines.push("}".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn primitive_and_compound_types_render() {
        assert_eq!(schema_to_type(&json!({ "type": "string" }), None), "string");
        assert_eq!(schema_to_type(&json!({ "type": "integer" }), None), "number");
        assert_eq!(
            schema_to_type(&json!({ "type": ["string", "null"] }), None),
            "string | null"
        );
        assert_eq!(
            schema_to_type(&json!({ "enum": ["a", "b"] }), None),
            "\"a\" | \"b\""
        );
        assert_eq!(
            schema_to_type(&json!({ "type": "array", "items": { "type": "number" } }), None),
            "Array<number>"
        );
        assert_eq!(
            schema_to_type(
                &json!({ "type": "object", "properties": { "b": { "type": "number" }, "a": { "type": "string" } }, "required": ["a"] }),
                None
            ),
            "{ a: string; b?: number; }"
        );
        assert_eq!(
            schema_to_type(&json!({ "type": "object" }), None),
            "{ [key: string]: unknown; }"
        );
    }

    #[test]
    fn described_properties_render_one_per_line() {
        let rendered = schema_to_type(
            &json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Where to look" } },
                "required": ["path"],
            }),
            None,
        );
        assert_eq!(rendered, "{\n  // Where to look\n  path: string;\n}");
    }

    #[test]
    fn local_references_resolve_and_recursive_ones_stop() {
        let schema = json!({
            "$defs": { "node": { "type": "object", "properties": { "next": { "$ref": "#/$defs/node" } } } },
            "$ref": "#/$defs/node",
        });
        assert_eq!(schema_to_type(&schema, None), "{ next?: unknown; }");
        assert_eq!(
            schema_to_type(&json!({ "$ref": "https://example.com/x" }), None),
            "unknown"
        );
    }

    #[test]
    fn oversized_input_types_become_unknown() {
        let schema = json!({ "type": "object", "properties": { "a": { "type": "string" } } });
        assert_eq!(schema_to_type(&schema, Some(5)), "unknown");
    }

    #[test]
    fn mcp_results_render_as_call_tool_results() {
        let schema = json!({
            "type": "object",
            "properties": {
                "content": { "type": "array", "items": { "type": "object" } },
                "isError": { "type": "boolean" },
                "_meta": { "type": "object" },
                "structuredContent": { "type": "object", "properties": { "n": { "type": "number" } }, "required": ["n"] },
            },
        });
        assert_eq!(
            render_output_type(Some(&schema)),
            "CallToolResult<{ n: number; }>"
        );
        assert_eq!(render_output_type(None), "unknown");
        assert_eq!(
            render_output_type(Some(&json!({ "type": "string" }))),
            "string"
        );
    }

    #[test]
    fn a_sample_carries_the_description_and_declaration() {
        let sample = render_sample(&Declaration {
            name: "my-tool".into(),
            description: "Does things".into(),
            input_schema: Some(json!({ "type": "object", "properties": {} })),
            output_schema: Some(json!({ "type": "string" })),
        });
        assert!(sample.starts_with("Does things"));
        assert!(sample.contains("my_tool(args: { [key: string]: unknown; }): Promise<string>;"));
    }

    #[test]
    fn percent_encoded_reference_segments_decode() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
