use crate::extension_broker::action_needs;
use crate::extension_broker::request_needs;
pub use crate::extension_broker::Broker;
use micro_agent::Hooks;
use micro_agent::ToolDecision;
use micro_extensions::message_from_json;
use micro_extensions::message_json;
use micro_extensions::Capability;
use micro_extensions::FromHost;
use micro_extensions::Host;
use micro_extensions::Translator;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Answer whatever the extensions ask for, for as long as the host is running.
pub async fn serve(
    host: Arc<Host>,
    workspace: PathBuf,

    sandbox: micro_sandbox::Sandbox,

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

            FromHost::Ui {
                id,
                extension,
                payload,
            } => {
                let method = payload
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let waits_on_a_reader =
                    matches!(method, "select" | "confirm" | "input" | "custom" | "editor");
                if waits_on_a_reader {
                    let asker = asker.clone();
                    let host = Arc::clone(&host);
                    let broker = broker.clone();
                    tokio::spawn(async move {
                        let answer = show(
                            &payload,
                            extension.as_deref(),
                            &broker,
                            asker.as_ref(),
                            Some(&host),
                        )
                        .await;
                        if let Some(id) = id {
                            let _ = host.answer(&id, answer).await;
                        }
                    });
                } else {
                    let answer = show(
                        &payload,
                        extension.as_deref(),
                        &broker,
                        asker.as_ref(),
                        Some(&host),
                    )
                    .await;
                    if let Some(id) = id {
                        if host.answer(&id, answer).await.is_err() {
                            break;
                        }
                    }
                }
            }

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

    reclaim(&host, &broker, asker.as_ref()).await;
}

/// Take back everything the extensions were granted, and say so in the ledger.
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

/// Offer the host every key the interface reads, for as long as `ctx.ui.onTerminalInput` has
/// something registered.
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

pub async fn serve_host_asks(host: Arc<Host>, mut asks: micro_tui::HostAsks) {
    while let Some(mut ask) = asks.recv().await {
        let answer = match ask.event.as_str() {
            "component_input" => {
                let component_id = ask
                    .payload
                    .get("componentId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let data = ask
                    .payload
                    .get("data")
                    .and_then(Value::as_str)
                    .unwrap_or_default();

                let text = ask.payload.get("text").and_then(Value::as_str);
                let consumed = host
                    .send_component_input(component_id, data, text)
                    .await
                    .unwrap_or(false);
                let lines = component_lines(Some(&host), component_id).await;
                json!({ "consume": consumed, "lines": lines })
            }

            "get_suggestions" => host
                .ask_event("get_suggestions", ask.payload.clone())
                .await
                .ok()
                .and_then(|results| results.into_iter().next())
                .unwrap_or_else(|| json!({})),

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
#[allow(clippy::too_many_arguments)]
async fn answer(
    request: &str,
    payload: &Value,
    extension: Option<&str>,
    workspace: &Path,
    sandbox: &micro_sandbox::Sandbox,
    broker: &Broker,
    state: &Arc<tokio::sync::RwLock<State>>,
    session: &Arc<tokio::sync::Mutex<micro_session::Session>>,
    asker: Option<&micro_tui::UiAsker>,
) -> Value {
    if let Some(needs) = request_needs(request) {
        let named = match request {
            "exec" => payload
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or(request),
            "run_builtin_tool" => payload
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or(request),
            other => other,
        };
        if !broker.allows(extension, needs, named) {
            return json!({ "error": broker.refusal(extension, needs) });
        }
    }

    match request {
        "exec" => exec(payload, workspace, sandbox).await,
        "run_builtin_tool" => run_builtin_tool(payload, workspace, sandbox).await,

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

        "get_context" => {
            let state = state.read().await;
            let scoped_models = resolve_scoped_models(&state.scoped_models);

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

                "activeTools": state.tools,
                "allTools": state.all_tools,
                "commands": state.all_commands,
                "sessionName": session_name,
                "session": session_snapshot(&*session.lock().await),
            });

            if payload
                .get("commandContext")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
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

            let before = payload.get("position").and_then(Value::as_str) == Some("before");
            let Some(through) = (if before {
                position.checked_sub(1)
            } else {
                Some(position)
            }) else {
                return json!({ "cancelled": true, "error": "nothing comes before the first entry" });
            };
            queued(asker, &format!("/fork {through}")).await
        }
        other => json!({ "error": format!("micro cannot answer `{other}`") }),
    }
}

async fn queued(asker: Option<&micro_tui::UiAsker>, line: &str) -> Value {
    match asker {
        Some(asker) => {
            asker
                .ask("send_user_message", line.to_string(), None, Vec::new())
                .await;
            json!({ "cancelled": false })
        }

        None => json!({ "cancelled": true }),
    }
}

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

                "thinkingLevel": Value::Null,
            })
        })
        .collect();
    Value::Array(matched)
}

