//! Entry point. With no subcommand it opens the interface; `--print` runs one prompt and
//! exits.

mod approver;
mod commands;
mod headless;
mod extensions;
mod runtime;
mod share;
mod subcommands;

use anyhow::Result;
use clap::Parser;
use clap::Subcommand;
use micro_types::Message;
use micro_types::ThinkingLevel;
use runtime::Selection;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "micro", version, about = "A small coding agent")]
struct Cli {
    /// The prompt to run. Without --print this seeds the interface.
    prompt: Vec<String>,

    #[command(subcommand)]
    command: Option<Command>,

    /// Run the prompt and exit instead of opening the interface.
    #[arg(short = 'p', long)]
    print: bool,

    /// Take commands as JSON lines on stdin and answer on stdout, with no interface.
    #[arg(long, conflicts_with = "print")]
    rpc: bool,

    /// Model to use: an id, a provider-qualified id, a unique prefix, or an alias.
    #[arg(short, long, env = "MICRO_MODEL")]
    model: Option<String>,

    /// Provider to use, when the model alone does not determine one.
    #[arg(long, env = "MICRO_PROVIDER")]
    provider: Option<String>,

    /// Extended thinking effort.
    #[arg(long, value_parser = parse_thinking, default_value = "off")]
    thinking: ThinkingLevel,

    /// Workspace root. Tools cannot read or write outside it.
    #[arg(short = 'C', long, default_value = ".")]
    cwd: PathBuf,

    /// Resume a saved session by id.
    #[arg(long, value_name = "ID")]
    resume: Option<String>,

    /// Resume the most recent session for this workspace.
    #[arg(long = "continue", conflicts_with = "resume")]
    continue_latest: bool,

    /// Suppress tool progress on stderr.
    #[arg(short, long)]
    quiet: bool,

    /// How much the agent may do without asking.
    #[arg(long, value_parser = parse_mode, default_value = "cautious")]
    approve: micro_policy::Mode,
}

