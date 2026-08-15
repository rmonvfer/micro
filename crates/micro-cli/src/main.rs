//! Entry point. With no subcommand it opens the interface; `--print` runs one prompt and
//! exits.

mod capabilities;
mod commands;
mod extensions;
mod headless;
mod remote;
mod runtime;
mod sandbox;
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
    #[arg(long, value_parser = parse_thinking)]
    thinking: Option<ThinkingLevel>,

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

    /// What commands may touch: read-only, workspace-write, or full.
    #[arg(long = "sandbox", value_name = "POLICY")]
    sandbox: Option<String>,

    /// Stop this session once it has spent this many dollars. Zero is no ceiling.
    #[arg(long = "budget", value_name = "AMOUNT")]
    budget: Option<f64>,

    /// Set one config value for this run: `-c theme=dracula`, `-c show_images=false`.
    ///
    /// The key is a dotted path into the config file. Repeat the flag for more than one.
    #[arg(
        short = 'c',
        long = "config",
        value_name = "KEY=VALUE",
        action = clap::ArgAction::Append
    )]
    config_override: Vec<String>,
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
    /// Itemize what a session cost.
    Bill {
        /// The session to bill, rather than the latest one from this workspace.
        session: Option<String>,
        /// Show what one turn added to the bill, and why.
        #[arg(long = "diff", value_name = "TURN")]
        diff: Option<u64>,
    },
    /// Say why a turn paid for a prompt the provider already had.
    WhyMiss {
        /// The session to explain.
        session: String,
        /// Which turn, rather than the most recent one.
        turn: Option<u64>,
    },
    /// Try the sandbox out.
    Sandbox {
        #[command(subcommand)]
        action: SandboxAction,
    },
}

#[derive(Subcommand)]
enum SandboxAction {
    /// Run a command the way a session's own tools would, and say what became of it.
    Try {
        /// The policy to try, in place of the one this workspace would run under.
        #[arg(long = "sandbox", value_name = "POLICY")]
        sandbox: Option<String>,
        /// The command to run, after `--`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
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
    /// Show what a session recorded, turn by turn.
    Show {
        id: String,
        /// One turn of it, rather than a listing of all of them.
        #[arg(long)]
        turn: Option<u64>,
        /// Print the request as it went to the provider, rebuilt from what was recorded.
        #[arg(long)]
        raw: bool,
    },
    /// Print a session's whole ledger as JSONL.
    Export { id: String },
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
        other => Err(format!(
            "unknown tui mode: {other}; expected regular or fullscreen"
        )),
    }
}

fn thinking_from_settings(level: micro_config::Thinking) -> ThinkingLevel {
    match level {
        micro_config::Thinking::Off => ThinkingLevel::Off,
        micro_config::Thinking::Minimal => ThinkingLevel::Minimal,
        micro_config::Thinking::Low => ThinkingLevel::Low,
        micro_config::Thinking::Medium => ThinkingLevel::Medium,
        micro_config::Thinking::High => ThinkingLevel::High,
        micro_config::Thinking::XHigh => ThinkingLevel::XHigh,
        micro_config::Thinking::Max => ThinkingLevel::Max,
    }
}

