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
pub async fn serve(host: Arc<Host>, workspace: PathBuf) {
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
            FromHost::Action { action, payload } => carry_out(&action, &payload),
            FromHost::Ui { id, payload } => {
                let answer = show(&payload);
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
fn carry_out(action: &str, payload: &Value) {
    match action {
        // These reach the conversation, which belongs to whoever is driving the run. Until
        // that path exists they are reported rather than silently dropped.
        "send_user_message" | "send_message" | "set_session_name" | "set_thinking_level" => {
            eprintln!(
                "note: an extension asked to {action}, which micro cannot do from here yet"
            );
            let _ = payload;
        }
        other => eprintln!("note: an extension asked for `{other}`, which micro does not know"),
    }
}

/// Show the user what an extension wants shown, and say what came back.
fn show(payload: &Value) -> Value {
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match method {
        "notify" => {
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            eprintln!("{message}");
            json!({})
        }
        // Asking a question needs somebody to answer it, and out here there is nobody.
        // Cancelling is the honest answer rather than a made-up one.
        _ => json!({ "cancelled": true }),
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

    #[test]
    fn a_question_nobody_can_answer_comes_back_cancelled() {
        let answer = show(&json!({ "method": "select", "title": "pick", "options": ["a"] }));
        assert_eq!(answer["cancelled"], true);
    }
}
