//! Entry point. With no subcommand it opens the interface; `--print` runs one prompt and
//! exits.

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

    /// Comma-separated allowlist of tool names to enable.
    #[arg(long, short = 't', value_delimiter = ',')]
    tools: Vec<String>,

    /// Comma-separated denylist of tool names to disable.
    #[arg(long = "exclude-tools", short = 'x', value_delimiter = ',')]
    exclude_tools: Vec<String>,

    /// How much of the terminal to take: regular draws inline, fullscreen takes it all.
    #[arg(long = "tui-mode", value_parser = parse_tui_mode)]
    tui_mode: Option<micro_config::TuiMode>,

    /// Load skills from this path as well, which may be a directory or one `.md` file.
    #[arg(long = "skill", value_name = "PATH")]
    skills: Vec<PathBuf>,

    /// Do not look for skills at all.
    #[arg(long = "no-skills", visible_short_alias = 's')]
    no_skills: bool,

    /// Load an extension from this path as well.
    #[arg(long = "extension", short = 'e', value_name = "PATH")]
    extensions: Vec<String>,

    /// Do not load any extension.
    #[arg(long = "no-extensions")]
    no_extensions: bool,

    /// Load prompt templates from this path as well.
    #[arg(long = "prompt-template", value_name = "PATH")]
    prompt_templates: Vec<PathBuf>,

    /// Do not look for prompt templates.
    #[arg(long = "no-prompt-templates")]
    no_prompt_templates: bool,

    /// Do not read AGENTS.md or any other instruction file.
    #[arg(long = "no-context-files")]
    no_context_files: bool,

    /// Palette to paint in: dark, light, or auto.
    #[arg(long = "theme", value_name = "NAME")]
    theme: Option<String>,

    /// Trust this project for this run, without being asked and without remembering.
    #[arg(short = 'a', long)]
    approve: bool,

    /// Do not trust this project for this run, whatever was decided before.
    #[arg(long = "no-approve", conflicts_with = "approve")]
    no_approve: bool,
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

/// Whether this project may run what it ships.
///
/// A project carrying none of it is used without a question. One that does is answered by
/// what the run was told outright, then by whatever was decided about it before, then by
/// the standing answer, and only then by asking. With nobody at a terminal there is
/// nobody to ask, so it is not trusted.
async fn project_trusted(
    root: &std::path::Path,
    settings: &micro_config::Settings,
    has_ui: bool,
    told: Option<bool>,
) -> bool {
    // Said on the command line, this settles it for this run alone and is not written
    // down: a scripted run says what it wants every time rather than leaving a decision
    // behind on the machine it happened to run on.
    if let Some(told) = told {
        return told;
    }

    if !micro_config::requires_decision(root) {
        return true;
    }

    let mut store = micro_config::TrustStore::load().await.unwrap_or_default();
    if let Some(decision) = store.decision(root) {
        return decision.trusted;
    }

    match settings.default_project_trust {
        micro_config::ProjectTrust::Always => return true,
        micro_config::ProjectTrust::Never => return false,
        micro_config::ProjectTrust::Ask => {}
    }
    if !has_ui {
        return false;
    }

    let trusted = ask_about_trust(root);
    store.decide(root, trusted);
    if let Err(error) = store.save().await {
        eprintln!("note: the decision was not saved: {error}");
    }
    trusted
}

