//! Running slash commands on behalf of the interface.
//!
//! [`micro_commands`] decides what a command means; this carries out the part of it that
//! needs the credential store, the catalog or the session log. What it cannot do it says
//! so, rather than reporting a change that did not happen: the agent is the interface's,
//! and the conversation of record lives inside it.

use async_trait::async_trait;
use micro_auth::AuthStore;
use micro_commands::CommandContext;
use micro_commands::CommandOutcome;
use micro_commands::Picker;
use micro_models::Catalog;
use micro_models::ModelDef;
use micro_session::Session;
use micro_session::SessionStore;
use micro_tui::Applied;
use micro_tui::Commands;
use micro_tui::ConversationState;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Everything a command reads, which is what the agent was built from minus the agent.
pub struct CliCommands {
    catalog: Catalog,
    auth: Arc<AuthStore>,
    sessions: SessionStore,
    workspace: PathBuf,
    provider: String,
    model: ModelDef,
    /// The open session, shared with the task writing it: branching and renaming have to
    /// reach the log that is actually being appended to, not a second copy of it.
    session: Arc<Mutex<Session>>,
    /// Held alongside so a command can be answered without waiting on the writer.
    session_id: String,
    /// Where micro keeps what it remembers between runs, which is where a trust decision
    /// is written.
    home: PathBuf,
    /// Whether skills are announced to the model at all, so `/reload` rebuilds what the
    /// run was built with rather than something else.
    skills_enabled: bool,
    scoped_models: Vec<String>,
    /// What this run was told to look at beyond the usual places, so a reload looks in the
    /// same places the first load did.
    resources: crate::runtime::Resources,
    tree_filter: micro_config::TreeFilter,
    /// Show only the newest entry when the changelog is asked for.
    collapse_changelog: bool,
    /// How hard the model is being asked to reason, so a model swap keeps it.
    thinking: micro_types::ThinkingLevel,
    /// The extension host, so a command an extension registered can be run.
    extensions: Option<Arc<micro_extensions::Host>>,
    /// How a phone reaches the interface, once this session has been handed to one.
    seam: crate::remote::Seam,
    /// Where the run is copied to while a phone is watching.
    mirror: crate::remote::Mirror,
    /// What the phone is told about the session.
    snapshot: Arc<Mutex<crate::remote::Snapshot>>,
    /// Whether a phone already has this session, so a second `/remote` says so
    /// rather than opening a second connection.
    remote_started: bool,
    /// Warn that a subscription credential bills per token here. Said once a run, as ohm
    /// says it: repeating it every model swap would train the reader to skip it.
    anthropic_extra_usage: bool,
    warned_about_extra_usage: bool,
    /// The user's own prompt files, which become commands named after them.
    prompts: Vec<micro_prompts::PromptTemplate>,
    /// Whether the line just dispatched asked to step to the neighboring model rather than
    /// naming one — set in `dispatch`, read and reset in `swap_to`, since that is where
    /// ohm's `model_select` needs to say `"cycle"` instead of `"set"`.
    model_source: &'static str,
    /// Every tool this run actually offers the model, so `/reload` can tell which extension
    /// tools are still owed their line in the system prompt and which were left out by
    /// `--tools` or `--exclude-tools`.
    tool_names: Vec<String>,
}

/// Everything a host is built from. Gathered into one value because a run assembles all
/// of it in one place, and a constructor with nine positional arguments is a place to make
/// a mistake.
pub struct HostParts {
    pub catalog: Catalog,
    pub auth: Arc<AuthStore>,
    pub sessions: SessionStore,
    pub workspace: PathBuf,
    pub provider: String,
    pub model: ModelDef,
    pub session: Arc<Mutex<Session>>,
    pub session_id: String,
    pub home: PathBuf,
    pub skills_enabled: bool,
    /// The models the workspace put on its shortlist, which the model list opens on.
    pub scoped_models: Vec<String>,
    pub resources: crate::runtime::Resources,
    pub tree_filter: micro_config::TreeFilter,
    pub collapse_changelog: bool,
    pub thinking: micro_types::ThinkingLevel,
    pub anthropic_extra_usage: bool,
    pub extensions: Option<Arc<micro_extensions::Host>>,
    pub prompts: Vec<micro_prompts::PromptTemplate>,
    pub tool_names: Vec<String>,
    /// How a phone reaches the interface, once this session has been handed to one.
    pub seam: crate::remote::Seam,
    /// Where the run is copied to while a phone is watching.
    pub mirror: crate::remote::Mirror,
    /// What the phone is told about the session, kept current as it changes.
    pub snapshot: std::sync::Arc<Mutex<crate::remote::Snapshot>>,
}

impl CliCommands {
    pub fn new(parts: HostParts) -> Self {
        CliCommands {
            catalog: parts.catalog,
            auth: parts.auth,
            sessions: parts.sessions,
            workspace: parts.workspace,
            provider: parts.provider,
            model: parts.model,
            session: parts.session,
            session_id: parts.session_id,
            home: parts.home,
            skills_enabled: parts.skills_enabled,
            scoped_models: parts.scoped_models,
            resources: parts.resources,
            tree_filter: parts.tree_filter,
            collapse_changelog: parts.collapse_changelog,
            thinking: parts.thinking,
            extensions: parts.extensions,
            anthropic_extra_usage: parts.anthropic_extra_usage,
            warned_about_extra_usage: false,
            prompts: parts.prompts,
            seam: parts.seam,
            mirror: parts.mirror,
            snapshot: parts.snapshot,
            remote_started: false,
            model_source: "set",
            tool_names: parts.tool_names,
        }
    }

    /// Resolve a model into everything the agent needs to run it: a client for its provider
    /// and a credential to reach it.
    ///
    /// This is the half a command cannot do by itself. The interface applies the result,
    /// because the agent is its, but only here are the catalog and the credential store.
    /// Write the chosen model to the settings, so the next run starts on it.
    fn remember_model(&self, model: &ModelDef) -> Result<(), String> {
        let path = self.home.join(micro_config::FILE_NAME);
        let mut config = micro_config::Config::load_from(&path)
            .map_err(|error| format!("cannot read the settings: {error}"))?;
        config.model = Some(model.qualified_id());
        config.provider = Some(model.provider.clone());
        config
            .save_to(&path)
            .map_err(|error| format!("cannot write the settings: {error}"))
    }

    /// Write the chosen thinking level to the settings, so the next run uses it.
    fn remember_thinking(&self, level: micro_types::ThinkingLevel) -> Result<(), String> {
        let path = self.home.join(micro_config::FILE_NAME);
        let mut config = micro_config::Config::load_from(&path)
            .map_err(|error| format!("cannot read the settings: {error}"))?;
        config.thinking = Some(match level {
            micro_types::ThinkingLevel::Off => micro_config::Thinking::Off,
            micro_types::ThinkingLevel::Minimal => micro_config::Thinking::Minimal,
            micro_types::ThinkingLevel::Low => micro_config::Thinking::Low,
            micro_types::ThinkingLevel::Medium => micro_config::Thinking::Medium,
            micro_types::ThinkingLevel::High => micro_config::Thinking::High,
            micro_types::ThinkingLevel::XHigh => micro_config::Thinking::XHigh,
            micro_types::ThinkingLevel::Max => micro_config::Thinking::Max,
        });
        config
            .save_to(&path)
            .map_err(|error| format!("cannot write the settings: {error}"))
    }