/// The tool snippets and guidelines that went into the system prompt's tools section.
pub fn all_tools(
    registered: &[micro_extensions::Registered],
    builtin: &[micro_types::ToolDefinition],
    names: &[String],
) -> Value {
    let described: Vec<Value> = names
        .iter()
        .map(|name| {
            let own = builtin.iter().find(|tool| &tool.name == name);

            let owner = registered
                .iter()
                .find(|extension| extension.tools.iter().any(|tool| &tool.name == name));
            let found =
                owner.and_then(|extension| extension.tools.iter().find(|tool| &tool.name == name));
            let source = owner
                .map(|extension| extension.path.clone())
                .unwrap_or_default();
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

pub fn tool_prompt_options(
    tools: &[micro_extensions::RegisteredTool],
    active: &[String],
) -> (Value, Vec<String>) {
    let active: std::collections::HashSet<&str> = active.iter().map(String::as_str).collect();
    let mut snippets = serde_json::Map::new();
    let mut guidelines = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tool in tools
        .iter()
        .filter(|tool| active.contains(tool.name.as_str()))
    {
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

            "timestamp": meta.created_at,
            "cwd": meta.workspace.display().to_string(),
            "parentSession": meta.parent,
        },
        "entries": entries,
        "labels": labels,
    })
}

/// Run a program on the extension's behalf.
async fn exec(payload: &Value, workspace: &Path, sandbox: &micro_sandbox::Sandbox) -> Value {
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

            if confined && micro_sandbox::is_likely_denied(&result.status, &stderr) {
                answer["denied"] = json!(true);
                answer["policy"] = json!(sandbox.policy().name());
            }
            answer
        }
        Err(error) => json!({ "error": format!("cannot run {command}: {error}") }),
    }
}

/// Run one of micro's own built-in tools on an extension's behalf.
async fn run_builtin_tool(
    payload: &Value,
    workspace: &Path,
    sandbox: &micro_sandbox::Sandbox,
) -> Value {
    use micro_tools::Tool;

    let Some(tool_name) = payload.get("tool").and_then(Value::as_str) else {
        return json!({ "error": "run_builtin_tool needs a tool name" });
    };
    let arguments = payload
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let root = workspace.to_path_buf();

    let guard = micro_tools::Guard::new(sandbox.clone());

    let result: Result<String, String> = match tool_name {
        "read" => {
            micro_tools::Read::new(root, guard)
                .execute(&arguments)
                .await
        }
        "write" => {
            micro_tools::Write::new(root, guard)
                .execute(&arguments)
                .await
        }
        "edit" => {
            micro_tools::Edit::new(root, guard)
                .execute(&arguments)
                .await
        }
        "ls" => micro_tools::Ls::new(root, guard).execute(&arguments).await,
        "find" => {
            micro_tools::Find::new(root, guard)
                .execute(&arguments)
                .await
        }
        "grep" => {
            micro_tools::Grep::new(root, guard)
                .execute(&arguments)
                .await
        }
        "bash" => {
            micro_tools::Bash::new(root, guard)
                .execute(&arguments)
                .await
        }
        other => Err(format!("unknown builtin tool: {other}")),
    };

    match result {
        Ok(text) => json!({ "result": text }),
        Err(error) => json!({ "error": error }),
    }
}

