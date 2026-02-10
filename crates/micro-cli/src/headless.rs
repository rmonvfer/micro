//! Non-interactive mode: run one prompt to completion and stream it to stdout.

use anyhow::bail;
use anyhow::Result;
use micro_agent::Agent;
use micro_types::AgentEvent;
use micro_types::Message;
use micro_types::StreamEvent;
use std::io::Write as _;

/// Run one exchange, printing assistant text to stdout and progress to stderr so the
/// answer stays pipeable.
pub async fn run(mut agent: Agent, prompt: Message, quiet: bool) -> Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        agent.run(prompt, &sender).await;
    });

    let mut failure = None;
    while let Some(event) = receiver.recv().await {
        match event {
            AgentEvent::MessageDelta { event } => match event {
                StreamEvent::TextDelta { delta, .. } => {
                    print!("{delta}");
                    let _ = std::io::stdout().flush();
                }
                StreamEvent::TextEnd { .. } => println!(),
                _ => {}
            },
            AgentEvent::ToolStart {
                name, arguments, ..
            } if !quiet => {
                eprintln!("  · {name} {}", summarize(&arguments));
            }
            AgentEvent::ToolEnd {
                name,
                output,
                is_error,
                ..
            } if is_error && !quiet => {
                eprintln!("  ! {name}: {}", output.lines().next().unwrap_or_default());
            }
            AgentEvent::Retry {
                attempt,
                max_attempts,
                delay_ms,
            } if !quiet => eprintln!("  … retry {attempt}/{max_attempts} in {delay_ms}ms"),
            AgentEvent::MessageEnd {
                message: Message::Assistant(assistant),
            } => {
                if let Some(error) = assistant.error {
                    failure = Some(error);
                }
            }
            _ => {}
        }
    }

    task.await?;
    match failure {
        Some(error) => bail!(error),
        None => Ok(()),
    }
}

/// A one-line hint of what a tool was asked to do.
fn summarize(arguments: &serde_json::Value) -> String {
    for key in ["path", "command", "pattern"] {
        if let Some(value) = arguments.get(key).and_then(serde_json::Value::as_str) {
            let first = value.lines().next().unwrap_or_default();
            return match first.chars().count() > 60 {
                true => format!("{}…", first.chars().take(60).collect::<String>()),
                false => first.to_string(),
            };
        }
    }
    String::new()
}
