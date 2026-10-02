//! Tools the model is told about only once it goes looking for them.

use crate::truncate;
use crate::Loadout;
use crate::LoadoutChanges;
use crate::Tool;
use crate::ToolContext;
use crate::ToolOutput;
use async_trait::async_trait;
use micro_types::ContentBlock;
use micro_types::ToolAnnotations;
use micro_types::ToolDefinition;
use micro_types::ToolExecutionMode;
use micro_types::ToolExposure;
use micro_types::ToolNamespace;
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

/// How many tools a search answers with when the caller does not say.
const DEFAULT_LIMIT: usize = 8;

/// How long a search waits for tools still on their way before answering with what it has.
const ARRIVAL_WAIT: Duration = Duration::from_secs(30);

/// A tool reached in another way than it would be on its own, such as one left for the search to
/// find.
pub struct Exposed {
    tool: Arc<dyn Tool>,
    exposure: ToolExposure,
}

impl Exposed {
    pub fn new(tool: Arc<dyn Tool>, exposure: ToolExposure) -> Self {
        Exposed { tool, exposure }
    }
}

#[async_trait]
impl Tool for Exposed {
    fn definition(&self) -> ToolDefinition {
        self.tool.definition()
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn namespace(&self) -> Option<ToolNamespace> {
        self.tool.namespace()
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        self.tool.annotations()
    }

    fn output_schema(&self) -> Option<Value> {
        self.tool.output_schema()
    }

    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        self.tool.execution_mode()
    }

    fn prepare_loadout(&self, loadout: &Loadout) -> LoadoutChanges {
        self.tool.prepare_loadout(loadout)
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        self.tool.execute(arguments).await
    }

    async fn execute_reporting(
        &self,
        arguments: &Value,
        progress: &crate::Progress,
    ) -> Result<String, String> {
        self.tool.execute_reporting(arguments, progress).await
    }

    async fn execute_content(
        &self,
        arguments: &Value,
        progress: &crate::Progress,
    ) -> Result<Vec<ContentBlock>, String> {
        self.tool.execute_content(arguments, progress).await
    }

    async fn call(&self, arguments: &Value, context: &ToolContext<'_>) -> ToolOutput {
        self.tool.call(arguments, context).await
    }
}

/// Tools that become callable while a session runs, such as those of a server that connects in
/// the background. They are never declared to the model: a search finds them, and the model then
/// calls them by name.
#[derive(Clone, Default)]
pub struct Arrivals {
    inner: Arc<ArrivalsInner>,
}

#[derive(Default)]
struct ArrivalsInner {
    tools: RwLock<Vec<Arc<dyn Tool>>>,
    /// The groups whose tools are expected, so a search can name them before they arrive.
    groups: RwLock<Vec<String>>,
    /// How many sources are still on their way.
    pending: tokio::sync::watch::Sender<usize>,
}

/// A source of tools that has not arrived yet. Dropping it says the source is done, whether or
/// not it delivered anything.
pub struct Expected {
    arrivals: Arrivals,
}

impl Drop for Expected {
    fn drop(&mut self) {
        self.arrivals
            .inner
            .pending
            .send_modify(|pending| *pending = pending.saturating_sub(1));
    }
}

impl Arrivals {
    /// Say that tools of `group` are on their way.
    pub fn expect(&self, group: impl Into<String>) -> Expected {
        self.announce(group);
        self.inner.pending.send_modify(|pending| *pending += 1);
        Expected {
            arrivals: self.clone(),
        }
    }

    /// Name a group whose tools may arrive later, without anything waiting for them.
    pub fn announce(&self, group: impl Into<String>) {
        let group = group.into();
        let mut groups = write(&self.inner.groups);
        if !groups.contains(&group) {
            groups.push(group);
        }
    }

    /// Make tools callable, replacing any already here under the same name.
    pub fn add(&self, arrived: Vec<Arc<dyn Tool>>) {
        let mut tools = write(&self.inner.tools);
        for tool in arrived {
            let name = tool.definition().name;
            tools.retain(|kept| kept.definition().name != name);
            tools.push(tool);
        }
    }

    /// Take away every tool whose name starts with `prefix`, for a source that went away.
    pub fn remove_prefixed(&self, prefix: &str) {
        write(&self.inner.tools).retain(|tool| !tool.definition().name.starts_with(prefix));
    }

    pub fn find(&self, name: &str) -> Option<Arc<dyn Tool>> {
        read(&self.inner.tools)
            .iter()
            .find(|tool| tool.definition().name == name)
            .cloned()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        read(&self.inner.tools)
            .iter()
            .map(|tool| tool.definition())
            .collect()
    }

    pub fn groups(&self) -> Vec<String> {
        read(&self.inner.groups).clone()
    }

