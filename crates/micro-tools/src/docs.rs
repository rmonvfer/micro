//! Embedded micro documentation for the agent to consult without filesystem access.

use crate::truncate;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;

struct Document {
    path: &'static str,
    content: &'static str,
}

const DOCUMENTS: &[Document] = &[
    Document {
        path: "README.md",
        content: include_str!("../../../README.md"),
    },
    Document {
        path: "docs/getting-started.md",
        content: include_str!("../../../docs/getting-started.md"),
    },
    Document {
        path: "docs/cli-reference.md",
        content: include_str!("../../../docs/cli-reference.md"),
    },
    Document {
        path: "docs/configuration.md",
        content: include_str!("../../../docs/configuration.md"),
    },
    Document {
        path: "docs/tools.md",
        content: include_str!("../../../docs/tools.md"),
    },
    Document {
        path: "docs/sandbox.md",
        content: include_str!("../../../docs/sandbox.md"),
    },
    Document {
        path: "docs/security.md",
        content: include_str!("../../../docs/security.md"),
    },
    Document {
        path: "docs/extensions.md",
        content: include_str!("../../../docs/extensions.md"),
    },
    Document {
        path: "docs/project-context.md",
        content: include_str!("../../../docs/project-context.md"),
    },
    Document {
        path: "docs/sessions.md",
        content: include_str!("../../../docs/sessions.md"),
    },
    Document {
        path: "docs/architecture.md",
        content: include_str!("../../../docs/architecture.md"),
    },
    Document {
        path: "docs/providers.md",
        content: include_str!("../../../docs/providers.md"),
    },
    Document {
        path: "docs/rpc.md",
        content: include_str!("../../../docs/rpc.md"),
    },
    Document {
        path: "docs/remote-control.md",
        content: include_str!("../../../docs/remote-control.md"),
    },
    Document {
        path: "docs/ledger.md",
        content: include_str!("../../../docs/ledger.md"),
    },
];

/// Read or search documentation embedded in the micro binary.
pub struct MicroDocs;

impl MicroDocs {
    pub fn new() -> Self {
        MicroDocs
    }
}

impl Default for MicroDocs {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MicroDocs {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "micro_docs".into(),
            description: "Read or search micro's built-in documentation. Use this when asked about micro itself, its configuration, extensions, tools, sandbox, or architecture.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Embedded document path to read, such as docs/extensions.md. Omit to list documents."
                    },
                    "query": {
                        "type": "string",
                        "description": "Case-insensitive text to search across embedded documentation."
                    },
                    "offset": {
                        "type": "number",
                        "description": "First line to read, starting at 1. Default 1."
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum lines to return when reading a document. Default 200."
                    }
                }
            }),
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = arguments.get("path").and_then(Value::as_str);
        let query = arguments.get("query").and_then(Value::as_str);
        match (path, query) {
            (Some(path), _) => read(path, arguments),
            (None, Some(query)) => search(query),
            (None, None) => Ok(list()),
        }
    }
}

fn list() -> String {
    let paths = DOCUMENTS
        .iter()
        .map(|document| document.path)
        .collect::<Vec<_>>()
        .join("\n");
    format!("Embedded micro documentation:\n{paths}")
}

fn read(path: &str, arguments: &Value) -> Result<String, String> {
    let document = document(path)?;
    let offset = arguments
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .clamp(1, 1_000) as usize;
    let lines = document.content.lines().collect::<Vec<_>>();
    let start = offset.saturating_sub(1).min(lines.len());
    let end = start.saturating_add(limit).min(lines.len());
    let content = lines[start..end]
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{}:{}", start + index + 1, line))
        .collect::<Vec<_>>()
        .join("\n");
    let suffix = (end < lines.len()).then(|| format!("\n… {} more lines", lines.len() - end));
    Ok(truncate(&format!(
        "{}\n{}{}",
        document.path,
        content,
        suffix.unwrap_or_default()
    )))
}

fn search(query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("query must not be empty".to_string());
    }
    let needle = query.to_ascii_lowercase();
    let matches = DOCUMENTS
        .iter()
        .flat_map(|document| {
            document
                .content
                .lines()
                .enumerate()
                .filter(|(_, line)| line.to_ascii_lowercase().contains(&needle))
                .map(|(index, line)| format!("{}:{}:{}", document.path, index + 1, line.trim()))
        })
        .take(100)
        .collect::<Vec<_>>();
    match matches.is_empty() {
        true => Ok(format!(
            "No embedded micro documentation matches {query:?}."
        )),
        false => Ok(truncate(&matches.join("\n"))),
    }
}

fn document(path: &str) -> Result<&'static Document, String> {
    DOCUMENTS
        .iter()
        .find(|document| document.path == path)
        .ok_or_else(|| format!("No embedded document at {path:?}.\n{}", list()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lists_documents_when_not_given_a_path_or_query() {
        let output = MicroDocs::new().execute(&json!({})).await.unwrap();
        assert!(output.contains("docs/extensions.md"));
    }

    #[tokio::test]
    async fn reads_numbered_document_lines() {
        let output = MicroDocs::new()
            .execute(&json!({ "path": "docs/sandbox.md", "limit": 2 }))
            .await
            .unwrap();
        assert!(
            output.contains("docs/sandbox.md\n1:# Command sandbox"),
            "{output}"
        );
    }

    #[tokio::test]
    async fn searches_across_embedded_documents() {
        let output = MicroDocs::new()
            .execute(&json!({ "query": "writable roots" }))
            .await
            .unwrap();
        assert!(output.contains("docs/sandbox.md:"), "{output}");
    }
}