    /// Run one of the user's own prompt files, if the line names one.
    fn prompt_command(&self, line: &str) -> Option<CommandOutcome> {
        let (name, arguments) = command_parts(line)?;
        let template = self.prompts.iter().find(|template| template.name == name)?;
        Some(CommandOutcome::Send {
            prompt: template.render(arguments),
        })
    }

    /// What a sign-in leaves behind, however the credential was collected.
    ///
    /// A credential for the service already in use reaches the running agent now. Waiting
    /// for a restart to pick it up would make signing in look like it failed.
    async fn signed_in(&mut self, provider: &str) -> Applied {
        if micro_auth::canonical_provider(provider)
            == micro_auth::canonical_provider(&self.model.provider)
        {
            let model = self.model.clone();
            return match self.swap_to(&model).await {
                Applied::Model { swap, .. } => Applied::Model {
                    swap,
                    note: Some(format!("Signed in to {provider}.")),
                },
                other => other,
            };
        }

        Applied::note(format!(
            "Signed in to {provider}. Run `/model` to use one of its models."
        ))
    }

    async fn swap_to(&mut self, model: &ModelDef) -> Applied {
        let resolved = match micro_provider::resolve(&self.auth, model).await {
            Ok(resolved) => resolved,
            Err(error) => {
                return Applied::error(format!(
                    "Cannot use {}: {error}. Try `micro auth login {}`.",
                    model.qualified_id(),
                    model.provider
                ))
            }
        };

        if resolved.api_key.is_blank() {
            return Applied::error(format!(
                "No credential for {}. Run `micro auth login {}`.",
                model.provider, model.provider
            ));
        }

        // Read before they are overwritten below, and the source is consumed rather than
        // merely read: the line that led here is spent, and a later swap it did not ask
        // for — reapplying the model already in use after signing in, say — starts fresh.
        let previous_model = self.model.clone();
        let source = std::mem::replace(&mut self.model_source, "set");

        // Kept in step so the next command reports the model that is actually running.
        self.provider = model.provider.clone();
        self.model = model.clone();
        let session_model = model.qualified_id();
        if let Err(error) = self.session.lock().await.set_model_id(session_model).await {
            return Applied::error(format!("Could not update the session model: {error}"));
        }
        // And remembered, so the next run starts on it. Choosing a model is a decision
        // about how to work, not about this conversation.
        let remembered = self.remember_model(model);
        // Ohm skips this event outright when the model did not actually change; the same
        // guard applies here, since reapplying the model already in use is not a selection
        // to report.
        if previous_model.qualified_id() != model.qualified_id() {
            crate::extensions::announce(
                self.extensions.as_ref(),
                "model_select",
                serde_json::json!({
                    "model": model_json(model),
                    "previousModel": model_json(&previous_model),
                    "source": source,
                }),
            )
            .await;
        }

        let mut note = match self.subscription_warning(resolved.api_key.as_str(), &model.provider) {
            Some(warning) => format!("Model: {}\n{warning}", model.qualified_id()),
            None => format!("Model: {}", model.qualified_id()),
        };
        if let Err(error) = remembered {
            note.push_str(&format!("\nIt was not remembered for next time: {error}"));
        }

        Applied::Model {
            swap: Box::new(micro_agent::ModelSwap {
                provider: resolved.client,
                // The effort the user chose belongs to them, not to the model they were
                // using when they chose it.
                model: crate::runtime::with_host(
                    model.to_runtime(self.thinking),
                    resolved.base_url.as_deref(),
                ),
                api_key: resolved.api_key,
                context_window: model.context_window as usize,
            }),
            note: Some(note),
        }
    }

    /// Anthropic's subscription credentials are OAuth tokens, and a third-party harness
    /// spending one is billed per token rather than against the plan. Said once.
    fn subscription_warning(&mut self, api_key: &str, provider: &str) -> Option<String> {
        if !self.anthropic_extra_usage || self.warned_about_extra_usage {
            return None;
        }
        if micro_auth::canonical_provider(provider) != "anthropic"
            || !api_key.starts_with(ANTHROPIC_OAUTH_PREFIX)
        {
            return None;
        }
        self.warned_about_extra_usage = true;
        Some(ANTHROPIC_SUBSCRIPTION_AUTH_WARNING.to_string())
    }