    /// Whether anything has been or will be delivered here.
    pub fn is_empty(&self) -> bool {
        read(&self.inner.groups).is_empty() && read(&self.inner.tools).is_empty()
    }

    /// Wait until no source is still on its way, or `within` has passed.
    pub async fn settled(&self, within: Duration) {
        let mut pending = self.inner.pending.subscribe();
        let _ = tokio::time::timeout(within, pending.wait_for(|pending| *pending == 0)).await;
    }
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The one tool that stands in for all the deferred ones.
pub struct ToolSearch {
    hidden: Vec<ToolDefinition>,
    arrivals: Option<Arrivals>,
}

impl ToolSearch {
    pub fn new(tools: &[Arc<dyn Tool>]) -> Self {
        ToolSearch {
            hidden: tools
                .iter()
                .filter(|tool| tool.exposure().is_searchable())
                .map(|tool| tool.definition())
                .collect(),
            arrivals: None,
        }
    }

    /// Search the tools that arrive while the session runs as well, waiting for any still on
    /// their way.
    pub fn with_arrivals(mut self, arrivals: Arrivals) -> Self {
        self.arrivals = Some(arrivals);
        self
    }

    /// Whether there is anything to search, so a caller can leave the tool out entirely when
    /// nothing was deferred.
    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty() && self.arrivals.as_ref().is_none_or(Arrivals::is_empty)
    }

    /// The names on offer, grouped by the prefix they share. Groups of tools that arrive later
    /// are named from the start, so the description does not change when they come.
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
        for group in self.arrivals.iter().flat_map(Arrivals::groups) {
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        groups
    }

    /// Every definition a search looks through, once whatever is on its way has arrived.
    async fn searchable(&self) -> Vec<ToolDefinition> {
        let mut searchable = self.hidden.clone();
        if let Some(arrivals) = &self.arrivals {
            arrivals.settled(ARRIVAL_WAIT).await;
            for definition in arrivals.definitions() {
                if !searchable.iter().any(|kept| kept.name == definition.name) {
                    searchable.push(definition);
                }
            }
        }
        searchable
    }
}

#[async_trait]
impl Tool for ToolSearch {
    fn definition(&self) -> ToolDefinition {
        let groups = self.groups();
        ToolDefinition {
            name: "tool_search".into(),
            description: format!(
                "Find tools that are available but not listed. {} can be called, in these \
                 groups: {}. Search before saying something cannot be done: the tool for it \
                 may be here. The answer gives each tool's name, what it does, and its \
                 arguments; call it by name afterwards, the same as any other tool.",
                match self.arrivals.is_some() {
                    true => "Further tools".to_string(),
                    false => format!("{} further tools", self.hidden.len()),
                },
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

        let searchable = self.searchable().await;
        let matched: Vec<&ToolDefinition> = searchable
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
                Arc::new(Exposed::new(
                    Arc::new(Named(name, description)),
                    ToolExposure::Deferred,
                )) as Arc<dyn Tool>
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
        let tool = Exposed::new(
            Arc::new(Named("read", "Read a file")),
            ToolExposure::Deferred,
        );
        assert_eq!(tool.definition().name, "read");
        assert_eq!(tool.exposure(), ToolExposure::Deferred);
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

    /// A search waits for tools still on their way, and finds them once they are here.
    #[tokio::test]
    async fn a_search_waits_for_tools_on_their_way() {
        let arrivals = Arrivals::default();
        let expected = arrivals.expect("mcp__slow");
        let search = ToolSearch::new(&[]).with_arrivals(arrivals.clone());
        assert!(!search.is_empty(), "something is on its way");
        assert!(search.definition().description.contains("mcp__slow"));

        let delivering = arrivals.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            delivering.add(vec![Arc::new(Named("mcp__slow__ping", "Answer a ping"))]);
            drop(expected);
        });

        let found = search.execute(&json!({ "query": "ping" })).await.unwrap();
        assert!(found.contains("mcp__slow__ping"), "{found}");
        assert!(arrivals.find("mcp__slow__ping").is_some());
    }

    #[test]
    fn a_tool_that_arrives_again_replaces_the_one_before() {
        let arrivals = Arrivals::default();
        arrivals.add(vec![Arc::new(Named("mcp__a__x", "first"))]);
        arrivals.add(vec![Arc::new(Named("mcp__a__x", "second"))]);
        assert_eq!(arrivals.definitions().len(), 1);
        assert_eq!(arrivals.definitions()[0].description, "second");

        arrivals.remove_prefixed("mcp__a__");
        assert!(arrivals.find("mcp__a__x").is_none());
    }

    #[test]
    fn nothing_deferred_means_nothing_to_search() {
        assert!(ToolSearch::new(&[]).is_empty());
        assert!(!search().is_empty());
    }
}
