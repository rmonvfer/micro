//! Tools the model is told about only once it goes looking for them.

use crate::truncate;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ContentBlock;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;

/// How many tools a search answers with when the caller does not say.
const DEFAULT_LIMIT: usize = 8;

pub struct Deferred(Arc<dyn Tool>);

impl Deferred {
    pub fn new(tool: Arc<dyn Tool>) -> Self {
        Deferred(tool)
    }
}

#[async_trait]
impl Tool for Deferred {
    fn definition(&self) -> ToolDefinition {
        self.0.definition()
    }

    fn deferred(&self) -> bool {
        true
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        self.0.execute(arguments).await
    }

    async fn execute_reporting(
        &self,
        arguments: &Value,
        progress: &crate::Progress,
    ) -> Result<String, String> {
        self.0.execute_reporting(arguments, progress).await
    }

    async fn execute_content(
        &self,
        arguments: &Value,
        progress: &crate::Progress,
    ) -> Result<Vec<ContentBlock>, String> {
        self.0.execute_content(arguments, progress).await
    }
}

/// The one tool that stands in for all the deferred ones.
pub struct ToolSearch {
    hidden: Vec<ToolDefinition>,
}

impl ToolSearch {
    pub fn new(tools: &[Arc<dyn Tool>]) -> Self {
        ToolSearch {
            hidden: tools
                .iter()
                .filter(|tool| tool.deferred())
                .map(|tool| tool.definition())
                .collect(),
        }
    }

    /// Whether there is anything to search, so a caller can leave the tool out entirely when
    /// nothing was deferred.
    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty()
    }

    /// The names on offer, grouped by the prefix they share.
    fn groups(&self) -> Vec<String> {
        let mut groups: Vec<String> = Vec::new();
        for definition in &self.hidden {
            let group = definition
                .name
                .rsplit_once("__")
                .map_or(definition.name.as_str(), |(prefix, _)| prefix)
                .to_string();
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        groups
    }
}

#[async_trait]
impl Tool for ToolSearch {
    fn definition(&self) -> ToolDefinition {
        let groups = self.groups();
        ToolDefinition {
            name: "tool_search".into(),
            description: format!(
                "Find tools that are available but not listed. {} further tools can be \
                 called, in these groups: {}. Search before saying something cannot be \
                 done: the tool for it may be here. The answer gives each tool's name, \
                 what it does, and its arguments; call it by name afterwards, the same as \
                 any other tool.",
                self.hidden.len(),
                groups.join(", ")
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What the tool would do, or part of its name. \
                                        Leave out to list what there is.",
                    },
                    "limit": {
                        "type": "number",
                        "description": "How many to return (default 8)",
                    },
                },
            }),
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| limit as usize)
            .unwrap_or(DEFAULT_LIMIT)
            .max(1);

        let matched: Vec<&ToolDefinition> = self
            .hidden
            .iter()
            .filter(|definition| {
                let haystack =
                    format!("{} {}", definition.name, definition.description).to_lowercase();
                query.split_whitespace().all(|word| haystack.contains(word))
            })
            .collect();

        if matched.is_empty() {
            return Ok(format!(
                "No tool matches `{query}`. The groups on offer are: {}.",
                self.groups().join(", ")
            ));
        }

        let total = matched.len();
        let described: Vec<Value> = matched
            .iter()
            .take(limit)
            .map(|definition| {
                json!({
                    "name": definition.name,
                    "description": definition.description,
                    "parameters": definition.parameters,
                })
            })
            .collect();

        let mut answer = serde_json::to_string_pretty(&json!({ "tools": described }))
            .map_err(|error| format!("cannot describe the tools found: {error}"))?;

        if total > limit {
            answer.push_str(&format!(
                "\n\n{total} tools match; {limit} are shown. Search again with a narrower \
                 query, or raise the limit."
            ));
        }
        Ok(truncate(&answer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tool that does nothing, for asking what a search says about it.
    struct Named(&'static str, &'static str);

    #[async_trait]
    impl Tool for Named {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.0.into(),
                description: self.1.into(),
                parameters: json!({ "type": "object", "properties": {} }),
                constrained_sampling: None,
            }
        }
        async fn execute(&self, _arguments: &Value) -> Result<String, String> {
            Ok("ran".to_string())
        }
    }

    fn deferred(tools: &[(&'static str, &'static str)]) -> Vec<Arc<dyn Tool>> {
        tools
            .iter()
            .map(|(name, description)| {
                Arc::new(Deferred::new(Arc::new(Named(name, description)))) as Arc<dyn Tool>
            })
            .collect()
    }

    fn search() -> ToolSearch {
        ToolSearch::new(&deferred(&[
            ("mcp__github__create_issue", "Open an issue on a repository"),
            ("mcp__github__list_pulls", "List pull requests"),
            ("mcp__notes__append", "Add a line to today's note"),
        ]))
    }

    #[test]
    fn a_deferred_tool_is_still_the_tool_it_wraps() {
        let tool = Deferred::new(Arc::new(Named("read", "Read a file")));
        assert_eq!(tool.definition().name, "read");
        assert!(tool.deferred());
    }

    /// What the model is shown up front is a count and the groups, not every description.
    #[test]
    fn the_search_describes_what_there_is_without_listing_it() {
        let definition = search().definition();
        assert!(definition.description.contains('3'), "{definition:?}");
        assert!(definition.description.contains("mcp__github"));
        assert!(definition.description.contains("mcp__notes"));
        assert!(
            !definition.description.contains("Open an issue"),
            "a description is what the search is for, not what it advertises"
        );
    }

    #[tokio::test]
    async fn searching_by_what_a_tool_does_finds_it() {
        let found = search()
            .execute(&json!({ "query": "issue" }))
            .await
            .unwrap();
        assert!(found.contains("mcp__github__create_issue"), "{found}");
        assert!(!found.contains("mcp__notes__append"), "{found}");

        assert!(found.contains("parameters"), "{found}");
    }

    #[tokio::test]
    async fn every_word_has_to_match() {
        let found = search()
            .execute(&json!({ "query": "list pull" }))
            .await
            .unwrap();
        assert!(found.contains("list_pulls"), "{found}");
        assert!(!found.contains("create_issue"), "{found}");
    }

    #[tokio::test]
    async fn an_empty_query_lists_what_there_is() {
        let found = search().execute(&json!({})).await.unwrap();
        for name in ["create_issue", "list_pulls", "append"] {
            assert!(found.contains(name), "{name} missing from {found}");
        }
    }

    #[tokio::test]
    async fn a_query_that_matches_nothing_says_what_there_is_instead() {
        let found = search()
            .execute(&json!({ "query": "nothing like this" }))
            .await
            .unwrap();
        assert!(found.contains("No tool matches"), "{found}");
        assert!(found.contains("mcp__github"), "{found}");
    }

    /// A search that showed less than it found says so.
    #[tokio::test]
    async fn a_capped_search_says_it_was_capped() {
        let found = search().execute(&json!({ "limit": 1 })).await.unwrap();
        assert!(found.contains("3 tools match; 1 are shown"), "{found}");
    }

    #[test]
    fn nothing_deferred_means_nothing_to_search() {
        assert!(ToolSearch::new(&[]).is_empty());
        assert!(!search().is_empty());
    }
}
