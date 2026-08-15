//! Answering what extensions ask for.
//!
//! An extension never reaches into micro. It asks — for a command to be run, for the
//! session to be renamed, for the user to be told something — and this decides what
//! happens. That is what keeps someone else's code inside the same rules as everything
//! else: the ask arrives here, and here is where the workspace and the policy are.

use micro_agent::Hooks;
use micro_agent::ToolDecision;
use micro_extensions::message_from_json;
use micro_extensions::message_json;
use micro_extensions::Capability;
use micro_extensions::FromHost;
use micro_extensions::Grants;
use micro_extensions::Host;
use micro_extensions::Translator;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Where a fact about what an extension asked for goes.
///
/// The same channel the tools report a refused command on, so an extension crossing the
/// broker and a program the sandbox stopped are written to the session log in the order
/// they happened rather than by two paths that could interleave.
///
/// Held weakly. The log is finished by every way into it being let go, and the pump that
/// answers the extensions outlives the run — it stops only when the host it is answering
/// for is gone, which is after the log has been closed. A strong hold here would keep the
/// log waiting on a pump that is waiting on a host that nothing has told to leave yet.
pub type Crossings = tokio::sync::mpsc::WeakUnboundedSender<micro_types::LedgerEvent>;

/// Everything an ask is decided against: who may do what, and where the fact that it was
/// asked for is written down.
///
/// Carried together because they are always needed together — deciding without recording
/// would leave a refusal nobody could find afterwards, which is the opposite of the point.
#[derive(Clone)]
pub struct Broker {
    pub grants: Arc<Grants>,
    pub crossings: Option<Crossings>,
}

impl Broker {
    /// A broker that permits everything and records nothing, for a caller with no
    /// extensions loaded — and for the tests that are about what a request reaches rather
    /// than about who may reach it.
    pub fn open() -> Broker {
        Broker {
            grants: Arc::new(Grants::default()),
            crossings: None,
        }
    }

    /// Whether this ask may go ahead, recording either way what was asked for.
    ///
    /// `needs` is the capability the ask falls under, `name` what was asked for within it —
    /// the program for an `exec`, the method for a question put to the reader. What the
    /// ledger carries is the pair, so a bill can attribute a turn's spending to the
    /// extension that caused it and an audit can find what was refused.
    fn allows(&self, extension: Option<&str>, needs: Capability, name: &str) -> bool {
        let allowed = self.grants.allows(extension, needs);
        // Drawing and asking the reader questions happen many times a second while an
        // extension animates a widget, and a log of them would bury everything else in the
        // session. A refusal is still worth a line: it happens once, and it is the reason
        // something the reader expected to see is not there.
        if allowed && matches!(needs, Capability::Ui) {
            return true;
        }
        self.record(extension, needs.as_str(), name, allowed, None);
        allowed
    }

    /// Write down that an extension crossed the broker, whether or not it was let through.
    fn record(
        &self,
        extension: Option<&str>,
        kind: &str,
        name: &str,
        allowed: bool,
        detail: Option<Value>,
    ) {
        // Nothing to write to once the run's own log has been closed, which is what a
        // crossing arriving during teardown finds. The run is over; there is nothing left
        // to account for.
        let Some(crossings) = self.crossings.as_ref().and_then(|crossings| crossings.upgrade())
        else {
            return;
        };
        // Nothing to attribute is nothing to record: an ask that arrived without an
        // extension on it did not come from one.
        let Some(extension) = extension else {
            return;
        };
        let _ = crossings.send(micro_types::LedgerEvent::ExtensionCrossing {
            extension: self.grants.name_of(Some(extension)),
            kind: kind.to_string(),
            name: name.to_string(),
            allowed,
            detail,
        });
    }

    /// What an extension is told when it asked for something it may not do.
    ///
    /// Named rather than generic, and the same wording wherever it is said, so an extension
    /// can catch it and do something else instead of failing at whatever it tried next.
    fn refusal(&self, extension: Option<&str>, needs: Capability) -> String {
        format!(
            "capability '{}' not granted to {}",
            needs,
            self.grants.name_of(extension)
        )
    }

    /// Which of an event's answers may change what micro does.
    ///
    /// An extension answers an event by returning something from its handler, and that
    /// answer is how it rewrites a tool call, replaces the conversation or sets a header.
    /// Without the capability the handler still runs — it is running inside its own
    /// process, where nothing here reaches — but what it said is not acted on, and the fact
    /// that it tried is recorded.
    fn heeded(&self, answers: Vec<(Option<String>, Value)>, needs: Capability, name: &str) -> Vec<Value> {
        self.heeded_from(answers, needs, name)
            .into_iter()
            .map(|(_, answer)| answer)
            .collect()
    }

    /// The same, keeping who said what — for an answer whose being acted on is itself worth
    /// recording against the extension that gave it.
    fn heeded_from(
        &self,
        answers: Vec<(Option<String>, Value)>,
        needs: Capability,
        name: &str,
    ) -> Vec<(Option<String>, Value)> {
        answers
            .into_iter()
            .filter(|(source, _)| {
                let allowed = self.grants.allows(source.as_deref(), needs);
                if !allowed {
                    self.record(source.as_deref(), needs.as_str(), name, false, None);
                }
                allowed
            })
            .collect()
    }
}

/// The capability a request needs, or nothing when it only reads.
///
/// Every `get_*` is answered for anyone: reading what model is running or what the session
/// is called tells an extension about the run it is already part of, and refusing it would
/// break every extension without protecting anything.
fn request_needs(request: &str) -> Option<Capability> {
    match request {
        "exec" => Some(Capability::Exec),
        "run_builtin_tool" => Some(Capability::BuiltinTools),
        "provider_stream" => Some(Capability::ProviderStream),
        "append_entry" | "set_label" | "set_session_name" => Some(Capability::SessionWrite),
        // Asked rather than acted on, since `setModel` answers whether it could — and
        // confined the same way the action of that name is.
        "set_model" => Some(Capability::SessionControl),
        "reload" | "new_session" | "switch_session" | "navigate_tree" | "fork" => {
            Some(Capability::SessionControl)
        }
        _ => None,
    }
}

/// The capability an action needs, or nothing when micro does not know the action at all —
/// which is answered by saying so rather than by refusing it.
fn action_needs(action: &str) -> Option<Capability> {
    match action {
        "send_user_message" => Some(Capability::SendUserMessage),
        "send_message" => Some(Capability::SendMessage),
        // Which tools the model is told about is part of the request's own cacheable head,
        // so narrowing it is changing what the model is told rather than moving the
        // conversation — the same capability that covers rewriting the prompt.
        "set_active_tools" => Some(Capability::Context),
        // Written rather than asked about: pi answers nothing for these, so they arrive as
        // actions and are confined the same way the requests of the same name were.
        "append_entry" | "set_label" | "set_session_name" => Some(Capability::SessionWrite),
        "set_thinking_level" | "set_model" | "shutdown" | "compact" | "abort" => {
            Some(Capability::SessionControl)
        }
        "watch_terminal_input" | "unwatch_terminal_input" | "watch_autocomplete" => {
            Some(Capability::Ui)
        }
        _ => None,
    }
}

/// Answer whatever the extensions ask for, for as long as the host is running.
///
/// The stream of asks is taken out of the host first: waiting on it through the host would
/// hold its lock, and then nothing could be answered while nothing was being asked.
pub async fn serve(
    host: Arc<Host>,
    workspace: PathBuf,
    // Handed over rather than looked up: what confines a command is settled once, where the
    // run is assembled, and a task that had to go and find it could be handed a different
    // answer than the tools were built around.
    sandbox: micro_sandbox::Sandbox,
    // Who may ask for what, and where the fact that they asked is written down. Settled the
    // same way and at the same moment as the policy above, and for the same reason.
    broker: Broker,
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
                extension,
                payload,
            } => {
                let answer = answer(
                    &request,
                    &payload,
                    extension.as_deref(),
                    &workspace,
                    &sandbox,
                    &broker,
                    &state,
                    &session,
                    asker.as_ref(),
                )
                .await;
                if host.answer(&id, answer).await.is_err() {
                    break;
                }
            }
            // An action is carried out where it belongs; nothing goes back.
            FromHost::Action {
                action,
                extension,
                payload,
            } => {
                carry_out(
                    &action,
                    &payload,
                    extension.as_deref(),
                    &broker,
                    asker.as_ref(),
                    Some(&host),
                    Some(&state),
                    Some(&session),
                )
                .await
            }
            // `select`/`confirm`/`input`/`custom`/`editor` are questions: they stay open
            // until the reader answers them, which can be an arbitrarily long wait, and
            // while one is open the next `ui_request` — `customDone`, closing this same
            // question from the other side, among them — still has to reach the front of
            // this stream rather than queue up behind a question nobody has answered yet.
            // Those five are spawned for exactly that reason. Everything else `show`
            // answers — a status line, a notification, a widget, a header, a footer — is
            // answered at once and stays in this loop, so two of the same kind sent back to
            // back are still handled in the order they arrived: nothing here needs the
            // concurrency, and losing the order would mean the second could be overtaken by
            // the first.
            FromHost::Ui {
                id,
                extension,
                payload,
            } => {
                let method = payload.get("method").and_then(Value::as_str).unwrap_or_default();
                let waits_on_a_reader = matches!(method, "select" | "confirm" | "input" | "custom" | "editor");
                if waits_on_a_reader {
                    let asker = asker.clone();
                    let host = Arc::clone(&host);
                    let broker = broker.clone();
                    tokio::spawn(async move {
                        let answer =
                            show(&payload, extension.as_deref(), &broker, asker.as_ref(), Some(&host)).await;
                        if let Some(id) = id {
                            let _ = host.answer(&id, answer).await;
                        }
                    });
                } else {
                    let answer =
                        show(&payload, extension.as_deref(), &broker, asker.as_ref(), Some(&host)).await;
                    if let Some(id) = id {
                        if host.answer(&id, answer).await.is_err() {
                            break;
                        }
                    }
                }
            }
            // A component said its own lines are stale; fetched here, before the
            // interface is told, the same way `send_message` fetches a custom message's
            // lines before passing them on — the interface draws what it is given rather
            // than reaching back across the pipe itself.
            FromHost::ComponentChanged { component_id } => {
                if let Some(asker) = asker.as_ref() {
                    let lines = host
                        .render_component(&component_id, RENDER_WIDTH)
                        .await
                        .unwrap_or_default();
                    asker
                        .ask("component_changed", component_id, None, lines)
                        .await;
                }
            }
            FromHost::Failed { path, event, error } => {
                eprintln!("note: {path} failed handling {event}: {error}");
            }
        }
    }

    // Nothing is asking anymore, which means the host is gone: it exited, it was killed, or
    // it stopped answering. Whatever it left in the interface and on the agent would
    // outlive it — a widget nothing can redraw, a tool nothing can run — so it is taken
    // back here, the same way a deliberate deactivation takes it back.
    reclaim(&host, &broker, asker.as_ref()).await;
}

/// Take back everything the extensions were granted, and say so in the ledger.
///
/// Only what micro itself gave out: the tools it offered the model and what the interface
/// is drawing. What the extension did out in the world is its own to undo, which is what
/// the `deactivate` message it is sent beforehand is for.
async fn reclaim(host: &Arc<Host>, broker: &Broker, asker: Option<&micro_tui::UiAsker>) {
    for extension in &host.loaded().extensions {
        broker.record(
            Some(&extension.path),
            "deactivate",
            "host",
            true,
            Some(json!({ "tools": extension.tools.len() })),
        );
        let Some(asker) = asker else {
            continue;
        };
        let tools: Vec<String> = extension
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect();
        asker
            .ask("deactivate_extension", extension.path.clone(), None, tools)
            .await;
    }
}

