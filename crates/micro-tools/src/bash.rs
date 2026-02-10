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

        let wait = child.wait_with_output();
        let output = match tokio::time::timeout(Duration::from_millis(timeout_ms), wait).await {
            Ok(result) => result.map_err(|error| format!("command failed: {error}"))?,
            Err(_) => {
                // `wait_with_output` consumed the child, so the timeout path relies on
                // kill_on_drop to reap it when the future is dropped here.
                return Err(format!("command timed out after {timeout_ms}ms: {command}"));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut combined = String::new();
        combined.push_str(&stdout);
        if !stderr.is_empty() {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str(&stderr);
        }

        let code = output.status.code();
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
}
