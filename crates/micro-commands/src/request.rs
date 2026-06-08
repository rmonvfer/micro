//! Inspect one model-facing request from the session ledger.

use crate::{CommandContext, CommandOutcome};
use micro_types::{content_hash, ContentBlock, EventSource, Message};

pub(crate) async fn command(
    argument: Option<&str>,
    context: &CommandContext<'_>,
) -> CommandOutcome {
    let Some(session_id) = context.session_id else {
        return CommandOutcome::error("nothing to inspect: this conversation is not recorded");
    };
    let parts: Vec<&str> = argument.unwrap_or_default().split_whitespace().collect();
    let Some(turn) = parts.first().and_then(|value| value.parse::<u64>().ok()) else {
        return CommandOutcome::error("Usage: /request <turn> [--raw]");
    };
    if parts.len() > 2 || parts.get(1).is_some_and(|value| *value != "--raw") {
        return CommandOutcome::error("Usage: /request <turn> [--raw]");
    }

    let rebuilt = match context.sessions.reconstruct_turn(session_id, turn).await {
        Ok(turn) => turn,
        Err(error) => return CommandOutcome::error(format!("cannot read turn {turn}: {error}")),
    };
    let verified = rebuilt
        .recorded_request_body
        .as_ref()
        .is_some_and(|body| content_hash(body) == rebuilt.request_hash);
    if rebuilt.recorded_request_body.is_some() && !verified {
        return CommandOutcome::error(format!(
            "stored request body for turn {turn} failed hash verification"
        ));
    }

    let raw = parts.get(1) == Some(&"--raw");
    let text = if raw {
        match rebuilt.recorded_request_body.as_ref() {
            Some(body) => match String::from_utf8(body.clone()) {
                Ok(body) => body,
                Err(error) => {
                    return CommandOutcome::error(format!("request body is not UTF-8: {error}"))
                }
            },
            None => return CommandOutcome::error(
                "this session predates exact request-body retention; use `micro sessions show ... --raw` to attempt a verified reconstruction",
            ),
        }
    } else {
        human_readable(&rebuilt, verified)
    };
    CommandOutcome::inspect(format!("Request · turn {turn}"), text)
}

fn human_readable(turn: &micro_session::ReconstructedTurn, verified: bool) -> String {
    let mut out = format!(
        "Turn {}\n{}/{}\nrequest hash: {}\nverification: {}\n\n",
        turn.turn,
        turn.provider,
        turn.model_id,
        turn.request_hash,
        if verified {
            "verified"
        } else {
            "not retained (legacy session)"
        },
    );

    if let Some(prompt) = &turn.system_prompt {
        let mut offset = 0usize;
        for span in &turn.prefix_spans {
            let end = offset.saturating_add(span.bytes as usize).min(prompt.len());
            let text = prompt.get(offset..end).unwrap_or_default();
            out.push_str(&format!("{}\n{}\n\n", heading(&span.source), text));
            offset = end;
        }
        if turn.prefix_spans.is_empty() {
            out.push_str(&format!(
                "{}\n{}\n\n",
                heading(&EventSource::SystemPrompt),
                prompt
            ));
        }
    }
    for tool in &turn.tools {
        out.push_str(&format!(
            "{}\n{}\n\n",
            heading(&EventSource::Tool(tool.name.clone())),
            serde_json::to_string_pretty(tool).unwrap_or_default()
        ));
    }
    for message in &turn.messages {
        let (label, content) = match message {
            Message::User { content, .. } => (heading(&EventSource::User), content),
            Message::Assistant(message) => (heading(&EventSource::Model), &message.content),
            Message::ToolResult {
                tool_name, content, ..
            } => (
                format!("TOOL RESULT: {tool_name}  [tool_result:{tool_name}]"),
                content,
            ),
        };
        let text = content
            .iter()
            .map(ContentBlock::as_text)
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!("{label}\n{text}\n\n"));
    }
    out
}

fn heading(source: &EventSource) -> String {
    format!("{}  [{}]", source.to_string().to_uppercase(), source)
}