    fn context(&self, state: ConversationState) -> CommandContext<'_> {
        CommandContext {
            catalog: &self.catalog,
            auth: &self.auth,
            sessions: &self.sessions,
            workspace: &self.workspace,
            provider: &self.provider,
            model: Some(&self.model),
            session_id: Some(&self.session_id),
            message_count: state.message_count,
            usage: state.usage,
            collapse_changelog: self.collapse_changelog,
            scoped_models: &self.scoped_models,
            tree_filter: self.tree_filter,
        }
    }

    /// Every branch entry on the current path, in path order — the closest micro's tree
    /// comes to the `SessionEntry[]` ohm's compaction events carry. What compaction
    /// actually replaces is computed well below here, inside the agent loop, so the
    /// richer `preparation` object ohm builds ahead of time (with token counts and the
    /// exact stretch chosen) is not available to build from at this hook.
    async fn branch_entries(&self) -> Vec<serde_json::Value> {
        let session = self.session.lock().await;
        let tree = session.tree();
        let mut entries: Vec<(usize, &micro_session::Entry)> = tree
            .entries()
            .iter()
            .filter_map(|entry| tree.position_on_path(&entry.id).map(|position| (position, entry)))
            .collect();
        entries.sort_by_key(|(position, _)| *position);
        entries
            .into_iter()
            .map(|(_, entry)| {
                serde_json::json!({ "id": entry.id, "message": micro_extensions::message_json(&entry.message) })
            })
            .collect()
    }

    /// Continue the open conversation from an earlier entry.
    ///
    /// Nothing is deleted: what came after stays in the log as another branch, and the
    /// next message appended hangs off the entry that was chosen.
    async fn branch(&mut self, entry_id: &str) -> Applied {
        let old_leaf_id = self.session.lock().await.tree().head().map(str::to_string);
        if crate::extensions::cancelled(
            self.extensions.as_ref(),
            "session_before_tree",
            serde_json::json!({
                "preparation": {
                    "targetId": entry_id,
                    "oldLeafId": old_leaf_id,
                    // `/tree` only ever moves to an entry already on the current path, so
                    // that entry is its own common ancestor with the leaf it is leaving —
                    // unlike ohm's `navigateTree`, which can also land on an unrelated
                    // branch, this one never needs to search for where two branches meet.
                    "commonAncestorId": entry_id,
                    "entriesToSummarize": [],
                    // micro's `/tree` does not summarize the branch it leaves.
                    "userWantsSummary": false,
                },
            }),
        )
        .await
        {
            return Applied::note("An extension stopped the move");
        }

        let mut session = self.session.lock().await;
        if session.tree().head() == Some(entry_id) {
            return Applied::note("Already at this point");
        }
        if !session.branch_from(entry_id) {
            return Applied::error(format!(
                "There is no entry {entry_id} in this conversation. Run /tree to list them."
            ));
        }

        crate::extensions::announce(
            self.extensions.as_ref(),
            "session_tree",
            serde_json::json!({
                "newLeafId": session.tree().head(),
                "oldLeafId": old_leaf_id,
            }),
        )
        .await;

        // The branch is what the model is shown from here on, and what the scrollback is
        // rebuilt from, so the two never disagree.
        Applied::Conversation {
            messages: session.branch(),
            note: Some("Navigated to selected point".to_string()),
        }
    }

    /// Reopen another session in place of this one.
    ///
    /// The open session is swapped for the one that was chosen, so the task writing the
    /// log follows: from here on, what is said is appended to the reopened conversation
    /// rather than to the one that was left.
    async fn resume(&mut self, session_id: &str) -> Applied {
        // micro addresses a session by its id rather than by the file ohm would name here;
        // the id is what an extension can act on regardless — asking to resume it again,
        // say — so it stands in for `targetSessionFile`.
        if crate::extensions::cancelled(
            self.extensions.as_ref(),
            "session_before_switch",
            serde_json::json!({ "reason": "resume", "targetSessionFile": session_id }),
        )
        .await
        {
            return Applied::note("An extension stopped the switch");
        }

        let loaded = match self.sessions.load(session_id).await {
            Ok(loaded) => loaded,
            Err(error) => {
                return Applied::error(format!("Cannot open session {session_id}: {error}"))
            }
        };

        let messages = loaded.messages;
        let previous_session_file = self.session.lock().await.path().display().to_string();
        self.session_id = loaded.session.id().to_string();
        *self.session.lock().await = loaded.session;
        crate::extensions::announce(
            self.extensions.as_ref(),
            "session_start",
            serde_json::json!({ "reason": "resume", "previousSessionFile": previous_session_file }),
        )
        .await;

        Applied::Conversation {
            messages,
            note: Some("Resumed session".to_string()),
        }
    }

    /// Start over: a fresh session, and a conversation with nothing in it.
    ///
    /// The conversation that was left is not touched. It stays on disk under its own id,
    /// which is what makes starting over cheap.
    async fn start_new_session(&mut self) -> Applied {
        if crate::extensions::cancelled(
            self.extensions.as_ref(),
            "session_before_switch",
            serde_json::json!({ "reason": "new" }),
        )
        .await
        {
            return Applied::note("An extension stopped the switch");
        }

        let started = self
            .sessions
            .create(&self.workspace, self.model.qualified_id())
            .await;
        match started {
            Ok(session) => {
                let previous_session_file = self.session.lock().await.path().display().to_string();
                self.session_id = session.id().to_string();
                *self.session.lock().await = session;
                crate::extensions::announce(
                    self.extensions.as_ref(),
                    "session_start",
                    serde_json::json!({ "reason": "new", "previousSessionFile": previous_session_file }),
                )
                .await;
                Applied::Conversation {
                    messages: Vec::new(),
                    note: Some("New session started".to_string()),
                }
            }
            Err(error) => Applied::error(format!("Could not start a new session: {error}")),
        }
    }

    /// Bring in a session log written elsewhere and carry on from it.
    ///
    /// The imported file is copied, not adopted: it is left exactly as it was, and what is
    /// said from here on is written to a session of micro's own.
    async fn import(&mut self, path: &str) -> Applied {
        let source = match std::path::Path::new(path).is_absolute() {
            true => std::path::PathBuf::from(path),
            false => self.workspace.join(path),
        };

        let imported = match self
            .sessions
            .import(&source, &self.workspace, self.model.qualified_id())
            .await
        {
            Ok(imported) => imported,
            Err(error) => return Applied::error(format!("Failed to import session: {error}")),
        };

        let mut note = format!("Session imported from: {}", source.display());
        if imported.skipped_lines > 0 {
            note.push_str(&format!(
                " ({} line(s) could not be read and were left out)",
                imported.skipped_lines
            ));
        }

        let messages = imported.messages;
        self.session_id = imported.session.id().to_string();
        *self.session.lock().await = imported.session;

        Applied::Conversation {
            messages,
            note: Some(note),
        }
    }

    /// Put this session on the paired phone, or bond a phone to this machine.
    ///
    /// Pairing is the one-off, and the only thing that shows a link. Publishing shows
    /// nothing worth reading: the session appears in the app's list, which is where
    /// someone reaching for their phone is already looking.
    async fn remote(&mut self, action: micro_commands::RemoteAction) -> Applied {
        if let micro_commands::RemoteAction::Pair { qr } = action {
            return match crate::remote::pair(&self.home, qr).await {
                Ok(lines) => Applied::note(lines.join("\n")),
                Err(error) => Applied::error(format!("Could not pair a phone: {error}")),
            };
        }

        if !crate::remote::is_paired(&self.home) {
            return Applied::warning(
                "No phone is paired with this machine yet. Run /remote pair to bond one.",
            );
        }
        if self.remote_started {
            return Applied::note("This session is already on your phone.");
        }
        let models = self
            .catalog
            .models()
            .iter()
            .map(|model| micro_remote::AvailableModel {
                id: model.qualified_id(),
                name: model.name.clone(),
                provider: model.provider.clone(),
            })
            .collect();

        // The session's own name is what the phone lists it under, so it is read now
        // rather than left as the id it was seeded with.
        if let Ok(session) = self.session.try_lock() {
            let title = session.meta().title.clone();
            if !title.is_empty() {
                self.snapshot.lock().await.session_name = title;
            }
        }

        let started = crate::remote::start(
            &self.seam,
            &self.mirror,
            Arc::clone(&self.session),
            Arc::clone(&self.snapshot),
            self.session_id.clone(),
            models,
            &self.home,
        )
        .await;

        match started {
            Ok(()) => {
                self.remote_started = true;
                Applied::note("This session is on your phone.")
            }
            Err(error) => Applied::error(format!("Could not put this session on your phone: {error}")),
        }
    }

    /// Publish the conversation as a secret gist and say where it went.
    async fn share(&self) -> Applied {
        let Some(token) = crate::share::token() else {
            return Applied::error(format!(
                "No GitHub token. Set {} to one with the `gist` scope.",
                crate::share::TOKEN_VARIABLES.join(" or ")
            ));
        };

        let loaded = match self.sessions.load(&self.session_id).await {
            Ok(loaded) => loaded,
            Err(error) => return Applied::error(format!("Cannot read the session: {error}")),
        };
        if loaded.messages.is_empty() {
            return Applied::error("Nothing said yet.");
        }

        let title = match loaded.session.meta().title.is_empty() {
            true => self.session_id.clone(),
            false => loaded.session.meta().title.clone(),
        };

        match crate::share::publish(&title, &loaded.messages, &token).await {
            Ok(url) => Applied::note(format!("Gist: {url}")),
            Err(error) => Applied::error(format!("Failed to create gist: {error}")),
        }
    }

    /// Read the instruction files and skills again, and tell the model what they say now.
    ///
    /// Only the standing instructions change. The conversation is left exactly as it is,
    /// because nothing that was said stopped being true.
    async fn reload(&self) -> Applied {
        let trusted = !micro_config::requires_decision(&self.workspace)
            || micro_config::TrustStore::load_from(&self.home)
                .await
                .unwrap_or_default()
                .is_trusted(&self.workspace);
        let context = crate::runtime::load_context(
            &self.workspace,
            self.skills_enabled,
            trusted,
            &self.resources,
            self.extensions.as_deref(),
            &self.tool_names,
            "reload",
        )
        .await;

        let mut note = format!(
            "Reloaded {} and {}.",
            counted(context.instruction_files.len(), "context file"),
            counted(context.skills.len(), "skill")
        );
        for diagnostic in &context.diagnostics {
            note.push('\n');
            note.push_str(diagnostic);
        }

        Applied::SystemPrompt {
            prompt: context.system_prompt,
            note: Some(note),
        }
    }

    /// Run a command an extension registered, if this line names one.
    ///
    /// What the extension returns is shown as it comes back: a string is the answer, and
    /// anything else is described rather than dropped.
    async fn extension_command(&mut self, line: &str) -> Option<CommandOutcome> {
        let (name, arguments) = command_parts(line)?;
        let host = self.extensions.clone()?;
        if !host.commands().iter().any(|command| command.name == name) {
            return None;
        }

        Some(match host.call_command(name, arguments).await {
            Ok(serde_json::Value::Null) => CommandOutcome::info(format!("/{name} ran.")),
            Ok(serde_json::Value::String(said)) => CommandOutcome::info(said),
            Ok(other) => CommandOutcome::info(other.to_string()),
            Err(error) => CommandOutcome::error(error),
        })
    }

    /// Remember what was decided about this project.
    ///
    /// The decision is read when a run starts, so it takes effect from the next one: the
    /// policy this run is enforcing was settled before the first tool call.
    async fn trust(&self, trusted: bool) -> Applied {
        let mut store = match micro_config::TrustStore::load_from(&self.home).await {
            Ok(store) => store,
            Err(error) => return Applied::error(format!("Cannot read the trust store: {error}")),
        };
        store.decide(&self.workspace, trusted);
        if let Err(error) = store.save_to(&self.home).await {
            return Applied::error(format!("Could not save the decision: {error}"));
        }

        crate::extensions::announce(
            self.extensions.as_ref(),
            "project_trust",
            serde_json::json!({
                "path": self.workspace.display().to_string(),
                "decision": match trusted { true => "yes", false => "no" },
            }),
        )
        .await;

        Applied::note(format!(
            "Saved trust decision: {}. Restart micro for this to take effect.",
            match trusted {
                true => "trusted",
                false => "untrusted",
            }
        ))
    }

    /// Give the session a title of its own, in place of the derived one.
    async fn rename(&mut self, title: &str) -> Applied {
        match self.session.lock().await.rename(title).await {
            Ok(()) => {
                crate::extensions::announce(
                    self.extensions.as_ref(),
                    "session_info_changed",
                    serde_json::json!({ "name": title }),
                )
                .await;
                Applied::note(format!("Session name set: {title}"))
            }
            Err(error) => Applied::error(format!("Could not rename the session: {error}")),
        }
    }

    /// Copy the conversation up to a point into a session of its own, and carry on in the
    /// copy. The session it came from is left exactly as it was.
    async fn fork(&mut self, session_id: &str, through_index: usize, whole: bool) -> Applied {
        // Ohm addresses a fork by the tree entry to fork from; micro's own `/fork` takes a
        // position along the path instead, so the entry id ohm's event carries is looked
        // up from that position rather than being micro's own indexing. `/fork` keeps
        // everything up to and including that entry, which is what ohm calls "at" rather
        // than "before".
        let entry_id = {
            let session = self.session.lock().await;
            let tree = session.tree();
            tree.entries()
                .iter()
                .find(|entry| tree.position_on_path(&entry.id) == Some(through_index))
                .map(|entry| entry.id.clone())
                .unwrap_or_else(|| through_index.to_string())
        };
        // Asked before the copy is made, so refusing it leaves the session untouched
        // rather than reporting a fork that has already happened.
        if crate::extensions::cancelled(
            self.extensions.as_ref(),
            "session_before_fork",
            serde_json::json!({ "entryId": entry_id, "position": "at" }),
        )
        .await
        {
            return Applied::note("An extension stopped the fork");
        }

        let forked = match self.sessions.fork(session_id, through_index).await {
            Ok(forked) => forked,
            Err(error) => {
                return Applied::error(format!("Could not branch the conversation: {error}"))
            }
        };

        let messages = forked.branch();
        // A fork is a fresh session file, the same as `/new` and `/resume` are — which is
        // what ohm reports it as too: there is no separate "a fork happened" event, only
        // `session_start` with `reason: "fork"`.
        let previous_session_file = self.session.lock().await.path().display().to_string();
        self.session_id = forked.id().to_string();
        *self.session.lock().await = forked;
        crate::extensions::announce(
            self.extensions.as_ref(),
            "session_start",
            serde_json::json!({ "reason": "fork", "previousSessionFile": previous_session_file }),
        )
        .await;

        Applied::Conversation {
            messages,
            note: Some(
                match whole {
                    true => "Cloned to new session",
                    false => "Forked to new session",
                }
                .to_string(),
            ),
        }
    }
}