/// Put the question to whoever is at the terminal, before the interface takes it over.
fn ask_about_trust(root: &std::path::Path) -> bool {
    use std::io::BufRead as _;
    use std::io::Write as _;

    println!("Trust project folder?");
    println!("{}", root.display());
    println!();
    println!(
        "This allows micro to load {} settings and resources, and run this project's \
         extensions.",
        micro_config::PROJECT_DIR
    );
    print!("Trust it? [y/N] ");
    let _ = std::io::stdout().flush();

    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn parse_tui_mode(value: &str) -> Result<micro_config::TuiMode, String> {
    match value {
        "regular" | "inline" => Ok(micro_config::TuiMode::Regular),
        "fullscreen" => Ok(micro_config::TuiMode::Fullscreen),
        other => Err(format!("unknown tui mode: {other}; expected regular or fullscreen")),
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

/// Every long flag micro itself declares: those that stand alone, and those that take a
/// value. Read from the parser, and from every subcommand's parser, so the two can never
/// disagree about what micro knows.
fn own_flags() -> (Vec<String>, Vec<String>) {
    use clap::CommandFactory;

    let mut switches = Vec::new();
    let mut valued = Vec::new();

    fn walk(command: &clap::Command, switches: &mut Vec<String>, valued: &mut Vec<String>) {
        for argument in command.get_arguments() {
            let names = argument
                .get_long()
                .into_iter()
                .chain(argument.get_all_aliases().unwrap_or_default())
                .chain(argument.get_visible_aliases().unwrap_or_default());
            let into = match argument.get_action().takes_values() {
                true => &mut *valued,
                false => &mut *switches,
            };
            into.extend(names.map(str::to_string));
        }
        for inner in command.get_subcommands() {
            walk(inner, switches, valued);
        }
    }

    walk(&Cli::command(), &mut switches, &mut valued);
    // The parser writes these two itself and only names them once it has been built, which
    // is after this is asked. They are micro's own whatever the parser says.
    switches.push("help".to_string());
    switches.push("version".to_string());
    (switches, valued)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Flags micro does not know are held back rather than refused: an extension may have
    // declared one, and the extensions have not loaded yet. Which flags micro knows is
    // asked of the parser rather than listed here, because a list would drift from the
    // parser the first time a flag was added and take that flag with it.
    let (switches, valued) = own_flags();
    let (mine, given) = micro_extensions::split_unknown(
        std::env::args(),
        &switches.iter().map(String::as_str).collect::<Vec<_>>(),
        &valued.iter().map(String::as_str).collect::<Vec<_>>(),
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

    let resources = runtime::Resources {
        skills: cli.skills.clone(),
        no_skills: cli.no_skills,
        extensions: cli.extensions.clone(),
        no_extensions: cli.no_extensions,
        prompt_templates: cli.prompt_templates.clone(),
        no_prompt_templates: cli.no_prompt_templates,
        no_context_files: cli.no_context_files,
    };
    let selection = Selection {
        resources: resources.clone(),
        model: settings.model.clone(),
        provider: settings.provider.clone(),
        thinking: cli.thinking,
        tools: cli.tools.clone(),
        exclude_tools: cli.exclude_tools.clone(),
    };

    let resume = match (&cli.resume, cli.continue_latest) {
        (Some(id), _) => Some(id.clone()),
        (None, true) => Some(subcommands::latest_session(&root).await?),
        (None, false) => None,
    };

    // A project's own extensions and skills are things it asks micro to run, so whether
    // to run them is settled before anything of the project's is loaded.
    let has_ui = !cli.print && !cli.rpc;
    let told = match (cli.approve, cli.no_approve) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    };
    let trusted = project_trusted(&root, &settings, has_ui, told).await;
    let mut built =
        runtime::build(&root, &selection, resume.as_deref(), &settings, trusted, has_ui).await?;
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
    let session_id = session.lock().await.id().to_string();
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
            questions,
            // Without this every submitted line goes to the model, `/help` included.
            commands: Some(Box::new(built.commands)),
            notice: built.notice,
            provider: built.model.provider.clone(),
            subscription: built.subscription,
            auto_compact: settings.auto_compact,
            price: Some(built.model.cost.clone()),
            experimental: micro_config::experimental_enabled(),
            // Named on the command line for this run, in place of whatever was settled on.
            theme: cli.theme.as_deref().and_then(micro_tui::Theme::named),
            resources: built.resources,
            // Said on the command line for this run, otherwise whatever was settled on.
            tui_mode: match cli.tui_mode.unwrap_or(settings.tui_mode) {
                micro_config::TuiMode::Regular => micro_tui::TuiMode::Inline,
                micro_config::TuiMode::Fullscreen => micro_tui::TuiMode::Fullscreen,
            },
            ..micro_tui::TuiOptions::default()
        };
        // The conversation is persisted through the recorder, so the transcript the
        // interface hands back on exit is already on disk.
        let ran = micro_tui::run_with(built.agent, built.history, options)
            .await
            .map(|_| ());
        // Said on the way out, where a shell keeps it: the id is the one thing a reader
        // needs to come back, and it is not worth going and looking for.
        if ran.is_ok() {
            say_how_to_resume(&session_id);
        }
        ran
    };

    // The agent has been dropped by now, which closes the recorder and ends the writer.
    // Waiting for it guarantees every message reached the log before the process exits.
    let _ = writer.await;
    shut_down_extensions(extensions).await;
    result
}

/// Leave the line that brings this conversation back.
///
/// Only where a person will see it: piped into something else, it would be one more line
/// for that to deal with.
fn say_how_to_resume(session_id: &str) {
    use std::io::IsTerminal;
    if session_id.is_empty() || !std::io::stdout().is_terminal() {
        return;
    }
    println!("To resume this session: micro --resume {session_id}");
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

#[cfg(test)]
mod flag_tests {
    use super::*;

    /// Every flag micro declares has to be recognised as micro's own before the extensions
    /// load, or it is held back as one an extension might have declared and never reaches
    /// the parser. The list used to be written out by hand beside the parser, and drifted:
    /// `--tui-mode` was declared, never listed, and so silently did nothing.
    #[test]
    fn every_flag_micro_declares_is_known_to_be_its_own() {
        let (switches, valued) = own_flags();
        let known = |name: &str| {
            switches.iter().any(|flag| flag == name) || valued.iter().any(|flag| flag == name)
        };

        for flag in [
            "print", "rpc", "model", "provider", "thinking", "cwd", "resume", "continue",
            "quiet", "tools", "exclude-tools", "tui-mode", "approve", "no-approve", "skill",
            "no-skills", "extension", "no-extensions", "prompt-template",
            "no-prompt-templates", "no-context-files", "theme",
        ] {
            assert!(known(flag), "`--{flag}` is not recognised as micro's own");
        }

        // A flag that carries a value has to be in the other list, or the value after it
        // is read as the prompt.
        for flag in ["model", "cwd", "skill", "extension", "theme", "tui-mode"] {
            assert!(valued.iter().any(|known| known == flag), "`--{flag}` takes a value");
        }
        for flag in ["print", "rpc", "no-skills", "no-extensions"] {
            assert!(switches.iter().any(|known| known == flag), "`--{flag}` takes none");
        }

        // Subcommands declare flags too, and they are held to the same rule.
        assert!(known("local") && known("live") && known("overwrite"));
    }
}
