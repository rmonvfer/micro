//! Answering what extensions ask for.
//!
//! An extension never reaches into micro. It asks — for a command to be run, for the
//! session to be renamed, for the user to be told something — and this decides what
//! happens. That is what keeps someone else's code inside the same rules as everything
//! else: the ask arrives here, and here is where the workspace and the policy are.

use micro_extensions::FromHost;
use micro_extensions::Host;
use serde_json::json;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Answer whatever the extensions ask for, for as long as the host is running.
///
/// The stream of asks is taken out of the host first: waiting on it through the host would
/// hold its lock, and then nothing could be answered while nothing was being asked.
pub async fn serve(host: Arc<Host>, workspace: PathBuf, asker: Option<micro_tui::UiAsker>) {
    let Some(mut asks) = host.take_asks().await else {
        return;
    };

    while let Some(asked) = asks.recv().await {
        match asked {
            FromHost::Request {
                id,
                request,
                payload,
            } => {
                let answer = answer(&request, &payload, &workspace).await;
                if host.answer(&id, answer).await.is_err() {
                    return;
                }
            }
            // An action is carried out where it belongs; nothing goes back.
            FromHost::Action { action, payload } => {
                carry_out(&action, &payload, asker.as_ref()).await
            }
            FromHost::Ui { id, payload } => {
                let answer = show(&payload, asker.as_ref()).await;
                if let Some(id) = id {
                    if host.answer(&id, answer).await.is_err() {
                        return;
                    }
                }
            }
            FromHost::Failed { path, event, error } => {
                eprintln!("note: {path} failed handling {event}: {error}");
            }
        }
    }
}

/// What an extension gets back for a question.
async fn answer(request: &str, payload: &Value, workspace: &PathBuf) -> Value {
    match request {
        "exec" => exec(payload, workspace).await,
        "get_thinking_level" => json!({ "level": "off" }),
        other => json!({ "error": format!("micro cannot answer `{other}`") }),
    }
}

/// Run a program on the extension's behalf.
///
/// The command and its arguments are passed as they are written, with no shell between
/// them: an argument holding shell punctuation is an argument, not a second command.
async fn exec(payload: &Value, workspace: &PathBuf) -> Value {
    let Some(command) = payload.get("command").and_then(Value::as_str) else {
        return json!({ "error": "exec needs a command" });
    };
    let arguments: Vec<String> = payload
        .get("args")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let finished = tokio::process::Command::new(command)
        .args(&arguments)
        .current_dir(workspace)
        .stdin(std::process::Stdio::null())
        .output()
        .await;

    match finished {
        Ok(result) => json!({
            "stdout": String::from_utf8_lossy(&result.stdout),
            "stderr": String::from_utf8_lossy(&result.stderr),
            "code": result.status.code().unwrap_or(-1),
        }),
        Err(error) => json!({ "error": format!("cannot run {command}: {error}") }),
    }
}

/// Something an extension asked to have done.
///
/// Anything that reaches the conversation goes through the interface, because the
/// conversation is the interface's: it holds the agent and decides when a turn runs.
async fn carry_out(action: &str, payload: &Value, asker: Option<&micro_tui::UiAsker>) {
    match action {
        "send_user_message" => {
            let content = payload
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if content.trim().is_empty() {
                return;
            }
            match asker {
                Some(asker) => {
                    asker
                        .ask("send_user_message", content, None, Vec::new())
                        .await;
                }
                // Headless, there is no conversation to put it into.
                None => eprintln!("note: an extension tried to send a message with no session"),
            }
        }
        // A custom message is one an extension draws itself, which needs a renderer it
        // registered. Until there is one, saying it plainly is better than dropping it.
        "send_message" => {
            let said = payload
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !said.trim().is_empty() {
                if let Some(asker) = asker {
                    asker.ask("notify", said, None, Vec::new()).await;
                }
            }
        }
        other => eprintln!("note: an extension asked for `{other}`, which micro does not know"),
    }
}

