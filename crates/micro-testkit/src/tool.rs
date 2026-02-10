//! A [`Tool`] whose output a test decides, and which records how it was called.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use micro_tools::Tool;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;

/// A tool that returns canned output and counts its invocations.
///
/// By default it succeeds with `"ok"`. [`FakeTool::returning`] and [`FakeTool::failing`]
/// change the standing answer; [`FakeTool::responses`] queues a sequence for tests that
/// call the same tool more than once.
#[derive(Clone)]
pub struct FakeTool {
    inner: Arc<Inner>,
}

struct Inner {
    definition: ToolDefinition,
    /// Answers consumed one per call, before falling back to `standing`.
    queued: Mutex<VecDeque<Result<String, String>>>,
    standing: Mutex<Result<String, String>>,
    calls: Mutex<Vec<Value>>,
}

impl FakeTool {
    /// A tool named `name` that succeeds with `"ok"` and accepts any arguments.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        FakeTool {
            inner: Arc::new(Inner {
                definition: ToolDefinition {
                    description: format!("test double for {name}"),
                    name,
                    parameters: json!({ "type": "object", "properties": {} }),
                },
                queued: Mutex::new(VecDeque::new()),
                standing: Mutex::new(Ok("ok".to_string())),
                calls: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn with_description(self, description: impl Into<String>) -> Self {
        self.mutate(|definition| definition.description = description.into())
    }

    pub fn with_parameters(self, parameters: Value) -> Self {
        self.mutate(|definition| definition.parameters = parameters)
    }

    /// Succeed with `output` on every call.
    pub fn returning(self, output: impl Into<String>) -> Self {
        *self.inner.standing.lock().expect("standing lock") = Ok(output.into());
        self
    }

    /// Fail with `error` on every call. The agent turns this into a tool result flagged
    /// as an error rather than aborting the run.
    pub fn failing(self, error: impl Into<String>) -> Self {
        *self.inner.standing.lock().expect("standing lock") = Err(error.into());
        self
    }

    /// Answer successive calls from `responses`, falling back to the standing answer once
    /// they run out.
    pub fn responses(self, responses: impl IntoIterator<Item = Result<String, String>>) -> Self {
        self.inner
            .queued
            .lock()
            .expect("queued lock")
            .extend(responses);
        self
    }

    pub fn name(&self) -> &str {
        &self.inner.definition.name
    }

    /// The arguments of every call, oldest first.
    pub fn calls(&self) -> Vec<Value> {
        self.inner.calls.lock().expect("calls lock").clone()
    }

    pub fn call_count(&self) -> usize {
        self.inner.calls.lock().expect("calls lock").len()
    }

    /// The arguments of call `index`, panicking with a legible message when the tool was
    /// called fewer times than that.
    pub fn call(&self, index: usize) -> Value {
        let calls = self.calls();
        assert!(
            index < calls.len(),
            "expected `{}` to be called at least {} time(s), it ran {}",
            self.name(),
            index + 1,
            calls.len()
        );
        calls[index].clone()
    }

    /// Mutating the definition is only sound before the tool is shared with an agent,
    /// which is how the builder methods are used.
    fn mutate(self, edit: impl FnOnce(&mut ToolDefinition)) -> Self {
        let mut inner = Arc::try_unwrap(self.inner)
            .unwrap_or_else(|_| panic!("configure a FakeTool before sharing it with an agent"));
        edit(&mut inner.definition);
        FakeTool {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl Tool for FakeTool {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition.clone()
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        self.inner
            .calls
            .lock()
            .expect("calls lock")
            .push(arguments.clone());

        let queued = self.inner.queued.lock().expect("queued lock").pop_front();

        queued.unwrap_or_else(|| self.inner.standing.lock().expect("standing lock").clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_default_tool_succeeds_and_counts_its_calls() {
        let tool = FakeTool::new("read");

        assert_eq!(
            tool.execute(&json!({"path": "a.txt"})).await,
            Ok("ok".into())
        );
        assert_eq!(tool.call_count(), 1);
        assert_eq!(tool.call(0), json!({"path": "a.txt"}));
    }

    #[tokio::test]
    async fn a_configured_output_is_returned_every_call() {
        let tool = FakeTool::new("read").returning("contents");

        assert_eq!(tool.execute(&json!({})).await, Ok("contents".into()));
        assert_eq!(tool.execute(&json!({})).await, Ok("contents".into()));
        assert_eq!(tool.call_count(), 2);
    }

    #[tokio::test]
    async fn a_failing_tool_returns_its_error() {
        let tool = FakeTool::new("write").failing("disk full");
        assert_eq!(tool.execute(&json!({})).await, Err("disk full".into()));
    }

    #[tokio::test]
    async fn queued_responses_are_consumed_before_the_standing_one() {
        let tool = FakeTool::new("read")
            .returning("fallback")
            .responses([Ok("first".to_string()), Err("second failed".to_string())]);

        assert_eq!(tool.execute(&json!({})).await, Ok("first".into()));
        assert_eq!(tool.execute(&json!({})).await, Err("second failed".into()));
        assert_eq!(tool.execute(&json!({})).await, Ok("fallback".into()));
    }

    #[test]
    fn the_definition_reflects_the_configured_schema() {
        let tool = FakeTool::new("grep")
            .with_description("search files")
            .with_parameters(json!({
                "type": "object",
                "properties": { "pattern": { "type": "string" } },
                "required": ["pattern"]
            }));

        let definition = tool.definition();
        assert_eq!(definition.name, "grep");
        assert_eq!(definition.description, "search files");
        assert_eq!(definition.parameters["required"], json!(["pattern"]));
    }

    #[tokio::test]
    async fn clones_share_one_call_log() {
        let tool = FakeTool::new("read");
        let clone = tool.clone();

        clone.execute(&json!({"path": "a"})).await.unwrap();

        assert_eq!(tool.call_count(), 1);
        assert_eq!(tool.call(0), json!({"path": "a"}));
    }
}
