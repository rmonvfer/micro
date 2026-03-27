//! Headless operation over JSON lines.
//!
//! Commands arrive on stdin, one JSON object per line; responses and the agent's own
//! events go out on stdout the same way. It is the interface without a terminal: the same
//! agent, the same session, driven by a program instead of by a person.
//!
//! Everything a command changes is changed here rather than reported as intended. A
//! command micro cannot carry out comes back as `success: false` with the reason, which is
//! the one thing a caller can act on.

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
use micro_models::Catalog;
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
    session: Arc<Mutex<Session>>,
    catalog: Catalog,
    /// Prompts waiting behind the turn in flight.
    pending: Vec<Message>,
    auto_compaction: bool,
    workspace: std::path::PathBuf,
}

impl Rpc {
    pub fn new(
        agent: Agent,
        session: Arc<Mutex<Session>>,
        catalog: Catalog,
        workspace: impl Into<std::path::PathBuf>,
    ) -> Self {
        Rpc {
            agent,
            session,
            catalog,
            pending: Vec::new(),
            auto_compaction: true,
            workspace: workspace.into(),
        }
    }

    /// Read commands until the stream ends.
    ///
    /// Reading happens on its own task, so a line that arrives while a turn is running is
    /// available immediately rather than after the turn. That is what makes `abort` mean
    /// anything: the caller can be heard while the thing it wants to stop is still going.
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

            // Whatever was queued while the turn ran is run now, in the order it arrived.
            while let Some(prompt) = self.pending.first().cloned() {
                self.pending.remove(0);
                self.turn(prompt, &mut output, &mut incoming).await?;
            }
        }

        reading.abort();
        Ok(())
    }

    /// One line, parsed. An unreadable line is reported and skipped rather than ending
    /// the session: the next line may be perfectly good.
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

            // Nothing is running when one of these arrives here, so it is a prompt like
            // any other. Sent while a turn is running, they are taken by the turn itself.
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
                // Nothing is running when this arrives here, so aborting means dropping
                // whatever was waiting — both what is queued for a run and what is
                // queued behind one.
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
                let answer = match self.catalog.get(&provider, &model_id) {
                    Some(model) => {
                        let runtime = model.to_runtime(self.agent.model().thinking);
                        self.agent.set_runtime_model(runtime);
                        Response::with(
                            id,
                            name,
                            json!({ "provider": provider, "model_id": model_id }),
                        )
                    }
                    None => Response::failed(
                        id,
                        name,
                        format!("the catalog has no {provider}/{model_id}"),
                    ),
                };
                self.answer(answer, output).await?;
            }

            Command::CycleModel { .. } => {
                let answer = match self.cycle_model() {
                    Some(model) => Response::with(id, name, model),
                    None => Response::failed(id, name, "the catalog is empty"),
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
                // The model is told what the caller ran unless it was asked not to be, so
                // the next turn knows what happened in the workspace.
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

            // Nothing runs in the background here, so there is never a command to stop.
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
    /// Run one turn, listening while it runs.
    ///
    /// A caller is heard mid-turn: `abort` drops the turn where it stands, and a steer or
    /// follow-up joins the queue to run after it. Anything else waits, because it is about
    /// a state the turn is still changing.
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
        // Taken before the run, since the run borrows the agent for as long as it lasts.
        let steering = self.agent.steering();

        // Scoped so the borrow of the agent ends with the turn, leaving whatever arrived
        // while it ran to be dealt with afterwards.
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
                            // Steering reaches the turn at its next boundary; a
                            // follow-up waits for it to be over. Both go to the run
                            // itself rather than being held and replayed afterwards.
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
                            // Anything else is about state this turn is still changing, so it
                            // is answered once the turn has finished changing it.
                            _ => deferred.push(raw),
                        }
                    }
                    _ = &mut turn => break,
                }
            }

            // Dropping the future abandons the turn. Whatever it already reported still
            // belongs on the stream.
            drop(turn);
        }

        while let Ok(event) = receiver.try_recv() {
            output.write_all(line(&event).as_bytes()).await?;
        }
        output.flush().await?;

        if aborted {
            // What was queued behind an abandoned turn was queued behind the thing that
            // was abandoned, so it goes with it.
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
            // Only reachable between turns: a turn defers everything but abort and
            // steer until it has finished.
            is_streaming: false,
            is_compacting: false,
            session_id: meta.id.clone(),
            session_file: Some(session.path().display().to_string()),
            session_name: (!meta.title.is_empty()).then(|| meta.title.clone()),
            auto_compaction_enabled: self.auto_compaction,
            message_count: self.agent.messages().len(),
            // Everything waiting to be said, whether it is queued for a run or behind
            // one that is going.
            pending_message_count: self.pending.len() + self.agent.steering().waiting(),
        }
    }

    /// The next model in the catalog, wrapping at the end.
    fn cycle_model(&mut self) -> Option<Value> {
        let models = self.catalog.models();
        if models.is_empty() {
            return None;
        }
        let current = self.agent.model().id.clone();
        let position = models.iter().position(|model| model.id == current);
        let next = match position {
            Some(position) => &models[(position + 1) % models.len()],
            None => &models[0],
        };
        let runtime = next.to_runtime(self.agent.model().thinking);
        let described = json!({ "provider": next.provider, "model_id": next.id });
        self.agent.set_runtime_model(runtime);
        Some(described)
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
        // A new session starts empty, including of anything said to the one before it.
        let _ = self.agent.steering().take_all();
        Ok(session_id)
    }

    async fn switch_session(&mut self, path: &str) -> Result<usize, String> {
        // A caller may name the file or the id; the id is what the store knows.
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
        if !session.branch_from(entry_id) {
            return Err(format!("there is no entry {entry_id} in this conversation"));
        }
        let messages = session.branch();
        let count = messages.len();
        drop(session);
        self.agent.set_messages(messages);
        Ok(count)
    }

    /// Copy the conversation up to an entry into a session of its own, and carry on in
    /// the copy. What it was copied from is left exactly as it was.
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
        let finished = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.workspace)
            .stdin(std::process::Stdio::null())
            .output()
            .await;

        match finished {
            Ok(result) => {
                let mut text = String::from_utf8_lossy(&result.stdout).into_owned();
                let errors = String::from_utf8_lossy(&result.stderr);
                if !errors.is_empty() {
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str(&errors);
                }
                BashResult {
                    output: text.trim_end().to_string(),
                    code: result.status.code().unwrap_or(-1),
                    failed: !result.status.success(),
                }
            }
            Err(error) => BashResult {
                output: format!("cannot run the command: {error}"),
                code: -1,
                failed: true,
            },
        }
    }
}

struct BashResult {
    output: String,
    code: i32,
    failed: bool,
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
        ThinkingLevel::Off => ThinkingLevel::Low,
        ThinkingLevel::Low => ThinkingLevel::Medium,
        ThinkingLevel::Medium => ThinkingLevel::High,
        ThinkingLevel::High => ThinkingLevel::Off,
    }
}
