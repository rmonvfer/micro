//! The policy the tools hold themselves to, and where their decisions are recorded.

use micro_sandbox::Decision;
use micro_sandbox::Sandbox;
use micro_sandbox::SandboxPolicy;
use micro_types::LedgerEvent;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use tokio::sync::mpsc::UnboundedSender;

struct OneTimeSandbox {
    command: String,
    sandbox: Sandbox,
}

/// One policy, two enforcers, one record.
#[derive(Clone)]
pub struct Guard {
    sandbox: Arc<RwLock<Sandbox>>,
    one_time_sandbox: Arc<Mutex<Option<OneTimeSandbox>>>,
    events: Option<UnboundedSender<LedgerEvent>>,
}

impl Guard {
    pub fn new(sandbox: Sandbox) -> Self {
        Guard {
            sandbox: Arc::new(RwLock::new(sandbox)),
            one_time_sandbox: Arc::new(Mutex::new(None)),
            events: None,
        }
    }

    /// The default policy around `workspace`, for a caller with no run behind it to ask.
    pub fn for_workspace(workspace: impl AsRef<Path>) -> Self {
        Guard::new(Sandbox::new(SandboxPolicy::default(), workspace.as_ref()))
    }

    /// Send the decisions worth keeping to `events`, which is the session's ledger.
    pub fn recording(mut self, events: UnboundedSender<LedgerEvent>) -> Self {
        self.events = Some(events);
        self
    }

    /// The sandbox currently governing tools in this session.
    pub fn sandbox(&self) -> Sandbox {
        self.sandbox.read().expect("sandbox lock poisoned").clone()
    }

    /// Create a sandbox with the current workspace and protected paths under `policy`.
    pub fn sandbox_with_policy(&self, policy: SandboxPolicy) -> Sandbox {
        self.sandbox().with_policy(policy)
    }

    /// Replace the policy for tools that run after this call.
    pub fn set_sandbox(&self, sandbox: Sandbox) {
        *self.sandbox.write().expect("sandbox lock poisoned") = sandbox;
    }

    /// Let exactly the named shell command use `sandbox`, then return to the session policy.
    pub fn grant_once(&self, command: impl Into<String>, sandbox: Sandbox) {
        *self
            .one_time_sandbox
            .lock()
            .expect("sandbox grant lock poisoned") = Some(OneTimeSandbox {
            command: command.into(),
            sandbox,
        });
    }

    /// Take the one-command grant when it was made for `command`.
    pub fn take_one_time_sandbox(&self, command: &str) -> Option<Sandbox> {
        let mut grant = self
            .one_time_sandbox
            .lock()
            .expect("sandbox grant lock poisoned");
        match grant.as_ref().is_some_and(|grant| grant.command == command) {
            true => grant.take().map(|grant| grant.sandbox),
            false => None,
        }
    }

    /// The policy's name, for a message that has to say which policy refused.
    pub fn policy(&self) -> &'static str {
        self.sandbox().policy().name()
    }

    /// Whether a path may be read, as something a tool can return.
    pub(crate) fn read(&self, path: &Path) -> Result<(), String> {
        self.judge("read", path, self.sandbox().check_read(path))
    }

    /// Whether a path may be written to, created or removed.
    pub(crate) fn write(&self, path: &Path) -> Result<(), String> {
        self.judge("write", path, self.sandbox().check_write(path))
    }

    fn judge(&self, operation: &str, path: &Path, decision: Decision) -> Result<(), String> {
        if decision.allowed {
            return Ok(());
        }
        self.record(operation, path.display().to_string(), false);
        Err(decision.reason)
    }

    /// Write one decision into the ledger.
    pub(crate) fn record(&self, operation: &str, path_or_host: String, allowed: bool) {
        let Some(events) = &self.events else {
            return;
        };
        let _ = events.send(LedgerEvent::SandboxDecision {
            policy: self.sandbox().policy().name().to_string(),
            operation: operation.to_string(),
            path_or_host,
            allowed,
            tool_call_id: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn workspace(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-guard-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn a_refusal_reaches_the_ledger_naming_the_policy_that_made_it() {
        let root = workspace("refusal");
        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let guard = Guard::for_workspace(&root).recording(sender);

        let refused = guard.write(Path::new("/etc/hosts")).unwrap_err();
        assert!(refused.contains("workspace-write"), "{refused}");

        match events.try_recv().expect("a decision was recorded") {
            LedgerEvent::SandboxDecision {
                policy,
                operation,
                path_or_host,
                allowed,
                ..
            } => {
                assert_eq!(policy, "workspace-write");
                assert_eq!(operation, "write");
                assert_eq!(path_or_host, "/etc/hosts");
                assert!(!allowed);
            }
            other => panic!("expected a sandbox decision, got {other:?}"),
        }
    }

    #[test]
    fn ordinary_work_leaves_nothing_behind() {
        let root = workspace("allowed");
        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let guard = Guard::for_workspace(&root).recording(sender);

        guard.write(&root.join("notes.txt")).expect("inside");
        guard.read(Path::new("/etc/hosts")).expect("reads are free");

        assert!(events.try_recv().is_err());
    }

    #[test]
    fn guard_without_ledger_still_refuses() {
        let root = workspace("no-ledger");
        let guard = Guard::for_workspace(&root);
        assert!(guard.write(Path::new("/etc/hosts")).is_err());
        assert!(guard.write(&root.join("a.txt")).is_ok());
    }
}