#[async_trait]
impl Commands for CliCommands {
    /// What the user typed, before anything is done with it. An extension may rewrite it,
    /// or swallow it by answering that it handled it.
    ///
    /// Ohm's three input sources are interactive typing, the RPC transport, and an
    /// extension sending itself a message; RPC input never reaches this method at all — it
    /// is answered by `micro-rpc`, a separate pump that does not go through `Commands` —
    /// so what arrives here, print-mode's one-shot prompt included, is always reported as
    /// `"interactive"`: neither of the other two.
    async fn submitted(&mut self, line: String) -> Option<String> {
        let answers = crate::extensions::consult(
            self.extensions.as_ref(),
            "input",
            serde_json::json!({ "text": line, "source": "interactive" }),
        )
        .await;

        let mut line = line;
        for answer in answers {
            match answer.get("action").and_then(serde_json::Value::as_str) {
                Some("handled") => return None,
                Some("transform") => {
                    if let Some(text) = answer.get("text").and_then(serde_json::Value::as_str) {
                        line = text.to_string();
                    }
                }
                // `"continue"`, or anything else — an extension that didn't answer ohm's
                // shape at all — changes nothing.
                _ => {}
            }
        }
        Some(line)
    }

    async fn shortcut(&mut self, key: &str) -> bool {
        let Some(host) = self.extensions.clone() else {
            return false;
        };
        let bound = host.loaded().extensions.iter().any(|extension| {
            extension
                .shortcuts
                .iter()
                .any(|shortcut| shortcut.key == key)
        });
        if !bound {
            return false;
        }

        // A shortcut is a command with no name: whoever registered it decides what it does.
        let _ = host
            .ask_event("shortcut", serde_json::json!({ "key": key }))
            .await;
        true
    }

    async fn thinking_changed(&mut self, level: micro_types::ThinkingLevel) {
        let previous_level = self.thinking;
        self.thinking = level;
        if let Err(error) = self.remember_thinking(level) {
            eprintln!("note: thinking level was not remembered for next time: {error}");
        }
        crate::extensions::announce(
            self.extensions.as_ref(),
            "thinking_level_select",
            serde_json::json!({
                "level": format!("{level:?}").to_lowercase(),
                "previousLevel": format!("{previous_level:?}").to_lowercase(),
            }),
        )
        .await;
    }

    async fn compacting(&mut self) -> bool {
        // micro's own auto-compaction — triggered by the context threshold or by overflow
        // recovery — runs inside the agent loop, which has no extension hook of its own
        // for compaction at all. Only `/compact`, dispatched through here, can be asked
        // about, so `reason` is always `"manual"` and `willRetry` is always `false`.
        !crate::extensions::cancelled(
            self.extensions.as_ref(),
            "session_before_compact",
            serde_json::json!({
                "branchEntries": self.branch_entries().await,
                "reason": "manual",
                "willRetry": false,
            }),
        )
        .await
    }

