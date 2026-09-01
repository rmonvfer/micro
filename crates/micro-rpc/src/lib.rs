//! Headless operation over JSON lines.

mod jsonl;
mod protocol;

pub use jsonl::line;
pub use jsonl::Lines;
pub use protocol::Command;
pub use protocol::Image;
pub use protocol::Response;
pub use protocol::SessionState;
pub use protocol::SlashCommand;

use micro_agent::Agent;
use micro_agent::ModelSwap;
use micro_auth::AuthStore;
use micro_models::Catalog;
use micro_models::ModelDef;
use micro_session::Session;
use micro_types::AgentEvent;
use micro_types::ContentBlock;
use micro_types::Message;
use micro_types::ThinkingLevel;
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// Everything the mode needs to answer a command.
pub struct Rpc {
    agent: Agent,
    auth: Result<Arc<AuthStore>, String>,
    session: Arc<Mutex<Session>>,
    catalog: Catalog,
    /// Prompts waiting behind the turn in flight.
    pending: Vec<Message>,
    auto_compaction: bool,
}

impl Rpc {
    pub fn new(
        agent: Agent,
        session: Arc<Mutex<Session>>,
        catalog: Catalog,
        _workspace: impl Into<std::path::PathBuf>,
    ) -> Self {
        Rpc {
            agent,
            auth: AuthStore::open()
                .map(Arc::new)
                .map_err(|error| error.to_string()),
            session,
            catalog,
            pending: Vec::new(),
            auto_compaction: true,
        }
    }

    /// Use a specific credential store when the caller already has one open.
    pub fn with_auth_store(mut self, auth: Arc<AuthStore>) -> Self {
        self.auth = Ok(auth);
        self
    }

    /// Read commands until the stream ends.
    pub async fn run<R, W>(&mut self, input: R, output: W) -> std::io::Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin,
    {
        let (sender, mut incoming) = tokio::sync::mpsc::unbounded_channel::<String>();
        let reading = tokio::spawn(async move {
            let mut lines = Lines::new(input);
            while let Ok(Some(raw)) = lines.next().await {
                if sender.send(raw).is_err() {
                    return;
                }
            }
        });

        let mut output = output;
        while let Some(raw) = incoming.recv().await {
            let Some(command) = self.read(&raw, &mut output).await? else {
                continue;
            };
            self.dispatch(command, &mut output, &mut incoming).await?;
            output.flush().await?;

            while let Some(prompt) = self.pending.first().cloned() {
                self.pending.remove(0);
                self.turn(prompt, &mut output, &mut incoming).await?;
            }
        }

        reading.abort();
        Ok(())
    }

    /// One line, parsed.
    async fn read<W>(&self, raw: &str, output: &mut W) -> std::io::Result<Option<Command>>
    where
        W: AsyncWrite + Unpin,
    {
        match serde_json::from_str(raw) {
            Ok(command) => Ok(Some(command)),
            Err(error) => {
                let answer = Response::failed(None, "unknown", format!("unreadable: {error}"));
                output.write_all(line(&answer).as_bytes()).await?;
                output.flush().await?;
                Ok(None)
            }
        }
    }

    async fn dispatch<W>(
        &mut self,
        command: Command,
        output: &mut W,
        incoming: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    ) -> std::io::Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        let id = command.id().map(str::to_string);
        let id = id.as_deref();
        let name = command.name();