fn parse_thinking(value: &str) -> Result<ThinkingLevel, String> {
    match value {
        "off" => Ok(ThinkingLevel::Off),
        "minimal" => Ok(ThinkingLevel::Minimal),
        "low" => Ok(ThinkingLevel::Low),
        "medium" => Ok(ThinkingLevel::Medium),
        "high" => Ok(ThinkingLevel::High),
        "xhigh" => Ok(ThinkingLevel::XHigh),
        "max" => Ok(ThinkingLevel::Max),
        other => Err(format!("unknown thinking level: {other}; expected off, minimal, low, medium, high, xhigh, or max")),
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

/// What a user settled once and left alone. A command-line argument still wins, which is
/// what `resolve_from_env` layers for us.
fn settled(cli: &Cli) -> micro_config::Settings {
    let mut settings = micro_config::Config::load_with(&cli.config_override)
        .and_then(|config| {
            config.resolve_from_env(&micro_config::Overrides {
                model: cli.model.clone(),
                provider: cli.provider.clone(),
                ..micro_config::Overrides::default()
            })
        })
        .unwrap_or_else(|error| {
            // A setting named on the command line is a thing just typed, so a bad one is
            // a mistake to correct rather than something to fall back from: carrying on
            // would run with settings other than the ones that were asked for.
            if matches!(error, micro_config::ConfigError::Override { .. }) {
                eprintln!("micro: {error}");
                std::process::exit(2);
            }
            eprintln!("note: {error}; using defaults");
            micro_config::Settings::default()
        });
    // A ceiling named on the command line stands for this run alone and is not written
    // down, the same way trust said outright is: a scripted run says what it is willing to
    // spend every time rather than leaving a limit behind on the machine it ran on.
    if let Some(budget) = cli.budget {
        settings.budget = budget.max(0.0);
    }
    settings
}

#[tokio::main]
async fn main() -> Result<()> {
    // Linux has no way to confine a command from the outside: the restrictions have to be
    // applied by the process that then becomes the command. So micro re-runs itself to run
    // anything a session spawns, and this is that second run recognizing itself — before
    // the parser, before the configuration, before anything that could fail and leave the
    // command running unconfined.
    #[cfg(target_os = "linux")]
    {
        let mut arguments = std::env::args();
        let program = arguments.next();
        if program.is_some() && arguments.next().as_deref() == Some(micro_sandbox::HELPER_ARG) {
            micro_sandbox::run_linux_helper(arguments);
        }
    }

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
                Some(SessionAction::Show { id, turn, raw }) => {
                    subcommands::sessions_show(id, *turn, *raw).await
                }
                Some(SessionAction::Export { id }) => subcommands::sessions_export(id).await,
                Some(SessionAction::Delete { id }) => subcommands::sessions_delete(id).await,
                None => subcommands::sessions_list(&root, false).await,
            };
        }
        Some(Command::Bill { session, diff }) => {
            let root = runtime::workspace(&cli.cwd)?;
            let id = match session {
                Some(id) => id.clone(),
                None => subcommands::latest_session(&root).await?,
            };
            return subcommands::bill(&id, *diff).await;
        }
        Some(Command::WhyMiss { session, turn }) => {
            return subcommands::why_miss(session, *turn).await
        }
        Some(Command::Sandbox { action }) => {
            let root = runtime::workspace(&cli.cwd)?;
            let settings = settled(&cli);
            return match action {
                SandboxAction::Try { sandbox, command } => {
                    sandbox::try_command(&root, sandbox.as_deref(), &settings, command).await
                }
            };
        }
        None => {}
    }

    let root = runtime::workspace(&cli.cwd)?;
    let settings = settled(&cli);

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
    // The reverse of the pair above: the interface asks the host about a key before acting
    // on it itself, for `ctx.ui.onTerminalInput`. Built alongside `asker`/`questions` for
    // the same reason and under the same condition — there is no terminal to read a key
    // from, and so nothing to offer, wherever there is no interface.
    let (terminal_input_asker, terminal_input_asks) = match cli.print || cli.rpc {
        true => (None, None),
        false => {
            let (asker, asks) = micro_tui::terminal_input_channel();
            (Some(asker), Some(asks))
        }
    };
    // Another reverse pair, for whatever else the interface needs from the host off its
    // render path — today, a keystroke for a `custom()` overlay that has focus.
    let (host_asker, host_asks) = match cli.print || cli.rpc {
        true => (None, None),
        false => {
            let (asker, asks) = micro_tui::host_ask_channel();
            (Some(asker), Some(asks))
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
    let thinking = cli
        .thinking
        .unwrap_or_else(|| thinking_from_settings(settings.thinking));
    let selection = Selection {
        resources: resources.clone(),
        model: settings.model.clone(),
        provider: settings.provider.clone(),
        thinking,
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
    // What an extension is told the run is, matching pi's own three: the interface, the
    // headless protocol, and one prompt run to completion and printed.
    let mode = match (cli.rpc, cli.print) {
        (true, _) => "rpc",
        (_, true) => "print",
        _ => "tui",
    };
    let told = match (cli.approve, cli.no_approve) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    };
    let trusted = project_trusted(&root, &settings, has_ui, told).await;
    // Settled before anything of the project's is loaded and before the first command can
    // be run, and settled once: the tools are built around it, and so is whatever an
    // extension asks micro to run.
    let confined = sandbox::around(
        sandbox::policy(cli.sandbox.as_deref(), &root, trusted, &settings)?,
        &root,
    );
    let mut built = runtime::build(
        &root,
        &selection,
        resume.as_deref(),
        &settings,
        trusted,
        has_ui,
        mode,
        confined.clone(),
    )
    .await?;
    // Extensions are told the session has begun, and told again when it ends, which is
    // where one that holds anything open gets to let go of it. `resources_discover` has
    // already been asked and acted on by the time `build` returns — it needs to happen
    // before skills and prompts are read, not after, so it lives inside `load_context`
    // rather than out here.
    let extensions = built.extensions.clone();
    if let Some(host) = extensions.as_ref() {
        let started = serde_json::json!({
            // A resumed run opens directly on the session it was told to; nothing before
            // this one was open in this process for `previousSessionFile` to name.
            "reason": if resume.is_some() { "resume" } else { "startup" },
        });
        let _ = host.notify("session_start", started).await;
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

        // What the model's tools section actually said, for `getSystemPromptOptions()` —
        // worked out once, here, rather than asked for again on every command a session
        // runs: what a loaded extension registered does not change over the run.
        let (tool_snippets, prompt_guidelines) =
            extensions::tool_prompt_options(&host.tools(), &built.tool_names);

        // What the pump answers questions from: what is running, and the session it runs
        // in. Filled in here because this is where both are known.
        let state = std::sync::Arc::new(tokio::sync::RwLock::new(extensions::State {
            thinking: format!("{thinking:?}").to_lowercase(),
            model: built.model.id.clone(),
            model_name: built.model.name.clone(),
            provider: built.model.provider.clone(),
            context_window: built.model.context_window,
            max_output_tokens: built.model.max_output_tokens,
            reasoning: built.model.reasoning,
            tools: built.tool_names.clone(),
            offered_tools: std::sync::Arc::clone(&built.offered_tools),
            commands: micro_commands::commands()
                .iter()
                .map(|command| command.name.to_string())
                .collect(),
            system_prompt: built.system_prompt.clone(),
            scoped_models: settings.scoped_models.clone(),
            custom_prompt: built.custom_prompt.clone(),
            appended_prompt: built.appended_prompt.clone(),
            context_files: built.context_files.clone(),
            skills: built.skills.clone(),
            tool_snippets,
            prompt_guidelines,
        }));
        tokio::spawn(extensions::serve(
            std::sync::Arc::clone(host),
            root.clone(),
            confined.clone(),
            built.broker.take().unwrap_or_else(extensions::Broker::open),
            asker.clone(),
            state,
            std::sync::Arc::clone(&built.session),
        ));
        // Spun up whenever there is a host to ask, whether or not anything has registered
        // `ctx.ui.onTerminalInput` yet: the interface itself decides whether a keystroke is
        // worth asking about — see `wants_terminal_input` — so this side only has to be
        // ready to answer when one arrives.
        if let Some(asks) = terminal_input_asks {
            tokio::spawn(extensions::serve_terminal_input(
                std::sync::Arc::clone(host),
                asks,
            ));
        }
        if let Some(asks) = host_asks {
            tokio::spawn(extensions::serve_host_asks(
                std::sync::Arc::clone(host),
                asks,
            ));
        }
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
        // Something that went wrong without stopping the run is said once, on the way
        // past, and the run goes on without it.
        for warning in &built.warnings {
            eprintln!("note: {warning}");
        }
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
                // The agent is what holds the recorder open, and nothing ran a turn, so
                // it is let go here: the writer below waits for that channel to close.
                drop(built.agent);
                if said.failed {
                    let _ = writer.await;
                    shut_down_extensions(extensions).await;
                    anyhow::bail!("{}", said.text);
                }
                println!("{}", said.text);
                Ok(())
            }
            None => headless::run(built.agent, Message::user(prompt), cli.quiet).await,
        }
    } else {
        let options = micro_tui::TuiOptions {
            cwd: root.clone(),
            model: built.model.qualified_id(),
            context_window: built.model.context_window,
            thinking,
            settings: micro_tui::Preferences::from(&settings),
            questions,
            terminal_input: terminal_input_asker,
            host_asker,
            self_framed_tools: built.self_framed_tools.clone(),
            // What extensions registered is typed and run like any other command, so the
            // menu offers it alongside the built-in ones rather than leaving a session
            // answering to something it never lists.
            extension_commands: built
                .extensions
                .as_ref()
                .map(|host| {
                    host.loaded()
                        .extensions
                        .iter()
                        .flat_map(|extension| extension.commands.iter())
                        .map(|command| micro_tui::MenuItem {
                            value: command.name.clone(),
                            description: command.description.clone(),
                            raw: None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            // Where a phone reaches this session, once `/remote` has handed it one.
            remote: Some(built.remote),
            // Without this every submitted line goes to the model, `/help` included.
            commands: Some(Box::new(built.commands)),
            // A warning joins the notice here rather than going to stderr, which the
            // interface is about to paint over.
            notice: match (built.notice, built.warnings.join("\n")) {
                (notice, said) if said.is_empty() => notice,
                (None, said) => Some(said),
                (Some(notice), said) => Some(format!("{notice}\n{said}")),
            },
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
    // Waiting for it guarantees every message reached the log before the process exits —
    // and, because the extensions are watching the same run, that everything the agent
    // reported has reached them too before the host holding them is let go.
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
async fn shut_down_extensions(extensions: Option<std::sync::Arc<micro_extensions::Host>>) {
    let Some(host) = extensions else {
        return;
    };
    // Not conditional on being the last holder: the pump, the hooks and the command
    // runner all keep one, and the host has to be told either way or it is killed with
    // the process, mid-sentence.
    host.shutdown("quit").await;
}

/// Run a slash command with nobody watching, and say what it printed.
///
/// `None` means the line was not a command, and belongs to the model. Only commands whose
/// whole answer is text can run here: anything that would open a picker or change the
/// running conversation needs an interface to change.
use micro_tui::Commands as _;

/// What a headless slash command answered, and whether it should end the run the way an
/// uncaught error would. Carried together rather than as a lone `bool` at the call site,
/// which would read as nothing more than "the second thing `run_command_headlessly` hands
/// back."
struct HeadlessCommand {
    text: String,
    /// Set for a command that answered by erroring — `/nonsense`, or an extension's own
    /// handler that threw before it could write anything else — the same command outcome
    /// [`micro_commands::CommandOutcome::error`] gives a raised handler that never reaches
    /// `CliCommands::extension_command`'s `Ok` arm. pi's own print mode exits nonzero for a
    /// command that raised rather than answered; this is how `--print` matches it.
    failed: bool,
}

async fn run_command_headlessly(commands: &mut commands::CliCommands, line: &str) -> Option<HeadlessCommand> {
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
        return Some(HeadlessCommand {
            text: text.to_string(),
            failed: outcome.is_error(),
        });
    }

    // Whatever the command settled on, said the way it would be said on screen. A run
    // without an interface applies nothing else: there is no agent to hand a new model
    // to and no scrollback to rebuild. None of these settle into an error outcome — that
    // path always answers through `outcome.text()` above instead.
    let note = match commands.apply(outcome).await {
        micro_tui::Applied::Note { text, .. } => Some(text),
        micro_tui::Applied::Conversation { note, .. } => note,
        micro_tui::Applied::SystemPrompt { note, .. } => note,
        micro_tui::Applied::Model { note, .. } => note,
        micro_tui::Applied::Nothing => None,
    };
    note.map(|text| HeadlessCommand { text, failed: false })
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
            "print",
            "rpc",
            "model",
            "provider",
            "thinking",
            "cwd",
            "resume",
            "continue",
            "quiet",
            "tools",
            "exclude-tools",
            "tui-mode",
            "approve",
            "no-approve",
            "skill",
            "no-skills",
            "extension",
            "no-extensions",
            "prompt-template",
            "no-prompt-templates",
            "no-context-files",
            "theme",
            "sandbox",
            "budget",
        ] {
            assert!(known(flag), "`--{flag}` is not recognised as micro's own");
        }

        // A flag that carries a value has to be in the other list, or the value after it
        // is read as the prompt.
        for flag in [
            "model",
            "cwd",
            "skill",
            "extension",
            "theme",
            "tui-mode",
            "budget",
        ] {
            assert!(
                valued.iter().any(|known| known == flag),
                "`--{flag}` takes a value"
            );
        }
        for flag in ["print", "rpc", "no-skills", "no-extensions"] {
            assert!(
                switches.iter().any(|known| known == flag),
                "`--{flag}` takes none"
            );
        }

        // Subcommands declare flags too, and they are held to the same rule.
        assert!(known("local") && known("live") && known("overwrite"));
        assert!(known("raw") && valued.iter().any(|known| known == "turn"));
        assert!(valued.iter().any(|known| known == "diff"));
    }
}
