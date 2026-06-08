//! A tool that asks the host for narrowly scoped sandbox access.

use crate::Guard;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;

/// The only additional capabilities an agent may request interactively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessCapability {
    Network,
    TemporaryWrite,
}

impl AccessCapability {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "network" => Some(AccessCapability::Network),
            "temporary-write" => Some(AccessCapability::TemporaryWrite),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AccessCapability::Network => "network",
            AccessCapability::TemporaryWrite => "temporary-write",
        }
    }
}

/// What the host needs to explain an access request to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRequest {
    pub capability: AccessCapability,
    pub command: String,
    pub reason: String,
}

/// The host asks the user and returns the chosen grant scope.
#[async_trait]
pub trait AccessApprover: Send + Sync {
    async fn approve(&self, request: AccessRequest) -> AccessApproval;
}

/// The answer to an access request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessApproval {
    Once,
    Session,
    Denied,
}

/// Ask for network or temporary-directory access before retrying a command.
pub struct RequestSandboxAccess {
    guard: Guard,
    approver: Arc<dyn AccessApprover>,
}

impl RequestSandboxAccess {
    pub fn new(guard: Guard, approver: Arc<dyn AccessApprover>) -> Self {
        RequestSandboxAccess { guard, approver }
    }
}

#[async_trait]
impl Tool for RequestSandboxAccess {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "request_sandbox_access".into(),
            description: "Ask the user for additional command access when a sandbox denial blocks the task. Use only after a denied command and request the narrowest capability.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "capability": {
                        "type": "string",
                        "enum": ["network", "temporary-write"],
                        "description": "The needed command capability"
                    },
                    "command": {
                        "type": "string",
                        "description": "The exact command to retry if access is approved"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why the command needs this access"
                    }
                },
                "required": ["capability", "command", "reason"]
            }),
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let capability = arguments
            .get("capability")
            .and_then(Value::as_str)
            .and_then(AccessCapability::parse)
            .ok_or_else(|| "capability must be network or temporary-write".to_string())?;
        let command = arguments
            .get("command")
            .and_then(Value::as_str)
            .filter(|command| !command.trim().is_empty())
            .ok_or_else(|| "command must not be empty".to_string())?;
        let reason = arguments
            .get("reason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.trim().is_empty())
            .ok_or_else(|| "reason must not be empty".to_string())?;
        let request = AccessRequest {
            capability,
            command: command.to_string(),
            reason: reason.to_string(),
        };
        let policy = match capability {
            AccessCapability::Network => self.guard.sandbox().policy().clone().with_network(),
            AccessCapability::TemporaryWrite => self
                .guard
                .sandbox()
                .policy()
                .clone()
                .with_writable_root(std::env::temp_dir()),
        };

        match self.approver.approve(request).await {
            AccessApproval::Once => {
                self.guard
                    .grant_once(command, self.guard.sandbox_with_policy(policy));
                Ok(format!(
                    "Approved {} access once. Retry the requested command now.",
                    capability.name()
                ))
            }
            AccessApproval::Session => {
                self.guard
                    .set_sandbox(self.guard.sandbox_with_policy(policy));
                Ok(format!(
                    "Approved {} access for this session. Retry the requested command now.",
                    capability.name()
                ))
            }
            AccessApproval::Denied => Err("Sandbox access was not approved.".to_string()),
        }
    }
}