        match command {
            Command::Prompt {
                message, images, ..
            } => {
                let prompt = build_prompt(&message, images);
                let answer = Response::ok(id, name);
                output.write_all(line(&answer).as_bytes()).await?;
                output.flush().await?;
                self.turn(prompt, output, incoming).await?;
            }

            Command::Steer {
                message, images, ..
            }
            | Command::FollowUp {
                message, images, ..
            } => {
                self.pending.push(build_prompt(&message, images));
                let answer = Response::ok(id, name);
                output.write_all(line(&answer).as_bytes()).await?;
                output.flush().await?;
            }

            Command::Abort { .. } => {
                self.pending.clear();
                let _ = self.agent.steering().take_all();
                self.answer(Response::ok(id, name), output).await?;
            }

            Command::NewSession { .. } => {
                let answer = match self.new_session().await {
                    Ok(session_id) => Response::with(id, name, json!({ "session_id": session_id })),
                    Err(error) => Response::failed(id, name, error),
                };
                self.answer(answer, output).await?;
            }

            Command::GetState { .. } => {
                let state = self.state().await;
                let answer = match serde_json::to_value(state) {
                    Ok(value) => Response::with(id, name, value),
                    Err(error) => Response::failed(id, name, error.to_string()),
                };
                self.answer(answer, output).await?;
            }

            Command::SetModel {
                provider, model_id, ..
            } => {
                let model = self.catalog.get(&provider, &model_id).cloned();
                let answer = match model {
                    Some(model) => match self.select_model(&model).await {
                        Ok(model) => Response::with(id, name, model),
                        Err(error) => Response::failed(id, name, error),
                    },
                    None => Response::failed(
                        id,
                        name,
                        format!("the catalog has no {provider}/{model_id}"),
                    ),
                };
                self.answer(answer, output).await?;
            }

            Command::CycleModel { .. } => {
                let answer = match self.cycle_model().await {
                    Ok(Some(model)) => Response::with(id, name, model),
                    Ok(None) => Response::failed(id, name, "the catalog is empty"),
                    Err(error) => Response::failed(id, name, error),
                };
                self.answer(answer, output).await?;
            }

            Command::GetAvailableModels { .. } => {
                let models: Vec<Value> = self
                    .catalog
                    .models()
                    .iter()
                    .map(|model| {
                        json!({
                            "id": model.id,
                            "provider": model.provider,
                            "name": model.name,
                            "context_window": model.context_window,
                            "max_output_tokens": model.max_output_tokens,
                            "reasoning": model.reasoning,
                        })
                    })
                    .collect();
                self.answer(
                    Response::with(id, name, json!({ "models": models })),
                    output,
                )
                .await?;
            }

            Command::SetThinkingLevel { level, .. } => {
                self.agent.set_thinking(level);
                self.answer(Response::with(id, name, json!({ "level": level })), output)
                    .await?;
            }

            Command::CycleThinkingLevel { .. } => {
                let level = next_level(self.agent.model().thinking);
                self.agent.set_thinking(level);
                self.answer(Response::with(id, name, json!({ "level": level })), output)
                    .await?;
            }

            Command::Compact { .. } => {
                let answer = match self.agent.compact_now().await {
                    Ok(summary) => {
                        Response::with(id, name, json!({ "summary": summary_text(&summary) }))
                    }
                    Err(refusal) => Response::failed(id, name, refusal.to_string()),
                };
                self.answer(answer, output).await?;
            }

            Command::SetAutoCompaction { enabled, .. } => {
                self.auto_compaction = enabled;
                self.agent.set_auto_compaction(enabled);
                self.answer(Response::ok(id, name), output).await?;
            }

            Command::Bash {
                command,
                exclude_from_context,
                ..
            } => {
                let result = self.bash(&command).await;

                if !exclude_from_context {
                    self.agent.record(Message::user(format!(
                        "<bash command=\"{command}\">\n{}\n</bash>",
                        result.output
                    )));
                }
                self.answer(
                    Response::with(
                        id,
                        name,
                        json!({
                            "output": result.output,
                            "exit_code": result.code,
                            "failed": result.failed,
                        }),
                    ),
                    output,
                )
                .await?;
            }

            Command::AbortBash { .. } => {
                self.answer(Response::ok(id, name), output).await?;
            }

            Command::GetSessionStats { .. } => {
                let session = self.session.lock().await;
                let meta = session.meta();
                self.answer(
                    Response::with(
                        id,
                        name,
                        json!({
                            "session_id": meta.id,
                            "session_file": session.path().display().to_string(),
                            "message_count": meta.message_count,
                            "created_at": meta.created_at,
                            "updated_at": meta.updated_at,
                            "model_id": meta.model_id,
                            "title": meta.title,
                        }),
                    ),
                    output,
                )
                .await?;
            }

            Command::SwitchSession { session_path, .. } => {
                let answer = match self.switch_session(&session_path).await {
                    Ok(count) => Response::with(id, name, json!({ "message_count": count })),
                    Err(error) => Response::failed(id, name, error),
                };
                self.answer(answer, output).await?;
            }

            Command::NavigateTree { entry_id, .. } => {
                let answer = match self.branch(&entry_id).await {
                    Ok(count) => Response::with(id, name, json!({ "message_count": count })),
                    Err(error) => Response::failed(id, name, error),
                };
                self.answer(answer, output).await?;
            }

            Command::Fork { entry_id, .. } => {
                let answer = match self.fork_at(&entry_id).await {
                    Ok((session_id, count)) => Response::with(
                        id,
                        name,
                        json!({ "session_id": session_id, "message_count": count }),
                    ),
                    Err(error) => Response::failed(id, name, error),
                };
                self.answer(answer, output).await?;
            }

            Command::Clone { .. } => {
                let answer = match self.clone_session().await {
                    Ok(session_id) => Response::with(id, name, json!({ "session_id": session_id })),
                    Err(error) => Response::failed(id, name, error),
                };
                self.answer(answer, output).await?;
            }

            Command::GetEntries { since, .. } => {
                let session = self.session.lock().await;
                let entries: Vec<Value> = session
                    .tree()
                    .entries()
                    .iter()
                    .skip_while(|entry| match &since {
                        Some(since) => entry.id != *since,
                        None => false,
                    })
                    .map(|entry| {
                        json!({
                            "id": entry.id,
                            "parent_id": entry.parent_id,
                            "timestamp": entry.timestamp,
                            "message": protocol::message_json(&entry.message),
                        })
                    })
                    .collect();
                drop(session);
                self.answer(
                    Response::with(id, name, json!({ "entries": entries })),
                    output,
                )
                .await?;
            }

            Command::GetTree { .. } => {
                let session = self.session.lock().await;
                let rows: Vec<Value> = session
                    .tree()
                    .outline()
                    .iter()
                    .map(|row| {
                        json!({
                            "id": row.entry.id,
                            "parent_id": row.entry.parent_id,
                            "depth": row.depth,
                            "on_path": row.on_path,
                            "is_head": row.is_head,
                        })
                    })
                    .collect();
                drop(session);
                self.answer(Response::with(id, name, json!({ "tree": rows })), output)
                    .await?;
            }

            Command::GetLastAssistantText { .. } => {
                let text = self
                    .agent
                    .messages()
                    .iter()
                    .rev()
                    .find_map(|message| match message {
                        Message::Assistant(assistant) => {
                            let text = assistant.text();
                            (!text.trim().is_empty()).then_some(text)
                        }
                        _ => None,
                    });
                self.answer(Response::with(id, name, json!({ "text": text })), output)
                    .await?;
            }

            Command::SetSessionName { name: title, .. } => {
                let answer = match self.session.lock().await.rename(&title).await {
                    Ok(()) => Response::ok(id, name),
                    Err(error) => Response::failed(id, name, error.to_string()),
                };
                self.answer(answer, output).await?;
            }

            Command::GetMessages { .. } => {
                let messages: Vec<Value> = self
                    .agent
                    .messages()
                    .iter()
                    .map(protocol::message_json)
                    .collect();
                self.answer(
                    Response::with(id, name, json!({ "messages": messages })),
                    output,
                )
                .await?;
            }

            Command::GetCommands { .. } => {
                let commands: Vec<SlashCommand> = micro_commands_list();
                let data = serde_json::to_value(&commands).unwrap_or(Value::Null);
                self.answer(
                    Response::with(id, name, json!({ "commands": data })),
                    output,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn answer<W>(&self, response: Response, output: &mut W) -> std::io::Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        output.write_all(line(&response).as_bytes()).await
    }

    /// Run one turn, writing every event the agent reports as it happens.
    async fn turn<W>(
        &mut self,
        prompt: Message,
        output: &mut W,
        incoming: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    ) -> std::io::Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        let mut deferred: Vec<String> = Vec::new();
        let mut aborted = false;

        let steering = self.agent.steering();

        {
            let turn = self.agent.run(prompt, &sender);
            tokio::pin!(turn);

            loop {
                tokio::select! {
                    biased;
                    Some(event) = receiver.recv() => {
                        output.write_all(line(&event).as_bytes()).await?;
                        output.flush().await?;
                    }
                    Some(raw) = incoming.recv() => {
                        match serde_json::from_str::<Command>(&raw) {
                            Ok(Command::Abort { id, .. }) => {
                                let answer = Response::ok(id.as_deref(), "abort");
                                output.write_all(line(&answer).as_bytes()).await?;
                                output.flush().await?;
                                aborted = true;
                                break;
                            }

                            Ok(Command::Steer { id, message, images }) => {
                                steering.steer(build_prompt(&message, images));
                                let answer = Response::ok(id.as_deref(), "steer");
                                output.write_all(line(&answer).as_bytes()).await?;
                                output.flush().await?;
                            }
                            Ok(Command::FollowUp { id, message, images }) => {
                                steering.follow_up(build_prompt(&message, images));
                                let answer = Response::ok(id.as_deref(), "follow_up");
                                output.write_all(line(&answer).as_bytes()).await?;
                                output.flush().await?;
                            }

                            _ => deferred.push(raw),
                        }
                    }
                    _ = &mut turn => break,
                }
            }
        }

        while let Ok(event) = receiver.try_recv() {
            output.write_all(line(&event).as_bytes()).await?;
        }
        output.flush().await?;

        if aborted {
            let _ = steering.take_all();
        }

        for raw in deferred {
            if let Some(command) = self.read(&raw, output).await? {
                Box::pin(self.dispatch(command, output, incoming)).await?;
            }
        }
        Ok(())
    }

    async fn state(&self) -> SessionState {
        let session = self.session.lock().await;
        let meta = session.meta();
        SessionState {
            model: self.agent.model().id.clone(),
            provider: self.agent.model().provider.clone(),
            thinking_level: self.agent.model().thinking,

            is_streaming: false,
            is_compacting: false,
            session_id: meta.id.clone(),
            session_file: Some(session.path().display().to_string()),
            session_name: (!meta.title.is_empty()).then(|| meta.title.clone()),
            auto_compaction_enabled: self.auto_compaction,
            message_count: self.agent.messages().len(),

            pending_message_count: self.pending.len() + self.agent.steering().waiting(),
        }
    }

    /// The next model in the catalog, wrapping at the end.
    async fn cycle_model(&mut self) -> Result<Option<Value>, String> {
        let models = self.catalog.models();
        if models.is_empty() {
            return Ok(None);
        }
        let current_provider = self.agent.model().provider.clone();
        let current_model = self.agent.model().id.clone();
        let position = models
            .iter()
            .position(|model| model.provider == current_provider && model.id == current_model);
        let next = match position {
            Some(position) => models[(position + 1) % models.len()].clone(),
            None => models[0].clone(),
        };
        self.select_model(&next).await.map(Some)
    }

    /// Replace every model-dependent part of the running agent.
    async fn select_model(&mut self, model: &ModelDef) -> Result<Value, String> {
        let auth = self
            .auth
            .as_ref()
            .map_err(|error| format!("cannot open the credential store: {error}"))?;
        let resolved = micro_provider::resolve(auth, model)
            .await
            .map_err(|error| format!("cannot use {}: {error}", model.qualified_id()))?;
        if resolved.api_key.is_blank() {
            return Err(format!(
                "no credential for {}; run `micro auth login {}`",
                model.provider, model.provider
            ));
        }

        let mut runtime = model.to_runtime(self.agent.model().thinking);
        if let Some(base_url) = resolved.base_url.filter(|url| !url.trim().is_empty()) {
            runtime.base_url = base_url;
        }

        let qualified = model.qualified_id();
        self.session
            .lock()
            .await
            .set_model_id(qualified)
            .await
            .map_err(|error| format!("cannot update the session model: {error}"))?;

        self.agent.set_model(ModelSwap {
            provider: resolved.client,
            model: runtime,
            api_key: resolved.api_key,
            context_window: model.context_window as usize,
            cost: model.cost.clone(),
        });

        Ok(json!({ "provider": model.provider, "model_id": model.id }))
    }

    async fn new_session(&mut self) -> Result<String, String> {
        let (workspace, model_id) = {
            let session = self.session.lock().await;
            (
                session.meta().workspace.clone(),
                session.meta().model_id.clone(),
            )
        };
        let store = micro_session::SessionStore::from_env().map_err(|error| error.to_string())?;
        let started = store
            .create(&workspace, model_id)
            .await
            .map_err(|error| error.to_string())?;

        let session_id = started.id().to_string();
        *self.session.lock().await = started;
        self.agent.set_messages(Vec::new());
        self.pending.clear();

        let _ = self.agent.steering().take_all();
        Ok(session_id)
    }

    async fn switch_session(&mut self, path: &str) -> Result<usize, String> {
        let id = std::path::Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(path);
        let store = micro_session::SessionStore::from_env().map_err(|error| error.to_string())?;
        let loaded = store.load(id).await.map_err(|error| error.to_string())?;

        let count = loaded.messages.len();
        self.agent.set_messages(loaded.messages);
        *self.session.lock().await = loaded.session;
        Ok(count)
    }

    async fn branch(&mut self, entry_id: &str) -> Result<usize, String> {
        let mut session = self.session.lock().await;
        if !session
            .branch_from(entry_id)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err(format!("there is no entry {entry_id} in this conversation"));
        }
        let messages = session.branch();
        let count = messages.len();
        drop(session);
        self.agent.set_messages(messages);
        Ok(count)
    }

