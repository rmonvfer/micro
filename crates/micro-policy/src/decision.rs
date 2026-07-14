//! What the policy concluded, and how the user is asked when it cannot conclude alone.

use async_trait::async_trait;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// The verdict on one tool invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// The user has to say. The reason explains what made this worth asking about.
    Ask {
        reason: String,
    },
    /// Refused outright. The reason is handed to the model as a tool error so it can
    /// choose something else.
    Deny {
        reason: String,
    },
}

impl Decision {
    pub fn ask(reason: impl Into<String>) -> Self {
        Decision::Ask {
            reason: reason.into(),
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Decision::Deny {
            reason: reason.into(),
        }
    }

    /// How restrictive this verdict is. Combining verdicts always keeps the strictest, so
    /// one dangerous part of a command decides the whole of it.
    pub fn severity(&self) -> u8 {
        match self {
            Decision::Allow => 0,
            Decision::Ask { .. } => 1,
            Decision::Deny { .. } => 2,
        }
    }

    /// The stricter of two verdicts.
    pub fn strictest(self, other: Decision) -> Decision {
        if other.severity() > self.severity() {
            other
        } else {
            self
        }
    }
}

/// What a rule in the config file says about a tool or an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Allow,
    Ask,
    Deny,
}

impl Rule {
    pub(crate) fn decision(self, subject: &str) -> Decision {
        match self {
            Rule::Allow => Decision::Allow,
            Rule::Ask => Decision::ask(format!("policy asks before running {subject}")),
            Rule::Deny => Decision::deny(format!("policy forbids {subject}")),
        }
    }
}

/// What the policy wants the user to decide on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub tool: String,
    /// The command for `bash`, the path for a file tool, or nothing for a tool that has
    /// neither.
    pub subject: Option<String>,
    pub arguments: Value,
    /// Why the policy could not decide on its own.
    pub reason: String,
    /// What a session-wide grant would remember. Answering [`Approval::Session`] allows
    /// exactly this again, and nothing broader.
    pub key: String,
}

/// The user's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approval {
    /// Run it this once.
    Once,
    /// Run it now and whenever the identical invocation comes up again this session.
    Session,
    /// Refuse, with a message the model reads as the tool error.
    Denied(String),
}

/// Asks the user. Implemented by whatever owns the interface, so this crate never draws
/// anything or reads a key.
#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, request: &ApprovalRequest) -> Approval;
}

/// An approver that refuses everything, for headless runs where nobody can answer.
pub struct DenyEverything;

#[async_trait]
impl Approver for DenyEverything {
    async fn approve(&self, _request: &ApprovalRequest) -> Approval {
        Approval::Denied(
            "this needs approval, and nobody is available to give it; ask the user to run \
             it or to widen the policy"
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strictest_verdict_wins() {
        assert_eq!(
            Decision::Allow.strictest(Decision::ask("x")),
            Decision::ask("x")
        );
        assert_eq!(
            Decision::ask("x").strictest(Decision::Allow),
            Decision::ask("x")
        );
        assert_eq!(
            Decision::ask("x").strictest(Decision::deny("y")),
            Decision::deny("y")
        );
        assert_eq!(
            Decision::deny("y").strictest(Decision::ask("x")),
            Decision::deny("y")
        );
        assert_eq!(Decision::Allow.strictest(Decision::Allow), Decision::Allow);
    }

    #[tokio::test]
    async fn the_headless_approver_refuses() {
        let request = ApprovalRequest {
            tool: "bash".into(),
            subject: Some("ls".into()),
            arguments: Value::Null,
            reason: "test".into(),
            key: "bash:ls".into(),
        };
        assert!(matches!(
            DenyEverything.approve(&request).await,
            Approval::Denied(_)
        ));
    }
}
