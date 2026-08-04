//! Shell execution.

use crate::required_str;
use crate::truncate;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;

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
                    "command": { "type": "string", "description": "The command to run" },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Timeout in milliseconds, default 120000, max 600000",
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
        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let child = tokio::process::Command::new("sh")
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
                loop {
                    let line = tokio::select! {
                        line = out.next_line() => line,
                        line = err.next_line() => line,
                    };
                    match line {
                        Ok(Some(line)) => {
                            let mut held = collected.lock().await;
                            held.push_str(&line);
                            held.push('\n');
                            progress.report(held.clone());
                        }
                        // One stream ending is not both; the other is drained by the
                        // wait below, and whatever it printed is still collected.
                        Ok(None) | Err(_) => break,
                    }
                }
            }
        };

        let waiting = async {
            tokio::join!(reading, child.wait())
        };
        let status = match tokio::time::timeout(Duration::from_millis(timeout_ms), waiting).await {
            Ok((_, status)) => status.map_err(|error| format!("command failed: {error}"))?,
            Err(_) => return Err(format!("command timed out after {timeout_ms}ms: {command}")),
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
            .execute(&json!({ "command": "sleep 30", "timeout_ms": 250 }))
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