/// Offer the host every key the interface reads, for as long as `ctx.ui.onTerminalInput`
/// has something registered.
///
/// A channel of its own rather than another arm on [`serve`]'s: this one runs the other
/// way — the interface asks, the host answers — so it has no `FromHost` case to share, and
/// mixing the two into one loop would make a keystroke wait behind whatever `serve` was
/// last doing instead of the other way around.
pub async fn serve_terminal_input(host: Arc<Host>, mut asks: micro_tui::TerminalInputAsks) {
    while let Some(mut ask) = asks.recv().await {
        let answers = host
            .ask_event("terminal_input", json!({ "data": ask.data }))
            .await
            .unwrap_or_default();
        let consumed = answers.iter().any(|answer| {
            answer
                .get("consume")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
        ask.answer(json!({ "consume": consumed }));
    }
}

/// Answer whatever the interface asks off its render path: a keystroke for a `custom()`
/// overlay that has focus, a completion list for an extension's own menu, what committing
/// one of its items should write. A channel of its own for the same reason
/// [`serve_terminal_input`]'s is: this runs the other way from [`serve`], and mixing the two
/// into one loop would make an answer wait behind whatever `serve` was last doing.
pub async fn serve_host_asks(host: Arc<Host>, mut asks: micro_tui::HostAsks) {
    while let Some(mut ask) = asks.recv().await {
        let answer = match ask.event.as_str() {
            "component_input" => {
                let component_id = ask
                    .payload
                    .get("componentId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let data = ask.payload.get("data").and_then(Value::as_str).unwrap_or_default();
                // Only `setEditorComponent`'s replacement shares the built-in editor's
                // buffer, so only its keys arrive carrying the text to go with them.
                let text = ask.payload.get("text").and_then(Value::as_str);
                let consumed = host
                    .send_component_input(component_id, data, text)
                    .await
                    .unwrap_or(false);
                let lines = component_lines(Some(&host), component_id).await;
                json!({ "consume": consumed, "lines": lines })
            }
            // What `getSuggestions` answered, or nothing when there is no
            // `addAutocompleteProvider` chain registered to ask — `ask_event` already tells
            // the two apart, an empty `results` for the second and one entry for the first,
            // since a run's extensions share one process and its provider chain is one.
            "get_suggestions" => host
                .ask_event("get_suggestions", ask.payload.clone())
                .await
                .ok()
                .and_then(|results| results.into_iter().next())
                .unwrap_or_else(|| json!({})),
            // What `applyCompletion` answered: the edit committing the chosen item makes.
            "apply_completion" => host
                .ask_event("apply_completion", ask.payload.clone())
                .await
                .ok()
                .and_then(|results| results.into_iter().next())
                .unwrap_or_else(|| json!({})),
            other => json!({ "error": format!("micro cannot answer `{other}`") }),
        };
        ask.answer(answer);
    }
}

/// What an extension gets back for a question.
///
/// A question outside what the extension was granted is answered by name — the same wording
/// every time, so an extension can catch it — and the run carries on. Refusing is not an
/// error in micro; it is the answer.
#[allow(clippy::too_many_arguments)]
async fn answer(
    request: &str,
    payload: &Value,
    extension: Option<&str>,
    workspace: &PathBuf,
    sandbox: &micro_sandbox::Sandbox,
    broker: &Broker,
    state: &Arc<tokio::sync::RwLock<State>>,
    session: &Arc<tokio::sync::Mutex<micro_session::Session>>,
    asker: Option<&micro_tui::UiAsker>,
) -> Value {
    if let Some(needs) = request_needs(request) {
        // Named for the ledger by what was actually asked for, not only by the request:
        // "exec git" says more after the fact than "exec" does.
        let named = match request {
            "exec" => payload.get("command").and_then(Value::as_str).unwrap_or(request),
            "run_builtin_tool" => payload.get("tool").and_then(Value::as_str).unwrap_or(request),
            other => other,
        };
        if !broker.allows(extension, needs, named) {
            return json!({ "error": broker.refusal(extension, needs) });
        }
    }

    match request {
        "exec" => exec(payload, workspace, sandbox).await,
        "run_builtin_tool" => run_builtin_tool(payload, workspace, sandbox).await,
        // The pi-ai compatibility shim's facade for provider streaming and the model
        // catalog — see `crates/micro-extensions/host/compat/ai/**`. Both answer from
        // micro's own provider clients and catalog rather than a second copy of either.
        "provider_stream" => provider_stream(payload).await,
        "model_catalog" => model_catalog(payload),
        "get_thinking_level" => json!({ "level": state.read().await.thinking }),
        "get_active_tools" | "get_all_tools" => json!({ "tools": state.read().await.tools }),
        "get_commands" => json!({ "commands": state.read().await.commands }),
        "get_model" => {
            let state = state.read().await;
            json!({
                "model": {
                    "id": state.model,
                    "name": state.model_name,
                    "provider": state.provider,
                    "contextWindow": state.context_window,
                    "maxOutputTokens": state.max_output_tokens,
                    "reasoning": state.reasoning,
                },
            })
        }
        "get_system_prompt" => json!({ "systemPrompt": state.read().await.system_prompt }),
        // Everything the context every extension call is built from, in one round trip
        // rather than the three `get_model`/`get_thinking_level`/`get_system_prompt`
        // would otherwise cost: this is asked once per tool call, per command and per
        // event, so the extra requests would be paid on every one of them rather than
        // only by whichever extension actually reads `micro.getModel()` and the rest.
        "get_context" => {
            let state = state.read().await;
            let scoped_models = resolve_scoped_models(&state.scoped_models);
            // A session nobody has named answers with nothing rather than an empty string,
            // which is the difference `getSessionName()`'s `string | undefined` describes.
            let session_name = {
                let title = session.lock().await.meta().title.clone();
                match title.is_empty() {
                    true => Value::Null,
                    false => Value::String(title),
                }
            };
            let mut response = json!({
                "model": {
                    "id": state.model,
                    "name": state.model_name,
                    "provider": state.provider,
                    "contextWindow": state.context_window,
                    "maxOutputTokens": state.max_output_tokens,
                    "reasoning": state.reasoning,
                },
                "thinkingLevel": state.thinking,
                "systemPrompt": state.system_prompt,
                "scopedModels": scoped_models,
                // pi answers `getActiveTools()` and `getSessionName()` synchronously, so
                // both ride along with every snapshot rather than being fetched when one
                // is asked for.
                "activeTools": state.tools,
                "allTools": state.all_tools,
                "commands": state.all_commands,
                "sessionName": session_name,
                "session": session_snapshot(&*session.lock().await),
            });
            // `getSystemPromptOptions()` is only ever offered on a command's own
            // context, never a tool's or an event handler's — see `commandContext` on
            // `contextFor` in `context.ts` — so this is worth assembling only when the
            // caller says this snapshot is for one.
            if payload.get("commandContext").and_then(Value::as_bool).unwrap_or(false) {
                response["systemPromptOptions"] = system_prompt_options(&state);
            }
            response
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
        // Answered rather than acted on and forgotten, because pi's `setModel` reports
        // whether it could — false when nothing is signed in to the service the chosen
        // model is served by. The change itself is still the interface's to make, so this
        // asks for it the same way the action did and reports that the ask went through.
        "set_model" => {
            let named = payload
                .get("model")
                .and_then(|model| model.get("id").and_then(Value::as_str))
                .or_else(|| payload.get("model").and_then(Value::as_str));
            let Some(model) = named else {
                return json!({ "ok": false, "error": "no model to switch to" });
            };
            let Some(asker) = asker else {
                return json!({ "ok": false, "error": "there is no interface to switch in" });
            };
            asker
                .ask(
                    "send_user_message",
                    format!("/model {model}"),
                    None,
                    Vec::new(),
                )
                .await;
            json!({ "ok": true })
        }
        // These five all move the conversation somewhere else, which is the interface's
        // to do: it holds the agent, and a slash command is how anything else asks it to.
        // Typed as if the reader had asked for it themselves, the same way `/model` and
        // `/thinking` already are above — reusing a path that is already trusted with
        // arbitrary user text rather than opening a second one just for extensions.
        //
        // What comes back says only that the request was queued, not that it has
        // finished: this path has no way to be told when a fork or a resume completes,
        // only that the interface accepted the line. A refusal from an extension's own
        // `session_before_*` handler, or a session that will not read, both still happen
        // — they just are not reported back through this particular answer.
        "reload" => queued(asker, "/reload").await,
        "new_session" => queued(asker, "/new").await,
        "switch_session" => match payload.get("sessionPath").and_then(Value::as_str) {
            Some(session_path) => queued(asker, &format!("/resume {session_path}")).await,
            None => json!({ "cancelled": true, "error": "no session to switch to" }),
        },
        "navigate_tree" => match payload.get("targetId").and_then(Value::as_str) {
            Some(target_id) => queued(asker, &format!("/tree {target_id}")).await,
            None => json!({ "cancelled": true, "error": "no entry to navigate to" }),
        },
        "fork" => {
            let Some(entry_id) = payload.get("entryId").and_then(Value::as_str) else {
                return json!({ "cancelled": true, "error": "no entry to fork from" });
            };
            let Some(position) = session.lock().await.tree().position_on_path(entry_id) else {
                return json!({
                    "cancelled": true,
                    "error": format!("{entry_id} is not on the current conversation"),
                });
            };
            // `/fork` takes through_index inclusive of the entry, which is what "at"
            // means; "before" is one short of it.
            let before = payload.get("position").and_then(Value::as_str) == Some("before");
            let Some(through) = (if before { position.checked_sub(1) } else { Some(position) })
            else {
                return json!({ "cancelled": true, "error": "nothing comes before the first entry" });
            };
            queued(asker, &format!("/fork {through}")).await
        }
        other => json!({ "error": format!("micro cannot answer `{other}`") }),
    }
}

/// Type a line into the interface as if the reader had, and say whether there was an
/// interface to type it into.
///
/// Used for the handful of requests answered by dispatching a slash command rather than
/// by reading state directly: what comes back from the interface is only an
/// acknowledgement that the line was queued, so that is all this reports too.
async fn queued(asker: Option<&micro_tui::UiAsker>, line: &str) -> Value {
    match asker {
        Some(asker) => {
            asker
                .ask("send_user_message", line.to_string(), None, Vec::new())
                .await;
            json!({ "cancelled": false })
        }
        // With no interface to type it into, this is the same as the reader cancelling.
        None => json!({ "cancelled": true }),
    }
}

/// The models a `--models`/scoped-models setting matches, resolved against the catalog —
/// what pi calls `scopedModels`.
///
/// Loaded fresh rather than carried through the extension host for the life of the run:
/// scoping is unset for most runs, so the read only costs anything on the runs that asked
/// for it. This is the same prefix match `/model`'s own shortlist uses.
fn resolve_scoped_models(patterns: &[String]) -> Value {
    if patterns.is_empty() {
        return Value::Array(Vec::new());
    }
    let catalog =
        micro_models::Catalog::load().unwrap_or_else(|_| micro_models::Catalog::bundled());
    let matched: Vec<Value> = catalog
        .models()
        .iter()
        .filter(|model| {
            patterns.iter().any(|pattern| {
                model.qualified_id().starts_with(pattern.as_str())
                    || model.id.starts_with(pattern.as_str())
            })
        })
        .map(|model| {
            json!({
                "model": {
                    "id": model.id,
                    "name": model.name,
                    "provider": model.provider,
                    "contextWindow": model.context_window,
                    "maxOutputTokens": model.max_output_tokens,
                    "reasoning": model.reasoning,
                },
                // micro's scoping is a plain prefix list; it carries no per-model
                // thinking-level override the way pi's "model:high" pattern syntax does.
                "thinkingLevel": Value::Null,
            })
        })
        .collect();
    Value::Array(matched)
}

/// The tool snippets and guidelines that went into the system prompt's tools section —
/// structured here rather than flattened into the prose `micro_extensions::prompt_section`
/// builds, but filtered the same way it filters: only a tool actually offered to the model
/// contributes its snippet or its guidelines, the same as a tool the run left out
/// contributes neither to the prompt itself.
///
/// Called once, at the same point `State` itself is built, rather than per `get_context`
/// request: what a loaded extension registered does not change over the life of a run, so
/// there is nothing to gain by asking again on every command, and `State` holding the
/// result rather than a handle onto the host is what keeps answering a plain request from
/// needing a running one to test against.
/// Every tool that exists, as `getAllTools()` describes one.
///
/// A tool registered by an extension is named by the file that registered it; a built-in
/// belongs to micro itself and says so, rather than borrowing a path it never came from.
pub fn all_tools(
    registered: &[micro_extensions::Registered],
    builtin: &[micro_types::ToolDefinition],
    names: &[String],
) -> Value {
    let described: Vec<Value> = names
        .iter()
        .map(|name| {
            // A built-in describes itself; only an extension's tool has to be looked up by
            // the extension that registered it.
            let own = builtin.iter().find(|tool| &tool.name == name);
            // Which extension registered it, since the path belongs to the extension
            // rather than to the tool. Nothing found means micro's own.
            let owner = registered
                .iter()
                .find(|extension| extension.tools.iter().any(|tool| &tool.name == name));
            let found = owner
                .and_then(|extension| extension.tools.iter().find(|tool| &tool.name == name));
            let source = owner.map(|extension| extension.path.clone()).unwrap_or_default();
            json!({
                "name": name,
                "description": found
                    .map(|tool| tool.description.clone())
                    .or_else(|| own.map(|tool| tool.description.clone()))
                    .unwrap_or_default(),
                "parameters": found
                    .map(|tool| tool.parameters.clone())
                    .or_else(|| own.map(|tool| tool.parameters.clone()))
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                "promptGuidelines": found
                    .map(|tool| tool.prompt_guidelines.clone())
                    .unwrap_or_default(),
                "sourceInfo": {
                    "path": source,
                    "source": match found {
                        Some(_) => "extension",
                        None => "builtin",
                    },
                    "scope": "user",
                    "origin": "top-level",
                },
            })
        })
        .collect();
    Value::Array(described)
}

/// Every command that can be typed, as `getCommands()` describes one.
pub fn all_commands(registered: &[micro_extensions::Registered]) -> Value {
    let mut described: Vec<Value> = micro_commands::commands()
        .iter()
        .map(|command| {
            json!({
                "name": command.name,
                "description": command.description,
                // Nothing micro answers to itself came from an extension, a prompt or a
                // skill, which are the three sources pi distinguishes. Saying "extension"
                // for a built-in would be the wrong one of those three, so it says what it
                // is and leaves the path empty.
                "source": "builtin",
                "sourceInfo": {
                    "path": "",
                    "source": "builtin",
                    "scope": "user",
                    "origin": "top-level",
                },
            })
        })
        .collect();
    described.extend(registered.iter().flat_map(|extension| {
        extension.commands.iter().map(|command| {
            json!({
                "name": command.name,
                "description": command.description,
                "source": "extension",
                "sourceInfo": {
                    "path": extension.path,
                    "source": "extension",
                    "scope": "user",
                    "origin": "top-level",
                },
            })
        })
    }));
    Value::Array(described)
}

pub fn tool_prompt_options(tools: &[micro_extensions::RegisteredTool], active: &[String]) -> (Value, Vec<String>) {
    let active: std::collections::HashSet<&str> = active.iter().map(String::as_str).collect();
    let mut snippets = serde_json::Map::new();
    let mut guidelines = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tool in tools.iter().filter(|tool| active.contains(tool.name.as_str())) {
        if let Some(snippet) = tool
            .prompt_snippet
            .as_deref()
            .map(str::trim)
            .filter(|snippet| !snippet.is_empty())
        {
            snippets.insert(tool.name.clone(), json!(snippet));
        }
        for guideline in &tool.prompt_guidelines {
            let normalized = guideline.trim();
            if !normalized.is_empty() && seen.insert(normalized.to_string()) {
                guidelines.push(normalized.to_string());
            }
        }
    }
    (Value::Object(snippets), guidelines)
}

/// pi's `BuildSystemPromptOptions` — what went into the system prompt, not only what came
/// out of it. Answered only for a command's own context (see the `commandContext` check on
/// `get_context`), since pi itself only ever offers `getSystemPromptOptions()` there.
///
/// `cwd` is left for the caller to fill in from what it already has rather than sent here:
/// it never changes for the run, and `context.ts` already keeps it.
fn system_prompt_options(state: &State) -> Value {
    json!({
        "customPrompt": state.custom_prompt,
        "selectedTools": state.tools,
        "toolSnippets": state.tool_snippets,
        "promptGuidelines": state.prompt_guidelines,
        "appendSystemPrompt": state.appended_prompt,
        "contextFiles": state.context_files.iter().map(|(path, content)| json!({
            "path": path.display().to_string(),
            "content": content,
        })).collect::<Vec<_>>(),
        "skills": state.skills.iter().map(|skill| {
            let base_dir = skill.base_dir.display().to_string();
            let path = skill.path.display().to_string();
            // micro's own three sources — "project", "user", the literal path a
            // `--skill`/extension-added one was read from — map onto pi's `SourceInfo`
            // the way `createSyntheticSourceInfo` does for a skill with nowhere more
            // specific to point to: "path" is not a standing, rediscoverable location
            // the way the other two are, so it reads as "temporary" there too.
            let scope = match skill.source.as_str() {
                "project" => "project",
                "user" => "user",
                _ => "temporary",
            };
            json!({
                "name": skill.name,
                "description": skill.description,
                "filePath": path,
                "baseDir": base_dir,
                "sourceInfo": {
                    "path": path,
                    "source": skill.source,
                    "scope": scope,
                    "origin": "top-level",
                    "baseDir": base_dir,
                },
                "disableModelInvocation": !skill.model_invocable,
            })
        }).collect::<Vec<_>>(),
    })
}

/// Everything pi's `ReadonlySessionManager` needs, gathered in one pass so building all
/// fourteen of its read-only methods on the other side of the wire costs one round trip
/// rather than fourteen — the same reasoning `get_context` itself is built on.
///
/// The entries carry only what micro's own log holds: a message, something an extension
/// kept beside the conversation, or a compaction. pi's session log can also carry an entry
/// for a thinking-level change, a model change, a branch summary, or a display name —
/// micro does not record any of those as their own log entries, so those four `type`s of
/// pi's `SessionEntry` union simply never appear here. What is here is exactly what the
/// log holds, not a guess at what the others would have said.
fn session_snapshot(session: &micro_session::Session) -> Value {
    let tree = session.tree();
    let meta = session.meta();

    let mut entries: Vec<Value> = tree
        .entries()
        .iter()
        .map(|entry| {
            json!({
                "type": "message",
                "id": entry.id,
                "parentId": entry.parent_id,
                "timestamp": entry.timestamp,
                // Not `entry.message` as it stands: micro serializes a `Message` with
                // its own snake_case field names (`tool_call_id`, `is_error`, and so
                // on), and pi's handlers are written against camelCase
                // (`toolCallId`, `isError`) and a tool result's role spelled
                // `toolResult` rather than `tool_result`. `message_json` is the same
                // conversion the lifecycle events already go through, on the other
                // side of the same mismatch.
                "message": message_json(&entry.message),
            })
        })
        .collect();
    entries.extend(tree.customs().iter().map(|custom| {
        json!({
            "type": "custom",
            "id": custom.id,
            "parentId": custom.parent_id,
            "timestamp": custom.timestamp,
            "customType": custom.custom_type,
            "data": custom.data,
        })
    }));
    // A compaction has no id of its own in micro's log — it is recorded by the entry it
    // followed, not as an addressable entry — so one is made up here rather than left
    // out, distinct enough from a real entry id that it cannot collide with one.
    entries.extend(tree.compactions().iter().map(|compaction| {
        json!({
            "type": "compaction",
            "id": format!("compaction-{}", compaction.entry_id),
            "parentId": compaction.entry_id,
            "timestamp": compaction.timestamp,
            "summary": compaction.summary,
            "firstKeptEntryId": compaction.first_kept,
        })
    }));

    // Every entry's label, where it has one. Looked up one at a time because that is the
    // only way `Tree` gives a label back — there is no single call that hands over all of
    // them at once.
    let labelled: Vec<&String> = tree
        .entries()
        .iter()
        .map(|entry| &entry.id)
        .chain(tree.customs().iter().map(|custom| &custom.id))
        .collect();
    let labels: serde_json::Map<String, Value> = labelled
        .into_iter()
        .filter_map(|id| tree.label(id).map(|label| (id.clone(), json!(label))))
        .collect();

    json!({
        "cwd": meta.workspace.display().to_string(),
        "dir": session.path().parent().map(|dir| dir.display().to_string()),
        "id": session.id(),
        "file": session.path().display().to_string(),
        "name": (!meta.title.is_empty()).then(|| meta.title.clone()),
        "leafId": tree.head(),
        "header": {
            "id": meta.id,
            // Milliseconds since the epoch, not pi's ISO 8601 string: nothing in this
            // workspace formats one today, and adding a date-formatting dependency for a
            // single field felt like more than the field was worth. A real timestamp,
            // just not the same shape pi's is.
            "timestamp": meta.created_at,
            "cwd": meta.workspace.display().to_string(),
            "parentSession": meta.parent,
        },
        "entries": entries,
        "labels": labels,
    })
}

/// Run a program on the extension's behalf.
///
/// The command and its arguments are passed as they are written, with no shell between
/// them: an argument holding shell punctuation is an argument, not a second command.
///
/// Confined by the session's own policy. An extension is code the user installed rather
/// than code the model wrote, but it runs inside the same session and reaches the same
/// machine, and a policy that a program could step outside of by asking an extension to
/// ask for it would not be a policy.
async fn exec(payload: &Value, workspace: &PathBuf, sandbox: &micro_sandbox::Sandbox) -> Value {
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

    let wrapped = sandbox.wrap(command, arguments, workspace);
    let confined = wrapped.enforced;
    let finished = tokio::process::Command::from(wrapped.to_std_command())
        .stdin(std::process::Stdio::null())
        .output()
        .await;

    match finished {
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
            let mut answer = json!({
                "stdout": String::from_utf8_lossy(&result.stdout),
                "stderr": stderr.clone(),
                "code": result.status.code().unwrap_or(-1),
            });
            // Said in a field of its own rather than folded into the output: an extension
            // decides what to do about a refusal, and reading the platform's wording out
            // of stderr to find out is not something each of them should have to do.
            if confined && micro_sandbox::is_likely_denied(&result.status, &stderr) {
                answer["denied"] = json!(true);
                answer["policy"] = json!(sandbox.policy().name());
            }
            answer
        }
        Err(error) => json!({ "error": format!("cannot run {command}: {error}") }),
    }
}

/// Run one of micro's own built-in tools on an extension's behalf — what
/// `host/compat/coding-agent/index.ts`'s `createReadTool`/`createBashTool`/`createEditTool`/
/// `createWriteTool`/`createGrepTool`/`createFindTool`/`createLsTool` proxy to, so a pi
/// extension asking for pi's default built-ins gets the real `crates/micro-tools`
/// implementations — the same line-numbered reads, gitignore-aware search and
/// fuzzy-matched edits the model's own tools use — rather than a second, separate
/// implementation living on the TypeScript side of the wire.
///
/// `workspace` is used as the tool's root rather than the `root` field the payload also
/// carries: for every caller here that root is `ctx.cwd`, which is `where.cwd` in
/// `context.ts`, set once from this same `workspace` when the extension host loads — so
/// trusting the value already authoritative on this side costs nothing and asks nothing
/// of the sender.
async fn run_builtin_tool(
    payload: &Value,
    workspace: &PathBuf,
    sandbox: &micro_sandbox::Sandbox,
) -> Value {
    use micro_tools::Tool;

    let Some(tool_name) = payload.get("tool").and_then(Value::as_str) else {
        return json!({ "error": "run_builtin_tool needs a tool name" });
    };
    let arguments = payload.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let root = workspace.clone();
    // The same instances in every respect that matters, the policy included: an extension
    // reaching for micro's own tools gets micro's own tools, held to what the session is
    // held to.
    let guard = micro_tools::Guard::new(sandbox.clone());

    let result: Result<String, String> = match tool_name {
        "read" => micro_tools::Read::new(root, guard).execute(&arguments).await,
        "write" => micro_tools::Write::new(root, guard).execute(&arguments).await,
        "edit" => micro_tools::Edit::new(root, guard).execute(&arguments).await,
        "ls" => micro_tools::Ls::new(root, guard).execute(&arguments).await,
        "find" => micro_tools::Find::new(root, guard).execute(&arguments).await,
        "grep" => micro_tools::Grep::new(root, guard).execute(&arguments).await,
        "bash" => micro_tools::Bash::new(root, guard).execute(&arguments).await,
        other => Err(format!("unknown builtin tool: {other}")),
    };

    match result {
        Ok(text) => json!({ "result": text }),
        Err(error) => json!({ "error": error }),
    }
}

/// Map a pi-ai `Api` id to the wire protocol micro-provider actually speaks. Not every id
/// pi-ai defines has a Rust client behind it: Mistral's Conversations API and pi's own
/// internal `pi-messages` format have no `micro_models::WireApi` counterpart, so those (and
/// anything else unrecognized) return `None` and the caller reports a specific, named
/// refusal rather than guessing at one.
///
/// Three pi-ai ids collapse onto `OpenaiResponses`: Azure and Codex are the same protocol
/// reached differently, and `client_for` itself tells those apart by `model.provider`
/// (`canonical_provider(provider) == OPENAI_CODEX`/`AZURE_PROVIDER`), not by a distinct
/// wire id — so passing the model's own provider through, unchanged, is what selects the
/// right client on the other side of this.
fn wire_api_from_str(api: &str) -> Option<micro_models::WireApi> {
    use micro_models::WireApi;
    match api {
        "anthropic-messages" => Some(WireApi::AnthropicMessages),
        "openai-completions" => Some(WireApi::OpenaiCompletions),
        "openai-responses" | "azure-openai-responses" | "openai-codex-responses" => {
            Some(WireApi::OpenaiResponses)
        }
        "google-generative-ai" => Some(WireApi::GoogleGenerativeAi),
        "google-vertex" => Some(WireApi::GoogleVertex),
        "bedrock-converse-stream" => Some(WireApi::BedrockConverseStream),
        _ => None,
    }
}

fn headers_from_json(value: Option<&Value>) -> std::collections::BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(name, value)| value.as_str().map(|value| (name.clone(), value.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// A `micro_types::Model` built from what an extension's own provider registration told
/// this about it, not from micro's own catalog: a custom provider (see
/// `pi.registerProvider`'s docs) picks its own id, base URL and token limits, so those are
/// read from the payload rather than looked up.
fn model_from_json(value: &Value) -> micro_types::Model {
    let field_str = |name: &str| value.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
    let thinking = match value.get("thinkingLevel").and_then(Value::as_str) {
        Some("minimal") => micro_types::ThinkingLevel::Minimal,
        Some("low") => micro_types::ThinkingLevel::Low,
        Some("medium") => micro_types::ThinkingLevel::Medium,
        Some("high") => micro_types::ThinkingLevel::High,
        Some("xhigh") => micro_types::ThinkingLevel::XHigh,
        Some("max") => micro_types::ThinkingLevel::Max,
        _ => micro_types::ThinkingLevel::Off,
    };

    micro_types::Model {
        id: field_str("id"),
        provider: field_str("provider"),
        base_url: field_str("baseUrl"),
        max_tokens: value.get("maxTokens").and_then(Value::as_u64).unwrap_or(4096) as u32,
        thinking,
        reasoning: value.get("reasoning").and_then(Value::as_bool).unwrap_or(false),
        compat: micro_types::Compat::default(),
        headers: headers_from_json(value.get("headers")),
    }
}

/// A tool definition as pi-ai's `Tool` shape carries it: a name, a description, and a
/// TypeBox-compiled JSON Schema for its parameters. `micro_types::ToolDefinition.parameters`
/// is already raw JSON, so no translation happens beyond reading the fields out.
fn tool_from_json(value: &Value) -> Option<micro_types::ToolDefinition> {
    Some(micro_types::ToolDefinition {
        name: value.get("name")?.as_str()?.to_string(),
        description: value.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
        parameters: value.get("parameters").cloned().unwrap_or_else(|| json!({})),
        constrained_sampling: micro_types::ConstrainedSampling::from_wire(value.get("constrainedSampling").cloned()),
    })
}

/// pi-ai's `Context` (`systemPrompt`/`messages`/`tools`), read into micro's own shape.
/// `messages` reuses [`message_from_json`] rather than a second parser: it is already the
/// inverse of `message_json`, which is what an extension's own messages are written
/// against (see `crates/micro-extensions/src/events.rs`'s header comment on why ohm's and
/// pi's shapes are one and the same here).
fn context_from_json(value: &Value) -> micro_types::Context {
    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(message_from_json).collect())
        .unwrap_or_default();
    let tools = value
        .get("tools")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(tool_from_json).collect())
        .unwrap_or_default();

    micro_types::Context {
        system_prompt: value.get("systemPrompt").and_then(Value::as_str).map(str::to_string),
        messages,
        tools,
        headers: headers_from_json(value.get("headers")).into_iter().collect(),
        cache_key: value.get("cacheKey").and_then(Value::as_str).map(str::to_string),
    }
}

/// Drain a provider's stream to completion, translating each `StreamEvent` into pi-ai's
/// own `AssistantMessageEvent` shape. Not a second implementation of that translation: it
/// is the same [`Translator`] micro's own agent loop uses to talk to extensions, fed each
/// event as a `MessageDelta` the way a real turn would, so a provider event and a tool
/// event reach an extension through one codepath rather than two that could drift apart.
async fn drain_provider_stream(
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<micro_types::StreamEvent>,
) -> Vec<Value> {
    let mut translator = Translator::new();
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        let is_terminal = matches!(
            event,
            micro_types::StreamEvent::Done { .. } | micro_types::StreamEvent::Error { .. }
        );
        let payload = translator.payload_of(&micro_types::AgentEvent::MessageDelta { event });
        if let Some(assistant_event) = payload.get("assistantMessageEvent") {
            events.push(assistant_event.clone());
        }
        if is_terminal {
            break;
        }
    }
    events
}

/// Run one request against whichever provider client micro-provider has for
/// `payload["api"]`, and hand back the full ordered sequence of pi-ai-shaped
/// `AssistantMessageEvent`s — what `@earendil-works/pi-ai/compat`'s `anthropicMessagesApi`/
/// `openAIResponsesApi`/etc. actually call this for, on behalf of an extension's own
/// `streamSimple`.
///
/// `FromHost::Request`/`answer()` is single-response — there is no channel for trickling
/// interim events back the way `tool_update` does for a running tool call — so this
/// collects the whole stream here rather than as it arrives. An extension's own
/// `for await (const event of stream)` reads the identical sequence either way; only the
/// wall-clock delivery changes, not the contract.
///
/// The extension supplies its own `apiKey` in the payload — resolved however its own
/// `ProviderAuth`/OAuth flow decided, the same as `custom-provider-gitlab-duo/index.ts`
/// fetching its own GitLab token. Nothing here reads micro's own `AuthStore`; a credential
/// this host itself holds never crosses this path.
async fn provider_stream(payload: &Value) -> Value {
    let Some(api) = payload.get("api").and_then(Value::as_str) else {
        return json!({ "error": "provider_stream needs an api id" });
    };
    let Some(wire_api) = wire_api_from_str(api) else {
        return json!({
            "error": format!(
                "micro has no provider client for pi-ai's \"{api}\" API — micro-provider doesn't speak this wire protocol (see crates/micro-models::WireApi)"
            ),
        });
    };
    let Some(model_value) = payload.get("model") else {
        return json!({ "error": "provider_stream needs a model" });
    };
    let model = model_from_json(model_value);
    let context = context_from_json(payload.get("context").unwrap_or(&Value::Null));
    let Some(api_key) = payload.get("apiKey").and_then(Value::as_str) else {
        return json!({
            "error": "provider_stream needs an apiKey — the extension's own credential, not one of micro's",
        });
    };

    let client = micro_provider::client_for(wire_api, &model.provider);
    let receiver = client.stream(model, context, api_key.to_string());
    let events = drain_provider_stream(receiver).await;
    json!({ "events": events })
}

/// Answer pi-ai's `getBuiltinModel`/`getBuiltinModels`/`getBuiltinProviders` from micro's
/// own bundled catalog (`crates/micro-models`) rather than a second copy of pi's generated
/// model data — which, see the report to the team, is not even present in a checked-out pi
/// source tree to vendor. `payload["provider"]`, when given, narrows `models` to one
/// provider; `providers` always lists every provider the catalog carries, so a caller can
/// tell "no provider by that name" apart from "that provider has no models".
///
/// The shape itself — `micro_models::catalog_json` — is shared with
/// `crates/micro-extensions/src/compat.rs`, which writes the same catalog to a static file
/// a pi extension's own synchronous `getBuiltinModel` reads directly. One function is what
/// keeps this live answer and that static one from quietly disagreeing.
fn model_catalog(payload: &Value) -> Value {
    let catalog = micro_models::Catalog::bundled();
    let provider_filter = payload.get("provider").and_then(Value::as_str);
    micro_models::catalog_json(&catalog, provider_filter)
}

/// Something an extension asked to have done.
///
/// Anything that reaches the conversation goes through the interface, because the
/// conversation is the interface's: it holds the agent and decides when a turn runs.
async fn carry_out(
    action: &str,
    payload: &Value,
    extension: Option<&str>,
    broker: &Broker,
    asker: Option<&micro_tui::UiAsker>,
    host: Option<&Arc<Host>>,
    state: Option<&Arc<tokio::sync::RwLock<State>>>,
    session: Option<&Arc<Mutex<micro_session::Session>>>,
) {
    // Nothing goes back to an action, so a refusal is said where a person will find it and
    // written where an audit will: the extension is not waiting to be told.
    if let Some(needs) = action_needs(action) {
        if !broker.allows(extension, needs, action) {
            eprintln!("note: {}", broker.refusal(extension, needs));
            return;
        }
    }

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
            let lines = drawn.unwrap_or_else(|| content.lines().map(str::to_string).collect());
            if lines.is_empty() {
                return;
            }
            if let Some(asker) = asker {
                asker.ask("custom_message", custom_type, None, lines).await;
            }
        }
        // Three that write the session and answer nothing, the way pi's own do. What went
        // wrong is reported where a reader can see it rather than handed back to a caller
        // that, by the shape of the call, is not waiting for it.
        "append_entry" => {
            let Some(session) = session else { return };
            let custom_type = payload
                .get("customType")
                .and_then(Value::as_str)
                .unwrap_or("custom");
            let data = payload.get("data").cloned().unwrap_or(Value::Null);
            if let Err(error) = session.lock().await.append_custom(custom_type, data).await {
                eprintln!("note: an extension could not keep an entry: {error}");
            }
        }
        "set_label" => {
            let Some(session) = session else { return };
            let Some(entry_id) = payload.get("entryId").and_then(Value::as_str) else {
                return;
            };
            let label = payload
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Err(error) = session.lock().await.set_label(entry_id, label).await {
                eprintln!("note: an extension could not label an entry: {error}");
            }
        }
        "set_session_name" => {
            let Some(session) = session else { return };
            let Some(name) = payload.get("name").and_then(Value::as_str) else {
                return;
            };
            if let Err(error) = session.lock().await.rename(name).await {
                eprintln!("note: an extension could not name the session: {error}");
            }
        }
        // Which tools the model is told about from the next turn on. Unlike the two below
        // it needs no command to carry it: the agent reads the offered list each time it
        // describes its tools, so writing it here is all it takes.
        "set_active_tools" => {
            let named: Vec<String> = payload
                .get("toolNames")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if let Some(state) = state {
                let mut state = state.write().await;
                state.tools = named.clone();
                *state
                    .offered_tools
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(named);
            }
        }
        // Both of these change what the next turn runs, which is the interface's to do:
        // it holds the agent, and a command is how anything else asks it to.
        "set_thinking_level" => {
            if let (Some(asker), Some(level)) =
                (asker, payload.get("level").and_then(Value::as_str))
            {
                asker
                    .ask(
                        "send_user_message",
                        format!("/thinking {level}"),
                        None,
                        Vec::new(),
                    )
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
                    .ask(
                        "send_user_message",
                        format!("/model {model}"),
                        None,
                        Vec::new(),
                    )
                    .await;
            }
        }
        // Neither of these waits to be told it worked, which is what lets an extension
        // call them without an `await`: the interface picks the request up on its own
        // time, the same as a reader's own `/quit` or `/compact` would be.
        "shutdown" => {
            if let Some(asker) = asker {
                asker
                    .ask("send_user_message", "/quit", None, Vec::new())
                    .await;
            }
        }
        // micro's `/compact` takes no argument, so a custom instruction an extension
        // passed is not carried across — there is nowhere on this path to put it.
        "compact" => {
            if let Some(asker) = asker {
                asker
                    .ask("send_user_message", "/compact", None, Vec::new())
                    .await;
            }
        }
        // Not typed as a slash command, unlike the two above: interrupting is a
        // keypress, not a line, so the interface answers this one specially rather than
        // routing it through the editor the way `/quit` and `/compact` are.
        "abort" => {
            if let Some(asker) = asker {
                asker.ask("abort", String::new(), None, Vec::new()).await;
            }
        }
        // The first `ctx.ui.onTerminalInput` registration and the last one going away —
        // told rather than polled, so the interface offers a key to the host only for as
        // long as anything there wants to be asked about it.
        "watch_terminal_input" => {
            if let Some(asker) = asker {
                asker
                    .ask("watch_terminal_input", "", None, Vec::new())
                    .await;
            }
        }
        "unwatch_terminal_input" => {
            if let Some(asker) = asker {
                asker
                    .ask("unwatch_terminal_input", "", None, Vec::new())
                    .await;
            }
        }
        // Every trigger character an `addAutocompleteProvider` registration has declared,
        // carried in `options` the same way `select`'s choices are — the interface is told
        // the whole set each time rather than asked to reconcile a difference against what
        // it already knew.
        "watch_autocomplete" => {
            if let Some(asker) = asker {
                let triggers = payload
                    .get("triggers")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                asker.ask("watch_autocomplete", "", None, triggers).await;
            }
        }
        other => eprintln!("note: an extension asked for `{other}`, which micro does not know"),
    }
}

