//! What a tool answers with, and how a running tool calls other tools.

use crate::Progress;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ContentBlock;
use micro_types::ToolAnnotations;
use micro_types::ToolDefinition;
use micro_types::ToolExposure;
use micro_types::ToolNamespace;
use micro_types::Usage;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// What a tool answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolOutput {
    /// What the model reads.
    pub content: Vec<ContentBlock>,
    /// The answer as data matching the tool's output schema, for callers that read it as data
    /// rather than as text. A failed call may carry one too.
    pub structured: Option<Value>,
    pub is_error: bool,
    /// Tokens the tool spent itself, such as on a model it ran.
    pub usage: Option<Usage>,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        ToolOutput::content(vec![ContentBlock::text(text)])
    }

    pub fn content(content: Vec<ContentBlock>) -> Self {
        ToolOutput {
            content,
            ..ToolOutput::default()
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        ToolOutput {
            is_error: true,
            ..ToolOutput::text(text)
        }
    }

    /// The same answer, carrying `structured` as its data.
    pub fn with_structured(mut self, structured: Value) -> Self {
        self.structured = Some(structured);
        self
    }

    /// Every text block, one after another.
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .map(ContentBlock::as_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// What `execute_content` answers with, as an output.
    pub fn from_result(result: Result<Vec<ContentBlock>, String>) -> Self {
        match result {
            Ok(content) => ToolOutput::content(content),
            Err(error) => ToolOutput::error(error),
        }
    }
}

/// What a tool is handed while it runs, beyond its arguments.
#[derive(Clone)]
pub struct ToolContext<'a> {
    /// The id of the call being answered.
    pub call_id: &'a str,
    /// Where the tool says what it is doing while it does it.
    pub progress: Progress,
    tools: Option<&'a dyn ToolCaller>,
}

impl<'a> ToolContext<'a> {
    pub fn new(call_id: &'a str, progress: Progress) -> Self {
        ToolContext {
            call_id,
            progress,
            tools: None,
        }
    }

    /// Let the tool call other tools while it runs.
    pub fn with_tools(mut self, tools: &'a dyn ToolCaller) -> Self {
        self.tools = Some(tools);
        self
    }

    /// The other tools this one may call, when the run lets it call any.
    pub fn tools(&self) -> Option<&'a dyn ToolCaller> {
        self.tools
    }
}

/// How a running tool reaches other tools. Calls go through the same checks as the model's own
/// calls, and are recorded on the calling tool's result.
#[async_trait]
pub trait ToolCaller: Send + Sync {
    /// The tools a running tool may call.
    fn callable(&self) -> Vec<CallableTool>;

    /// Wait for tools still on their way, such as those of servers connecting in the background.
    async fn arrived(&self);

    /// Call `name` on behalf of the running tool. A failure is an output with `is_error` set.
    async fn call(&self, name: &str, arguments: Value) -> NestedOutcome;
}

/// What one call a running tool made came to.
#[derive(Debug, Clone, PartialEq)]
pub struct NestedOutcome {
    /// The id the call was made under.
    pub id: String,
    pub output: ToolOutput,
}

/// A tool as another tool sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct CallableTool {
    pub definition: ToolDefinition,
    pub exposure: ToolExposure,
    pub namespace: Option<ToolNamespace>,
    pub annotations: Option<ToolAnnotations>,
    /// The shape of `ToolOutput::structured`, when the tool answers with data.
    pub output_schema: Option<Value>,
}

impl CallableTool {
    pub fn of(tool: &dyn Tool) -> Self {
        CallableTool {
            definition: tool.definition(),
            exposure: tool.exposure(),
            namespace: tool.namespace(),
            annotations: tool.annotations(),
            output_schema: tool.output_schema(),
        }
    }
}

/// The tools of a run as a tool that orchestrates them sees them, so it can change how they are
/// presented to the model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loadout {
    /// The tools declared to the model.
    pub declared: Vec<CallableTool>,
    /// The tools another tool may call.
    pub callable: Vec<CallableTool>,
}

/// How a tool asks for the loadout to be presented while it is declared.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadoutChanges {
    /// Descriptions that replace a declared tool's own, by tool name.
    pub descriptions: BTreeMap<String, String>,
    /// Declared tools whose declarations requests leave out, while they stay callable.
    pub hidden: Vec<String>,
}

/// Write `text` to a private file of its own in the temporary directory, for output too long to
/// hand over whole. Says where it went.
pub async fn spill(prefix: &str, text: &str) -> Result<PathBuf, String> {
    use tokio::io::AsyncWriteExt as _;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}-{}.log",
        std::process::id(),
        micro_types::now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&path).await?;
        file.write_all(text.as_bytes()).await?;
        file.flush().await
    };
    written
        .await
        .map(|()| path)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_execute_result_becomes_an_output() {
        let ok = ToolOutput::from_result(Ok(vec![ContentBlock::text("fine")]));
        assert!(!ok.is_error);
        assert_eq!(ok.text_content(), "fine");

        let failed = ToolOutput::from_result(Err("broken".into()));
        assert!(failed.is_error);
        assert_eq!(failed.text_content(), "broken");
        assert_eq!(failed.structured, None);
    }

    #[tokio::test]
    async fn spilled_text_can_be_read_back() {
        let path = spill("micro-tools-test", "all of it").await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "all of it");
        let _ = std::fs::remove_file(path);
    }
}
