//! Shell execution.

use crate::required_str;
use crate::truncate;
use crate::Guard;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

/// The longest a command may be given, which is as long as a millisecond count fits in a signed
/// 32-bit integer.
const MAX_TIMEOUT_MS: u64 = 2_147_483_647;

/// The shell a command is run in.
fn shell() -> PathBuf {
    if Path::new("/bin/bash").exists() {
        return PathBuf::from("/bin/bash");
    }
    if let Some(found) = on_path("bash") {
        return found;
    }
    PathBuf::from("sh")
}

/// Where an executable sits on `PATH`, if it is on it.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// How long the command may run, in milliseconds.
fn timeout_for(arguments: &Value) -> Result<Option<u64>, String> {
    let Some(seconds) = arguments.get("timeout") else {
        return Ok(None);
    };
    if seconds.is_null() {
        return Ok(None);
    }
    let seconds = seconds
        .as_f64()
        .ok_or_else(|| "invalid timeout: must be a number of seconds".to_string())?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err("invalid timeout: must be a finite number of seconds".to_string());
    }
    let milliseconds = (seconds * 1000.0).round() as u64;
    if milliseconds > MAX_TIMEOUT_MS {
        return Err(format!(
            "invalid timeout: maximum is {} seconds",
            MAX_TIMEOUT_MS / 1000
        ));
    }
    Ok(Some(milliseconds))
}

pub struct Bash {
    root: PathBuf,
    guard: Guard,
}

impl Bash {
    pub fn new(root: PathBuf, guard: Guard) -> Self {
        Bash { root, guard }
    }
}