#[derive(Subcommand)]
enum Command {
    /// Manage provider credentials.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// List models in the catalog.
    Models {
        /// Only show models matching this query.
        query: Option<String>,
        /// Merge live provider listings before showing the catalog.
        #[arg(long)]
        live: bool,
    },
    /// Install an extension package.
    Install {
        /// npm:name, a repository URL, or a path.
        source: String,
        /// Install into this project rather than for every project.
        #[arg(short, long)]
        local: bool,
    },
    /// Remove an installed extension package.
    #[command(alias = "uninstall")]
    Remove {
        /// The source it was installed from.
        source: String,
        /// Remove it from this project rather than from every project.
        #[arg(short, long)]
        local: bool,
    },
    /// List the extension packages that are installed.
    List,
    /// Inspect saved sessions.
    Sessions {
        #[command(subcommand)]
        action: Option<SessionAction>,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Sign in to a provider.
    Login { provider: String },
    /// Adopt the credentials agent47 already holds.
    Import {
        /// Replace credentials micro already has.
        #[arg(long)]
        overwrite: bool,
    },
    /// Remove a stored credential.
    Logout { provider: String },
    /// Show which providers are configured.
    Status,
}

#[derive(Subcommand)]
enum SessionAction {
    /// List sessions, most recent first.
    List {
        /// Include sessions from every workspace.
        #[arg(long)]
        all: bool,
    },
    /// Delete a session.
    Delete { id: String },
}

fn parse_mode(value: &str) -> Result<micro_policy::Mode, String> {
    match value {
        "cautious" => Ok(micro_policy::Mode::Cautious),
        "workspace" => Ok(micro_policy::Mode::Workspace),
        "unrestricted" => Ok(micro_policy::Mode::Unrestricted),
        other => Err(format!(
            "unknown approval mode `{other}`: expected cautious, workspace, or unrestricted"
        )),
    }
}

fn parse_thinking(value: &str) -> Result<ThinkingLevel, String> {
    match value {
        "off" => Ok(ThinkingLevel::Off),
        "low" => Ok(ThinkingLevel::Low),
        "medium" => Ok(ThinkingLevel::Medium),
        "high" => Ok(ThinkingLevel::High),
        other => Err(format!("unknown thinking level: {other}")),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Flags micro does not know are held back rather than refused: an extension may have
    // declared one, and the extensions have not loaded yet.
    let (mine, given) = micro_extensions::split_unknown(
        std::env::args(),
        &["print", "rpc", "quiet", "continue", "local", "live", "all", "overwrite", "help", "version"],
        &["model", "provider", "thinking", "cwd", "resume", "approve"],
    );
    let cli = Cli::parse_from(mine);

    match &cli.command {
        Some(Command::Auth { action }) => {
            return match action {
                AuthAction::Login { provider } => subcommands::auth_login(provider).await,
                AuthAction::Import { overwrite } => subcommands::auth_import(*overwrite).await,
                AuthAction::Logout { provider } => subcommands::auth_logout(provider).await,
                AuthAction::Status => subcommands::auth_status().await,
            }
        }
        Some(Command::Models { query, live }) => {
            return subcommands::models(query.as_deref(), *live).await
        }
        Some(Command::Install { source, local }) => {
            let root = runtime::workspace(&cli.cwd)?;
            return subcommands::install(source, *local, &root).await;
        }
        Some(Command::Remove { source, local }) => {
            let root = runtime::workspace(&cli.cwd)?;
            return subcommands::remove(source, *local, &root).await;
        }
        Some(Command::List) => return subcommands::list_packages().await,
        Some(Command::Sessions { action }) => {
            // Sessions are recorded against the resolved workspace, so listing has to use
            // the same one rather than wherever the shell happens to be.
            let root = runtime::workspace(&cli.cwd)?;
            return match action {
                Some(SessionAction::List { all }) => subcommands::sessions_list(&root, *all).await,
                Some(SessionAction::Delete { id }) => subcommands::sessions_delete(id).await,
                None => subcommands::sessions_list(&root, false).await,
            };
        }
        None => {}
    }

    let root = runtime::workspace(&cli.cwd)?;
    // What a user settled once and left alone. A command-line argument still wins, which
    // is what `resolve_from_env` layers for us.
    let settings = micro_config::Config::load()
        .and_then(|config| {
            config.resolve_from_env(&micro_config::Overrides {
                model: cli.model.clone(),
                provider: cli.provider.clone(),
                ..micro_config::Overrides::default()
            })
        })
        .unwrap_or_else(|error| {
            eprintln!("note: {error}; using defaults");
            micro_config::Settings::default()
        });

    // Each front end answers the policy its own way: the non-interactive path prompts on
    // the terminal, while the interface routes requests to a modal over the transcript.
    // Extensions ask their questions through the interface, when there is one.
    let (asker, questions) = match cli.print || cli.rpc {
        true => (None, None),
        false => {
            let (asker, requests) = micro_tui::ui_channel();
            (Some(asker), Some(requests))
        }
    };

    let (approver, approvals): (std::sync::Arc<dyn micro_policy::Approver>, _) = match (
        cli.print, cli.rpc,
    ) {
        // Nobody is at a terminal to answer in RPC mode, so a call the policy cannot
        // decide is refused rather than left waiting for an answer that cannot come.
        (_, true) => (std::sync::Arc::new(micro_policy::DenyEverything), None),
        // With `--print` the user is at the terminal, and is asked there.
        (true, false) => (std::sync::Arc::new(approver::TerminalApprover), None),
        (false, false) => {
            let (approver, requests) = micro_tui::approval_channel();
            (approver, Some(requests))
        }
    };

    let selection = Selection {
        model: settings.model.clone(),
        provider: settings.provider.clone(),
        thinking: cli.thinking,
        mode: cli.approve,
        approver,
    };

    let resume = match (&cli.resume, cli.continue_latest) {
        (Some(id), _) => Some(id.clone()),
        (None, true) => Some(subcommands::latest_session(&root).await?),
        (None, false) => None,
    };

    let mut built = runtime::build(&root, &selection, resume.as_deref(), &settings).await?;
    // Extensions are told the session has begun, and told again when it ends, which is
    // where one that holds anything open gets to let go of it.
    let extensions = built.extensions.clone();
    if let Some(host) = extensions.as_ref() {
        let started = serde_json::json!({
            "session_id": built.session.lock().await.id(),
            "workspace": root.display().to_string(),
            "model": built.model.qualified_id(),
        });
        let _ = host.notify("session_start", started).await;
        // What micro found on disk for this project, so an extension can add to it.
        let _ = host
            .notify(
                "resources_discover",
                serde_json::json!({ "workspace": root.display().to_string() }),
            )
            .await;
    }

    if let Some(host) = extensions.as_ref() {
        // A flag written on the command line reaches whoever declared it. One nobody
        // declared is said out loud, so a typo is visible rather than silent.
        let declared = host.flags();
        for flag in &given {
            match declared.iter().find(|known| known.name == flag.name) {
                Some(known) => {
                    let value = match (known.r#type.as_str(), &flag.value) {
                        ("string", Some(value)) => serde_json::json!(value),
                        ("string", None) => serde_json::json!(""),
                        (_, Some(value)) => serde_json::json!(!matches!(
                            value.as_str(),
                            "false" | "no" | "0" | "off"
                        )),
                        (_, None) => serde_json::json!(true),
                    };
                    let _ = host.set_flag(&flag.name, value).await;
                }
                None => eprintln!("note: nothing declared a `--{}` flag", flag.name),
            }
        }

        // What the pump answers questions from: what is running, and the session it runs
        // in. Filled in here because this is where both are known.
        let state = std::sync::Arc::new(tokio::sync::RwLock::new(extensions::State {
            thinking: format!("{:?}", cli.thinking).to_lowercase(),
            model: built.model.id.clone(),
            provider: built.model.provider.clone(),
            tools: built.tool_names.clone(),
            commands: micro_commands::commands()
                .iter()
                .map(|command| command.name.to_string())
                .collect(),
        }));
        tokio::spawn(extensions::serve(
            std::sync::Arc::clone(host),
            root.clone(),
            asker.clone(),
            state,
            std::sync::Arc::clone(&built.session),
        ));
    }

    let session = std::sync::Arc::clone(&built.session);
    let writer = runtime::persist(built.session, built.recorder);
    let prompt = cli.prompt.join(" ");

    if cli.rpc {
        let mut rpc = micro_rpc::Rpc::new(
            built.agent,
            session,
            micro_models::Catalog::load().unwrap_or_else(|_| micro_models::Catalog::bundled()),
            root.clone(),
        );
        let outcome = rpc
            .run(tokio::io::stdin(), tokio::io::stdout())
            .await
            .map_err(anyhow::Error::from);
        // The agent lives inside the mode, and the writer runs until the agent's recorder
        // closes. Letting go of the mode first is what ends it.
        drop(rpc);
        let _ = writer.await;
        shut_down_extensions(extensions).await;
        return outcome;
    }

    let result = if cli.print {
        if prompt.trim().is_empty() {
            anyhow::bail!("--print needs a prompt");
        }
        // Signing in is something micro does at a prompt, and there is no prompt here,
        // so a run that cannot authenticate says so instead of spending a request.
        if let Some(notice) = &built.notice {
            drop(built.agent);
            let _ = writer.await;
            shut_down_extensions(extensions).await;
            anyhow::bail!("{notice}");
        }
        // What was typed goes past the extensions first here too, so a rewrite works the
        // same whether or not there is an interface.
        let prompt = match built.commands.submitted(prompt).await {
            Some(prompt) => prompt,
            None => {
                drop(built.agent);
                let _ = writer.await;
                shut_down_extensions(extensions).await;
                return Ok(());
            }
        };

        // A slash command is run rather than sent: it is an instruction to micro, and
        // handing it to the model would answer a question nobody asked.
        match run_command_headlessly(&mut built.commands, &prompt).await {
            Some(said) => {
                println!("{said}");
                // The agent is what holds the recorder open, and nothing ran a turn, so
                // it is let go here: the writer below waits for that channel to close.
                drop(built.agent);
                Ok(())
            }
            None => headless::run(built.agent, Message::user(prompt), cli.quiet).await,
        }
    } else {
        let options = micro_tui::TuiOptions {
            cwd: root.clone(),
            model: built.model.qualified_id(),
            context_window: built.model.context_window,
            thinking: cli.thinking,
            settings: micro_tui::Preferences::from(&settings),
            approvals,
            questions,
            // Without this every submitted line goes to the model, `/help` included.
            commands: Some(Box::new(built.commands)),
            notice: built.notice,
            ..micro_tui::TuiOptions::default()
        };
        // The conversation is persisted through the recorder, so the transcript the
        // interface hands back on exit is already on disk.
        micro_tui::run_with(built.agent, built.history, options)
            .await
            .map(|_| ())
    };

    // The agent has been dropped by now, which closes the recorder and ends the writer.
    // Waiting for it guarantees every message reached the log before the process exits.
    let _ = writer.await;
    shut_down_extensions(extensions).await;
    result
}

/// Let the extension host go, once nothing else needs it.
///
/// The host holds someone else's code in another process; leaving it running would
/// outlive the session that started it.
async fn shut_down_extensions(
    extensions: Option<std::sync::Arc<micro_extensions::Host>>,
) {
    let Some(host) = extensions else {
        return;
    };
    // Not conditional on being the last holder: the pump, the hooks and the command
    // runner all keep one, and the host has to be told either way or it is killed with
    // the process, mid-sentence.
    host.shutdown().await;
}

/// Run a slash command with nobody watching, and say what it printed.
///
/// `None` means the line was not a command, and belongs to the model. Only commands whose
/// whole answer is text can run here: anything that would open a picker or change the
/// running conversation needs an interface to change.
use micro_tui::Commands as _;

async fn run_command_headlessly(
    commands: &mut commands::CliCommands,
    line: &str,
) -> Option<String> {
    let line = line.trim();
    if !line.starts_with('/') {
        return None;
    }

    let state = micro_tui::ConversationState {
        message_count: 0,
        usage: micro_types::Usage::default(),
    };
    let outcome = commands.dispatch(line, state).await?;
    if let Some(text) = outcome.text() {
        return Some(text.to_string());
    }

    // Whatever the command settled on, said the way it would be said on screen. A run
    // without an interface applies nothing else: there is no agent to hand a new model
    // to and no scrollback to rebuild.
    match commands.apply(outcome).await {
        micro_tui::Applied::Note { text, .. } => Some(text),
        micro_tui::Applied::Conversation { note, .. } => note,
        micro_tui::Applied::SystemPrompt { note, .. } => note,
        micro_tui::Applied::Model { note, .. } => note,
        micro_tui::Applied::Nothing => None,
    }
}