    /// Copy the conversation up to an entry into a session of its own, and carry on in the copy.
    async fn fork_at(&mut self, entry_id: &str) -> Result<(String, usize), String> {
        let (id, through_index) = {
            let session = self.session.lock().await;
            let position = session
                .tree()
                .position_on_path(entry_id)
                .ok_or_else(|| format!("there is no entry {entry_id} in this conversation"))?;
            (session.id().to_string(), position)
        };

        let store = micro_session::SessionStore::from_env().map_err(|error| error.to_string())?;
        let forked = store
            .fork(&id, through_index)
            .await
            .map_err(|error| error.to_string())?;

        let session_id = forked.id().to_string();
        let messages = forked.branch();
        let count = messages.len();
        *self.session.lock().await = forked;
        self.agent.set_messages(messages);
        Ok((session_id, count))
    }

    async fn clone_session(&mut self) -> Result<String, String> {
        let (id, count) = {
            let session = self.session.lock().await;
            (session.id().to_string(), session.branch().len())
        };
        if count == 0 {
            return Err("nothing to clone yet".to_string());
        }

        let store = micro_session::SessionStore::from_env().map_err(|error| error.to_string())?;
        let cloned = store
            .fork(&id, count - 1)
            .await
            .map_err(|error| error.to_string())?;

        let session_id = cloned.id().to_string();
        *self.session.lock().await = cloned;
        Ok(session_id)
    }

