//! The tools the model can call, and the trait every tool implements.

mod bash;
mod deferred;
mod files;
mod fuzzy;
mod mutations;
mod search;

pub use bash::Bash;
pub use deferred::Deferred;
pub use deferred::ToolSearch;
pub use files::Edit;
pub use files::Ls;
pub use files::MultiEdit;
pub use files::Read;
pub use files::Write;
pub use search::Find;
pub use search::Grep;

use async_trait::async_trait;
use micro_types::ContentBlock;
use micro_types::ToolDefinition;
use micro_types::ToolExecutionMode;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

/// Output longer than this is truncated before it reaches the model.
pub const MAX_OUTPUT_CHARS: usize = 30_000;

/// A capability the model can invoke. `Err` is returned to the model as a failed
/// tool result rather than aborting the turn, so the model can correct itself.
#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    /// Whether this tool is left out of the list the model is given, to be found by
    /// searching instead. Almost everything is worth describing up front, which is the
    /// default; see [`Deferred`] for when it is not.
    fn deferred(&self) -> bool {
        false
    }

    /// Whether this tool must run alone among a turn's tool calls, or may run alongside
    /// them.
    ///
    /// `None` is the common case: the tool has no opinion, and whichever default the
    /// turn's own caller runs a batch under applies. A tool overrides this only when
    /// running it at the same time as something else in the same turn would be wrong —
    /// two calls that would race over the same state, say.
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        None
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String>;

    /// Run, reporting what is happening as it happens.
    ///
    /// A tool that has nothing to report until it finishes takes the default: the same
    /// run, with nothing said along the way.
    async fn execute_reporting(
        &self,
        arguments: &Value,
        progress: &Progress,
    ) -> Result<String, String> {
        let _ = progress;
        self.execute(arguments).await
    }

    /// Run, answering with content the model reads rather than with text alone.
    ///
    /// Almost everything a tool has to say is text, which is the default. A tool that
    /// hands back something the model looks at rather than reads — an image — says so
    /// here, and the blocks travel to the provider as they are.
    async fn execute_content(
        &self,
        arguments: &Value,
        progress: &Progress,
    ) -> Result<Vec<ContentBlock>, String> {
        self.execute_reporting(arguments, progress)
            .await
            .map(|text| vec![ContentBlock::text(text)])
    }
}

/// Where a tool says what it is doing while it does it.
///
/// A sink with nowhere to send drops what it is given, so a tool never has to ask whether
/// anyone is listening.
#[derive(Clone, Default)]
pub struct Progress {
    sender: Option<tokio::sync::mpsc::UnboundedSender<String>>,
}

impl Progress {
    pub fn new(sender: tokio::sync::mpsc::UnboundedSender<String>) -> Self {
        Progress {
            sender: Some(sender),
        }
    }

    /// Say what has happened so far. Whoever is listening decides what to do with it.
    pub fn report(&self, text: impl Into<String>) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(text.into());
        }
    }
}

/// The default tool set, rooted at `root`.
pub fn builtin_tools(root: impl Into<PathBuf>) -> Vec<Arc<dyn Tool>> {
    let root = root.into();
    vec![
        Arc::new(Read::new(root.clone())),
        Arc::new(Write::new(root.clone())),
        Arc::new(Edit::new(root.clone())),
        Arc::new(MultiEdit::new(root.clone())),
        Arc::new(Ls::new(root.clone())),
        Arc::new(Grep::new(root.clone())),
        Arc::new(Find::new(root.clone())),
        Arc::new(Bash::new(root)),
    ]
}

/// Resolve a model-supplied path against the workspace root, rejecting anything that
/// escapes it. Traversal is checked on the lexical path so a missing file still resolves.
pub(crate) fn resolve_path(root: &Path, candidate: &str) -> Result<PathBuf, String> {
    if candidate.trim().is_empty() {
        return Err("path must not be empty".to_string());
    }

    let requested = Path::new(candidate);
    let joined = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };

    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!("path escapes the workspace: {candidate}"));
                }
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }

    if !normalized.starts_with(root) {
        return Err(format!("path escapes the workspace: {candidate}"));
    }
    Ok(normalized)
}

/// Truncate long output around the middle, keeping the head and tail the model needs.
pub(crate) fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_OUTPUT_CHARS / 2).collect();
    let tail: String = text
        .chars()
        .skip(text.chars().count() - MAX_OUTPUT_CHARS / 2)
        .collect();
    let omitted = text.chars().count() - MAX_OUTPUT_CHARS;
    format!("{head}\n\n… {omitted} characters omitted …\n\n{tail}")
}

pub(crate) fn required_str(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing required argument: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_under_the_root() {
        let root = Path::new("/work");
        assert_eq!(
            resolve_path(root, "src/main.rs").unwrap(),
            PathBuf::from("/work/src/main.rs")
        );
    }

    #[test]
    fn traversal_out_of_the_root_is_rejected() {
        let root = Path::new("/work");
        assert!(resolve_path(root, "../etc/passwd").is_err());
        assert!(resolve_path(root, "src/../../etc/passwd").is_err());
        assert!(resolve_path(root, "/etc/passwd").is_err());
    }

    #[test]
    fn traversal_that_stays_inside_the_root_is_allowed() {
        let root = Path::new("/work");
        assert_eq!(
            resolve_path(root, "src/../README.md").unwrap(),
            PathBuf::from("/work/README.md")
        );
    }

    #[test]
    fn short_output_is_returned_verbatim() {
        assert_eq!(truncate("hello"), "hello");
    }

    #[test]
    fn long_output_keeps_head_and_tail() {
        let text = "a".repeat(MAX_OUTPUT_CHARS + 100);
        let truncated = truncate(&text);
        assert!(truncated.contains("characters omitted"));
        assert!(truncated.len() < text.len());
    }
}