    async fn compacted(&mut self, summary: &str) {
        // `compacted` is handed only the finished summary text; the richer
        // `CompactionEntry` ohm reports — its own id, the first entry still kept, when it
        // happened — is not, so `compactionEntry` here carries just the part that is.
        crate::extensions::announce(
            self.extensions.as_ref(),
            "session_compact",
            serde_json::json!({
                "compactionEntry": { "summary": summary },
                "fromExtension": false,
                "reason": "manual",
                "willRetry": false,
            }),
        )
        .await;
    }

    async fn before_bash(
        &mut self,
        command: &str,
        exclude_from_context: bool,
        cwd: &str,
    ) -> Option<micro_tui::BashRun> {
        let answers = crate::extensions::consult(
            self.extensions.as_ref(),
            "user_bash",
            serde_json::json!({
                "command": command,
                "excludeFromContext": exclude_from_context,
                "cwd": cwd,
            }),
        )
        .await;

        // The first extension to answer with anything at all decides, the same way ohm's
        // own runner stops at the first `user_bash` handler that returns something.
        // `operations` — a custom execution strategy — has nowhere to plug in here: `!`
        // always shells out directly, unlike the `bash` tool, which is built around a
        // swappable executor. Only a full `result` is honoured; an answer that sets only
        // `operations` is skipped, and the command runs as it would have.
        answers.iter().find_map(|answer| {
            let result = answer.get("result")?;
            let output = result
                .get("output")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let failed = result.get("cancelled").and_then(serde_json::Value::as_bool).unwrap_or(false)
                || result
                    .get("exitCode")
                    .and_then(serde_json::Value::as_i64)
                    .is_some_and(|code| code != 0);
            Some(micro_tui::BashRun { output, failed })
        })
    }

    /// What a submitted line means, in the order the names are claimed.
    ///
    /// Built-in commands are matched first and cannot be taken over: a name micro answers
    /// to has to keep answering, or an installed extension could quietly replace `/quit`.
    /// Then the user's own prompt files, then whatever the extensions registered.
    fn begin_model_refresh(
        &mut self,
    ) -> Option<tokio::sync::oneshot::Receiver<micro_tui::Listings>> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // Copilot lists what an account may reach only when asked with its own token, so
        // the credential is read here where the store is, not in the task.
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            // Opened again rather than shared: the store is a file, and the task outlives
            // the borrow that started it. The token says which host serves this account;
            // only an individual plan is served by the default one.
            let copilot = match micro_auth::AuthStore::open() {
                Ok(store) => {
                    store
                        .resolve(micro_auth::GITHUB_COPILOT)
                        .await
                        .ok()
                        .map(|credential| {
                            let token = credential.token().to_string();
                            let base = micro_auth::copilot::base_url_from_token(&token)
                                .unwrap_or_else(|| micro_models::COPILOT_BASE_URL.to_string());
                            (token, base)
                        })
                }
                Err(_) => None,
            };
            let mut listings = micro_tui::Listings::default();
            match micro_models::fetch_openrouter(&client).await {
                Ok(models) => listings.models.extend(models),
                Err(error) => listings.errors.push(error.to_string()),
            }
            if let Some((token, base)) = &copilot {
                match micro_models::fetch_copilot(&client, token, base).await {
                    Ok(models) => listings.models.extend(models),
                    Err(error) => listings.errors.push(error.to_string()),
                }
            }
            let _ = sender.send(listings);
        });
        Some(receiver)
    }

    async fn apply_model_refresh(&mut self, listings: micro_tui::Listings) -> Option<Picker> {
        if !listings.models.is_empty() {
            self.catalog.merge_listing(listings.models);
        }
        // The list is rebuilt the way it was built in the first place, so a model that has
        // just appeared is in it and everything else reads exactly as it did.
        let context = self.context(ConversationState::default());
        match micro_commands::dispatch("/model", &context).await {
            Some(CommandOutcome::Choose(picker)) => Some(picker),
            _ => None,
        }
    }

    async fn dispatch(&mut self, line: &str, state: ConversationState) -> Option<CommandOutcome> {
        // `/model next|previous` steps to a neighbor rather than naming one, which is what
        // ohm's `model_select` calls a `"cycle"` rather than a `"set"`. Read here, from the
        // line itself, because by the time a `CommandOutcome::SetModel` reaches `swap_to`
        // the two look identical.
        self.model_source = match command_parts(line) {
            Some(("model", argument)) if matches!(argument.trim(), "next" | "previous" | "prev") => "cycle",
            _ => "set",
        };

        // A name micro answers to is answered by micro. Trying the extensions first would
        // let an installed one quietly take over `/quit`.
        let claimed =
            command_parts(line).is_some_and(|(name, _)| micro_commands::find(name).is_some());
        if claimed {
            return micro_commands::dispatch(line, &self.context(state)).await;
        }
        if let Some(outcome) = self.prompt_command(line) {
            return Some(outcome);
        }
        if let Some(outcome) = self.extension_command(line).await {
            return Some(outcome);
        }
        // Nobody claimed it. What comes back is either the unknown-command message or
        // nothing at all, for a line that was never a command.
        micro_commands::dispatch(line, &self.context(state)).await
    }

    async fn apply(&mut self, outcome: CommandOutcome) -> Applied {
        match outcome {
            // A device login is entirely this side of the seam: poll GitHub, store what it
            // returns. Nothing about the running agent changes.
            CommandOutcome::Fork {
                session_id,
                through_index,
                whole,
            } => self.fork(&session_id, through_index, whole).await,

            // Branching happens in the session that is open, so the conversation the
            // interface holds is replaced by the branch that was chosen.
            CommandOutcome::Branch { entry_id } => self.branch(&entry_id).await,

            CommandOutcome::Rename { title } => self.rename(&title).await,

            CommandOutcome::Trust { trusted } => self.trust(trusted).await,

            CommandOutcome::Reload => self.reload().await,

            CommandOutcome::Import { path } => self.import(&path).await,

            CommandOutcome::Share => self.share().await,

            CommandOutcome::RemoteControl { action } => self.remote(action).await,

            CommandOutcome::SetModel { model } => self.swap_to(&model).await,

            // Naming a provider means running its first model, since a provider on its own
            // is not something the agent can be pointed at.
            CommandOutcome::SetProvider { provider } => {
                let canonical = micro_auth::canonical_provider(provider).to_string();
                match self
                    .catalog
                    .models()
                    .iter()
                    .find(|model| model.provider == canonical)
                    .cloned()
                {
                    Some(model) => self.swap_to(&model).await,
                    None => Applied::error(format!("No models are known for {provider}.")),
                }
            }

            CommandOutcome::Resume { session_id } => self.resume(&session_id).await,

            CommandOutcome::Clear => self.start_new_session().await,

            // The interface carries these out itself; reaching here means it did not.
            other => Applied::error(format!("Nothing here knows how to carry out {other:?}.")),
        }
    }

    async fn finish_device_login(
        &mut self,
        pending: Box<micro_auth::PendingDeviceLogin>,
    ) -> Applied {
        let provider = pending.provider.clone();
        if let Err(error) = self.auth.complete_device_login(&pending).await {
            return Applied::error(format!("Sign-in failed: {error}"));
        }
        self.signed_in(&provider).await
    }

    async fn store_api_key(&mut self, provider: String, key: String) -> Applied {
        if let Err(error) = self.auth.store_api_key(&provider, &key) {
            return Applied::error(error.to_string());
        }
        self.signed_in(&provider).await
    }
}