/// Map a pi-ai `Api` id to the wire protocol micro-provider actually speaks.
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
                .filter_map(|(name, value)| {
                    value
                        .as_str()
                        .map(|value| (name.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn model_from_json(value: &Value) -> micro_types::Model {
    let field_str = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
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
        max_tokens: value
            .get("maxTokens")
            .and_then(Value::as_u64)
            .unwrap_or(4096) as u32,
        thinking,
        reasoning: value
            .get("reasoning")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        compat: micro_types::Compat::default(),
        headers: headers_from_json(value.get("headers")),
    }
}

fn tool_from_json(value: &Value) -> Option<micro_types::ToolDefinition> {
    Some(micro_types::ToolDefinition {
        name: value.get("name")?.as_str()?.to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        parameters: value
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({})),
        constrained_sampling: micro_types::ConstrainedSampling::from_wire(
            value.get("constrainedSampling").cloned(),
        ),
    })
}

/// pi-ai's `Context` (`systemPrompt`/`messages`/`tools`), read into micro's own shape.
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
        system_prompt: value
            .get("systemPrompt")
            .and_then(Value::as_str)
            .map(str::to_string),
        messages,
        tools,
        headers: headers_from_json(value.get("headers"))
            .into_iter()
            .collect(),
        cache_key: value
            .get("cacheKey")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Drain a provider's stream to completion, translating each `StreamEvent` into pi-ai's own
/// `AssistantMessageEvent` shape.
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

async fn provider_stream(payload: &Value) -> Value {
    let Some(api) = payload.get("api").and_then(Value::as_str) else {
        return json!({ "error": "provider_stream needs an api id" });
    };
    let Some(wire_api) = wire_api_from_str(api) else {
        return json!({
            "error": format!("micro does not support pi-ai API \"{api}\""),
        });
    };
    let Some(model_value) = payload.get("model") else {
        return json!({ "error": "provider_stream needs a model" });
    };
    let model = model_from_json(model_value);
    let context = context_from_json(payload.get("context").unwrap_or(&Value::Null));
    let Some(api_key) = payload.get("apiKey").and_then(Value::as_str) else {
        return json!({
            "error": "provider_stream needs the extension's apiKey",
        });
    };

    let client = micro_provider::client_for(wire_api, &model.provider);
    let receiver = client.stream(model, context, api_key.to_string());
    let events = drain_provider_stream(receiver).await;
    json!({ "events": events })
}

fn model_catalog(payload: &Value) -> Value {
    let catalog = micro_models::Catalog::bundled();
    let provider_filter = payload.get("provider").and_then(Value::as_str);
    micro_models::catalog_json(&catalog, provider_filter)
}

/// Something an extension asked to have done.
#[allow(clippy::too_many_arguments)]
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

                None => eprintln!("note: an extension tried to send a message with no session"),
            }
        }

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

        "shutdown" => {
            if let Some(asker) = asker {
                asker
                    .ask("send_user_message", "/quit", None, Vec::new())
                    .await;
            }
        }

        "compact" => {
            if let Some(asker) = asker {
                asker
                    .ask("send_user_message", "/compact", None, Vec::new())
                    .await;
            }
        }

        "abort" => {
            if let Some(asker) = asker {
                asker.ask("abort", String::new(), None, Vec::new()).await;
            }
        }

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

