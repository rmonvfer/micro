//! Assembling a ready-to-run agent from the user's configuration, credentials, and
//! workspace.

use crate::commands::CliCommands;
use anyhow::anyhow;
use anyhow::Context as _;
use anyhow::Result;
use micro_agent::Agent;
use micro_auth::AuthStore;
use micro_context::InstructionLoader;
use micro_models::Catalog;
use micro_models::ModelDef;
use micro_session::Session;
use micro_session::SessionStore;
use micro_types::Message;
use micro_types::ThinkingLevel;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

const BASE_PROMPT: &str = "You are micro, a coding agent working in a terminal. Use the \
provided tools to inspect and change the workspace. Read a file before editing it. Prefer \
the search tools over shell commands for finding code. Be concise: report what you did and \
what the user should do next.";

/// How the user asked for a model and provider before anything was resolved.
pub struct Selection {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub thinking: ThinkingLevel,
    pub mode: micro_policy::Mode,
    /// Whoever answers when the policy cannot decide on its own.
    pub approver: Arc<dyn micro_policy::Approver>,
}

/// Everything a run needs, resolved and ready.
pub struct Runtime {
    pub agent: Agent,
    /// Shared, because branching and renaming reach the same open session the writer
    /// task is appending to.
    pub session: Arc<Mutex<Session>>,
    pub history: Vec<Message>,
    pub model: ModelDef,
    pub recorder: tokio::sync::mpsc::UnboundedReceiver<Message>,
    /// How the interface runs slash commands. Built here because this is where the
    /// catalog, the credentials and the session store already are.
    pub commands: CliCommands,
}

/// Resolve a model from the catalog, reporting candidates rather than guessing when the
/// query matches more than one.
pub fn pick_model(catalog: &Catalog, selection: &Selection) -> Result<ModelDef> {
    let query = match &selection.model {
        Some(query) => query.clone(),
        None => return default_model(catalog, selection.provider.as_deref()),
    };

    let resolution = catalog.resolve(&query);
    if let Some(model) = resolution.model() {
        return Ok(model.clone());
    }

    let candidates = resolution.candidates();
    if candidates.is_empty() {
        return Err(anyhow!(
            "no model matches `{query}`. Run `micro models` to see what is available."
        ));
    }

    let names: Vec<String> = candidates
        .iter()
        .take(10)
        .map(|model| model.qualified_id())
        .collect();
    Err(anyhow!(
        "`{query}` matches {} models:\n  {}",
        candidates.len(),
        names.join("\n  ")
    ))
}

/// The first model of the requested provider, or of any provider that has a credential.
fn default_model(catalog: &Catalog, provider: Option<&str>) -> Result<ModelDef> {
    let wanted = provider.map(micro_auth::canonical_provider);
    catalog
        .models()
        .iter()
        .find(|model| wanted.is_none_or(|provider| model.provider == provider))
        .cloned()
        .ok_or_else(|| match provider {
            Some(provider) => anyhow!("no models known for provider `{provider}`"),
            None => anyhow!("the model catalog is empty"),
        })
}

/// Build the agent, opening or resuming a session and wiring durable persistence.
pub async fn build(
    root: &Path,
    selection: &Selection,
    resume: Option<&str>,
    settings: &micro_config::Settings,
) -> Result<Runtime> {
    // Set before any provider is built, since a client carries the timeout it was made
    // with.
    micro_provider::set_idle_timeout(settings.http_idle_timeout);
    let store = AuthStore::open().context("cannot open the credential store")?;
    let catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());
    // A workspace that has been given a shortlist may only use what is on it, so a model
    // outside it cannot be reached by cycling or by a stale config.
    let catalog = match settings.scoped_models.is_empty() {
        true => catalog,
        false => scoped(catalog, &settings.scoped_models),
    };
    let model = pick_model(&catalog, selection)?;

    let provider_name = selection
        .provider
        .clone()
        .unwrap_or_else(|| model.provider.clone());
    let resolved = micro_provider::resolve(&store, &provider_name)
        .await
        .with_context(|| {
            format!(
                "no usable credential for `{provider_name}`. Run `micro auth login \
                 {provider_name}`."
            )
        })?;

    // A stored credential can be present and still be empty, which every provider reports
    // as a missing authentication header rather than as a bad key. Catching it here says
    // what to do about it instead of spending a request to find out.
    if resolved.api_key.trim().is_empty() {
        return Err(anyhow!(
            "the credential for `{provider_name}` is empty. Run `micro auth login \
             {provider_name}` to replace it."
        ));
    }

    let sessions = SessionStore::from_env().context("cannot open the session store")?;
    let (session, history) = match resume {
        Some(id) => {
            let loaded = sessions.load(id).await?;
            if loaded.skipped_lines > 0 {
                eprintln!(
                    "note: skipped {} unreadable line(s) in session {id}",
                    loaded.skipped_lines
                );
            }
            (loaded.session, loaded.messages)
        }
        None => (
            sessions.create(root, model.qualified_id()).await?,
            Vec::new(),
        ),
    };

    let context = load_context(root, settings.skill_commands).await;
    if !settings.quiet_startup {
        for diagnostic in &context.diagnostics {
            eprintln!("note: {diagnostic}");
        }
    }

    // Every tool goes through the policy, so approval is enforced at the one place tools
    // actually execute rather than at each call site.
    let mut policy = micro_policy::Policy::load()
        .await
        .unwrap_or_else(|_| micro_policy::Policy::new(selection.mode));
    // A project that was vouched for with `/trust` starts wider than the cautious
    // default, which is the whole point of having said so.
    let trust = micro_policy::TrustStore::load().await.unwrap_or_default();
    // A project nobody has decided about takes the standing answer, which is cautious
    // unless the user has said otherwise.
    let assumed = match (trust.decision(root).is_none(), settings.default_project_trust) {
        (true, true) => micro_policy::Mode::Workspace,
        _ => selection.mode,
    };
    policy.mode = trust.mode_for(root, assumed);
    let engine = Arc::new(micro_policy::PolicyEngine::new(
        policy,
        root.to_path_buf(),
        Arc::clone(&selection.approver),
    ));
    let tools = micro_policy::gated_tools(micro_tools::builtin_tools(root.to_path_buf()), engine);

    let (recorder, receiver) = tokio::sync::mpsc::unbounded_channel();
    let agent = Agent::new(
        Arc::clone(&resolved.client),
        tools,
        model.to_runtime(selection.thinking),
        resolved.api_key.clone(),
    )
    .with_system_prompt(context.system_prompt)
    .with_history(history.clone())
    .with_context_window(model.context_window as usize)
    .with_recorder(recorder);
    // Compaction is what keeps a long conversation inside the window; turned off, the
    // conversation is left to grow and the provider decides when it will not take more.
    let agent = match settings.auto_compact {
        true => agent,
        false => agent.without_compaction(),
    };

    let session_id = session.id().to_string();
    let session = Arc::new(Mutex::new(session));
    let commands = CliCommands::new(crate::commands::HostParts {
        catalog,
        auth: store,
        sessions,
        workspace: root.to_path_buf(),
        provider: provider_name,
        model: model.clone(),
        session: Arc::clone(&session),
        session_id,
        home: micro_policy::micro_home().unwrap_or_default(),
        skills_enabled: settings.skill_commands,
        collapse_changelog: settings.collapse_changelog,
    });

    Ok(Runtime {
        agent,
        session,
        history,
        model,
        recorder: receiver,
        commands,
    })
}