/// What an Anthropic subscription credential looks like. The plan's own tokens are OAuth
/// tokens, and they carry this prefix wherever they are stored.
const ANTHROPIC_OAUTH_PREFIX: &str = "sk-ant-oat";

/// Said when a subscription credential is used from here, in ohm's words.
const ANTHROPIC_SUBSCRIPTION_AUTH_WARNING: &str =
    "Anthropic subscription auth is active. Third-party harness usage draws from extra \
     usage and is billed per token, not your Claude plan limits. Manage extra usage at \
     https://claude.ai/settings/usage. Disable this warning in /settings.";

/// `1 skill` but `2 skills`, so a count reads as a sentence.
fn counted(count: usize, thing: &str) -> String {
    match count {
        1 => format!("1 {thing}"),
        other => format!("{other} {thing}s"),
    }
}

/// A model the way `model_select` and `get_model` both describe one to an extension.
///
/// Kept to these few fields rather than ohm's full `Model<any>` — thinking-level mapping,
/// per-model headers, cost tiers — because that is all a `ModelDef` itself carries; an
/// extension reading further than this is reading past what micro tracked.
fn model_json(model: &ModelDef) -> serde_json::Value {
    serde_json::json!({
        "id": model.id,
        "name": model.name,
        "provider": model.provider,
        "contextWindow": model.context_window,
        "maxOutputTokens": model.max_output_tokens,
        "reasoning": model.reasoning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::Message;
    use std::sync::atomic::AtomicU32;
    use std::sync::atomic::Ordering;

    /// A host rooted in this process's own scratch directory, so no test reads or writes a
    /// real credential file or session log.
    async fn host(label: &str) -> (CliCommands, PathBuf) {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "micro-cli-commands-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let catalog = Catalog::bundled();
        let model = catalog
            .resolve("anthropic/claude-opus-5")
            .model()
            .expect("the bundled catalog carries this model")
            .clone();

        // A real session, because branching and renaming reach the log rather than a
        // name kept beside it.
        let sessions = SessionStore::new(root.join("sessions"));
        let session = sessions
            .create(&workspace, "anthropic/claude-opus-5")
            .await
            .unwrap();
        let session_id = session.id().to_string();

        let host = CliCommands::new(HostParts {
            catalog,
            auth: Arc::new(AuthStore::open_at(root.join("auth.json")).unwrap()),
            sessions,
            workspace,
            provider: "anthropic".to_string(),
            model,
            session: Arc::new(Mutex::new(session)),
            session_id,
            home: root.join("home"),
            skills_enabled: true,
            scoped_models: Vec::new(),
            resources: Default::default(),
            tree_filter: Default::default(),
            collapse_changelog: false,
            thinking: micro_types::ThinkingLevel::Off,
            extensions: None,
            anthropic_extra_usage: true,
            prompts: Vec::new(),
            tool_names: Vec::new(),
            // No phone is ever handed this session in a test, but the seam is built the
            // same way it is in a real run: the interface's half is simply dropped.
            seam: crate::remote::Seam::build().0,
            mirror: Default::default(),
            snapshot: Default::default(),
        });
        (host, root)
    }

    fn state(message_count: usize) -> ConversationState {
        ConversationState {
            message_count,
            usage: micro_types::Usage::default(),
        }
    }

    fn note(applied: &Applied) -> &str {
        match applied {
            Applied::Note { text, .. } => text,
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ordinary_text_is_left_for_the_model() {
        let (mut host, _root) = host("prompt").await;
        assert!(host.dispatch("explain this file", state(0)).await.is_none());
        assert!(host.dispatch("/usr/bin/env", state(0)).await.is_none());
    }

    #[tokio::test]
    async fn help_answers_without_reaching_the_model() {
        let (mut host, _root) = host("help").await;
        let outcome = host.dispatch("/help", state(0)).await.expect("a command");

        let text = outcome.text().expect("help is a message");
        assert!(text.contains("/model"), "{text}");
        assert!(text.contains("/quit"), "{text}");
    }

    #[tokio::test]
    async fn an_unknown_command_suggests_the_nearest_one() {
        let (mut host, _root) = host("unknown").await;
        let outcome = host.dispatch("/modl", state(0)).await.expect("a command");

        assert!(outcome.is_error());
        assert!(outcome.text().unwrap().contains("did you mean /model"));
    }

    /// The list offers what can actually answer, and marks what is running.
    #[tokio::test]
    async fn the_model_picker_offers_what_is_signed_in_and_marks_the_model_in_use() {
        let (mut host, _root) = host("model-picker").await;
        host.auth.store_api_key("anthropic", "sk-ant-test").unwrap();

        let outcome = host.dispatch("/model", state(0)).await.expect("a command");

        let CommandOutcome::Choose(picker) = outcome else {
            panic!("expected a picker");
        };
        let current: Vec<&str> = picker
            .items
            .iter()
            .filter(|item| item.current)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(current, vec!["claude-opus-5"]);

        // Everything offered is served by something there is a credential for. Which
        // providers those are depends on the environment the test runs in, so the
        // property is asserted rather than the list.
        for item in &picker.items {
            // The row names the model; the badge beside it names who serves it.
            let provider = item.detail.trim_matches(['[', ']']);
            assert!(
                host.auth.status_of(provider).is_authenticated(),
                "{} is offered without a credential",
                item.label
            );
        }
        assert!(
            picker.hint.is_some(),
            "the list should say what it leaves out"
        );
    }

    /// Choosing a model is a decision about how to work, so the next run starts on it.
    #[tokio::test]
    async fn switching_model_is_remembered_for_next_time() {
        let (mut host, root) = host("remember-model").await;
        host.auth.store_api_key("anthropic", "sk-ant-test").unwrap();

        let outcome = host
            .dispatch("/model anthropic/claude-sonnet-5", state(0))
            .await
            .expect("a command");
        let applied = host.apply(outcome).await;
        assert!(!applied.is_error(), "{applied:?}");

        let saved =
            micro_config::Config::load_from(root.join("home").join(micro_config::FILE_NAME))
                .expect("the settings were written");
        assert_eq!(saved.model.as_deref(), Some("anthropic/claude-sonnet-5"));
        assert_eq!(saved.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            host.session.lock().await.meta().model_id,
            "anthropic/claude-sonnet-5"
        );
    }

    /// Without a credential the swap cannot be built, and the report says which provider to
    /// sign in to. What it must never say is "restart" — switching model is something the
    /// running interface does now.
    #[tokio::test]
    async fn switching_model_without_a_credential_says_how_to_sign_in() {
        let (mut host, _root) = host("set-model").await;
        let outcome = host
            .dispatch("/model anthropic/claude-opus-5", state(0))
            .await
            .expect("a command");

        let applied = host.apply(outcome).await;
        assert!(applied.is_error(), "{applied:?}");
        assert!(note(&applied).contains("anthropic"), "{}", note(&applied));
        assert!(!note(&applied).contains("Restart"), "{}", note(&applied));
    }

    #[tokio::test]
    async fn switching_model_with_a_credential_hands_back_a_swap() {
        let (mut host, _root) = host("set-model-ok").await;
        host.auth
            .set("anthropic", micro_auth::Credential::api_key("sk-test"))
            .expect("stored");

        let outcome = host
            .dispatch("/model anthropic/claude-opus-5", state(0))
            .await
            .expect("a command");

        match host.apply(outcome).await {
            Applied::Model { swap, note } => {
                assert_eq!(swap.model.id, "claude-opus-5");
                assert_eq!(swap.provider.name(), "anthropic");
                assert!(swap.context_window > 0);
                assert!(note.unwrap_or_default().contains("claude-opus-5"));
            }
            other => panic!("expected a model swap, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn switching_provider_runs_one_of_its_models() {
        let (mut host, _root) = host("set-provider").await;
        host.auth
            .set("openrouter", micro_auth::Credential::api_key("sk-test"))
            .expect("stored");

        let outcome = host
            .dispatch("/provider openrouter", state(0))
            .await
            .expect("a command");

        match host.apply(outcome).await {
            Applied::Model { swap, .. } => assert_eq!(swap.provider.name(), "openrouter"),
            other => panic!("expected a model swap, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cycling_thinking_is_remembered_for_next_time() {
        let (mut host, root) = host("remember-thinking").await;

        host.thinking_changed(micro_types::ThinkingLevel::High)
            .await;

        let saved =
            micro_config::Config::load_from(root.join("home").join(micro_config::FILE_NAME))
                .expect("the settings were written");
        assert_eq!(saved.thinking, Some(micro_config::Thinking::High));
    }

    /// Starting over leaves the old conversation on disk under its own id and opens a
    /// session with nothing in it, which is what the writer appends to from then on.
    #[tokio::test]
    async fn clearing_opens_a_new_session() {
        let (mut host, _root) = host("clear").await;
        let first = host.session_id.clone();
        host.session
            .lock()
            .await
            .append(&Message::user("before"))
            .await
            .unwrap();

        match host.apply(CommandOutcome::Clear).await {
            Applied::Conversation { messages, note } => {
                assert!(messages.is_empty());
                assert_eq!(note.as_deref(), Some("New session started"));
            }
            other => panic!("expected a conversation, got {other:?}"),
        }

        assert_ne!(host.session_id, first, "a different session is open");
        assert_eq!(host.session.lock().await.id(), host.session_id);
        // What was said before is still there to go back to.
        let kept = host.sessions.load(&first).await.unwrap();
        assert_eq!(kept.messages.len(), 1);
    }

    /// Resuming swaps the open session, so the conversation the model sees and the log
    /// being written are the same one.
    #[tokio::test]
    async fn resuming_opens_the_chosen_session() {
        let (mut host, _root) = host("resume").await;
        let mut other = host
            .sessions
            .create(&host.workspace, "anthropic/claude-opus-5")
            .await
            .unwrap();
        other.append(&Message::user("said earlier")).await.unwrap();
        let other_id = other.id().to_string();
        drop(other);

        match host
            .apply(CommandOutcome::Resume {
                session_id: other_id.clone(),
            })
            .await
        {
            Applied::Conversation { messages, note } => {
                assert_eq!(messages.len(), 1);
                assert_eq!(note.as_deref(), Some("Resumed session"));
            }
            other => panic!("expected a conversation, got {other:?}"),
        }

        assert_eq!(host.session_id, other_id);
        assert_eq!(host.session.lock().await.id(), other_id);
    }

    #[tokio::test]
    async fn resuming_something_that_is_not_there_says_so() {
        let (mut host, _root) = host("resume-missing").await;
        let applied = host
            .apply(CommandOutcome::Resume {
                session_id: "20240101-000000-abcd".into(),
            })
            .await;
        assert!(applied.is_error(), "{applied:?}");
    }

    /// Branching moves where the next message hangs off, and hands back the conversation
    /// along the branch so the agent and the screen agree on what was said.
    #[tokio::test]
    async fn branching_continues_from_an_earlier_entry() {
        let (mut host, _root) = host("branch").await;
        {
            let mut session = host.session.lock().await;
            session.append(&Message::user("question")).await.unwrap();
            session.append(&Message::user("answer one")).await.unwrap();
        }

        match host
            .apply(CommandOutcome::Branch {
                entry_id: "1".into(),
            })
            .await
        {
            Applied::Conversation { messages, note } => {
                assert_eq!(messages.len(), 1, "back to just the question");
                assert_eq!(note.as_deref(), Some("Navigated to selected point"));
            }
            other => panic!("expected a conversation, got {other:?}"),
        }

        // The next message hangs off the branch, and the abandoned one is still there.
        host.session
            .lock()
            .await
            .append(&Message::user("answer two"))
            .await
            .unwrap();
        let session = host.session.lock().await;
        assert_eq!(session.branch().len(), 2);
        assert_eq!(session.tree().entries().len(), 3);
    }

    #[tokio::test]
    async fn branching_from_an_entry_that_is_not_there_says_so() {
        let (mut host, _root) = host("branch-missing").await;
        let applied = host
            .apply(CommandOutcome::Branch {
                entry_id: "17".into(),
            })
            .await;
        assert!(applied.is_error(), "{applied:?}");
    }

    #[tokio::test]
    async fn naming_the_session_sticks() {
        let (mut host, _root) = host("name").await;
        host.session
            .lock()
            .await
            .append(&Message::user("a question that would become the title"))
            .await
            .unwrap();

        let outcome = host
            .dispatch("/name the good one", state(1))
            .await
            .expect("a command");
        let applied = host.apply(outcome).await;
        assert_eq!(note(&applied), "Session name set: the good one");

        let meta = host.sessions.meta(&host.session_id).await.unwrap();
        assert_eq!(meta.title, "the good one");

        // Asking without an argument reports the name rather than setting an empty one.
        let outcome = host.dispatch("/name", state(1)).await.expect("a command");
        assert_eq!(outcome.text(), Some("Session name: the good one"));
    }

    #[tokio::test]
    async fn forking_carries_on_in_a_copy_and_leaves_the_original() {
        let (mut host, _root) = host("fork").await;
        let original = host.session_id.clone();
        {
            let mut session = host.session.lock().await;
            session.append(&Message::user("first")).await.unwrap();
            session.append(&Message::user("second")).await.unwrap();
        }

        let outcome = host.dispatch("/fork 0", state(2)).await.expect("a command");
        match host.apply(outcome).await {
            Applied::Conversation { messages, .. } => assert_eq!(messages.len(), 1),
            other => panic!("expected a conversation, got {other:?}"),
        }

        // The fork is a second session, and it is the one being written to now.
        assert_ne!(host.session_id, original);
        assert_eq!(host.sessions.list().await.unwrap().len(), 2);
        assert_eq!(
            host.sessions.load(&original).await.unwrap().messages.len(),
            2,
            "the session it came from is untouched"
        );
    }

    /// Cloning copies the whole conversation as it stands, which is a fork through its
    /// last message.
    #[tokio::test]
    async fn cloning_copies_the_conversation_as_it_stands() {
        let (mut host, _root) = host("clone").await;
        let original = host.session_id.clone();
        {
            let mut session = host.session.lock().await;
            session.append(&Message::user("first")).await.unwrap();
            session.append(&Message::user("second")).await.unwrap();
        }

        let outcome = host.dispatch("/clone", state(2)).await.expect("a command");
        match host.apply(outcome).await {
            Applied::Conversation { messages, .. } => assert_eq!(messages.len(), 2),
            other => panic!("expected a conversation, got {other:?}"),
        }
        assert_ne!(host.session_id, original);
    }

    /// Importing copies a log written elsewhere into a session of micro's own, and
    /// carries on in it.
    #[tokio::test]
    async fn importing_carries_on_from_the_imported_log() {
        let (mut host, root) = host("import").await;
        let original = host.session_id.clone();

        // A log in micro's own format, which is what /import is given.
        let source = root.join("elsewhere.jsonl");
        let written = format!(
            "{}\n{}\n",
            serde_json::to_string(&micro_session::Entry::new(
                "1",
                None,
                Message::user("said elsewhere")
            ))
            .unwrap(),
            "{ not a line }"
        );
        std::fs::write(&source, written).unwrap();

        let outcome = host
            .dispatch(&format!("/import {}", source.display()), state(0))
            .await
            .expect("a command");
        match host.apply(outcome).await {
            Applied::Conversation { messages, note } => {
                assert_eq!(messages.len(), 1);
                let note = note.expect("a note");
                assert!(note.contains("1 line(s) could not be read"), "{note}");
            }
            other => panic!("expected a conversation, got {other:?}"),
        }

        assert_ne!(host.session_id, original);
        assert_eq!(host.session.lock().await.id(), host.session_id);
    }

    #[tokio::test]
    async fn importing_something_unreadable_says_so() {
        let (mut host, _root) = host("import-missing").await;
        let applied = host
            .apply(CommandOutcome::Import {
                path: "not-here.jsonl".into(),
            })
            .await;
        assert!(applied.is_error(), "{applied:?}");
    }

    /// Reloading re-reads what the model was told and leaves the conversation alone.
    #[tokio::test]
    async fn reloading_replaces_the_standing_instructions() {
        let (mut host, _root) = host("reload").await;
        match host.apply(CommandOutcome::Reload).await {
            Applied::SystemPrompt { prompt, note } => {
                assert!(!prompt.trim().is_empty());
                let note = note.expect("a note");
                assert!(note.starts_with("Reloaded "), "{note}");
            }
            other => panic!("expected a system prompt, got {other:?}"),
        }
    }

    /// Sharing without a token says what to set rather than failing at GitHub.
    #[tokio::test]
    async fn sharing_without_a_token_names_the_variable() {
        let (mut host, _root) = host("share").await;
        if crate::share::token().is_some() {
            return;
        }
        let applied = host.apply(CommandOutcome::Share).await;
        let text = note(&applied);
        assert!(text.contains("GITHUB_TOKEN"), "{text}");
    }

    /// A project vouched for is remembered, and the decision is what a later run reads.
    #[tokio::test]
    async fn trusting_a_project_is_remembered() {
        let (mut host, root) = host("trust").await;
        let outcome = host.dispatch("/trust", state(0)).await.expect("a command");
        let text = note(&host.apply(outcome).await).to_string();
        assert_eq!(
            text,
            "Saved trust decision: trusted. Restart micro for this to take effect."
        );

        let store = micro_config::TrustStore::load_from(root.join("home"))
            .await
            .unwrap();
        assert!(store.is_trusted(&host.workspace));

        let outcome = host
            .dispatch("/trust off", state(0))
            .await
            .expect("a command");
        assert_eq!(
            note(&host.apply(outcome).await),
            "Saved trust decision: untrusted. Restart micro for this to take effect."
        );
        let store = micro_config::TrustStore::load_from(root.join("home"))
            .await
            .unwrap();
        assert!(!store.is_trusted(&host.workspace));
    }

    /// A subscription credential is billed per token from here, which is worth saying
    /// once and not worth saying twice.
    #[tokio::test]
    async fn a_subscription_credential_is_flagged_once() {
        let (mut host, _root) = host("extra-usage").await;

        let warning = host.subscription_warning("sk-ant-oat01-abc", "anthropic");
        assert!(
            warning
                .as_deref()
                .is_some_and(|text| text.contains("billed per token")),
            "{warning:?}"
        );
        assert_eq!(
            host.subscription_warning("sk-ant-oat01-abc", "anthropic"),
            None,
            "said once"
        );
    }

    #[tokio::test]
    async fn an_api_key_is_not_a_subscription() {
        let (mut host, _root) = host("api-key-usage").await;
        assert_eq!(
            host.subscription_warning("sk-ant-api03-abc", "anthropic"),
            None
        );
        assert_eq!(
            host.subscription_warning("sk-ant-oat01-abc", "openrouter"),
            None
        );
    }

    #[tokio::test]
    async fn the_subscription_warning_can_be_turned_off() {
        let (mut host, _root) = host("no-usage-warning").await;
        host.anthropic_extra_usage = false;
        assert_eq!(
            host.subscription_warning("sk-ant-oat01-abc", "anthropic"),
            None
        );
    }

    #[tokio::test]
    async fn a_pasted_key_is_stored() {
        let (mut host, _root) = host("store-key").await;
        let applied = host
            .store_api_key("openrouter".into(), "sk-or-test".into())
            .await;

        assert!(note(&applied).contains("openrouter"));
        assert_eq!(
            host.auth.get("openrouter").map(|c| c.token().to_string()),
            Some("sk-or-test".to_string())
        );
    }

    #[tokio::test]
    async fn a_blank_key_is_refused_rather_than_stored() {
        let (mut host, _root) = host("blank-key").await;
        let applied = host.store_api_key("openrouter".into(), "   ".into()).await;

        assert!(applied.is_error(), "{applied:?}");
        assert!(host.auth.get("openrouter").is_none());
    }

    #[tokio::test]
    async fn signing_out_takes_effect_immediately() {
        let (mut host, _root) = host("logout").await;
        host.auth.store_api_key("openrouter", "sk-or-test").unwrap();

        let outcome = host
            .dispatch("/logout openrouter", state(0))
            .await
            .expect("a command");

        assert!(!outcome.is_error(), "{outcome:?}");
        assert!(host.auth.get("openrouter").is_none());
    }

    /// `!` runs the shell unless an extension takes over what running it means. Tested
    /// directly against `before_bash` — a `!` line is only ever read by the interactive
    /// TUI, which these tests do not drive — with a real extension host, since what
    /// matters is that ohm's `user_bash` shape actually reaches one.
    #[tokio::test]
    async fn an_extension_can_take_over_what_a_bang_command_does() {
        if micro_extensions::which_bun().is_none() {
            return;
        }
        let (mut host, root) = host("bang-bash").await;
        let extension = root.join("intercept.ts");
        std::fs::write(
            &extension,
            r#"
export default (micro) => {
    micro.on("user_bash", (event) => {
        if (event.command !== "rm -rf /") {
            return;
        }
        return {
            result: { output: "refused", exitCode: 1, cancelled: false, truncated: false },
        };
    });
};
"#,
        )
        .unwrap();
        let extension_host = micro_extensions::Host::start(
            &root.join("home"),
            &[extension],
            &root.join("workspace"),
            false,
            false,
            "tui",
        )
        .await
        .expect("the host starts");
        host.extensions = Some(Arc::new(extension_host));

        let taken_over = host.before_bash("rm -rf /", false, "/workspace").await;
        assert_eq!(
            taken_over,
            Some(micro_tui::BashRun {
                output: "refused".into(),
                failed: true,
            })
        );

        // A command the extension does not care about is not taken over, and the shell
        // runs it as it always would.
        let untouched = host.before_bash("ls", false, "/workspace").await;
        assert_eq!(untouched, None);

        if let Some(host) = host.extensions.take() {
            host.shutdown("quit").await;
        }
    }
}

/// The name and arguments of a slash command, or nothing when the line is not one.
fn command_parts(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix('/')?;
    Some(match rest.split_once(char::is_whitespace) {
        Some((name, arguments)) => (name, arguments.trim()),
        None => (rest, ""),
    })
}
