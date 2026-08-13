//! Shell execution.

use crate::required_str;
use crate::truncate;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

/// The longest a command may be given, which is as long as a millisecond count fits in a
/// signed 32-bit integer. There is no default: a command runs until it is done, the way
/// it would in a terminal, and a caller that wants a limit says so.
const MAX_TIMEOUT_MS: u64 = 2_147_483_647;

/// The shell a command is run in.
///
/// A command written for an agent is written for bash, so bash is what runs it where
/// there is one. `sh` is a last resort rather than the default: on a system where it is
/// dash or ash, bash-only syntax fails in ways that read as the command being wrong.
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

/// How long the command may run, in milliseconds. Nothing asked for means no limit.
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
}

impl Bash {
    pub fn new(root: PathBuf) -> Self {
        Bash { root }
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

        let child = tokio::process::Command::new(shell())
            .arg("-c")
            .arg(&command)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // The child dies with the parent rather than outliving an aborted turn.
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("cannot start command: {error}"))?;

        // Read both streams as they arrive, so what the command has printed is known
        // before it finishes rather than only after.
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
                // One stream ending is not both. A command that prints only to stdout
                // closes stderr straight away, so an ended stream is retired and the
                // other keeps being read until it ends too.
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

        let waiting = async {
            tokio::join!(reading, child.wait())
        };
        let status = match timeout_ms {
            Some(limit) => match tokio::time::timeout(Duration::from_millis(limit), waiting).await {
                Ok((_, status)) => status.map_err(|error| format!("command failed: {error}"))?,
                Err(_) => {
                    let seconds = limit as f64 / 1000.0;
                    return Err(format!("command timed out after {seconds}s: {command}"));
                }
            },
            // Nothing was asked for, so the command runs until it is done.
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

        match code {
            Some(0) => Ok(body),
            Some(code) => Err(format!("exit code {code}\n{body}")),
            None => Err(format!("terminated by signal\n{body}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-bash-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn captures_stdout() {
        let output = Bash::new(scratch("stdout"))
            .execute(&json!({ "command": "echo hello" }))
            .await
            .unwrap();
        assert_eq!(output, "hello");
    }

    #[tokio::test]
    async fn reports_a_non_zero_exit_code_as_an_error() {
        let error = Bash::new(scratch("exit"))
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

        let output = Bash::new(root)
            .execute(&json!({ "command": "ls" }))
            .await
            .unwrap();
        assert!(output.contains("marker.txt"));
    }

    #[tokio::test]
    async fn a_hanging_command_hits_the_timeout() {
        let error = Bash::new(scratch("timeout"))
            .execute(&json!({ "command": "sleep 30", "timeout": 0.25 }))
            .await
            .unwrap_err();
        assert!(error.contains("timed out"));
    }

    /// A command that prints as it goes is reported as it goes, and each report carries
    /// everything printed so far.
    #[tokio::test]
    async fn a_running_command_says_what_it_has_printed() {
        let root = scratch("reporting");
        let bash = Bash::new(root.clone());
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
        // Each report is everything so far, not only the newest line.
        assert!(seen.last().unwrap().contains("first"));
        assert!(seen.last().unwrap().contains("third"));
        assert!(output.contains("first") && output.contains("third"), "{output}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tool that says nothing until it is done is still run, and reports nothing.
    #[tokio::test]
    async fn a_silent_command_reports_nothing() {
        let root = scratch("silent");
        let bash = Bash::new(root.clone());
        let (sender, mut reported) = tokio::sync::mpsc::unbounded_channel();

        bash.execute_reporting(&json!({ "command": "true" }), &crate::Progress::new(sender))
            .await
            .expect("it ran");

        assert!(reported.try_recv().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