/// Show the user what an extension wants shown, and say what came back.
///
/// With no interface to ask through — a headless run — a question is cancelled rather than
/// answered with something nobody chose.
async fn show(payload: &Value, asker: Option<&micro_tui::UiAsker>) -> Value {
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let text = |name: &str| {
        payload
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    let Some(asker) = asker else {
        return match method {
            "notify" => {
                if let Some(message) = text("message") {
                    eprintln!("{message}");
                }
                json!({})
            }
            _ => json!({ "cancelled": true }),
        };
    };

    match method {
        "notify" => {
            asker
                .ask("notify", text("message").unwrap_or_default(), None, Vec::new())
                .await
        }
        "select" => {
            let options = payload
                .get("options")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            asker
                .ask("select", text("title").unwrap_or_default(), None, options)
                .await
        }
        "confirm" => {
            asker
                .ask(
                    "confirm",
                    text("title").unwrap_or_default(),
                    text("message"),
                    Vec::new(),
                )
                .await
        }
        "input" => {
            asker
                .ask(
                    "input",
                    text("title").unwrap_or_default(),
                    text("placeholder"),
                    Vec::new(),
                )
                .await
        }
        // Anything else has nowhere to be shown, and saying so beats pretending.
        other => json!({ "cancelled": true, "error": format!("micro cannot show `{other}`") }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exec_runs_a_command_and_reports_what_it_printed() {
        let answer = exec(
            &json!({ "command": "echo", "args": ["hello"] }),
            &std::env::temp_dir(),
        )
        .await;

        assert_eq!(answer["stdout"], "hello\n");
        assert_eq!(answer["code"], 0);
    }

    /// The arguments go to the program, not to a shell, so punctuation in one is data.
    #[tokio::test]
    async fn an_argument_is_never_a_second_command() {
        let answer = exec(
            &json!({ "command": "echo", "args": ["hello; echo goodbye"] }),
            &std::env::temp_dir(),
        )
        .await;

        assert_eq!(answer["stdout"], "hello; echo goodbye\n");
    }

    #[tokio::test]
    async fn a_command_that_is_not_there_is_reported() {
        let answer = exec(
            &json!({ "command": "nothing-like-this-exists", "args": [] }),
            &std::env::temp_dir(),
        )
        .await;
        assert!(answer["error"].as_str().unwrap().contains("cannot run"));
    }

    #[tokio::test]
    async fn a_request_micro_does_not_know_is_answered_rather_than_ignored() {
        let answer = answer("fly", &json!({}), &std::env::temp_dir()).await;
        assert!(answer["error"].as_str().unwrap().contains("fly"));
    }

    /// A message an extension sends goes into the conversation through the interface.
    #[tokio::test]
    async fn a_message_from_an_extension_reaches_the_conversation() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let sending = tokio::spawn(async move {
            carry_out(
                "send_user_message",
                &json!({ "content": "look at the tests" }),
                Some(&asker),
            )
            .await
        });

        let mut request = requests.recv().await.expect("a message");
        assert_eq!(request.method, "send_user_message");
        assert_eq!(request.title, "look at the tests");
        request.answer(json!({ "queued": true }));
        sending.await.unwrap();
    }

    #[tokio::test]
    async fn an_empty_message_is_not_sent_at_all() {
        let (asker, mut requests) = micro_tui::ui_channel();
        carry_out("send_user_message", &json!({ "content": "   " }), Some(&asker)).await;
        assert!(requests.try_recv().is_none());
    }

    /// A headless run has nobody to ask, and says so rather than choosing for them.
    #[tokio::test]
    async fn a_question_with_no_interface_comes_back_cancelled() {
        let answer = show(&json!({ "method": "select", "title": "pick", "options": ["a"] }), None).await;
        assert_eq!(answer["cancelled"], true);
    }

    /// With an interface, the question reaches it and the answer comes back.
    #[tokio::test]
    async fn a_question_reaches_the_interface() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(
                &json!({ "method": "select", "title": "pick one", "options": ["a", "b"] }),
                Some(&asker),
            )
            .await
        });

        let mut request = requests.recv().await.expect("a question");
        assert_eq!(request.method, "select");
        assert_eq!(request.title, "pick one");
        assert_eq!(request.options, vec!["a", "b"]);
        request.answer(json!({ "value": "b" }));

        assert_eq!(showing.await.unwrap()["value"], "b");
    }
}
