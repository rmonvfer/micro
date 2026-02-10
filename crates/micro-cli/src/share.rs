//! Publishing a conversation as a secret GitHub gist.
//!
//! Secret rather than public: a gist is shared by sending someone the link, and a
//! conversation with a codebase in it is not something to put in a search index. Secret is
//! still not private, which is why the address is reported in full rather than as "done".

use micro_types::ContentBlock;
use micro_types::Message;
use serde_json::json;
use serde_json::Value;

/// Where the token is looked for, in order. The first one set wins.
pub const TOKEN_VARIABLES: [&str; 2] = ["GITHUB_TOKEN", "GH_TOKEN"];

const GISTS_URL: &str = "https://api.github.com/gists";

/// A GitHub token from the environment, if one is there to be had.
pub fn token() -> Option<String> {
    TOKEN_VARIABLES
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|token| token.trim().to_string())
        .find(|token| !token.is_empty())
}

/// Upload a conversation and return the address it can be read at.
pub async fn publish(title: &str, conversation: &[Message], token: &str) -> Result<String, String> {
    let body = json!({
        "description": format!("micro session: {title}"),
        "public": false,
        "files": { "conversation.md": { "content": markdown(title, conversation) } },
    });

    let response = reqwest::Client::new()
        .post(GISTS_URL)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "micro")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("cannot reach GitHub: {error}"))?;

    let status = response.status();
    let payload = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(explain(status.as_u16(), &payload));
    }

    serde_json::from_str::<Value>(&payload)
        .ok()
        .and_then(|gist| {
            gist.get("html_url")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| "GitHub accepted the gist but did not say where it is".to_string())
}

/// What a rejection means, in terms of what to do about it.
fn explain(status: u16, payload: &str) -> String {
    let said = serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|body| {
            body.get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| payload.trim().to_string());

    match status {
        401 => format!("GitHub rejected the token: {said}"),
        403 | 404 => format!(
            "The token cannot create gists: {said}. It needs the `gist` scope, which a \
             Copilot sign-in does not grant."
        ),
        other => format!("GitHub answered {other}: {said}"),
    }
}

/// The conversation as a document someone else can read.
fn markdown(title: &str, conversation: &[Message]) -> String {
    let mut out = format!("# {title}\n\n");
    for message in conversation {
        match message {
            Message::User { content, .. } => {
                out.push_str("## Prompt\n\n");
                out.push_str(text_of(content).trim());
                out.push_str("\n\n");
            }
            Message::Assistant(assistant) => {
                let text = assistant.text();
                if !text.trim().is_empty() {
                    out.push_str("## Answer\n\n");
                    out.push_str(text.trim());
                    out.push_str("\n\n");
                }
                for block in &assistant.content {
                    if let ContentBlock::ToolCall { name, .. } = block {
                        out.push_str(&format!("### Tool: {name}\n\n"));
                    }
                }
            }
            // Tool output is the workspace's, not the conversation's, and a shared log is
            // read for what was said. What was called is already named above it.
            Message::ToolResult { .. } => {}
        }
    }
    out
}

fn text_of(content: &[ContentBlock]) -> String {
    content
        .iter()
        .map(ContentBlock::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::AssistantMessage;
    use micro_types::StopReason;
    use micro_types::Usage;

    fn assistant(text: &str) -> Message {
        Message::Assistant(AssistantMessage {
            content: vec![ContentBlock::text(text)],
            provider: "openrouter".into(),
            model: "gemini-3-pro".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        })
    }

    #[test]
    fn a_conversation_reads_as_a_document() {
        let conversation = vec![
            Message::user("what does this do"),
            assistant("it counts things"),
        ];
        let out = markdown("counting", &conversation);

        assert!(out.starts_with("# counting\n\n"), "{out}");
        assert!(out.contains("## Prompt\n\nwhat does this do\n\n"), "{out}");
        assert!(out.contains("## Answer\n\nit counts things\n\n"), "{out}");
    }

    #[test]
    fn an_answer_with_nothing_said_is_left_out() {
        let conversation = vec![assistant("   ")];
        assert_eq!(markdown("quiet", &conversation), "# quiet\n\n");
    }

    /// A rejection is only useful if it says what to do about it.
    #[test]
    fn a_rejection_names_what_is_missing() {
        let denied = explain(403, r#"{"message":"Resource not accessible"}"#);
        assert!(denied.contains("`gist` scope"), "{denied}");

        let bad_token = explain(401, r#"{"message":"Bad credentials"}"#);
        assert!(bad_token.contains("Bad credentials"), "{bad_token}");

        let odd = explain(500, "upstream fell over");
        assert!(odd.contains("500"), "{odd}");
        assert!(odd.contains("upstream fell over"), "{odd}");
    }
}