    async fn bash(&self, command: &str) -> BashResult {
        match self.agent.execute_bash(command).await {
            Ok(content) => BashResult {
                output: match content
                    .iter()
                    .map(ContentBlock::as_text)
                    .collect::<Vec<_>>()
                    .join("")
                {
                    output if output == "(no output)" => String::new(),
                    output => output,
                },
                code: 0,
                failed: false,
            },
            Err(error) => BashResult::failed(error),
        }
    }
}

struct BashResult {
    output: String,
    code: i32,
    failed: bool,
}

impl BashResult {
    fn failed(error: String) -> Self {
        let code = error
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(3)
            .find_map(|words| match words {
                ["exit", "code", code] => code.parse().ok(),
                _ => None,
            })
            .unwrap_or(-1);
        let output = error
            .strip_prefix("exit code ")
            .and_then(|failure| {
                failure
                    .split_once('\n')
                    .map(|(_, output)| output.to_string())
            })
            .unwrap_or(error);
        BashResult {
            output,
            code,
            failed: true,
        }
    }
}

/// Every command a caller may invoke through a prompt.
fn micro_commands_list() -> Vec<SlashCommand> {
    micro_commands::commands()
        .iter()
        .map(|command| SlashCommand {
            name: command.name.to_string(),
            description: command.description.to_string(),
            source: "builtin".to_string(),
        })
        .collect()
}