/// A registered component's lines at a guessed width.
async fn component_lines(host: Option<&Arc<Host>>, component_id: &str) -> Vec<String> {
    match host {
        Some(host) => host
            .render_component(component_id, RENDER_WIDTH)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Tell the interface a component's id now backs this slot.
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

        "editor" => {
            asker
                .ask(
                    "editor",
                    text("title").unwrap_or_default(),
                    text("prefill"),
                    Vec::new(),
                )
                .await
        }

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

        "setTitle" => {
            asker
                .ask(
                    "set_title",
                    text("title").unwrap_or_default(),
                    None,
                    Vec::new(),
                )
                .await
        }
        "setWorkingMessage" => {
            asker
                .ask("set_working_message", "", text("message"), Vec::new())
                .await
        }
        "setWorkingVisible" => {
            let visible = payload
                .get("visible")
                .and_then(Value::as_bool)
                .unwrap_or(true);
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
            asker
                .ask("set_working_indicator", title, interval, frames)
                .await
        }
        "setHiddenThinkingLabel" => {
            asker
                .ask("set_hidden_thinking_label", "", text("label"), Vec::new())
                .await
        }

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
                .ask(
                    method,
                    text("title").unwrap_or_default(),
                    text("detail"),
                    lines,
                )
                .await
        }
        "setEditorText" => {
            asker
                .ask("set_editor_text", "", text("text"), Vec::new())
                .await
        }
        "pasteToEditor" => {
            asker
                .ask("paste_to_editor", "", text("text"), Vec::new())
                .await
        }

        "setTheme" => {
            let colors = payload.get("colors").map(Value::to_string);
            asker
                .ask(
                    "set_theme",
                    text("name").unwrap_or_default(),
                    colors,
                    Vec::new(),
                )
                .await
        }
        "setToolsExpanded" => {
            let expanded = payload
                .get("expanded")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            asker
                .ask("set_tools_expanded", expanded.to_string(), None, Vec::new())
                .await
        }

        "custom" => {
            let component_id = text("componentId").unwrap_or_default();
            let lines = component_lines(host, &component_id).await;
            asker.ask("custom", component_id, None, lines).await
        }

        "customDone" => {
            let result = payload.get("result").cloned().unwrap_or(Value::Null);
            asker
                .ask("custom_done", "", Some(result.to_string()), Vec::new())
                .await
        }

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
            None => {
                asker
                    .ask("set_editor_component", "", None, Vec::new())
                    .await
            }
        },

        other => json!({ "cancelled": true, "error": format!("micro cannot show `{other}`") }),
    }
}

/// How wide a renderer is told the screen is.
const RENDER_WIDTH: usize = 80;

/// What micro is running, as an extension asking would see it.
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
    /// Which tools the model is told about, shared with the agent so `setActiveTools` reaches the
    /// next turn.
    pub offered_tools: Arc<std::sync::RwLock<Option<Vec<String>>>>,
    pub commands: Vec<String>,
    /// Every tool that exists, described the way `getAllTools()` answers.
    pub all_tools: Value,
    /// Every command that can be typed, described the way `getCommands()` answers.
    pub all_commands: Value,
    /// What the model was told before the conversation started, so an extension asking can be
    /// answered without waiting on the agent.
    pub system_prompt: String,

    pub scoped_models: Vec<String>,
    /// What went into the system prompt, kept apart from the assembled `system_prompt` above.
    pub custom_prompt: Option<String>,
    pub appended_prompt: Option<String>,
    pub context_files: Vec<(PathBuf, String)>,
    pub skills: Vec<micro_skills::Skill>,

    pub tool_snippets: Value,
    pub prompt_guidelines: Vec<String>,
}

/// Tell the extensions something happened somewhere other than inside a turn.
pub async fn announce(host: Option<&Arc<Host>>, event: &str, payload: Value) {
    if let Some(host) = host {
        let _ = host.notify(event, payload).await;
    }
}