/// A registered component's lines at a guessed width — the pump is not the interface and
/// does not know the real one, the same compromise [`RENDER_WIDTH`] already makes for a
/// custom message's own renderer. Empty with no interface to draw it in anyway, or when the
/// component itself did not answer in time.
async fn component_lines(host: Option<&Arc<Host>>, component_id: &str) -> Vec<String> {
    match host {
        Some(host) => host
            .render_component(component_id, RENDER_WIDTH)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Tell the interface a component's id now backs this slot — `"header"`, `"footer"`, or
/// `"widget:<key>"` — so a later `component_changed` push, which names only the id, knows
/// where the lines it fetches belong.
async fn component_slot(asker: &micro_tui::UiAsker, component_id: &str, slot: &str) {
    asker
        .ask(
            "register_component_slot",
            component_id.to_string(),
            Some(slot.to_string()),
            Vec::new(),
        )
        .await;
}

/// Show the user what an extension wants shown, and say what came back.
///
/// With no interface to ask through — a headless run — a question is cancelled rather than
/// answered with something nobody chose.
async fn show(
    payload: &Value,
    extension: Option<&str>,
    broker: &Broker,
    asker: Option<&micro_tui::UiAsker>,
    host: Option<&Arc<Host>>,
) -> Value {
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // Refused the same way a question nobody is there to answer is: an extension drawing
    // without the capability is told the same thing as one drawing with nobody watching,
    // which is a case every one of them already handles.
    if !broker.allows(extension, Capability::Ui, method) {
        return json!({
            "cancelled": true,
            "error": broker.refusal(extension, Capability::Ui),
        });
    }
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
                .ask(
                    "notify",
                    text("message").unwrap_or_default(),
                    None,
                    Vec::new(),
                )
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
        // A multi-line editor rather than one line: the detail carries the prefill the same
        // way `input`'s carries a placeholder, and the answer is the same `{ value }` shape.
        "editor" => {
            asker
                .ask("editor", text("title").unwrap_or_default(), text("prefill"), Vec::new())
                .await
        }
        // Not a question: a line an extension keeps in the footer. Clearing it is saying
        // it with no text.
        "setStatus" => {
            asker
                .ask_from(
                    extension.map(str::to_string),
                    "set_status",
                    text("statusKey").unwrap_or_default(),
                    text("statusText"),
                    Vec::new(),
                )
                .await
        }
        // Everything from here down is a request rather than a question: told once,
        // answered at once, with nobody waiting on a reader to choose anything.
        "setTitle" => {
            asker
                .ask("set_title", text("title").unwrap_or_default(), None, Vec::new())
                .await
        }
        "setWorkingMessage" => {
            asker
                .ask("set_working_message", "", text("message"), Vec::new())
                .await
        }
        "setWorkingVisible" => {
            let visible = payload.get("visible").and_then(Value::as_bool).unwrap_or(true);
            asker
                .ask("set_working_visible", visible.to_string(), None, Vec::new())
                .await
        }
        "setWorkingIndicator" => {
            let title = match payload.get("reset").and_then(Value::as_bool) {
                Some(true) => "reset",
                _ => "set",
            };
            let frames = payload
                .get("frames")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let interval = payload
                .get("intervalMs")
                .and_then(Value::as_u64)
                .map(|value| value.to_string());
            asker.ask("set_working_indicator", title, interval, frames).await
        }
        "setHiddenThinkingLabel" => {
            asker
                .ask("set_hidden_thinking_label", "", text("label"), Vec::new())
                .await
        }
        // A widget backed by a live component carries an id rather than lines; its first
        // content is fetched here, the same as a header's or a footer's, before the
        // interface is told the widget's key now belongs to that component — see
        // `component_slot` and `component_lines`.
        "setWidget" => {
            let key = text("key").unwrap_or_default();
            let owner = extension.map(str::to_string);
            match text("componentId") {
                Some(component_id) => {
                    component_slot(asker, &component_id, &format!("widget:{key}")).await;
                    let lines = component_lines(host, &component_id).await;
                    asker
                        .ask_from(owner, "set_widget", key, text("placement"), lines)
                        .await
                }
                None => {
                    let lines = payload
                        .get("lines")
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
                        .ask_from(owner, "set_widget", key, text("placement"), lines)
                        .await
                }
            }
        }
        // `setHeader`/`setFooter` only ever carry a component — pi declares no plain-lines
        // form for either — except when clearing one, which carries neither a component nor
        // any lines at all.
        "setHeader" => match text("componentId") {
            Some(component_id) => {
                component_slot(asker, &component_id, "header").await;
                let lines = component_lines(host, &component_id).await;
                asker
                    .ask_from(extension.map(str::to_string), "set_header", "", None, lines)
                    .await
            }
            None => asker.ask("set_header", "", None, Vec::new()).await,
        },
        "setFooter" => match text("componentId") {
            Some(component_id) => {
                component_slot(asker, &component_id, "footer").await;
                let lines = component_lines(host, &component_id).await;
                asker
                    .ask_from(extension.map(str::to_string), "set_footer", "", None, lines)
                    .await
            }
            None => asker.ask("set_footer", "", None, Vec::new()).await,
        },
        // A tool's renderCall/renderResult already drew and sent its own first frame —
        // nothing here fetches it again, unlike `setHeader`/`setFooter`/`setWidget`'s
        // component form, which only ever carry an id. The title names the call it
        // belongs to; the detail is the component id, so a later `component_changed` push
        // — naming only the id — can still be told apart from every other tool row's.
        "tool_call_rendered" | "tool_result_rendered" => {
            let lines = payload
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
                .ask(method, text("title").unwrap_or_default(), text("detail"), lines)
                .await
        }
        "setEditorText" => {
            asker
                .ask("set_editor_text", "", text("text"), Vec::new())
                .await
        }
        "pasteToEditor" => {
            asker.ask("paste_to_editor", "", text("text"), Vec::new()).await
        }
        // The name goes in the title; a snapshot of colors an extension resolved itself —
        // most likely one `getTheme` handed back — rides in the detail as the JSON object
        // it already is, carried whole rather than taken apart into a string apiece.
        "setTheme" => {
            let colors = payload.get("colors").map(Value::to_string);
            asker
                .ask("set_theme", text("name").unwrap_or_default(), colors, Vec::new())
                .await
        }
        "setToolsExpanded" => {
            let expanded = payload.get("expanded").and_then(Value::as_bool).unwrap_or(false);
            asker
                .ask("set_tools_expanded", expanded.to_string(), None, Vec::new())
                .await
        }
        // A question rather than a push: the component's id and its first lines open the
        // overlay, and this does not answer until the overlay closes — either the reader
        // backing out, or the component finishing on its own through `customDone` below.
        "custom" => {
            let component_id = text("componentId").unwrap_or_default();
            let lines = component_lines(host, &component_id).await;
            asker.ask("custom", component_id, None, lines).await
        }
        // The component decided it was finished. The result rides in the detail as the
        // JSON it already is, the same way a theme snapshot does for `setTheme`.
        "customDone" => {
            let result = payload.get("result").cloned().unwrap_or(Value::Null);
            asker
                .ask("custom_done", "", Some(result.to_string()), Vec::new())
                .await
        }
        // The title carries the component's id, fetched and answered the same way a
        // header's or a footer's is; empty restores the built-in editor.
        "setEditorComponent" => match text("componentId") {
            Some(component_id) => {
                let lines = component_lines(host, &component_id).await;
                asker
                    .ask_from(
                        extension.map(str::to_string),
                        "set_editor_component",
                        component_id,
                        None,
                        lines,
                    )
                    .await
            }
            None => asker.ask("set_editor_component", "", None, Vec::new()).await,
        },
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
    pub model_name: String,
    pub provider: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub reasoning: bool,
    pub tools: Vec<String>,
    /// Which tools the model is told about, shared with the agent so `setActiveTools`
    /// reaches the next turn. `None` inside means nothing has been narrowed.
    pub offered_tools: Arc<std::sync::RwLock<Option<Vec<String>>>>,
    pub commands: Vec<String>,
    /// Every tool that exists, described the way `getAllTools()` answers — name,
    /// description, parameters, guidelines and where it came from. Settled once: what a
    /// tool is does not change after loading, only whether it is currently offered.
    pub all_tools: Value,
    /// Every command that can be typed, described the way `getCommands()` answers.
    pub all_commands: Value,
    /// What the model was told before the conversation started, so an extension asking
    /// can be answered without waiting on the agent. This is the base prompt the session
    /// was built with — a `context` handler that rewrites it for a single turn is not
    /// reflected back here, since nothing currently writes that rewrite back to this
    /// field.
    pub system_prompt: String,
    /// The raw `--models`/scoped-models patterns this run was configured with, unresolved:
    /// resolving them against the catalog is deferred to whoever asks, since most runs
    /// never do.
    pub scoped_models: Vec<String>,
    /// What went into the system prompt, kept apart from the assembled `system_prompt`
    /// above — pi's `getSystemPromptOptions()`, which wants the ingredients, not only the
    /// dish they were combined into. Static for the run, the same as `system_prompt`
    /// itself: neither is updated by anything short of a restart.
    pub custom_prompt: Option<String>,
    pub appended_prompt: Option<String>,
    pub context_files: Vec<(PathBuf, String)>,
    pub skills: Vec<micro_skills::Skill>,
    /// A tool's snippet, by name, for the tools that actually made it into the prompt's
    /// tools section — see `tool_prompt_options`.
    pub tool_snippets: Value,
    pub prompt_guidelines: Vec<String>,
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
    /// Who may change what, and where the fact that they did is written down. An answer
    /// that rewrites the prompt or the conversation is the broadest thing an extension can
    /// do without asking for anything, so it is held to the same manifest as an `exec`.
    broker: Broker,
    /// What the model is told before the conversation, as the agent holds it.
    ///
    /// A handle rather than a copy: an extension replacing the prompt is asking the agent
    /// for a change, not keeping one of its own. Holding a second copy here is what made
    /// `/reload` a no-op with any extension loaded — the copy was written over the reloaded
    /// prompt on the way to every request — and it left the change unrecorded besides.
    prefix: micro_agent::PrefixControl,
    /// Where this run operates, for `before_agent_start`'s `systemPromptOptions` — the
    /// one field ohm's own default carries when nothing richer is on hand.
    cwd: String,
    /// The arguments a tool call was started with, kept by call id until it answers.
    /// `Hooks::after_tool` is not handed them itself, and `tool_result` is written against
    /// ohm's shape, which carries them as `input`.
    call_arguments: Mutex<HashMap<String, Value>>,
}

impl ExtensionHooks {
    pub fn new(
        host: Arc<Host>,
        broker: Broker,
        prefix: micro_agent::PrefixControl,
        cwd: String,
    ) -> Self {
        ExtensionHooks {
            host,
            broker,
            prefix,
            cwd,
            call_arguments: Mutex::new(HashMap::new()),
        }
    }
}

// `after_provider_response` is not fired from here. Ohm's version carries the raw HTTP
// status and headers, read after the response arrives but before its body is consumed —
// data that belongs to the HTTP call itself. `Hooks::after_response` runs well past that,
// with the stream already fully read into an `AssistantMessage` and no status or headers
// left to hand over. Reporting the wrong data under the right name would be worse than
// this event simply never firing, so it does not.
#[async_trait::async_trait]
impl Hooks for ExtensionHooks {
    async fn before_tool(&self, id: &str, name: &str, arguments: &Value) -> ToolDecision {
        self.call_arguments
            .lock()
            .await
            .insert(id.to_string(), arguments.clone());

        let Ok(answers) = self
            .host
            .ask_event_attributed(
                "tool_call",
                json!({ "toolCallId": id, "toolName": name, "input": arguments }),
            )
            .await
        else {
            return ToolDecision::Proceed;
        };
        // An answer is how an extension changes what a call does; listening without the
        // capability leaves it watching, which is what an extension that never declared it
        // asked for.
        let answers = self.broker.heeded(answers, Capability::Events, "tool_call");

        // A refusal wins over a rewrite wherever both are said: the extension that wants
        // the call not to happen is not answered by another one changing it.
        let refusal =
            answers.iter().find_map(
                |answer| match answer.get("block").and_then(Value::as_bool) {
                    Some(true) => Some(
                        answer
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("an extension blocked this call")
                            .to_string(),
                    ),
                    _ => None,
                },
            );
        if let Some(reason) = refusal {
            return ToolDecision::Refuse(reason);
        }

        // Every extension was asked with the same arguments, so two rewrites cannot be
        // applied one after the other and still mean what either of them meant. The first
        // one stands, the way the first refusal does.
        answers
            .iter()
            .find_map(|answer| answer.get("input").cloned())
            .map_or(ToolDecision::Proceed, ToolDecision::Rewrite)
    }

    async fn after_tool(
        &self,
        id: &str,
        name: &str,
        output: String,
        is_error: bool,
    ) -> (String, bool) {
        // Removed rather than merely read: the call this result belongs to is over either
        // way, and a stale entry from an id micro reused would be answered to the wrong
        // call.
        let input = self
            .call_arguments
            .lock()
            .await
            .remove(id)
            .unwrap_or_else(|| json!({}));

        let asked = self
            .host
            .ask_event_attributed(
                "tool_result",
                json!({
                    "toolCallId": id,
                    "toolName": name,
                    "input": input,
                    "content": [{ "type": "text", "text": output }],
                    "isError": is_error,
                }),
            )
            .await;

        let Ok(answers) = asked else {
            return (output, is_error);
        };
        let answers = self.broker.heeded(answers, Capability::Events, "tool_result");

        // Each answer is applied in turn, so a later extension sees what an earlier one
        // wrote rather than what the tool originally said. `Hooks::after_tool` can only
        // hand the model text, so an image block in a rewritten `content` has nowhere to
        // go here and is left out rather than silently dropped as if it were never there —
        // `details` and `usage` are absent from this hook the same way and for the same
        // reason.
        let mut output = output;
        let mut is_error = is_error;
        for answer in answers {
            if let Some(content) = answer.get("content").and_then(Value::as_array) {
                output = content
                    .iter()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("");
            }
            if let Some(failed) = answer.get("isError").and_then(Value::as_bool) {
                is_error = failed;
            }
        }
        (output, is_error)
    }

    async fn before_agent_start(
        &self,
        prompt: &micro_types::Message,
    ) -> Option<micro_types::Message> {
        // The prompt a run starts on is always a user message; text and images are read
        // out of it the way ohm reads them off its own `UserMessage`.
        let (text, images) = match prompt {
            micro_types::Message::User { content, .. } => (
                content.iter().map(micro_types::ContentBlock::as_text).collect::<String>(),
                content
                    .iter()
                    .filter(|block| matches!(block, micro_types::ContentBlock::Image { .. }))
                    .map(micro_extensions::content_json)
                    .collect::<Vec<_>>(),
            ),
            _ => (String::new(), Vec::new()),
        };

        let mut payload = json!({
            "prompt": text,
            "systemPrompt": self.prefix.system_prompt(),
            "systemPromptOptions": { "cwd": self.cwd },
        });
        if !images.is_empty() {
            payload["images"] = json!(images);
        }

        let Ok(answers) = self
            .host
            .ask_event_attributed("before_agent_start", payload)
            .await
        else {
            return None;
        };
        // Replacing the system prompt is changing what every turn of this run is told, and
        // is charged for — the broadest thing on this side of the broker, and the one an
        // extension has to have said it would do.
        let answers = self
            .broker
            .heeded_from(answers, Capability::Context, "system_prompt");

        // Ohm's own result appends `message` alongside the prompt rather than replacing
        // it; `Hooks::before_agent_start` can only replace the prompt outright, so that
        // half of the result has nowhere to go and is not honoured. `systemPrompt` is:
        // each answer is applied in turn, so the last extension to set it has the final
        // say — and this runs before the first turn of the run, so the request that turn
        // sends already reflects it.
        for (source, answer) in &answers {
            if let Some(replacement) = answer.get("systemPrompt").and_then(Value::as_str) {
                self.broker.record(
                    source.as_deref(),
                    Capability::Context.as_str(),
                    "system_prompt",
                    true,
                    None,
                );
                // A replaced prompt is no longer the one the run assembled, so the spans
                // describing that one would be describing something else. What is true of
                // this prompt is that an extension wrote all of it.
                let span = micro_types::PrefixSpan {
                    source: micro_types::EventSource::Extension(
                        self.broker.grants.name_of(source.as_deref()),
                    ),
                    bytes: replacement.len() as u64,
                    hash: micro_types::content_hash(replacement.as_bytes()),
                };
                self.prefix.change(replacement, vec![span], "extension");
            }
        }
        None
    }

    async fn before_request(&self, context: micro_types::Context) -> micro_types::Context {
        // The system prompt is not touched here. It was settled at the turn boundary, hash
        // and all, and a request that rewrote it on the way out would be a request the
        // session cannot account for. What an extension may still change is the
        // conversation, which is nobody's cached prefix.
        let mut context = context;

        let asked = self
            .host
            .ask_event_attributed(
                "context",
                json!({
                    "messages": context.messages.iter().map(micro_extensions::message_json).collect::<Vec<_>>(),
                }),
            )
            .await;

        if let Ok(answers) = asked {
            // Each answer is applied in turn, so a later extension sees what an earlier
            // one changed rather than what the agent originally assembled. An answer
            // whose messages do not parse as ohm's shape changes nothing, rather than
            // clearing the conversation.
            for (source, answer) in self
                .broker
                .heeded_from(answers, Capability::Context, "messages")
            {
                let Some(messages) = answer.get("messages").and_then(Value::as_array) else {
                    continue;
                };
                let replaced: Vec<micro_types::Message> =
                    messages.iter().filter_map(micro_extensions::message_from_json).collect();
                if replaced.len() == messages.len() {
                    self.broker.record(
                        source.as_deref(),
                        Capability::Context.as_str(),
                        "messages",
                        true,
                        Some(json!({ "messages": replaced.len() })),
                    );
                    context.messages = replaced;
                }
            }
        }

        // Announced separately, because ohm reports the request itself as its own moment.
        // What is handed over is the same context an extension can already read and
        // rewrite through `context` above, not the literal per-provider request body:
        // that is assembled deep inside the HTTP client, well past where an extension can
        // be asked about it, so nothing answered here can replace it — unlike ohm, whose
        // own version of this event runs in the same process as the request it shapes.
        let _ = self
            .host
            .notify(
                "before_provider_request",
                json!({ "payload": {
                    "systemPrompt": context.system_prompt,
                    "messages": context.messages.iter().map(micro_extensions::message_json).collect::<Vec<_>>(),
                } }),
            )
            .await;

        // Headers are their own moment, and their own answer: what comes back is put on
        // the request, replacing anything the provider would have set itself.
        if let Ok(answers) = self
            .host
            .ask_event_attributed("before_provider_headers", json!({ "headers": {} }))
            .await
        {
            for (source, answer) in self
                .broker
                .heeded_from(answers, Capability::Context, "headers")
            {
                let Some(headers) = answer.get("headers").and_then(Value::as_object) else {
                    continue;
                };
                for (name, value) in headers {
                    let Some(value) = value.as_str() else {
                        continue;
                    };
                    self.broker.record(
                        source.as_deref(),
                        Capability::Context.as_str(),
                        "headers",
                        true,
                        Some(json!({ "header": name })),
                    );
                    context.headers.retain(|(held, _)| held != name);
                    context.headers.push((name.clone(), value.to_string()));
                }
            }
        }
        context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy these tests hand over. `full` on purpose: what a policy does to a command
    /// is proved where it is enforced — in `micro-tools`, and in the integration tests that
    /// run the built binary — while what is being checked here is that a request reaches
    /// the right implementation at all. Confining these would also mean re-running micro's
    /// own executable as the Linux helper, and the executable running a unit test is the
    /// test harness, which knows nothing about being a helper.
    fn unconfined() -> micro_sandbox::Sandbox {
        micro_sandbox::Sandbox::new(micro_sandbox::SandboxPolicy::Full, std::env::temp_dir())
    }

    #[tokio::test]
    async fn exec_runs_a_command_and_reports_what_it_printed() {
        let answer = exec(
            &json!({ "command": "echo", "args": ["hello"] }),
            &std::env::temp_dir(),
            &unconfined(),
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
            &unconfined(),
        )
        .await;

        assert_eq!(answer["stdout"], "hello; echo goodbye\n");
    }

    /// Which shape the failure arrives in depends on whether the session is confined: a
    /// command micro spawns itself cannot start at all, while under the sandbox it is the
    /// wrapper that starts and then reports what it could not run. Either way the
    /// extension is told the command failed, and which one it was.
    #[tokio::test]
    async fn a_command_that_is_not_there_is_reported() {
        let answer = exec(
            &json!({ "command": "nothing-like-this-exists", "args": [] }),
            &std::env::temp_dir(),
            &unconfined(),
        )
        .await;

        let said = format!("{}{}", answer["error"], answer["stderr"]);
        assert!(said.contains("nothing-like-this-exists"), "{answer}");
        assert_ne!(answer["code"], json!(0), "{answer}");
    }

    /// A scratch workspace of its own, so a builtin tool's writes land somewhere real and
    /// disposable rather than in whatever `std::env::temp_dir()` happens to hold already.
    fn scratch_workspace() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "micro-extensions-builtin-tool-{}-{}",
            std::process::id(),
            micro_types::now_ms()
        ));
        std::fs::create_dir_all(&root).expect("a scratch workspace");
        root
    }

    /// `createWriteTool`/`createReadTool` on the extension side proxy through exactly this
    /// request — real disk I/O through `crates/micro-tools`, not a second implementation on
    /// the TypeScript side of the wire.
    #[tokio::test]
    async fn run_builtin_tool_writes_and_reads_a_real_file() {
        let workspace = scratch_workspace();

        let write = run_builtin_tool(
            &json!({ "tool": "write", "arguments": { "path": "note.txt", "content": "hello there" } }),
            &workspace,
            &unconfined(),
        )
        .await;
        assert!(write["result"].as_str().unwrap().contains("note.txt"));
        assert_eq!(
            std::fs::read_to_string(workspace.join("note.txt")).unwrap(),
            "hello there"
        );

        let read = run_builtin_tool(
            &json!({ "tool": "read", "arguments": { "path": "note.txt" } }),
            &workspace,
            &unconfined(),
        )
        .await;
        assert!(read["result"].as_str().unwrap().contains("hello there"));
    }

    /// `edit`'s exact-string match is the same fuzzy-matched, ambiguity-refusing logic the
    /// model's own edit tool runs on — reached here rather than reimplemented.
    #[tokio::test]
    async fn run_builtin_tool_edits_a_real_file() {
        let workspace = scratch_workspace();
        std::fs::write(workspace.join("note.txt"), "hello there").unwrap();

        let edit = run_builtin_tool(
            &json!({
                "tool": "edit",
                "arguments": { "path": "note.txt", "old_string": "hello", "new_string": "goodbye" },
            }),
            &workspace,
            &unconfined(),
        )
        .await;
        assert!(edit["result"].as_str().unwrap().contains("Edited"));
        assert_eq!(
            std::fs::read_to_string(workspace.join("note.txt")).unwrap(),
            "goodbye there"
        );
    }

    /// Bash and ls reach `crates/micro-tools`'s real implementations too, not only the file
    /// tools — the same one request answers all seven of pi's default and read-only builtins.
    #[tokio::test]
    async fn run_builtin_tool_runs_a_real_shell_command() {
        let workspace = scratch_workspace();
        let bash = run_builtin_tool(
            &json!({ "tool": "bash", "arguments": { "command": "echo hi" } }),
            &workspace,
            &unconfined(),
        )
        .await;
        assert!(bash["result"].as_str().unwrap().contains("hi"));
    }

    /// A tool name outside the seven micro actually has is refused rather than silently
    /// doing nothing, the same as `answer()`'s own catch-all for a request it does not know.
    #[tokio::test]
    async fn run_builtin_tool_refuses_a_tool_it_does_not_have() {
        let answer = run_builtin_tool(
            &json!({ "tool": "teleport", "arguments": {} }),
            &scratch_workspace(),
            &unconfined(),
        )
        .await;
        assert!(answer["error"].as_str().unwrap().contains("teleport"));
    }

    /// Reached the same way every other request is: through `answer()`'s own dispatch, not
    /// only by calling the function directly.
    #[tokio::test]
    async fn an_extension_reaches_a_builtin_tool_through_answer() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let workspace = scratch_workspace();
        let response = answer(
            "run_builtin_tool",
            &json!({ "tool": "write", "arguments": { "path": "from-answer.txt", "content": "written" } }),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &scratch_session().await,
            None,
        )
        .await;
        assert!(response["result"].as_str().unwrap().contains("from-answer.txt"));
        assert_eq!(
            std::fs::read_to_string(workspace.join("from-answer.txt")).unwrap(),
            "written"
        );
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
            None,
            &std::env::temp_dir(),
            &unconfined(),
            &Broker::open(),
            &state,
            &scratch_session().await,
            None,
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
            model_name: "Gemini 3 Pro".into(),
            provider: "openrouter".into(),
            context_window: 1_000_000,
            max_output_tokens: 65_536,
            reasoning: true,
            offered_tools: Default::default(),
            tools: vec!["read".into(), "write".into()],
            commands: vec!["help".into()],
            all_tools: json!([]),
            all_commands: json!([]),
            system_prompt: "you are micro".into(),
            scoped_models: Vec::new(),
            custom_prompt: None,
            appended_prompt: None,
            context_files: Vec::new(),
            skills: Vec::new(),
            tool_snippets: json!({}),
            prompt_guidelines: Vec::new(),
        }));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        let level = answer(
            "get_thinking_level",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert_eq!(level["level"], "high");

        let tools = answer(
            "get_active_tools",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert_eq!(tools["tools"][0], "read");

        let model = answer("get_model", &json!({}), None, &workspace, &unconfined(), &Broker::open(), &state, &session, None).await;
        assert_eq!(model["model"]["id"], "gemini-3-pro");
        assert_eq!(model["model"]["name"], "Gemini 3 Pro");
        assert_eq!(model["model"]["provider"], "openrouter");
        assert_eq!(model["model"]["contextWindow"], 1_000_000);
        assert_eq!(model["model"]["maxOutputTokens"], 65_536);
        assert_eq!(model["model"]["reasoning"], true);

        let commands = answer(
            "get_commands",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert_eq!(commands["commands"][0], "help");

        let prompt = answer(
            "get_system_prompt",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert_eq!(prompt["systemPrompt"], "you are micro");
    }

    /// `get_context` answers the same three things as `get_model`, `get_thinking_level`
    /// and `get_system_prompt` together, since the extension context asks for all of
    /// them at once on every call rather than one request apiece.
    #[tokio::test]
    async fn get_context_answers_model_thinking_and_prompt_together() {
        let state = Arc::new(tokio::sync::RwLock::new(State {
            thinking: "medium".into(),
            model: "claude-opus-5".into(),
            model_name: "Claude Opus 5".into(),
            provider: "anthropic".into(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            reasoning: true,
            system_prompt: "you are micro".into(),
            ..State::default()
        }));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        let context = answer(
            "get_context",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert_eq!(context["model"]["id"], "claude-opus-5");
        assert_eq!(context["model"]["contextWindow"], 200_000);
        assert_eq!(context["thinkingLevel"], "medium");
        assert_eq!(context["systemPrompt"], "you are micro");
        // Unset in this test, so scoping matches nothing rather than everything.
        assert_eq!(context["scopedModels"], serde_json::json!([]));
    }

    /// A pattern matches by provider-qualified id or by bare id, the same prefix match
    /// `/model`'s own shortlist uses — and an unset scope answers empty rather than
    /// asking the whole catalog to stand in for "unscoped".
    #[tokio::test]
    async fn scoped_models_resolve_against_the_catalog() {
        let unscoped = resolve_scoped_models(&[]);
        assert_eq!(unscoped, serde_json::json!([]));

        // Several providers resell this model under their own id — some bare, some
        // (openrouter's, the gateway's) already carrying an "anthropic/" prefix of their
        // own — so this only checks that the provider actually running it is among what
        // was matched, not that it is the only one.
        let scoped = resolve_scoped_models(&["anthropic/claude-opus-5".to_string()]);
        let matches = scoped.as_array().expect("a list of scoped models");
        assert!(
            matches
                .iter()
                .any(|entry| entry["model"]["provider"] == "anthropic"
                    && entry["model"]["id"] == "claude-opus-5"),
            "{matches:?}"
        );
    }

    /// `get_context`'s `session` carries what `sessionManager`'s fourteen methods need:
    /// where the log lives, its entries with their parent chain intact, and what has been
    /// named.
    #[tokio::test]
    async fn get_context_carries_enough_of_the_session_to_answer_sessionmanager() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        {
            let mut session = session.lock().await;
            session
                .append(&micro_types::Message::user("hello"))
                .await
                .unwrap();
            // An assistant message with a tool call, and the tool result answering it —
            // a bare user message has no field whose name changes between micro's own
            // `Message` and pi's `AgentMessage`, so it cannot catch a mismatch between
            // them the way these two can.
            session
                .append(&micro_types::Message::Assistant(micro_types::AssistantMessage {
                    content: vec![micro_types::ContentBlock::ToolCall {
                        id: "call_1".into(),
                        name: "read".into(),
                        arguments: json!({ "path": "a.txt" }),
                        signature: None,
                    }],
                    provider: "test".into(),
                    model: "test-model".into(),
                    usage: micro_types::Usage::default(),
                    stop_reason: micro_types::StopReason::ToolUse,
                    error: None,
                    timestamp: 0,
                }))
                .await
                .unwrap();
            session
                .append(&micro_types::Message::tool_result(
                    "call_1",
                    "read",
                    "file contents",
                    false,
                ))
                .await
                .unwrap();
            session.append_custom("note", json!({ "kept": true })).await.unwrap();
            session.rename("a test session").await.unwrap();
        }
        let workspace = std::env::temp_dir();

        let context = answer(
            "get_context",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        let carried = &context["session"];

        assert_eq!(carried["id"], session.lock().await.id());
        assert_eq!(carried["name"], "a test session");
        // A custom entry hangs off wherever the conversation currently is without
        // becoming the head itself, so the leaf is still the last message.
        assert_eq!(carried["leafId"], "3");
        assert!(carried["file"].as_str().unwrap().ends_with(".jsonl"));

        let entries = carried["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 4, "{entries:?}");
        let message = |id: &str| {
            entries
                .iter()
                .find(|entry| entry["type"] == "message" && entry["id"] == id)
                .unwrap_or_else(|| panic!("no message entry {id} in {entries:?}"))
        };

        assert_eq!(message("1")["parentId"], serde_json::Value::Null);
        assert_eq!(message("1")["message"]["role"], "user");

        // Exactly the fields `micro_types::Message`/`AssistantMessage` hold under their
        // own snake_case names, but camelCased and, for a tool result, under pi's own
        // name for the role — this is what `message_json` is answering for.
        let assistant = &message("2")["message"];
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["stopReason"], "toolUse");
        assert_eq!(assistant["content"][0]["type"], "toolCall");
        assert_eq!(assistant["content"][0]["id"], "call_1");

        let tool_result = &message("3")["message"];
        assert_eq!(tool_result["role"], "toolResult");
        assert_eq!(tool_result["toolCallId"], "call_1");
        assert_eq!(tool_result["toolName"], "read");
        assert_eq!(tool_result["isError"], false);
        // None of these snake_case names should have leaked through.
        for absent in ["tool_call_id", "tool_name", "is_error", "stop_reason"] {
            assert!(
                assistant.get(absent).is_none() && tool_result.get(absent).is_none(),
                "{absent} leaked through unconverted"
            );
        }

        let custom = entries
            .iter()
            .find(|entry| entry["type"] == "custom")
            .expect("the custom entry");
        assert_eq!(custom["parentId"], "3");
        assert_eq!(custom["customType"], "note");
        assert_eq!(custom["data"]["kept"], true);
    }

    /// Only a tool actually offered to the model contributes its snippet or its
    /// guidelines — the same filter `micro_extensions::prompt_section` applies — and a
    /// guideline repeated by more than one tool is said once.
    #[test]
    fn tool_prompt_options_only_counts_tools_the_model_was_offered() {
        let offered = micro_extensions::RegisteredTool {
            name: "read".into(),
            description: String::new(),
            parameters: Value::Null,
            label: None,
            prompt_snippet: Some("  read a file  ".into()),
            prompt_guidelines: vec!["be careful".into(), "".into()],
            constrained_sampling: None,
            render_shell: None,
            execution_mode: None,
        };
        let also_offered = micro_extensions::RegisteredTool {
            name: "write".into(),
            description: String::new(),
            parameters: Value::Null,
            label: None,
            prompt_snippet: None,
            prompt_guidelines: vec!["be careful".into()],
            constrained_sampling: None,
            render_shell: None,
            execution_mode: None,
        };
        let left_out = micro_extensions::RegisteredTool {
            name: "search".into(),
            description: String::new(),
            parameters: Value::Null,
            label: None,
            prompt_snippet: Some("search the web".into()),
            prompt_guidelines: vec!["never lie".into()],
            constrained_sampling: None,
            render_shell: None,
            execution_mode: None,
        };

        let (snippets, guidelines) = tool_prompt_options(
            &[offered, also_offered, left_out],
            &["read".to_string(), "write".to_string()],
        );

        assert_eq!(snippets, json!({ "read": "read a file" }));
        assert_eq!(guidelines, vec!["be careful"]);
    }

    /// `getSystemPromptOptions()` is assembled only when the caller says this snapshot is
    /// for a command's own context — a tool call or an event dispatch asking the same
    /// `get_context` request without it gets none of the extra weight.
    #[tokio::test]
    async fn get_context_assembles_system_prompt_options_only_for_a_command() {
        let state = Arc::new(tokio::sync::RwLock::new(State {
            offered_tools: Default::default(),
            tools: vec!["read".into()],
            custom_prompt: Some("you are a pirate".into()),
            appended_prompt: Some("also: arr".into()),
            context_files: vec![(PathBuf::from("/ws/AGENTS.md"), "be nice".into())],
            skills: vec![micro_skills::Skill {
                name: "deploy".into(),
                description: "ships the app".into(),
                path: PathBuf::from("/ws/.micro/skills/deploy/SKILL.md"),
                base_dir: PathBuf::from("/ws/.micro/skills/deploy"),
                source: "project".into(),
                model_invocable: true,
            }],
            tool_snippets: json!({ "read": "read a file" }),
            prompt_guidelines: vec!["be careful".into()],
            ..State::default()
        }));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        let plain = answer(
            "get_context",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert!(plain.get("systemPromptOptions").is_none(), "{plain}");

        let for_a_command = answer(
            "get_context",
            &json!({ "commandContext": true }),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        let options = &for_a_command["systemPromptOptions"];
        assert_eq!(options["customPrompt"], "you are a pirate");
        assert_eq!(options["appendSystemPrompt"], "also: arr");
        assert_eq!(options["selectedTools"], json!(["read"]));
        assert_eq!(options["toolSnippets"], json!({ "read": "read a file" }));
        assert_eq!(options["promptGuidelines"], json!(["be careful"]));
        assert_eq!(options["contextFiles"][0]["path"], "/ws/AGENTS.md");
        assert_eq!(options["contextFiles"][0]["content"], "be nice");
        assert_eq!(options["skills"][0]["name"], "deploy");
        assert_eq!(options["skills"][0]["disableModelInvocation"], false);
        assert_eq!(options["skills"][0]["sourceInfo"]["scope"], "project");
        // `cwd` is not this answer's to give — `context.ts` fills it in from `where`.
        assert!(options.get("cwd").is_none(), "{options}");
    }

    /// Naming the session takes effect, and asking gives the name back.
    #[tokio::test]
    async fn an_extension_can_name_the_session_and_read_it_back() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        let unnamed = answer(
            "get_session_name",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert!(unnamed["name"].is_null(), "{unnamed}");

        let set = answer(
            "set_session_name",
            &json!({ "name": "the good one" }),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert_eq!(set["ok"], true);

        let named = answer(
            "get_session_name",
            &json!({}),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert_eq!(named["name"], "the good one");
    }

    /// `/reload` and `/new` are typed into the interface as if the reader had asked for
    /// them, and answering with `cancelled: false` says the line was queued.
    #[tokio::test]
    async fn an_extension_can_reload_and_start_a_new_session() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        let reloading = tokio::spawn(async move {
            answer(
                "reload",
                &json!({}),
                None,
                &workspace,
                &unconfined(),
                &Broker::open(),
                &state,
                &session,
                Some(&asker),
            )
            .await
        });

        let mut request = requests.recv().await.expect("a reload");
        assert_eq!(request.method, "send_user_message");
        assert_eq!(request.title, "/reload");
        request.answer(json!({ "queued": true }));

        assert_eq!(reloading.await.unwrap()["cancelled"], false);
    }

    /// With no interface to type into, these all answer `cancelled: true` at once rather
    /// than waiting on a reader who is not there.
    #[tokio::test]
    async fn session_navigation_with_no_interface_comes_back_cancelled() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        for request in ["reload", "new_session"] {
            let answered = answer(request, &json!({}), None, &workspace, &unconfined(), &Broker::open(), &state, &session, None).await;
            assert_eq!(answered["cancelled"], true, "{request}");
        }
    }

    /// `switch_session` and `navigate_tree` carry their argument onto the slash command
    /// they type, the same way a reader typing `/resume <id>` or `/tree <id>` would.
    #[tokio::test]
    async fn an_extension_can_switch_and_navigate_by_id() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        {
            let (asker, mut requests) = micro_tui::ui_channel();
            let workspace = workspace.clone();
            let state = Arc::clone(&state);
            let session = Arc::clone(&session);
            let switching = tokio::spawn(async move {
                answer(
                    "switch_session",
                    &json!({ "sessionPath": "abc123" }),
                    None,
                    &workspace,
                    &unconfined(),
                    &Broker::open(),
                    &state,
                    &session,
                    Some(&asker),
                )
                .await
            });
            let mut request = requests.recv().await.expect("a switch");
            assert_eq!(request.title, "/resume abc123");
            request.answer(json!({ "queued": true }));
            assert_eq!(switching.await.unwrap()["cancelled"], false);
        }

        {
            let (asker, mut requests) = micro_tui::ui_channel();
            let navigating = tokio::spawn(async move {
                answer(
                    "navigate_tree",
                    &json!({ "targetId": "7" }),
                    None,
                    &workspace,
                    &unconfined(),
                    &Broker::open(),
                    &state,
                    &session,
                    Some(&asker),
                )
                .await
            });
            let mut request = requests.recv().await.expect("a navigation");
            assert_eq!(request.title, "/tree 7");
            request.answer(json!({ "queued": true }));
            assert_eq!(navigating.await.unwrap()["cancelled"], false);
        }
    }

    /// `fork` turns pi's entry id into the message index `/fork` expects, and `position:
    /// "before"` stops one short of the entry rather than keeping it.
    #[tokio::test]
    async fn an_extension_can_fork_from_an_entry_by_id() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();
        {
            let mut session = session.lock().await;
            session.append(&micro_types::Message::user("one")).await.unwrap();
            session.append(&micro_types::Message::user("two")).await.unwrap();
            session
                .append(&micro_types::Message::user("three"))
                .await
                .unwrap();
        }

        let (asker, mut requests) = micro_tui::ui_channel();
        let forking = tokio::spawn(async move {
            answer(
                "fork",
                &json!({ "entryId": "2", "position": "before" }),
                None,
                &workspace,
                &unconfined(),
                &Broker::open(),
                &state,
                &session,
                Some(&asker),
            )
            .await
        });
        // Entry "2" sits at position 1 on the path; "before" forks up to position 0.
        let mut request = requests.recv().await.expect("a fork");
        assert_eq!(request.title, "/fork 0");
        request.answer(json!({ "queued": true }));
        assert_eq!(forking.await.unwrap()["cancelled"], false);
    }

    /// An entry that is not on the conversation is refused before anything is typed.
    #[tokio::test]
    async fn forking_from_an_entry_that_is_not_there_is_refused() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        let answered = answer(
            "fork",
            &json!({ "entryId": "not-real" }),
            None,
            &workspace,
            &unconfined(),
            &Broker::open(),
            &state,
            &session,
            None,
        )
        .await;
        assert_eq!(answered["cancelled"], true);
        assert!(answered["error"].as_str().unwrap().contains("not-real"));
    }

    /// `shutdown` and `compact` are actions, not requests: nothing is waited for, but the
    /// same `/quit` and `/compact` a reader would type still reach the interface.
    #[tokio::test]
    async fn shutdown_and_compact_are_typed_as_slash_commands() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let quitting = tokio::spawn(async move {
            carry_out("shutdown", &json!({}), None, &Broker::open(), Some(&asker), None, None, None).await
        });
        let mut request = requests.recv().await.expect("a quit");
        assert_eq!(request.title, "/quit");
        request.answer(json!({ "queued": true }));
        quitting.await.unwrap();

        let (asker, mut requests) = micro_tui::ui_channel();
        let compacting = tokio::spawn(async move {
            carry_out("compact", &json!({}), None, &Broker::open(), Some(&asker), None, None, None).await
        });
        let mut request = requests.recv().await.expect("a compact");
        assert_eq!(request.title, "/compact");
        request.answer(json!({ "queued": true }));
        compacting.await.unwrap();
    }

    /// `abort` reaches the interface as its own method, not a typed line — a keypress,
    /// not a command.
    #[tokio::test]
    async fn abort_reaches_the_interface_as_its_own_method() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let aborting =
            tokio::spawn(async move { carry_out("abort", &json!({}), None, &Broker::open(), Some(&asker), None, None, None).await });
        let mut request = requests.recv().await.expect("an abort");
        assert_eq!(request.method, "abort");
        request.answer(json!({ "interrupted": true }));
        aborting.await.unwrap();
    }

    /// A message an extension sends goes into the conversation through the interface.
    #[tokio::test]
    async fn a_message_from_an_extension_reaches_the_conversation() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let sending = tokio::spawn(async move {
            carry_out(
                "send_user_message",
                &json!({ "content": "look at the tests" }),
                None,
                &Broker::open(),
                Some(&asker),
                None,
                None,
                None,
            ).await
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
            None,
            &Broker::open(),
            Some(&asker),
            None,
            None,
            None,
        ).await;
        assert!(requests.try_recv().is_none());
    }

    /// A headless run has nobody to ask, and says so rather than choosing for them.
    #[tokio::test]
    async fn a_question_with_no_interface_comes_back_cancelled() {
        let answer = show(
            &json!({ "method": "select", "title": "pick", "options": ["a"] }),
            None,
            &Broker::open(),
            None,
            None,
        )
        .await;
        assert_eq!(answer["cancelled"], true);
    }

    /// With an interface, the question reaches it and the answer comes back.
    #[tokio::test]
    async fn a_question_reaches_the_interface() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(
                &json!({ "method": "select", "title": "pick one", "options": ["a", "b"] }),
                None,
                &Broker::open(),
                Some(&asker),
                None,
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

    /// `setTitle` carries the title in the request's title, the same field every other
    /// wire method uses for the one thing it names.
    #[tokio::test]
    async fn set_title_reaches_the_interface_by_its_title() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(&json!({ "method": "setTitle", "title": "a new title" }), None, &Broker::open(), Some(&asker), None).await
        });
        let mut request = requests.recv().await.expect("a title");
        assert_eq!(request.method, "set_title");
        assert_eq!(request.title, "a new title");
        request.answer(json!({}));
        showing.await.unwrap();
    }

    /// A widget's key, its lines and its placement each land in the field `ask_question`
    /// reads them back from.
    #[tokio::test]
    async fn set_widget_carries_its_key_lines_and_placement() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(
                &json!({
                    "method": "setWidget",
                    "key": "status",
                    "lines": ["one", "two"],
                    "placement": "belowEditor",
                }),
                None,
                &Broker::open(),
                Some(&asker),
                None,
            )
            .await
        });
        let mut request = requests.recv().await.expect("a widget");
        assert_eq!(request.method, "set_widget");
        assert_eq!(request.title, "status");
        assert_eq!(request.detail.as_deref(), Some("belowEditor"));
        assert_eq!(request.options, vec!["one", "two"]);
        request.answer(json!({}));
        showing.await.unwrap();
    }

    /// `setWorkingIndicator` with no options at all is told apart from one given an empty
    /// `frames` array by the `reset` flag, since both would otherwise carry the same
    /// (empty) options.
    #[tokio::test]
    async fn a_reset_working_indicator_is_told_apart_from_an_empty_one() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(&json!({ "method": "setWorkingIndicator", "reset": true }), None, &Broker::open(), Some(&asker), None).await
        });
        let mut request = requests.recv().await.expect("an indicator");
        assert_eq!(request.title, "reset");
        request.answer(json!({}));
        showing.await.unwrap();

        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(
                &json!({ "method": "setWorkingIndicator", "frames": [], "intervalMs": 150 }),
                None,
                &Broker::open(),
                Some(&asker),
                None,
            )
            .await
        });
        let mut request = requests.recv().await.expect("an indicator");
        assert_eq!(request.title, "set");
        assert_eq!(request.detail.as_deref(), Some("150"));
        assert!(request.options.is_empty());
        request.answer(json!({}));
        showing.await.unwrap();
    }

    /// A theme's colors are carried as the JSON object they already are, not taken apart
    /// into a string apiece.
    #[tokio::test]
    async fn set_theme_carries_a_snapshots_colors_whole() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(
                &json!({ "method": "setTheme", "name": "custom", "colors": { "accent": "#123456" } }),
                None,
                &Broker::open(),
                Some(&asker),
                None,
            )
            .await
        });
        let mut request = requests.recv().await.expect("a theme");
        assert_eq!(request.method, "set_theme");
        assert_eq!(request.title, "custom");
        let colors: Value = serde_json::from_str(&request.detail.clone().unwrap()).unwrap();
        assert_eq!(colors["accent"], "#123456");
        request.answer(json!({ "ok": true }));
        showing.await.unwrap();
    }

    /// The first `onTerminalInput` registration and the last one going away are told to the
    /// interface as actions, the same path `send_user_message` already takes.
    #[tokio::test]
    async fn watching_and_unwatching_terminal_input_reach_the_interface() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let watching =
            tokio::spawn(async move { carry_out("watch_terminal_input", &json!({}), None, &Broker::open(), Some(&asker), None, None, None).await });
        let mut request = requests.recv().await.expect("a watch");
        assert_eq!(request.method, "watch_terminal_input");
        request.answer(json!({}));
        watching.await.unwrap();

        let (asker, mut requests) = micro_tui::ui_channel();
        let unwatching = tokio::spawn(async move {
            carry_out("unwatch_terminal_input", &json!({}), None, &Broker::open(), Some(&asker), None, None, None).await
        });
        let mut request = requests.recv().await.expect("an unwatch");
        assert_eq!(request.method, "unwatch_terminal_input");
        request.answer(json!({}));
        unwatching.await.unwrap();
    }

    /// A key offered while nothing answers comes back not consumed, the same way any other
    /// `ask_event` with no host to reach does.
    #[tokio::test]
    async fn a_key_with_nothing_to_ask_is_not_consumed() {
        let (asker, mut asks) = micro_tui::terminal_input_channel();
        let asking = tokio::spawn(async move { asker.ask("j".to_string()).await });
        drop(asks.recv().await.expect("a key"));
        assert!(asking.await.unwrap().get("consume").is_none());
    }

    fn scratch(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "micro-cli-extensions-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// `ctx.ui.setWidget` with a factory reaches the screen: the host registers the
    /// component, `show` fetches its first lines and tells the interface which key it
    /// backs, and a `component_changed` this same component pushes on its own initiative —
    /// through `serve`'s real relay, not simulated — reaches that same widget.
    #[tokio::test]
    async fn a_widget_component_is_fetched_registered_and_kept_current() {
        if micro_extensions::which_bun().is_none() {
            return;
        }
        let root = scratch("widget-component");
        let extension = root.join("widget.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            let count = 0;
            ctx.ui.setWidget("status", (tui) => ({
                render: (width) => [`count: ${count} (width ${width})`],
                handleInput: () => {
                    count += 1;
                    tui.requestRender();
                    return { consume: true };
                },
            }));
            return "shown";
        },
    });
};
"#,
        )
        .unwrap();

        let host = Arc::new(
            Host::start(&root, &[extension], &root, true, false, "tui")
                .await
                .expect("the host starts"),
        );

        // Everything below runs through the same `serve` production wires everything
        // through — `take_asks` can only be taken once, so this is the only place in the
        // test that reads from the host directly.
        let (asker, mut requests) = micro_tui::ui_channel();
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        tokio::spawn({
            let host = Arc::clone(&host);
            let root = root.clone();
            async move { serve(host, root, unconfined(), Broker::open(), Some(asker), state, session).await }
        });

        let running = {
            let host = Arc::clone(&host);
            tokio::spawn(async move { host.call_command("probe", "").await })
        };

        let registering = requests.recv().await.expect("the slot is registered");
        assert_eq!(registering.method, "register_component_slot");
        assert_eq!(registering.detail.as_deref(), Some("widget:status"));
        let component_id = registering.title.clone();
        let mut registering = registering;
        registering.answer(json!({}));

        let mut setting = requests.recv().await.expect("the widget is set");
        assert_eq!(setting.method, "set_widget");
        assert_eq!(setting.title, "status");
        assert!(
            setting.options[0].starts_with("count: 0 (width"),
            "{:?}",
            setting.options
        );
        setting.answer(json!({}));
        assert_eq!(running.await.unwrap().unwrap(), json!("shown"));

        // The component reports a key was handled, on its own initiative, and the same
        // relay fetches its fresh lines and pushes them to whatever slot registered it.
        host.send_component_input(&component_id, "x", None)
            .await
            .expect("the key reaches the component");

        let changed = tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
            .await
            .expect("component_changed arrived in time")
            .expect("serve is still relaying");
        assert_eq!(changed.method, "component_changed");
        assert_eq!(changed.title, component_id);
        assert!(
            changed.options[0].starts_with("count: 1 (width"),
            "{:?}",
            changed.options
        );

        host.shutdown("quit").await;
    }

    /// `custom()` opens with a real component's first lines, and a key that makes the
    /// component call its own `done()` resolves the extension's call without micro ever
    /// having to answer the overlay itself — `done` decides locally, and only tells micro
    /// to close what it opened.
    #[tokio::test]
    async fn a_custom_overlay_resolves_through_the_components_own_done() {
        if micro_extensions::which_bun().is_none() {
            return;
        }
        let root = scratch("custom-overlay");
        let extension = root.join("custom.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            const result = await ctx.ui.custom((tui, theme, keybindings, done) => ({
                render: () => ["a custom overlay"],
                handleInput: (data) => {
                    if (data === "y") {
                        done({ picked: true });
                    }
                },
            }));
            return JSON.stringify(result);
        },
    });
};
"#,
        )
        .unwrap();

        let host = Arc::new(
            Host::start(&root, &[extension], &root, true, false, "tui")
                .await
                .expect("the host starts"),
        );

        let (asker, mut requests) = micro_tui::ui_channel();
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        tokio::spawn({
            let host = Arc::clone(&host);
            let root = root.clone();
            async move { serve(host, root, unconfined(), Broker::open(), Some(asker), state, session).await }
        });

        let running = {
            let host = Arc::clone(&host);
            tokio::spawn(async move { host.call_command("probe", "").await })
        };

        // Left open rather than answered — this is the same overlay `App::ask_question`
        // would hold in `self.question` until it closes one way or the other.
        let opening = requests.recv().await.expect("the overlay opens");
        assert_eq!(opening.method, "custom");
        assert_eq!(opening.options, vec!["a custom overlay"]);
        let component_id = opening.title.clone();

        host.send_component_input(&component_id, "y", None)
            .await
            .expect("the key reaches the component");

        let mut done = requests.recv().await.expect("done is told to micro");
        assert_eq!(done.method, "custom_done");
        let result: Value = serde_json::from_str(&done.detail.clone().unwrap()).unwrap();
        assert_eq!(result["picked"], true);
        done.answer(json!({}));
        // `done()` already resolved the extension's call locally, in the same process, the
        // moment the component called it — this overlay is dropped unanswered, the way one
        // an extension is finished with but micro has not yet been told to close would be,
        // and that must not stop the call from having already resolved.
        drop(opening);

        assert_eq!(
            running.await.unwrap().unwrap(),
            json!(r#"{"picked":true}"#)
        );

        host.shutdown("quit").await;
    }

    /// `setEditorComponent`'s first lines are fetched and pushed the same way a header's or
    /// a footer's are, and the component it registers answers `consume` for a key exactly
    /// the way `Host::send_component_input` already promises — `y` is consumed, `n` is not,
    /// which is what tells the event loop whether to fall the key through to the built-in
    /// editor once this reaches it (that half runs in `micro-tui`, not exercised here).
    #[tokio::test]
    async fn an_editor_component_is_fetched_registered_and_answers_consume() {
        if micro_extensions::which_bun().is_none() {
            return;
        }
        let root = scratch("editor-component");
        let extension = root.join("editor.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            ctx.ui.setEditorComponent(() => ({
                render: () => ["> vim mode"],
                handleInput: (data) => (data === "y" ? { consume: true } : undefined),
            }));
            return "shown";
        },
    });
};
"#,
        )
        .unwrap();

        let host = Arc::new(
            Host::start(&root, &[extension], &root, true, false, "tui")
                .await
                .expect("the host starts"),
        );

        let (asker, mut requests) = micro_tui::ui_channel();
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        tokio::spawn({
            let host = Arc::clone(&host);
            let root = root.clone();
            async move { serve(host, root, unconfined(), Broker::open(), Some(asker), state, session).await }
        });

        let running = {
            let host = Arc::clone(&host);
            tokio::spawn(async move { host.call_command("probe", "").await })
        };

        let setting = requests.recv().await.expect("the editor component is set");
        assert_eq!(setting.method, "set_editor_component");
        assert_eq!(setting.options, vec!["> vim mode"]);
        let component_id = setting.title.clone();
        assert_eq!(running.await.unwrap().unwrap(), json!("shown"));

        assert!(
            host.send_component_input(&component_id, "y", None).await.unwrap(),
            "a key the component handles is consumed"
        );
        assert!(
            !host.send_component_input(&component_id, "n", None).await.unwrap(),
            "a key it declines is not"
        );

        host.shutdown("quit").await;
    }

    /// `setStatus` twice in a row is answered in the order it was sent, even when this test
    /// is slow to answer the first — pinning the property a version that spawned every
    /// `ui_request` (rather than only the four that wait on a reader) would have broken.
    /// The second `set_status` must not be able to reach this test's `requests` channel
    /// until the first has actually been answered; if it could, this would be racing the
    /// same way `select`/`confirm`/`input`/`custom` are meant to, and a status line set
    /// twice in quick succession could settle on the wrong one.
    #[tokio::test]
    async fn two_status_pushes_in_a_row_are_answered_in_order_even_when_the_first_is_slow() {
        if micro_extensions::which_bun().is_none() {
            return;
        }
        let root = scratch("status-order");
        let extension = root.join("status.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.registerCommand("probe", {
        handler: async (args, ctx) => {
            ctx.ui.setStatus("key", "A");
            ctx.ui.setStatus("key", "B");
            return "sent";
        },
    });
};
"#,
        )
        .unwrap();

        let host = Arc::new(
            Host::start(&root, &[extension], &root, true, false, "tui")
                .await
                .expect("the host starts"),
        );

        let (asker, mut requests) = micro_tui::ui_channel();
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        tokio::spawn({
            let host = Arc::clone(&host);
            let root = root.clone();
            async move { serve(host, root, unconfined(), Broker::open(), Some(asker), state, session).await }
        });

        host.call_command("probe", "").await.unwrap();

        let mut first = requests.recv().await.expect("the first status arrives");
        assert_eq!(first.method, "set_status");
        assert_eq!(first.detail.as_deref(), Some("A"));

        // The second must not be sitting in the channel already: `serve` is still awaiting
        // this test's answer to the first, the same way it would await a reader, and has
        // not read the second `FromHost::Ui` off the host yet.
        assert!(
            requests.try_recv().is_none(),
            "the second status arrived before the first was answered"
        );

        // Deliberately slow to answer the first, so a version that let the two race would
        // have every opportunity to let the second one through first.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        first.answer(json!({ "ok": true }));

        let second = requests.recv().await.expect("the second status arrives");
        assert_eq!(second.method, "set_status");
        assert_eq!(second.detail.as_deref(), Some("B"));

        host.shutdown("quit").await;
    }

    /// Every pi-ai `Api` id micro-provider actually speaks maps to the right
    /// `WireApi`, and round-trips back through [`micro_models::wire_api_name`] to the id
    /// micro's own catalog reports (collapsed: Azure and Codex share `openai-responses`,
    /// since `client_for` tells them apart by provider, not by wire id). An id with no Rust
    /// client behind it — Mistral's Conversations API, pi's own internal format — is a
    /// named `None` rather than a guess.
    #[test]
    fn every_supported_api_id_maps_to_its_wire_protocol_and_back() {
        use micro_models::WireApi;

        assert_eq!(wire_api_from_str("anthropic-messages"), Some(WireApi::AnthropicMessages));
        assert_eq!(wire_api_from_str("openai-completions"), Some(WireApi::OpenaiCompletions));
        assert_eq!(wire_api_from_str("openai-responses"), Some(WireApi::OpenaiResponses));
        assert_eq!(wire_api_from_str("azure-openai-responses"), Some(WireApi::OpenaiResponses));
        assert_eq!(wire_api_from_str("openai-codex-responses"), Some(WireApi::OpenaiResponses));
        assert_eq!(wire_api_from_str("google-generative-ai"), Some(WireApi::GoogleGenerativeAi));
        assert_eq!(wire_api_from_str("google-vertex"), Some(WireApi::GoogleVertex));
        assert_eq!(wire_api_from_str("bedrock-converse-stream"), Some(WireApi::BedrockConverseStream));

        assert_eq!(wire_api_from_str("mistral-conversations"), None);
        assert_eq!(wire_api_from_str("pi-messages"), None);
        assert_eq!(wire_api_from_str("something-nobody-wrote"), None);

        for api in [
            WireApi::AnthropicMessages,
            WireApi::OpenaiCompletions,
            WireApi::OpenaiResponses,
            WireApi::GoogleGenerativeAi,
            WireApi::GoogleVertex,
            WireApi::BedrockConverseStream,
        ] {
            assert_eq!(
                wire_api_from_str(micro_models::wire_api_name(api)),
                Some(api),
                "{api:?} does not round-trip"
            );
        }
    }

    /// A model built from an extension's own `pi.registerProvider` config, not from
    /// micro's catalog: every field named in the payload lands on the runtime `Model`, and
    /// an absent `thinkingLevel` defaults to `Off` the same way a non-reasoning model
    /// would.
    #[test]
    fn a_model_is_read_from_what_the_extension_itself_registered() {
        let model = model_from_json(&json!({
            "id": "claude-sonnet-4-5",
            "provider": "custom-anthropic",
            "baseUrl": "https://api.anthropic.com",
            "maxTokens": 64000,
            "reasoning": true,
            "thinkingLevel": "high",
            "headers": { "anthropic-beta": "fine-grained-tool-streaming-2025-05-14" },
        }));

        assert_eq!(model.id, "claude-sonnet-4-5");
        assert_eq!(model.provider, "custom-anthropic");
        assert_eq!(model.base_url, "https://api.anthropic.com");
        assert_eq!(model.max_tokens, 64000);
        assert!(model.reasoning);
        assert_eq!(model.thinking, micro_types::ThinkingLevel::High);
        assert_eq!(
            model.headers.get("anthropic-beta").map(String::as_str),
            Some("fine-grained-tool-streaming-2025-05-14")
        );

        let bare = model_from_json(&json!({ "id": "m", "provider": "p", "baseUrl": "https://example.test" }));
        assert_eq!(bare.thinking, micro_types::ThinkingLevel::Off);
        assert!(!bare.reasoning);
        assert_eq!(bare.max_tokens, 4096, "a sane default when the extension left it out");
    }

    /// pi-ai's `Context` — a system prompt, a message history, and tool definitions — read
    /// into micro's own shape. Messages reuse [`message_from_json`], the same parser an
    /// extension's own answers to `context` handlers go through, so a message this reads
    /// is exactly a message `message_json` would have produced for the same content.
    #[test]
    fn a_context_carries_its_messages_and_tools_through_intact() {
        let context = context_from_json(&json!({
            "systemPrompt": "be brief",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 },
                {
                    "role": "toolResult",
                    "toolCallId": "call_1",
                    "toolName": "read",
                    "content": [{ "type": "text", "text": "contents" }],
                    "isError": false,
                    "timestamp": 2,
                },
            ],
            "tools": [
                { "name": "read", "description": "read a file", "parameters": { "type": "object" } },
            ],
            "cacheKey": "conversation-1",
        }));

        assert_eq!(context.system_prompt.as_deref(), Some("be brief"));
        assert_eq!(context.messages.len(), 2);
        assert!(matches!(context.messages[0], micro_types::Message::User { .. }));
        assert!(matches!(context.messages[1], micro_types::Message::ToolResult { .. }));
        assert_eq!(context.tools.len(), 1);
        assert_eq!(context.tools[0].name, "read");
        assert_eq!(context.cache_key.as_deref(), Some("conversation-1"));
    }

    /// [`drain_provider_stream`] turns micro's own `StreamEvent`s into pi-ai's
    /// `AssistantMessageEvent` shape — `contentIndex` rather than `index`, a `partial`
    /// riding along on every non-terminal event, a `done`/`error` carrying the finished
    /// message — without a second, competing translation: this is the same `Translator`
    /// micro's agent loop feeds a real turn through, just fed synthetic events instead of
    /// ones a live provider produced, so no network is needed to prove the shape is right.
    #[tokio::test]
    async fn provider_events_translate_into_pi_ais_assistant_message_event_shape() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(micro_types::StreamEvent::Start).unwrap();
        sender.send(micro_types::StreamEvent::TextStart { index: 0 }).unwrap();
        sender
            .send(micro_types::StreamEvent::TextDelta { index: 0, delta: "Hi".into() })
            .unwrap();
        sender
            .send(micro_types::StreamEvent::TextEnd { index: 0, text: "Hi there".into() })
            .unwrap();
        sender
            .send(micro_types::StreamEvent::Done {
                message: micro_types::AssistantMessage {
                    content: vec![micro_types::ContentBlock::text("Hi there")],
                    provider: "custom-anthropic".into(),
                    model: "claude-sonnet-4-5".into(),
                    usage: micro_types::Usage { input: 10, output: 4, cache_read: 0, cache_write: 0 },
                    stop_reason: micro_types::StopReason::Stop,
                    error: None,
                    timestamp: 12345,
                },
            })
            .unwrap();
        drop(sender);

        let events = drain_provider_stream(receiver).await;

        assert_eq!(events.len(), 5);
        assert_eq!(events[0]["type"], "start");
        assert_eq!(events[1]["type"], "text_start");
        assert_eq!(events[1]["contentIndex"], 0);
        assert_eq!(events[2]["type"], "text_delta");
        assert_eq!(events[2]["delta"], "Hi");
        assert_eq!(events[2]["partial"]["content"][0]["text"], "Hi", "accumulated so far, not just this delta");
        assert_eq!(events[3]["type"], "text_end");
        assert_eq!(events[3]["content"], "Hi there");
        assert_eq!(events[4]["type"], "done");
        assert_eq!(events[4]["message"]["content"][0]["text"], "Hi there");
        assert_eq!(events[4]["message"]["stopReason"], "stop");
        assert_eq!(events[4]["message"]["usage"]["input"], 10);
    }

    /// An error mid-stream is still a `done`-shaped terminal event with `type: "error"`,
    /// carrying whatever content had already arrived rather than discarding it — the same
    /// "what streamed in before the failure is real" contract `events.rs` documents.
    #[tokio::test]
    async fn a_stream_error_is_translated_with_whatever_arrived_before_it() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(micro_types::StreamEvent::Start).unwrap();
        sender.send(micro_types::StreamEvent::TextStart { index: 0 }).unwrap();
        sender
            .send(micro_types::StreamEvent::TextDelta { index: 0, delta: "partial".into() })
            .unwrap();
        sender
            .send(micro_types::StreamEvent::Error { message: "connection reset".into() })
            .unwrap();
        drop(sender);

        let events = drain_provider_stream(receiver).await;

        let last = events.last().unwrap();
        assert_eq!(last["type"], "error");
        assert_eq!(last["error"]["errorMessage"], "connection reset");
        assert_eq!(last["error"]["content"][0]["text"], "partial", "what streamed in before the failure is kept");
    }

    /// `provider_stream` refuses, by name, rather than guessing: no `api`, an `api` id
    /// with no Rust client behind it, no `model`, and no `apiKey` are each their own
    /// specific error — none of them silently answer with an empty event list.
    #[tokio::test]
    async fn provider_stream_names_what_it_is_missing_rather_than_guessing() {
        let no_api = provider_stream(&json!({})).await;
        assert!(no_api["error"].as_str().unwrap().contains("needs an api id"));

        let unknown_api = provider_stream(&json!({ "api": "mistral-conversations" })).await;
        assert!(
            unknown_api["error"].as_str().unwrap().contains("mistral-conversations"),
            "{unknown_api}"
        );

        let no_model = provider_stream(&json!({ "api": "anthropic-messages" })).await;
        assert!(no_model["error"].as_str().unwrap().contains("needs a model"));

        let no_api_key = provider_stream(&json!({
            "api": "anthropic-messages",
            "model": { "id": "claude-sonnet-4-5", "provider": "custom-anthropic", "baseUrl": "https://api.anthropic.com" },
        }))
        .await;
        assert!(no_api_key["error"].as_str().unwrap().contains("apiKey"));
    }

    /// The catalog facade answers from micro's own bundled catalog for real: a known
    /// provider with known models, not a stub returning an empty list. Filtering by
    /// provider narrows `models` without narrowing `providers`, so a caller can tell "this
    /// provider has no models" apart from "there is no such provider".
    #[test]
    fn model_catalog_answers_from_the_real_bundled_catalog() {
        let everything = model_catalog(&json!({}));
        let providers: Vec<&str> = everything["providers"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(providers.contains(&"anthropic"), "{providers:?}");
        assert!(!everything["models"].as_array().unwrap().is_empty());

        let anthropic_only = model_catalog(&json!({ "provider": "anthropic" }));
        let models = anthropic_only["models"].as_array().unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m["provider"] == "anthropic"));
        assert!(
            models.iter().any(|m| m["id"] == "claude-opus-5"),
            "{models:?}"
        );
        assert_eq!(models[0]["api"], "anthropic-messages");
        assert!(models[0]["cost"]["input"].as_f64().is_some());

        let unknown_provider = model_catalog(&json!({ "provider": "not-a-real-provider" }));
        assert!(unknown_provider["models"].as_array().unwrap().is_empty());
        assert!(
            unknown_provider["providers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "anthropic"),
            "providers list is unaffected by an unknown filter"
        );
    }
}