fn build_prompt(message: &str, images: Vec<Image>) -> Message {
    let mut content: Vec<ContentBlock> = images.into_iter().map(ContentBlock::from).collect();
    content.push(ContentBlock::text(message));
    Message::User {
        content,
        timestamp: micro_types::now_ms(),
    }
}

/// What a compaction summary says, for a caller that wants to show it.
fn summary_text(message: &Message) -> String {
    match message {
        Message::Assistant(assistant) => assistant.text(),
        Message::User { content, .. } => content
            .iter()
            .map(ContentBlock::as_text)
            .collect::<Vec<_>>()
            .join(""),
        Message::ToolResult { .. } => String::new(),
    }
}

fn next_level(level: ThinkingLevel) -> ThinkingLevel {
    match level {
        ThinkingLevel::Off => ThinkingLevel::Minimal,
        ThinkingLevel::Minimal => ThinkingLevel::Low,
        ThinkingLevel::Low => ThinkingLevel::Medium,
        ThinkingLevel::Medium => ThinkingLevel::High,
        ThinkingLevel::High => ThinkingLevel::XHigh,
        ThinkingLevel::XHigh => ThinkingLevel::Max,
        ThinkingLevel::Max => ThinkingLevel::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_agent::Budget;
    use micro_testkit::FakeProvider;
    #[cfg(target_os = "macos")]
    use micro_tools::Guard;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

    fn scratch(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "micro-rpc-{label}-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn catalog() -> Catalog {
        Catalog::from_json(
            r#"{
                "providers": {
                    "anthropic": {
                        "base_url": "https://anthropic.invalid",
                        "api": "anthropic-messages",
                        "models": [{
                            "id": "shared",
                            "context_window": 1000,
                            "cost": { "input": 1.0, "output": 2.0 }
                        }]
                    },
                    "openrouter": {
                        "base_url": "https://openrouter.invalid",
                        "api": "openai-completions",
                        "models": [{
                            "id": "shared",
                            "context_window": 64000,
                            "max_output_tokens": 8000,
                            "cost": { "input": 9.0, "output": 3.0 }
                        }]
                    }
                }
            }"#,
        )
        .unwrap()
    }

    async fn rpc_with(
        root: &std::path::Path,
        catalog: Catalog,
        tools: Vec<Arc<dyn micro_tools::Tool>>,
    ) -> Rpc {
        let current = catalog.get("anthropic", "shared").unwrap();
        let sessions = micro_session::SessionStore::new(root.join("sessions"));
        let session = sessions.create(root, current.qualified_id()).await.unwrap();
        let agent = Agent::new(
            Arc::new(FakeProvider::builder().name("anthropic").build()),
            tools,
            current.to_runtime(ThinkingLevel::Low),
            "anthropic-key",
        )
        .with_context_window(current.context_window as usize)
        .with_model_cost(current.cost.clone())
        .with_budget(Budget::new(100.0, current.cost.clone()));
        Rpc::new(agent, Arc::new(Mutex::new(session)), catalog, root)
    }

    #[tokio::test]
    async fn a_cross_provider_switch_replaces_the_client_limits_pricing_and_session_model() {
        let root = scratch("provider-swap");
        let catalog = catalog();
        let auth = Arc::new(AuthStore::open_at(root.join("auth.json")).unwrap());
        auth.store_api_key("anthropic", "anthropic-key").unwrap();
        auth.store_api_key("openrouter", "openrouter-key").unwrap();
        let selected = catalog.get("openrouter", "shared").unwrap().clone();
        let mut rpc = rpc_with(&root, catalog, Vec::new())
            .await
            .with_auth_store(auth);

        let response = rpc.select_model(&selected).await.unwrap();

        assert_eq!(response["provider"], "openrouter");
        assert_eq!(rpc.agent.provider_name(), "openrouter");
        assert_eq!(rpc.agent.model().provider, "openrouter");
        assert_eq!(rpc.agent.model().id, "shared");
        assert_eq!(rpc.agent.model().max_tokens, 8_000);
        assert_eq!(rpc.agent.context_window(), 64_000);
        assert_eq!(rpc.agent.model_cost(), Some(&selected.cost));
        assert_eq!(
            rpc.session.lock().await.meta().model_id,
            "openrouter/shared"
        );
    }

    #[tokio::test]
    async fn cycling_models_matches_the_provider_as_well_as_the_model_id() {
        let root = scratch("cycle-qualified");
        let catalog = catalog();
        let auth = Arc::new(AuthStore::open_at(root.join("auth.json")).unwrap());
        auth.store_api_key("anthropic", "anthropic-key").unwrap();
        auth.store_api_key("openrouter", "openrouter-key").unwrap();
        let mut rpc = rpc_with(&root, catalog, Vec::new())
            .await
            .with_auth_store(auth);

        let cycled = rpc.cycle_model().await.unwrap().unwrap();

        assert_eq!(cycled["provider"], "openrouter");
        assert_eq!(rpc.agent.provider_name(), "openrouter");
    }

    struct RefusingBash;

    #[async_trait::async_trait]
    impl micro_tools::Tool for RefusingBash {
        fn definition(&self) -> micro_types::ToolDefinition {
            micro_types::ToolDefinition {
                name: "bash".into(),
                description: String::new(),
                parameters: json!({ "type": "object" }),
                constrained_sampling: None,
            }
        }

        async fn execute(&self, _arguments: &Value) -> Result<String, String> {
            Err("denied by policy workspace-write: exit code 1\noperation not permitted".into())
        }
    }

    #[tokio::test]
    async fn rpc_bash_uses_the_agents_guarded_tool() {
        let root = scratch("guarded-tool");
        let rpc = rpc_with(&root, catalog(), vec![Arc::new(RefusingBash)]).await;

        let result = rpc.bash("echo escaped").await;

        assert!(result.failed);
        assert_eq!(result.code, 1);
        assert!(result.output.contains("denied by policy workspace-write"));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn rpc_bash_denies_outside_writes_and_network_under_workspace_write() {
        let root = scratch("sandbox-confinement");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let outside = root.join("outside.txt");
        let guard = Guard::new(micro_sandbox::Sandbox::new(
            micro_sandbox::SandboxPolicy::workspace_write(),
            &workspace,
        ));
        let tools = micro_tools::builtin_tools(workspace.clone(), guard);
        let rpc = rpc_with(&workspace, catalog(), tools).await;

        let write = rpc
            .bash(&format!("echo escaped > {}", outside.display()))
            .await;
        assert!(write.failed, "{}", write.output);
        assert!(!outside.exists());
        assert!(
            write.output.contains("denied by policy workspace-write"),
            "{}",
            write.output
        );

        let network = rpc.bash("exec 3<>/dev/tcp/1.1.1.1/80").await;
        assert!(network.failed, "{}", network.output);
        assert!(
            network.output.contains("denied by policy workspace-write"),
            "{}",
            network.output
        );
    }
}