/// Ask the extensions about something they are allowed to change, and hand back what they said.
pub async fn consult(host: Option<&Arc<Host>>, event: &str, payload: Value) -> Vec<Value> {
    match host {
        Some(host) => host.ask_event(event, payload).await.unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Ask the extensions whether something may go ahead, before it does.
pub async fn cancelled(host: Option<&Arc<Host>>, event: &str, payload: Value) -> bool {
    consult(host, event, payload).await.iter().any(|answer| {
        answer
            .get("cancel")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

/// Extensions deciding what a tool call may do.
pub struct ExtensionHooks {
    host: Arc<Host>,
    /// Who may change what, and where the fact that they did is written down.
    broker: Broker,
    /// What the model is told before the conversation, as the agent holds it.
    prefix: micro_agent::PrefixControl,
    /// Where this run operates, for `before_agent_start`'s `systemPromptOptions`.
    cwd: String,
    /// The arguments a tool call was started with, kept by call id until it answers.
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

        let answers = self.broker.heeded(answers, Capability::Events, "tool_call");

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
        let answers = self
            .broker
            .heeded(answers, Capability::Events, "tool_result");

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
        let (text, images) = match prompt {
            micro_types::Message::User { content, .. } => (
                content
                    .iter()
                    .map(micro_types::ContentBlock::as_text)
                    .collect::<String>(),
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

        let answers = self
            .broker
            .heeded_from(answers, Capability::Context, "system_prompt");

        for (source, answer) in &answers {
            if let Some(replacement) = answer.get("systemPrompt").and_then(Value::as_str) {
                self.broker.record(
                    source.as_deref(),
                    Capability::Context.as_str(),
                    "system_prompt",
                    true,
                    None,
                );

                let span = micro_types::PrefixSpan {
                    source: micro_types::EventSource::Extension(
                        self.broker.grants.name_of(source.as_deref()),
                    ),
                    bytes: replacement.len() as u64,
                    hash: micro_types::content_hash(replacement.as_bytes()),
                };
                self.prefix
                    .override_run(replacement, vec![span], "extension");
            }
        }
        None
    }

    async fn before_request(&self, context: micro_types::Context) -> micro_types::Context {
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
            for (source, answer) in
                self.broker
                    .heeded_from(answers, Capability::Context, "messages")
            {
                let Some(messages) = answer.get("messages").and_then(Value::as_array) else {
                    continue;
                };
                let replaced: Vec<micro_types::Message> = messages
                    .iter()
                    .filter_map(micro_extensions::message_from_json)
                    .collect();
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

    /// The policy these tests hand over.
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

    /// Which shape the failure arrives in depends on whether the session is confined: a command
    /// micro spawns itself cannot start at all.
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

    /// A scratch workspace of its own.
    fn scratch_workspace() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "micro-extensions-builtin-tool-{}-{}",
            std::process::id(),
            micro_types::now_ms()
        ));
        std::fs::create_dir_all(&root).expect("a scratch workspace");
        root
    }

    /// `createWriteTool`/`createReadTool` on the extension side proxy through exactly this request.
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

    /// `edit`'s exact-string match is the same fuzzy-matched, ambiguity-refusing logic the model's
    /// own edit tool runs on.
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

    /// Bash and ls reach `crates/micro-tools`'s real implementations too, not only the file tools.
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
        assert!(response["result"]
            .as_str()
            .unwrap()
            .contains("from-answer.txt"));
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

        let model = answer(
            "get_model",
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

        assert_eq!(context["scopedModels"], serde_json::json!([]));
    }

    /// A pattern matches by provider-qualified id or by bare id, the same prefix match `/model`'s
    /// own shortlist uses.
    #[tokio::test]
    async fn scoped_models_resolve_against_the_catalog() {
        let unscoped = resolve_scoped_models(&[]);
        assert_eq!(unscoped, serde_json::json!([]));

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

            session
                .append(&micro_types::Message::Assistant(
                    micro_types::AssistantMessage {
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
                    },
                ))
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
            session
                .append_custom("note", json!({ "kept": true }))
                .await
                .unwrap();
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

    /// Only a tool actually offered to the model contributes its snippet or its guidelines.
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

    /// `getSystemPromptOptions()` is assembled only when the caller says this snapshot is for a
    /// command's own context.
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

    #[tokio::test]
    async fn session_navigation_with_no_interface_comes_back_cancelled() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();

        for request in ["reload", "new_session"] {
            let answered = answer(
                request,
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
            assert_eq!(answered["cancelled"], true, "{request}");
        }
    }

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

    #[tokio::test]
    async fn an_extension_can_fork_from_an_entry_by_id() {
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        let workspace = std::env::temp_dir();
        {
            let mut session = session.lock().await;
            session
                .append(&micro_types::Message::user("one"))
                .await
                .unwrap();
            session
                .append(&micro_types::Message::user("two"))
                .await
                .unwrap();
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

    #[tokio::test]
    async fn shutdown_and_compact_are_typed_as_slash_commands() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let quitting = tokio::spawn(async move {
            carry_out(
                "shutdown",
                &json!({}),
                None,
                &Broker::open(),
                Some(&asker),
                None,
                None,
                None,
            )
            .await
        });
        let mut request = requests.recv().await.expect("a quit");
        assert_eq!(request.title, "/quit");
        request.answer(json!({ "queued": true }));
        quitting.await.unwrap();

        let (asker, mut requests) = micro_tui::ui_channel();
        let compacting = tokio::spawn(async move {
            carry_out(
                "compact",
                &json!({}),
                None,
                &Broker::open(),
                Some(&asker),
                None,
                None,
                None,
            )
            .await
        });
        let mut request = requests.recv().await.expect("a compact");
        assert_eq!(request.title, "/compact");
        request.answer(json!({ "queued": true }));
        compacting.await.unwrap();
    }

    #[tokio::test]
    async fn abort_reaches_the_interface_as_its_own_method() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let aborting = tokio::spawn(async move {
            carry_out(
                "abort",
                &json!({}),
                None,
                &Broker::open(),
                Some(&asker),
                None,
                None,
                None,
            )
            .await
        });
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
            None,
            &Broker::open(),
            Some(&asker),
            None,
            None,
            None,
        )
        .await;
        assert!(requests.try_recv().is_none());
    }

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

    /// `setTitle` carries the title in the request's title, the same field every other wire method
    /// uses for the one thing it names.
    #[tokio::test]
    async fn set_title_reaches_the_interface_by_its_title() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(
                &json!({ "method": "setTitle", "title": "a new title" }),
                None,
                &Broker::open(),
                Some(&asker),
                None,
            )
            .await
        });
        let mut request = requests.recv().await.expect("a title");
        assert_eq!(request.method, "set_title");
        assert_eq!(request.title, "a new title");
        request.answer(json!({}));
        showing.await.unwrap();
    }

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

    #[tokio::test]
    async fn a_reset_working_indicator_is_told_apart_from_an_empty_one() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let showing = tokio::spawn(async move {
            show(
                &json!({ "method": "setWorkingIndicator", "reset": true }),
                None,
                &Broker::open(),
                Some(&asker),
                None,
            )
            .await
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

    /// A theme's colors are carried as the JSON object they already are, not taken apart into a
    /// string apiece.
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

    #[tokio::test]
    async fn watching_and_unwatching_terminal_input_reach_the_interface() {
        let (asker, mut requests) = micro_tui::ui_channel();
        let watching = tokio::spawn(async move {
            carry_out(
                "watch_terminal_input",
                &json!({}),
                None,
                &Broker::open(),
                Some(&asker),
                None,
                None,
                None,
            )
            .await
        });
        let mut request = requests.recv().await.expect("a watch");
        assert_eq!(request.method, "watch_terminal_input");
        request.answer(json!({}));
        watching.await.unwrap();

        let (asker, mut requests) = micro_tui::ui_channel();
        let unwatching = tokio::spawn(async move {
            carry_out(
                "unwatch_terminal_input",
                &json!({}),
                None,
                &Broker::open(),
                Some(&asker),
                None,
                None,
                None,
            )
            .await
        });
        let mut request = requests.recv().await.expect("an unwatch");
        assert_eq!(request.method, "unwatch_terminal_input");
        request.answer(json!({}));
        unwatching.await.unwrap();
    }

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

        let (asker, mut requests) = micro_tui::ui_channel();
        let state = Arc::new(tokio::sync::RwLock::new(State::default()));
        let session = scratch_session().await;
        tokio::spawn({
            let host = Arc::clone(&host);
            let root = root.clone();
            async move {
                serve(
                    host,
                    root,
                    unconfined(),
                    Broker::open(),
                    Some(asker),
                    state,
                    session,
                )
                .await
            }
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
            async move {
                serve(
                    host,
                    root,
                    unconfined(),
                    Broker::open(),
                    Some(asker),
                    state,
                    session,
                )
                .await
            }
        });

        let running = {
            let host = Arc::clone(&host);
            tokio::spawn(async move { host.call_command("probe", "").await })
        };

        let opening = tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
            .await
            .expect("the overlay opens in time")
            .expect("the UI channel stays open");
        assert_eq!(opening.method, "custom");
        assert_eq!(opening.options, vec!["a custom overlay"]);
        let component_id = opening.title.clone();

        host.send_component_input(&component_id, "y", None)
            .await
            .expect("the key reaches the component");

        let mut done = tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
            .await
            .expect("custom_done arrives in time")
            .expect("the UI channel stays open");
        assert_eq!(done.method, "custom_done");
        let result: Value = serde_json::from_str(&done.detail.clone().unwrap()).unwrap();
        assert_eq!(result["picked"], true);
        done.answer(json!({}));

        drop(opening);

        assert_eq!(running.await.unwrap().unwrap(), json!(r#"{"picked":true}"#));

        host.shutdown("quit").await;
    }

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
            async move {
                serve(
                    host,
                    root,
                    unconfined(),
                    Broker::open(),
                    Some(asker),
                    state,
                    session,
                )
                .await
            }
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
            host.send_component_input(&component_id, "y", None)
                .await
                .unwrap(),
            "a key the component handles is consumed"
        );
        assert!(
            !host
                .send_component_input(&component_id, "n", None)
                .await
                .unwrap(),
            "a key it declines is not"
        );

        host.shutdown("quit").await;
    }

    /// `setStatus` twice in a row is answered in the order it was sent, even when this test is slow
    /// to answer the first.
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
            async move {
                serve(
                    host,
                    root,
                    unconfined(),
                    Broker::open(),
                    Some(asker),
                    state,
                    session,
                )
                .await
            }
        });

        host.call_command("probe", "").await.unwrap();

        let mut first = requests.recv().await.expect("the first status arrives");
        assert_eq!(first.method, "set_status");
        assert_eq!(first.detail.as_deref(), Some("A"));

        assert!(
            requests.try_recv().is_none(),
            "the second status arrived before the first was answered"
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        first.answer(json!({ "ok": true }));

        let second = requests.recv().await.expect("the second status arrives");
        assert_eq!(second.method, "set_status");
        assert_eq!(second.detail.as_deref(), Some("B"));

        host.shutdown("quit").await;
    }

    #[test]
    fn every_supported_api_id_maps_to_its_wire_protocol_and_back() {
        use micro_models::WireApi;

        assert_eq!(
            wire_api_from_str("anthropic-messages"),
            Some(WireApi::AnthropicMessages)
        );
        assert_eq!(
            wire_api_from_str("openai-completions"),
            Some(WireApi::OpenaiCompletions)
        );
        assert_eq!(
            wire_api_from_str("openai-responses"),
            Some(WireApi::OpenaiResponses)
        );
        assert_eq!(
            wire_api_from_str("azure-openai-responses"),
            Some(WireApi::OpenaiResponses)
        );
        assert_eq!(
            wire_api_from_str("openai-codex-responses"),
            Some(WireApi::OpenaiResponses)
        );
        assert_eq!(
            wire_api_from_str("google-generative-ai"),
            Some(WireApi::GoogleGenerativeAi)
        );
        assert_eq!(
            wire_api_from_str("google-vertex"),
            Some(WireApi::GoogleVertex)
        );
        assert_eq!(
            wire_api_from_str("bedrock-converse-stream"),
            Some(WireApi::BedrockConverseStream)
        );

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

        let bare = model_from_json(
            &json!({ "id": "m", "provider": "p", "baseUrl": "https://example.test" }),
        );
        assert_eq!(bare.thinking, micro_types::ThinkingLevel::Off);
        assert!(!bare.reasoning);
        assert_eq!(
            bare.max_tokens, 4096,
            "a sane default when the extension left it out"
        );
    }

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
        assert!(matches!(
            context.messages[0],
            micro_types::Message::User { .. }
        ));
        assert!(matches!(
            context.messages[1],
            micro_types::Message::ToolResult { .. }
        ));
        assert_eq!(context.tools.len(), 1);
        assert_eq!(context.tools[0].name, "read");
        assert_eq!(context.cache_key.as_deref(), Some("conversation-1"));
    }

    /// [`drain_provider_stream`] turns micro's own `StreamEvent`s into pi-ai's
    /// `AssistantMessageEvent` shape.
    #[tokio::test]
    async fn provider_events_translate_into_pi_ais_assistant_message_event_shape() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(micro_types::StreamEvent::Start).unwrap();
        sender
            .send(micro_types::StreamEvent::TextStart { index: 0 })
            .unwrap();
        sender
            .send(micro_types::StreamEvent::TextDelta {
                index: 0,
                delta: "Hi".into(),
            })
            .unwrap();
        sender
            .send(micro_types::StreamEvent::TextEnd {
                index: 0,
                text: "Hi there".into(),
            })
            .unwrap();
        sender
            .send(micro_types::StreamEvent::Done {
                message: micro_types::AssistantMessage {
                    content: vec![micro_types::ContentBlock::text("Hi there")],
                    provider: "custom-anthropic".into(),
                    model: "claude-sonnet-4-5".into(),
                    usage: micro_types::Usage {
                        input: 10,
                        output: 4,
                        cache_read: 0,
                        cache_write: 0,
                    },
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
        assert_eq!(
            events[2]["partial"]["content"][0]["text"], "Hi",
            "accumulated so far, not just this delta"
        );
        assert_eq!(events[3]["type"], "text_end");
        assert_eq!(events[3]["content"], "Hi there");
        assert_eq!(events[4]["type"], "done");
        assert_eq!(events[4]["message"]["content"][0]["text"], "Hi there");
        assert_eq!(events[4]["message"]["stopReason"], "stop");
        assert_eq!(events[4]["message"]["usage"]["input"], 10);
    }

    #[tokio::test]
    async fn a_stream_error_is_translated_with_whatever_arrived_before_it() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        sender.send(micro_types::StreamEvent::Start).unwrap();
        sender
            .send(micro_types::StreamEvent::TextStart { index: 0 })
            .unwrap();
        sender
            .send(micro_types::StreamEvent::TextDelta {
                index: 0,
                delta: "partial".into(),
            })
            .unwrap();
        sender
            .send(micro_types::StreamEvent::Error {
                message: "connection reset".into(),
            })
            .unwrap();
        drop(sender);

        let events = drain_provider_stream(receiver).await;

        let last = events.last().unwrap();
        assert_eq!(last["type"], "error");
        assert_eq!(last["error"]["errorMessage"], "connection reset");
        assert_eq!(
            last["error"]["content"][0]["text"], "partial",
            "what streamed in before the failure is kept"
        );
    }

    #[tokio::test]
    async fn provider_stream_names_what_it_is_missing_rather_than_guessing() {
        let no_api = provider_stream(&json!({})).await;
        assert!(no_api["error"]
            .as_str()
            .unwrap()
            .contains("needs an api id"));

        let unknown_api = provider_stream(&json!({ "api": "mistral-conversations" })).await;
        assert!(
            unknown_api["error"]
                .as_str()
                .unwrap()
                .contains("mistral-conversations"),
            "{unknown_api}"
        );

        let no_model = provider_stream(&json!({ "api": "anthropic-messages" })).await;
        assert!(no_model["error"]
            .as_str()
            .unwrap()
            .contains("needs a model"));

        let no_api_key = provider_stream(&json!({
            "api": "anthropic-messages",
            "model": { "id": "claude-sonnet-4-5", "provider": "custom-anthropic", "baseUrl": "https://api.anthropic.com" },
        }))
        .await;
        assert!(no_api_key["error"].as_str().unwrap().contains("apiKey"));
    }

    #[test]
    fn model_catalog_answers_from_the_real_bundled_catalog() {
        let everything = model_catalog(&json!({}));
        let providers: Vec<&str> = everything["providers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
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
