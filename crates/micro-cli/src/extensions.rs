//! Answering what extensions ask for.
//!
//! An extension never reaches into micro. It asks — for a command to be run, for the
//! session to be renamed, for the user to be told something — and this decides what
//! happens. That is what keeps someone else's code inside the same rules as everything
//! else: the ask arrives here, and here is where the workspace and the policy are.

use micro_agent::Hooks;
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
pub async fn serve(
    host: Arc<Host>,
    workspace: PathBuf,
    asker: Option<micro_tui::UiAsker>,
    state: Arc<tokio::sync::RwLock<State>>,
    session: Arc<tokio::sync::Mutex<micro_session::Session>>,
) {
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
                let answer = answer(&request, &payload, &workspace, &state, &session).await;
                if host.answer(&id, answer).await.is_err() {
                    return;
                }
            }
            // An action is carried out where it belongs; nothing goes back.
            FromHost::Action { action, payload } => {
                carry_out(&action, &payload, asker.as_ref(), Some(&host)).await
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
async fn answer(
    request: &str,
    payload: &Value,
    workspace: &PathBuf,
    state: &Arc<tokio::sync::RwLock<State>>,
    session: &Arc<tokio::sync::Mutex<micro_session::Session>>,
) -> Value {
    match request {
        "exec" => exec(payload, workspace).await,
        "get_thinking_level" => json!({ "level": state.read().await.thinking }),
        "get_active_tools" | "get_all_tools" => json!({ "tools": state.read().await.tools }),
        "get_commands" => json!({ "commands": state.read().await.commands }),
        "get_model" => {
            let state = state.read().await;
            json!({ "model": { "id": state.model, "provider": state.provider } })
        }
        "get_session_name" => {
            let session = session.lock().await;
            let name = session.meta().title.clone();
            json!({ "name": (!name.is_empty()).then_some(name) })
        }
        "append_entry" => {
            let custom_type = payload
                .get("customType")
                .and_then(Value::as_str)
                .unwrap_or("custom");
            let data = payload.get("data").cloned().unwrap_or(Value::Null);
            match session.lock().await.append_custom(custom_type, data).await {
                Ok(()) => json!({ "ok": true }),
                Err(error) => json!({ "error": error.to_string() }),
            }
        }
        "set_label" => {
            let Some(entry_id) = payload.get("entryId").and_then(Value::as_str) else {
                return json!({ "error": "no entry to label" });
            };
            let label = payload
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_string);
            match session.lock().await.set_label(entry_id, label).await {
                Ok(true) => json!({ "ok": true }),
                Ok(false) => json!({ "error": format!("there is no entry {entry_id}") }),
                Err(error) => json!({ "error": error.to_string() }),
            }
        }
        "get_entries" => {
            let session = session.lock().await;
            let customs: Vec<Value> = session
                .tree()
                .customs()
                .iter()
                .map(|custom| {
                    json!({
                        "id": custom.id,
                        "customType": custom.custom_type,
                        "data": custom.data,
                    })
                })
                .collect();
            json!({ "entries": customs })
        }
        "set_session_name" => {
            let Some(name) = payload.get("name").and_then(Value::as_str) else {
                return json!({ "error": "no name to set" });
            };
            match session.lock().await.rename(name).await {
                Ok(()) => json!({ "ok": true }),
                Err(error) => json!({ "error": error.to_string() }),
            }
        }
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
async fn carry_out(
    action: &str,
    payload: &Value,
    asker: Option<&micro_tui::UiAsker>,
    host: Option<&Arc<Host>>,
) {
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
        // A custom message is drawn by whoever registered a renderer for its type, and
        // said plainly when nobody did.
        "send_message" => {
            let message = payload.get("message").cloned().unwrap_or(Value::Null);
            let custom_type = message
                .get("customType")
                .and_then(Value::as_str)
                .unwrap_or("message")
                .to_string();
            let content = message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            let drawn = match host {
                Some(host) if host.draws(&custom_type) => Some(
                    host.render(&custom_type, &message, RENDER_WIDTH)
                        .await
                        .unwrap_or_else(|error| vec![format!("could not be drawn: {error}")]),
                ),
                _ => None,
            };
            let lines =
                drawn.unwrap_or_else(|| content.lines().map(str::to_string).collect());
            if lines.is_empty() {
                return;
            }
            if let Some(asker) = asker {
                asker.ask("custom_message", custom_type, None, lines).await;
            }
        }
        // Both of these change what the next turn runs, which is the interface's to do:
        // it holds the agent, and a command is how anything else asks it to.
        "set_thinking_level" => {
            if let (Some(asker), Some(level)) =
                (asker, payload.get("level").and_then(Value::as_str))
            {
                asker
                    .ask("send_user_message", format!("/thinking {level}"), None, Vec::new())
                    .await;
            }
        }
        "set_model" => {
            let named = payload
                .get("model")
                .and_then(|model| model.get("id").and_then(Value::as_str))
                .or_else(|| payload.get("model").and_then(Value::as_str));
            if let (Some(asker), Some(model)) = (asker, named) {
                asker
                    .ask("send_user_message", format!("/model {model}"), None, Vec::new())
                    .await;
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
        // Not a question: a line an extension keeps in the footer. Clearing it is saying
        // it with no text.
        "setStatus" => {
            asker
                .ask(
                    "set_status",
                    text("statusKey").unwrap_or_default(),
                    text("statusText"),
                    Vec::new(),
                )
                .await
        }
        // Anything else has nowhere to be shown, and saying so beats pretending.
        other => json!({ "cancelled": true, "error": format!("micro cannot show `{other}`") }),
    }
}

/// How wide a renderer is told the screen is.
///
/// A guess rather than the real width: the pump is not the interface and does not know it.
/// A renderer that cares can wrap for itself.
const RENDER_WIDTH: usize = 80;

/// What micro is running, as an extension asking would see it.
///
/// Kept beside the pump rather than reached for through the agent: the agent belongs to
/// whoever is driving the run, and an extension asking a question must not have to wait
/// for a turn to finish.
#[derive(Debug, Default)]
pub struct State {
    pub thinking: String,
    pub model: String,
    pub provider: String,
    pub tools: Vec<String>,
    pub commands: Vec<String>,
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

/// Ask the extensions whether something may go ahead, before it does.
///
/// One handler answering `{ cancel: true }` stops it. Everything else lets it through:
/// no answer, an answer without the field, or nobody listening. Refusing has to be said
/// outright, so a handler that only wanted to watch never blocks anything.
pub async fn cancelled(host: Option<&Arc<Host>>, event: &str, payload: Value) -> bool {
    consult(host, event, payload).await.iter().any(|answer| {
        answer
            .get("cancel")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
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
impl Hooks for ExtensionHooks {
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

    async fn before_agent_start(&self, prompt: &micro_types::Message) -> Option<micro_types::Message> {
        let answers = self
            .host
            .ask_event("before_agent_start", json!({ "message": prompt }))
            .await
            .ok()?;

        // The last replacement wins, so an extension later in the list has the final say.
        answers
            .iter()
            .filter_map(|answer| answer.get("message"))
            .filter_map(|message| serde_json::from_value(message.clone()).ok())
            .next_back()
    }

    async fn before_request(&self, context: micro_types::Context) -> micro_types::Context {
        let asked = self
            .host
            .ask_event(
                "context",
                json!({
                    "systemPrompt": context.system_prompt,
                    "messages": context.messages,
                }),
            )
            .await;

        let Ok(answers) = asked else {
            return context;
        };

        // Each answer is applied in turn, so a later extension sees what an earlier one
        // changed rather than what the agent originally assembled.
        let mut context = context;
        for answer in answers {
            if let Some(prompt) = answer.get("systemPrompt").and_then(Value::as_str) {
                context.system_prompt = Some(prompt.to_string());
            }
            if let Some(messages) = answer.get("messages") {
                if let Ok(replaced) = serde_json::from_value(messages.clone()) {
                    context.messages = replaced;
                }
            }
        }

        // Announced separately, because ohm reports the request itself as its own moment.
        let _ = self
            .host
            .notify(
                "before_provider_request",
                json!({ "messageCount": context.messages.len() }),
            )
            .await;

        // Headers are their own moment, and their own answer: what comes back is put on
        // the request, replacing anything the provider would have set itself.
        if let Ok(answers) = self
            .host
            .ask_event("before_provider_headers", json!({ "headers": {} }))
            .await
        {
            for answer in answers {
                let Some(headers) = answer.get("headers").and_then(Value::as_object) else {
                    continue;
                };
                for (name, value) in headers {
                    let Some(value) = value.as_str() else { continue };
                    context.headers.retain(|(held, _)| held != name);
                    context.headers.push((name.clone(), value.to_string()));
                }
            }
        }
        context
    }

    async fn after_response(&self, message: &micro_types::AssistantMessage) {
        let _ = self
            .host
            .notify(
                "after_provider_response",
                json!({
                    "message": message,
                    "usage": message.usage,
                    "stopReason": format!("{:?}", message.stop_reason),
                }),
            )
            .await;
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

    /// A scratch session, so a question about the session has one to ask about.
    async fn scratch_session() -> Arc<tokio::sync::Mutex<micro_session::Session>> {
        let root = std::env::temp_dir().join(format!(
            "micro-extensions-state-{}-{}",
            std::process::id(),
            micro_types::now_ms()
        ));
        let store = micro_session::SessionStore::new(root.join("sessions"));
        let session = store
            .create(&root, "anthropic/claude-opus-5")
            .await
            .expect("a session");
        Arc::new(tokio::sync::Mutex::new(session))
    }

    #[tokio::test]
    async fn a_request_micro_does_not_know_is_answered_rather_than_ignored() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let answer = answer(
            "fly",
            &json!({}),
            &std::env::temp_dir(),
            &state,
            &scratch_session().await,
        )
        .await;
        assert!(answer["error"].as_str().unwrap().contains("fly"));
    }

    /// What is running is answered from what the run knows, not made up.
    #[tokio::test]
    async fn an_extension_can_ask_what_is_running() {
        let state = Arc::new(tokio::sync::RwLock::new(State {
            thinking: "high".into(),
            model: "gemini-3-pro".into(),
            provider: "openrouter".into(),
            tools: vec!["read".into(), "write".into()],
            commands: vec!["help".into()],
        }));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        let level = answer("get_thinking_level", &json!({}), &workspace, &state, &session).await;
        assert_eq!(level["level"], "high");

        let tools = answer("get_active_tools", &json!({}), &workspace, &state, &session).await;
        assert_eq!(tools["tools"][0], "read");

        let model = answer("get_model", &json!({}), &workspace, &state, &session).await;
        assert_eq!(model["model"]["id"], "gemini-3-pro");
        assert_eq!(model["model"]["provider"], "openrouter");

        let commands = answer("get_commands", &json!({}), &workspace, &state, &session).await;
        assert_eq!(commands["commands"][0], "help");
    }

    /// Naming the session takes effect, and asking gives the name back.
    #[tokio::test]
    async fn an_extension_can_name_the_session_and_read_it_back() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        let unnamed = answer("get_session_name", &json!({}), &workspace, &state, &session).await;
        assert!(unnamed["name"].is_null(), "{unnamed}");

        let set = answer(
            "set_session_name",
            &json!({ "name": "the good one" }),
            &workspace,
            &state,
            &session,
        )
        .await;
        assert_eq!(set["ok"], true);

        let named = answer("get_session_name", &json!({}), &workspace, &state, &session).await;
        assert_eq!(named["name"], "the good one");
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
                None,
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
        carry_out(
            "send_user_message",
            &json!({ "content": "   " }),
            Some(&asker),
            None,
        )
        .await;
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
