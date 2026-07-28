//! Answering what extensions ask for.
//!
//! An extension never reaches into micro. It asks — for a command to be run, for the
//! session to be renamed, for the user to be told something — and this decides what
//! happens. That is what keeps someone else's code inside the same rules as everything
//! else: the ask arrives here, and here is where the workspace and the policy are.

use micro_agent::ToolHooks;
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

/// Tell the extensions something happened somewhere other than inside a turn.
///
/// The agent reports its own moments; these are the ones only the host knows about — what
/// the user typed, what they ran, what they switched to.
pub async fn announce(host: Option<&Arc<Host>>, event: &str, payload: Value) {
    if let Some(host) = host {
        let _ = host.notify(event, payload).await;
    }
}

/// Ask the extensions about something they are allowed to change, and hand back what they
/// said. Nothing to ask means nothing changed.
pub async fn consult(host: Option<&Arc<Host>>, event: &str, payload: Value) -> Vec<Value> {
    match host {
        Some(host) => host.ask_event(event, payload).await.unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Extensions deciding what a tool call may do.
///
/// Both moments are questions rather than announcements: `tool_call` may refuse the call,
/// and `tool_result` may rewrite what the model reads. An extension that answers nothing
/// changes nothing, which is what keeps a listener from accidentally intercepting.
pub struct ExtensionHooks {
    host: Arc<Host>,
}

impl ExtensionHooks {
    pub fn new(host: Arc<Host>) -> Self {
        ExtensionHooks { host }
    }
}

#[async_trait::async_trait]
impl ToolHooks for ExtensionHooks {
    async fn before_tool(&self, id: &str, name: &str, arguments: &Value) -> Option<String> {
        let answers = self
            .host
            .ask_event(
                "tool_call",
                json!({ "toolCallId": id, "toolName": name, "input": arguments }),
            )
            .await
            .ok()?;

        // The first refusal wins: a call blocked by anything is blocked.
        answers.iter().find_map(|answer| {
            match answer.get("block").and_then(Value::as_bool) {
                Some(true) => Some(
                    answer
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("an extension blocked this call")
                        .to_string(),
                ),
                _ => None,
            }
        })
    }

    async fn after_tool(
        &self,
        id: &str,
        name: &str,
        output: String,
        is_error: bool,
    ) -> (String, bool) {
        let asked = self
            .host
            .ask_event(
                "tool_result",
                json!({
                    "toolCallId": id,
                    "toolName": name,
                    "result": output,
                    "isError": is_error,
                }),
            )
            .await;

        let Ok(answers) = asked else {
            return (output, is_error);
        };

        // Each answer is applied in turn, so a later extension sees what an earlier one
        // wrote rather than what the tool originally said.
        let mut output = output;
        let mut is_error = is_error;
        for answer in answers {
            if let Some(content) = answer.get("content") {
                output = match content.as_str() {
                    Some(text) => text.to_string(),
                    None => content.to_string(),
                };
            }
            if let Some(failed) = answer.get("isError").and_then(Value::as_bool) {
                is_error = failed;
            }
        }
        (output, is_error)
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