/// Drain produced messages into the session log for as long as the run lasts, so a crash
/// leaves everything that was already said on disk.
pub fn persist(
    session: Arc<Mutex<Session>>,
    mut recorder: tokio::sync::mpsc::UnboundedReceiver<Message>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(message) = recorder.recv().await {
            if let Err(error) = session.lock().await.append(&message).await {
                eprintln!("warning: cannot write to the session log: {error}");
            }
        }
    })
}

/// The workspace the agent operates on. Tools cannot reach outside it.
pub fn workspace(requested: &Path) -> Result<PathBuf> {
    requested
        .canonicalize()
        .with_context(|| format!("workspace not found: {}", requested.display()))
}

/// Everything the model is told before the conversation starts: the instructions the
/// workspace carries, and the skills it can reach for.
///
/// Built in one place because `/reload` rebuilds exactly what a launch built, and a second
/// copy of the assembly would drift from it.
pub struct LoadedContext {
    pub system_prompt: String,
    /// The instruction files that contributed, in the order they were read.
    pub instruction_files: Vec<PathBuf>,
    pub skill_count: usize,
    /// Skills that could not be loaded, said in full rather than dropped in silence.
    pub diagnostics: Vec<String>,
}

pub async fn load_context(root: &Path, skills_enabled: bool) -> LoadedContext {
    let instructions = match InstructionLoader::from_env() {
        Ok(loader) => loader.load(root).await.unwrap_or_default(),
        Err(_) => Default::default(),
    };

    // Skills are announced by name and description only; the model reads a skill's file
    // when it decides one applies, which is what keeps a shelf of them out of the context.
    let home = micro_context::micro_home().unwrap_or_default();
    let skills = match skills_enabled {
        true => micro_skills::discover(root, &home).await,
        false => Default::default(),
    };

    let mut system_prompt = BASE_PROMPT.to_string();
    if !instructions.text.trim().is_empty() {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&instructions.text);
    }
    if let Some(section) = micro_skills::system_prompt_section(&skills.skills) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&section);
    }

    LoadedContext {
        system_prompt,
        instruction_files: instructions.sources,
        skill_count: skills.skills.len(),
        diagnostics: skills
            .diagnostics
            .iter()
            .map(|diagnostic| {
                format!(
                    "skill {} was not loaded: {}",
                    diagnostic.path.display(),
                    diagnostic.message
                )
            })
            .collect(),
    }
}

/// The catalog cut down to the models a workspace is allowed, matched by prefix so a
/// shortlist can name a provider, a family, or one exact model.
fn scoped(catalog: Catalog, allowed: &[String]) -> Catalog {
    let kept: Vec<ModelDef> = catalog
        .models()
        .iter()
        .filter(|model| {
            allowed.iter().any(|pattern| {
                model.qualified_id().starts_with(pattern.as_str())
                    || model.id.starts_with(pattern.as_str())
            })
        })
        .cloned()
        .collect();

    // A shortlist that matches nothing is a mistake worth surviving: the whole catalog is
    // more useful than no models at all.
    match kept.is_empty() {
        true => catalog,
        false => Catalog::from_models(kept),
    }
}