#[async_trait]
impl Tool for Bash {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".into(),
            description: "Run a shell command in the workspace root. Returns combined stdout \
                          and stderr along with the exit code."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Bash command to execute" },
                    "timeout": {
                        "type": "number",
                        "description": "Timeout in seconds (optional, no default timeout)",
                    },
                },
                "required": ["command"],
            }),
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        self.execute_reporting(arguments, &crate::Progress::default())
            .await
    }

    async fn execute_reporting(
        &self,
        arguments: &Value,
        progress: &crate::Progress,
    ) -> Result<String, String> {
        let command = required_str(arguments, "command")?;
        let timeout_ms = timeout_for(arguments)?;

        
        let shell = shell();
        let wrapped = self.guard.sandbox().wrap(
            &shell.to_string_lossy(),
            ["-c".to_string(), command.clone()],
            &self.root,
        );
        let confined = wrapped.enforced;
        let child = tokio::process::Command::from(wrapped.to_std_command())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("cannot start command: {error}"))?;

        
        let mut child = child;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let collected = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));

        let reading = {
            let collected = std::sync::Arc::clone(&collected);
            let progress = progress.clone();
            async move {
                use tokio::io::AsyncBufReadExt as _;
                let mut out = tokio::io::BufReader::new(stdout.unwrap()).lines();
                let mut err = tokio::io::BufReader::new(stderr.unwrap()).lines();
                
                let (mut out_ended, mut err_ended) = (false, false);
                while !(out_ended && err_ended) {
                    let line = tokio::select! {
                        line = out.next_line(), if !out_ended => match line {
                            Ok(Some(line)) => Some(line),
                            Ok(None) | Err(_) => {
                                out_ended = true;
                                None
                            }
                        },
                        line = err.next_line(), if !err_ended => match line {
                            Ok(Some(line)) => Some(line),
                            Ok(None) | Err(_) => {
                                err_ended = true;
                                None
                            }
                        },
                    };
                    if let Some(line) = line {
                        let mut held = collected.lock().await;
                        held.push_str(&line);
                        held.push('\n');
                        progress.report(held.clone());
                    }
                }
            }
        };

        let waiting = async { tokio::join!(reading, child.wait()) };
        let status = match timeout_ms {
            Some(limit) => {
                match tokio::time::timeout(Duration::from_millis(limit), waiting).await {
                    Ok((_, status)) => {
                        status.map_err(|error| format!("command failed: {error}"))?
                    }
                    Err(_) => {
                        let seconds = limit as f64 / 1000.0;
                        return Err(format!("command timed out after {seconds}s: {command}"));
                    }
                }
            }
            
            None => {
                let (_, status) = waiting.await;
                status.map_err(|error| format!("command failed: {error}"))?
            }
        };

        let combined = collected.lock().await.clone();
        let code = status.code();
        let body = if combined.trim().is_empty() {
            "(no output)".to_string()
        } else {
            truncate(combined.trim_end())
        };

        let failure = match code {
            Some(0) => return Ok(body),
            Some(code) => format!("exit code {code}\n{body}"),
            None => format!("terminated by signal\n{body}"),
        };

        
        if !micro_sandbox::is_likely_denied(&status, &body) {
            return Err(failure);
        }
        if !confined {
            
            self.guard.record("exec", command, true);
            return Err(failure);
        }
        self.guard.record("exec", command, false);
        Err(format!(
            "denied by policy {}: {failure}",
            self.guard.policy()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_sandbox::Sandbox;
    use micro_sandbox::SandboxPolicy;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-bash-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// A shell with nothing confining it.
    fn unconfined(root: PathBuf) -> Bash {
        let guard = Guard::new(Sandbox::new(SandboxPolicy::Full, root.clone()));
        Bash::new(root, guard)
    }

    #[tokio::test]
    async fn captures_stdout() {
        let output = unconfined(scratch("stdout"))
            .execute(&json!({ "command": "echo hello" }))
            .await
            .unwrap();
        assert_eq!(output, "hello");
    }

    #[tokio::test]
    async fn reports_a_non_zero_exit_code_as_an_error() {
        let error = unconfined(scratch("exit"))
            .execute(&json!({ "command": "echo oops >&2; exit 3" }))
            .await
            .unwrap_err();
        assert!(error.contains("exit code 3"));
        assert!(error.contains("oops"));
    }

    #[tokio::test]
    async fn runs_in_the_workspace_root() {
        let root = scratch("cwd");
        std::fs::write(root.join("marker.txt"), "").unwrap();

        let output = unconfined(root)
            .execute(&json!({ "command": "ls" }))
            .await
            .unwrap();
        assert!(output.contains("marker.txt"));
    }

    #[tokio::test]
    async fn a_hanging_command_hits_the_timeout() {
        let error = unconfined(scratch("timeout"))
            .execute(&json!({ "command": "sleep 30", "timeout": 0.25 }))
            .await
            .unwrap_err();
        assert!(error.contains("timed out"));
    }

    /// A command that prints as it goes is reported as it goes, and each report carries everything
    /// printed so far.
    #[tokio::test]
    async fn a_running_command_says_what_it_has_printed() {
        let root = scratch("reporting");
        let bash = unconfined(root.clone());
        let (sender, mut reported) = tokio::sync::mpsc::unbounded_channel();

        let output = bash
            .execute_reporting(
                &json!({ "command": "echo first; echo second; echo third" }),
                &crate::Progress::new(sender),
            )
            .await
            .expect("it ran");

        let mut seen = Vec::new();
        while let Ok(update) = reported.try_recv() {
            seen.push(update);
        }

        assert!(seen.len() >= 2, "it reported as it went: {seen:?}");
        assert!(seen[0].contains("first"));
        
        assert!(seen.last().unwrap().contains("first"));
        assert!(seen.last().unwrap().contains("third"));
        assert!(
            output.contains("first") && output.contains("third"),
            "{output}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tool that says nothing until it is done is still run, and reports nothing.
    #[tokio::test]
    async fn a_silent_command_reports_nothing() {
        let root = scratch("silent");
        let bash = unconfined(root.clone());
        let (sender, mut reported) = tokio::sync::mpsc::unbounded_channel();

        bash.execute_reporting(&json!({ "command": "true" }), &crate::Progress::new(sender))
            .await
            .expect("it ran");

        assert!(reported.try_recv().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// What the policy does to a command, run against the real thing.
#[cfg(all(test, target_os = "macos"))]
mod sandboxed {
    use super::*;
    use micro_sandbox::Sandbox;
    use micro_sandbox::SandboxPolicy;
    use micro_types::LedgerEvent;

    fn workspace(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("micro-bash-policy-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        (
            dir.canonicalize().unwrap(),
            workspace.canonicalize().unwrap(),
        )
    }

    #[tokio::test]
    async fn a_write_outside_the_workspace_is_refused_in_the_name_of_the_policy() {
        let (dir, workspace) = workspace("outside");
        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let guard = Guard::new(Sandbox::new(SandboxPolicy::workspace_write(), &workspace))
            .recording(sender);
        let target = dir.join("loot.txt");

        let error = Bash::new(workspace, guard)
            .execute(&json!({ "command": format!("echo taken > {}", target.display()) }))
            .await
            .expect_err("the policy does not allow this");

        assert!(
            error.contains("denied by policy workspace-write"),
            "the model is told which policy refused: {error}"
        );
        assert!(
            error.contains("exit code"),
            "and what the command itself said: {error}"
        );
        assert!(!target.exists(), "nothing was written");

        match events.try_recv().expect("the refusal was recorded") {
            LedgerEvent::SandboxDecision {
                policy,
                operation,
                allowed,
                ..
            } => {
                assert_eq!(policy, "workspace-write");
                assert_eq!(operation, "exec");
                assert!(!allowed);
            }
            other => panic!("expected a sandbox decision, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_write_inside_the_workspace_goes_through() {
        let (_dir, workspace) = workspace("inside");
        let guard = Guard::new(Sandbox::new(SandboxPolicy::workspace_write(), &workspace));

        Bash::new(workspace.clone(), guard)
            .execute(&json!({ "command": "echo kept > notes.txt" }))
            .await
            .expect("the workspace is writable");

        assert_eq!(
            std::fs::read_to_string(workspace.join("notes.txt")).unwrap(),
            "kept\n"
        );
    }

    
    #[tokio::test]
    async fn read_only_refuses_a_write_to_the_workspace_itself() {
        let (_dir, workspace) = workspace("read-only");
        let guard = Guard::new(Sandbox::new(SandboxPolicy::ReadOnly, &workspace));

        let error = Bash::new(workspace.clone(), guard)
            .execute(&json!({ "command": "echo nope > notes.txt" }))
            .await
            .expect_err("read-only writes nothing");

        assert!(error.contains("denied by policy read-only"), "{error}");
        assert!(!workspace.join("notes.txt").exists());
    }
}
