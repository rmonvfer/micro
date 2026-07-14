//! The seam where the policy is enforced.
//!
//! Wrapping the tools rather than teaching each one about permissions puts the check on
//! the single path every call takes, so a tool added later is gated by construction.

use crate::PolicyEngine;
use async_trait::async_trait;
use micro_tools::Tool;
use micro_types::ToolDefinition;
use serde_json::Value;
use std::sync::Arc;

/// A tool that consults the policy before doing anything.
pub struct Gated {
    inner: Arc<dyn Tool>,
    policy: Arc<PolicyEngine>,
}

impl Gated {
    /// Puts one tool behind the policy. The result is a `Tool` like any other, so callers
    /// cannot tell a gated tool from an ungated one and cannot reach past it.
    pub fn wrap(inner: Arc<dyn Tool>, policy: Arc<PolicyEngine>) -> Arc<dyn Tool> {
        Arc::new(Gated { inner, policy })
    }
}

#[async_trait]
impl Tool for Gated {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let name = self.definition().name;
        // A refusal is returned the same way any other tool failure is, so the model reads
        // it, explains it, and picks something else instead of stalling.
        self.policy.authorize(&name, arguments).await?;
        self.inner.execute(arguments).await
    }
}

/// Puts every tool in a set behind the same policy.
pub fn gated_tools(tools: Vec<Arc<dyn Tool>>, policy: Arc<PolicyEngine>) -> Vec<Arc<dyn Tool>> {
    tools
        .into_iter()
        .map(|tool| Gated::wrap(tool, policy.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Approval;
    use crate::ApprovalRequest;
    use crate::Approver;
    use crate::DenyEverything;
    use crate::Mode;
    use crate::Policy;
    use crate::Rule;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    /// Stands in for a real tool and records whether it ever ran.
    struct Spy {
        name: &'static str,
        runs: AtomicUsize,
    }

    impl Spy {
        fn new(name: &'static str) -> Arc<Self> {
            Arc::new(Spy {
                name,
                runs: AtomicUsize::new(0),
            })
        }

        fn runs(&self) -> usize {
            self.runs.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Tool for Spy {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.into(),
                description: "a tool that records being run".into(),
                parameters: json!({ "type": "object" }),
            }
        }

        async fn execute(&self, _arguments: &Value) -> Result<String, String> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok("ran".to_string())
        }
    }

    struct Approve(Approval);

    #[async_trait]
    impl Approver for Approve {
        async fn approve(&self, _request: &ApprovalRequest) -> Approval {
            self.0.clone()
        }
    }

    fn engine(policy: Policy, approver: Arc<dyn Approver>) -> Arc<PolicyEngine> {
        Arc::new(PolicyEngine::new(policy, "/work", approver).with_home("/Users/ramon"))
    }

    #[tokio::test]
    async fn an_allowed_call_reaches_the_tool() {
        let spy = Spy::new("bash");
        let gated = Gated::wrap(
            spy.clone(),
            engine(Policy::default(), Arc::new(DenyEverything)),
        );

        let output = gated
            .execute(&json!({ "command": "git status" }))
            .await
            .unwrap();

        assert_eq!(output, "ran");
        assert_eq!(spy.runs(), 1);
    }

    #[tokio::test]
    async fn a_denied_call_returns_an_error_and_never_runs_the_tool() {
        let spy = Spy::new("bash");
        let gated = Gated::wrap(
            spy.clone(),
            engine(Policy::default(), Arc::new(DenyEverything)),
        );

        let error = gated
            .execute(&json!({ "command": "rm -rf ~" }))
            .await
            .unwrap_err();

        assert!(error.contains("Refused by the workspace policy"));
        assert_eq!(spy.runs(), 0, "a refused command must not execute");
    }

    #[tokio::test]
    async fn a_call_the_user_refuses_never_runs_the_tool() {
        let spy = Spy::new("bash");
        let approver = Arc::new(Approve(Approval::Denied("no thanks".into())));
        let gated = Gated::wrap(spy.clone(), engine(Policy::default(), approver));

        let error = gated
            .execute(&json!({ "command": "cargo build" }))
            .await
            .unwrap_err();

        assert!(error.contains("no thanks"));
        assert_eq!(spy.runs(), 0);
    }

    #[tokio::test]
    async fn a_call_the_user_approves_runs_the_tool() {
        let spy = Spy::new("bash");
        let approver = Arc::new(Approve(Approval::Once));
        let gated = Gated::wrap(spy.clone(), engine(Policy::default(), approver));

        gated
            .execute(&json!({ "command": "cargo build" }))
            .await
            .unwrap();
        assert_eq!(spy.runs(), 1);
    }

    #[tokio::test]
    async fn gating_leaves_the_definition_the_model_sees_untouched() {
        let spy = Spy::new("bash");
        let original = spy.definition();
        let gated = Gated::wrap(spy, engine(Policy::default(), Arc::new(DenyEverything)));
        assert_eq!(gated.definition(), original);
    }

    #[tokio::test]
    async fn a_whole_tool_set_can_be_gated_at_once() {
        let read = Spy::new("read");
        let bash = Spy::new("bash");
        let tools: Vec<Arc<dyn Tool>> = vec![read.clone(), bash.clone()];
        let gated = gated_tools(tools, engine(Policy::default(), Arc::new(DenyEverything)));

        assert_eq!(gated.len(), 2);
        gated[0].execute(&json!({ "path": "a.rs" })).await.unwrap();
        assert!(gated[1]
            .execute(&json!({ "command": "rm -rf /" }))
            .await
            .is_err());

        assert_eq!(read.runs(), 1);
        assert_eq!(bash.runs(), 0);
    }

    #[tokio::test]
    async fn the_real_tool_set_is_gated_without_naming_each_tool() {
        let policy = Policy::new(Mode::Cautious).with_rule("bash:echo", Rule::Allow);
        let engine = engine(policy, Arc::new(DenyEverything));
        let gated = gated_tools(micro_tools::builtin_tools("/work"), engine);

        let names: Vec<String> = gated.iter().map(|tool| tool.definition().name).collect();
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"write".to_string()));

        // Nothing destructive is attempted here: the policy stops the call before the
        // shell is ever reached.
        let bash = gated
            .iter()
            .find(|tool| tool.definition().name == "bash")
            .unwrap();
        let error = bash
            .execute(&json!({ "command": "rm -rf ~" }))
            .await
            .unwrap_err();
        assert!(error.contains("Refused by the workspace policy"));

        let write = gated
            .iter()
            .find(|tool| tool.definition().name == "write")
            .unwrap();
        let error = write
            .execute(&json!({ "path": "a.txt", "content": "x" }))
            .await
            .unwrap_err();
        assert!(error.contains("nobody is available"));
    }
}
