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

/// The two things a run needs from a provider: something to talk to it with, and the
/// credential to talk with. Where they came from stops mattering here.
struct Resolved {
    client: std::sync::Arc<dyn micro_provider::Provider>,
    api_key: String,
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
    /// The extension host, when there was anything to load and a runtime to load it.
    pub extensions: Option<Arc<micro_extensions::Host>>,
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
    let mut catalog = Catalog::load().unwrap_or_else(|_| Catalog::bundled());

    // Extensions are loaded before a model is picked, because one of them may be what
    // serves it: a provider an extension declares is in the catalog by the time the
    // catalog is read.
    let extensions = load_extensions(root, settings).await;
    let declared = apply_declared_providers(&mut catalog, extensions.as_deref(), settings);
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
    // A provider an extension declared is not in the registry, and does not need to be:
    // it brought its own endpoint and its own credential, and the model says which wire
    // protocol to speak.
    let mut resolved = match micro_provider::resolve(&store, &provider_name).await {
        Ok(resolved) => Resolved {
            client: resolved.client,
            api_key: resolved.api_key,
        },
        Err(error) => match declared.get(&provider_name) {
            Some(key) => Resolved {
                client: micro_provider::client_for(model.api),
                api_key: key.clone(),
            },
            None => {
                return Err(anyhow::Error::new(error).context(format!(
                    "no usable credential for `{provider_name}`. Run `micro auth login \
                     {provider_name}`."
                )))
            }
        },
    };

    // A credential the store did not have may still have been declared alongside the
    // provider, which is where an extension puts one.
    if resolved.api_key.trim().is_empty() {
        if let Some(key) = declared.get(&provider_name) {
            resolved.api_key = key.clone();
        }
    }

    // A stored credential can be present and still be empty, which every provider reports
    // as a missing authentication header rather than as a bad key. Catching it here says
    // what to do about it instead of spending a request to find out.
    if resolved.api_key.trim().is_empty() {
        return Err(anyhow!(
            "the credential for `{provider_name}` is empty. Run `micro auth login \
             {provider_name}` to replace it."
        ));
    }

    // The Codex backend is the one provider with a choice about how it answers, so it is
    // built with what the user chose rather than with the default.
    if micro_auth::canonical_provider(&provider_name) == "openai-codex" {
        let transport = micro_provider::Transport::named(&settings.transport).unwrap_or_default();
        resolved.client = std::sync::Arc::new(micro_provider::Codex::new().with_transport(transport));
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
    // What extensions registered goes through the same policy as everything built in.
    let mut tools = micro_tools::builtin_tools(root.to_path_buf());
    if let Some(host) = extensions.as_ref() {
        let registered = host.tools();
        for tool in registered {
            tools.push(Arc::new(micro_extensions::ExtensionTool::new(
                tool.name,
                tool.description,
                tool.parameters,
                Arc::clone(host),
            )));
        }
    }
    let tools = micro_policy::gated_tools(tools, engine);

    let (recorder, receiver) = tokio::sync::mpsc::unbounded_channel();
    // Extensions watch the run rather than sitting in the middle of it: the events go to
    // whoever asked for the turn, and a copy comes here.
    let (watching, watched) = tokio::sync::mpsc::unbounded_channel();
    let agent = Agent::new(
        Arc::clone(&resolved.client),
        tools,
        model.to_runtime(selection.thinking),
        resolved.api_key.clone(),
    )
    .with_system_prompt(context.system_prompt)
    .with_history(history.clone())
    .with_context_window(model.context_window as usize)
    .with_recorder(recorder)
    .with_observer(watching);
    // Extensions get a say in what a tool call does, before it runs and after it answers.
    let agent = match extensions.as_ref() {
        Some(host) => agent.with_hooks(Arc::new(crate::extensions::ExtensionHooks::new(
            Arc::clone(host),
        ))),
        None => agent,
    };
    // Compaction is what keeps a long conversation inside the window; turned off, the
    // conversation is left to grow and the provider decides when it will not take more.
    let agent = match settings.auto_compact {
        true => agent,
        false => agent.without_compaction(),
    };

    if let Some(host) = extensions.as_ref() {
        tokio::spawn(forward_events(watched, Arc::clone(host)));
    }

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
        extensions: extensions.clone(),
        anthropic_extra_usage: settings.anthropic_extra_usage,
    });

    Ok(Runtime {
        agent,
        extensions,
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

/// Start the extension host, if there is anything to load.
///
/// Nothing here is fatal. An extension that will not load is named on stderr and the run
/// carries on without it, and a missing Bun means no extensions rather than no micro.
async fn load_extensions(
    root: &Path,
    settings: &micro_config::Settings,
) -> Option<Arc<micro_extensions::Host>> {
    let home = micro_policy::micro_home().unwrap_or_default();
    // A configured entry is a source rather than a path: `npm:thing` is installed
    // somewhere of micro's choosing, and that is where it is loaded from.
    let configured: Vec<String> = settings
        .extensions
        .iter()
        .map(|source| match micro_extensions::Source::parse(source) {
            Ok(parsed) => parsed
                .install_path(&home, root, false)
                .display()
                .to_string(),
            Err(_) => source.clone(),
        })
        .collect();
    let paths = micro_extensions::discover(root, &home, &configured);
    if paths.is_empty() {
        return None;
    }

    match micro_extensions::Host::start(&home, &paths).await {
        Ok(host) => {
            if !settings.quiet_startup {
                for failure in &host.loaded().errors {
                    eprintln!("note: {} was not loaded: {}", failure.path, failure.error);
                }
            }
            Some(Arc::new(host))
        }
        Err(error) => {
            if !settings.quiet_startup {
                eprintln!("note: extensions were not loaded: {error}");
            }
            None
        }
    }
}

/// Tell the extensions what the agent is doing, for as long as the run lasts.
///
/// Nothing waits on them: an extension that is slow, or that has stopped listening, holds
/// nothing up. What they change they change by asking, which is its own path.
async fn forward_events(
    mut watched: tokio::sync::mpsc::UnboundedReceiver<micro_types::AgentEvent>,
    host: Arc<micro_extensions::Host>,
) {
    while let Some(event) = watched.recv().await {
        let Some(name) = micro_extensions::name_of(&event) else {
            continue;
        };
        let payload = micro_extensions::payload_of(&event);
        if host.notify(name, payload).await.is_err() {
            // The host has gone; there is nobody left to tell.
            return;
        }
    }
}

/// Merge every provider the extensions declared into the catalog, and collect the
/// credentials they brought with them.
///
/// A declaration that cannot be read is reported and skipped: one bad provider should not
/// take the catalog down with it.
fn apply_declared_providers(
    catalog: &mut Catalog,
    extensions: Option<&micro_extensions::Host>,
    settings: &micro_config::Settings,
) -> std::collections::BTreeMap<String, String> {
    let mut keys = std::collections::BTreeMap::new();
    let Some(host) = extensions else {
        return keys;
    };

    for registered in host.providers() {
        let declared = match micro_extensions::declare(&registered.name, &registered.config) {
            Ok(declared) => declared,
            Err(error) => {
                if !settings.quiet_startup {
                    eprintln!("note: {error}");
                }
                continue;
            }
        };

        if let Err(error) = catalog.apply_overrides(&declared.catalog.to_string()) {
            if !settings.quiet_startup {
                eprintln!("note: {} was not applied: {error}", declared.name);
            }
            continue;
        }
        if let Some(key) = declared.api_key {
            keys.insert(declared.name, key);
        }
    }
    keys
}
