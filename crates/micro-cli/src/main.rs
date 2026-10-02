//! Entry point.

mod access;
mod archive;
mod bug_report;
mod capabilities;
mod codemode;
mod codemode_models;
mod commands;
mod default_tools;
mod extension_broker;
mod extensions;
mod headless;
mod llama;
mod mcp;
mod model_registry;
mod remote;
mod runtime;
mod sandbox;
mod share;
mod subcommands;
mod update;
mod virtual_models;

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
    /// The prompt to run.
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

    /// Workspace root.
    #[arg(short = 'C', long, default_value = ".")]
    cwd: PathBuf,

    /// Resume a saved session by id.
    #[arg(long, value_name = "ID")]
    resume: Option<String>,

    /// Resume the most recent session for this workspace.
    #[arg(long = "continue", conflicts_with = "resume")]
    continue_latest: bool,

    /// Resume this workspace's session with exactly this id, or start one under it.
    #[arg(
        long = "session-id",
        value_name = "ID",
        conflicts_with_all = ["resume", "continue_latest"]
    )]
    session_id: Option<String>,

    /// Name the session from the start.
    #[arg(short = 'n', long = "name", value_name = "NAME")]
    name: Option<String>,

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

    /// Stop this session once it has spent this many dollars.
    #[arg(long = "budget", value_name = "AMOUNT")]
    budget: Option<f64>,

    /// Set one config value for this run: `-c theme=dracula`, `-c show_images=false`.
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
        /// Which models to show: chat, image or classifier.
        #[arg(long = "type", value_name = "TYPE", default_value = "chat")]
        kind: String,
    },
    /// Install an extension package.
    Install {
        /// npm:name, a repository URL, or a path.
        source: String,

        #[arg(short, long)]
        local: bool,
    },
    /// Remove an installed extension package.
    #[command(alias = "uninstall")]
    Remove {
        source: String,

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
        /// The session to bill.
        session: String,
        /// Show what one turn added to the bill, and why.
        #[arg(long = "diff", value_name = "TURN")]
        diff: Option<u64>,
    },
    /// Say why a turn paid for a prompt the provider already had.
    WhyMiss {
        /// The session to explain.
        session: String,
        /// Which turn to compare with its parent turn.
        turn: u64,
    },
    /// Try the sandbox out.
    Sandbox {
        #[command(subcommand)]
        action: SandboxAction,
    },
    /// Check the latest release and update this managed installation.
    Update,
    /// Configure MCP servers, check them, and sign in to them.
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Connect to a llama.cpp router, and manage the models it serves.
    Llama {
        #[command(subcommand)]
        action: LlamaAction,
    },
}

#[derive(Subcommand)]
enum LlamaAction {
    /// Remember which router to use, after checking it answers.
    Connect {
        /// The router's address; defaults to http://127.0.0.1:8080.
        url: Option<String>,
        /// The key the router was started with, if it was started with --api-key.
        #[arg(long = "api-key", value_name = "KEY")]
        api_key: Option<String>,
    },
    /// List the router's models and the state each is in.
    Status,
    /// Search Hugging Face for GGUF models, or list the quantizations of `owner/repository`.
    Search { query: String },
    /// Have the router download `owner/repository[:quant]` from Hugging Face.
    Download { model: String },
    /// Load a model, waiting until it is serving.
    Load {
        model: String,
        /// Unload every other loaded model first.
        #[arg(long)]
        unload_others: bool,
    },
    /// Unload a model.
    Unload { model: String },
}

#[derive(clap::Args)]
struct McpAddArgs {
    name: String,
    /// The streamable HTTP endpoint, instead of a command.
    #[arg(long)]
    url: Option<String>,
    /// An HTTP header, as NAME=VALUE (repeatable).
    #[arg(long = "header", value_name = "NAME=VALUE")]
    headers: Vec<String>,
    /// Send `Authorization: Bearer ${NAME}`.
    #[arg(long, value_name = "NAME")]
    bearer_token_env_var: Option<String>,
    /// An environment variable for a stdio server, as NAME=VALUE (repeatable).
    #[arg(long = "env", value_name = "NAME=VALUE")]
    env: Vec<String>,
    /// Where a stdio server runs.
    #[arg(long)]
    cwd: Option<String>,
    /// What the server offers, in a sentence, for the system prompt.
    #[arg(long)]
    description: Option<String>,
    /// `direct`, `codemode` (the default), `deferred`, or `hidden`.
    #[arg(long)]
    exposure: Option<String>,
    /// A client registered with the authorization server ahead of time.
    #[arg(long)]
    oauth_client_id: Option<String>,
    /// Its secret; may be `${NAME}` or `!command`.
    #[arg(long)]
    oauth_client_secret: Option<String>,
    /// The fixed loopback port that client was registered with.
    #[arg(long)]
    oauth_callback_port: Option<u16>,
    /// The client name micro registers under.
    #[arg(long)]
    oauth_client_name: Option<String>,
    /// Send this provider's micro credential instead of signing in (global file only).
    #[arg(long, value_name = "PROVIDER")]
    auth_provider: Option<String>,
    /// Write the project's .micro/mcp.json instead of the global file.
    #[arg(short, long)]
    local: bool,
    /// The program and its arguments, after `--`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

#[derive(Subcommand)]
enum McpAction {
    /// Add or replace a server: `-- <command> [args...]` for stdio, `--url` for HTTP.
    Add(Box<McpAddArgs>),
    /// Remove a server.
    Remove {
        name: String,
        #[arg(short, long)]
        local: bool,
    },
    /// Connect every server and show its state and tools; exits 1 when one fails.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Sign in to a server through the browser.
    Login {
        name: String,
        /// Seconds to wait for the browser.
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Forget a server's stored sign-in.
    Logout { name: String },
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
    Login {
        provider: String,
        /// How to sign in: oauth, api_key, browser, copy_code or device_code.
        #[arg(long, value_name = "METHOD")]
        method: Option<String>,
    },
    /// Remove a stored credential.
    Logout { provider: String },
    /// Show which providers are configured.
    Status,
    /// Check that a provider's or a model's credential resolves; prints ready, not_ready or
    /// invalid and exits 0, 1 or 2.
    Check {
        #[command(flatten)]
        target: AuthTargetArgs,
        /// Write the result as JSON.
        #[arg(long)]
        json: bool,
        /// Emit the resolved credential when ready.
        #[arg(long)]
        credentials: bool,
        /// Leave an expired OAuth credential as it is instead of refreshing it.
        #[arg(long = "no-refresh")]
        no_refresh: bool,
    },
    /// Print the API key a provider resolves to.
    PrintApiKey {
        #[command(flatten)]
        target: AuthTargetArgs,
    },
    /// Print a provider's OAuth bearer token, refreshed first when it would lapse too soon.
    PrintBearerToken {
        #[command(flatten)]
        target: AuthTargetArgs,
        /// How long the token must stay valid, such as 30m or 1h.
        #[arg(long = "min-expiry", value_name = "DURATION")]
        min_expiry: Option<String>,
    },
}

/// Which credential an auth command is about: a provider, a model, or a name that is either.
#[derive(clap::Args)]
struct AuthTargetArgs {
    /// A provider, or a model whose provider is meant.
    target: Option<String>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
}

impl AuthTargetArgs {
    fn target(&self) -> subcommands::AuthTarget {
        subcommands::AuthTarget {
            name: self.target.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
        }
    }
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
async fn project_trusted(
    root: &std::path::Path,
    settings: &micro_config::Settings,
    has_ui: bool,
    told: Option<bool>,
) -> bool {
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

/// Every long flag micro itself declares for these arguments: those that stand alone, and those
/// that take a value. A subcommand's flags count only when the arguments name the subcommand, so
/// an extension may declare a flag a subcommand also has.
fn own_flags(arguments: &[String]) -> (Vec<String>, Vec<String>) {
    use clap::CommandFactory;

    let mut switches = Vec::new();
    let mut valued = Vec::new();

    fn walk(
        command: &clap::Command,
        arguments: &[String],
        switches: &mut Vec<String>,
        valued: &mut Vec<String>,
    ) {
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
            let invoked = std::iter::once(inner.get_name())
                .chain(inner.get_all_aliases())
                .any(|name| arguments.iter().any(|argument| argument == name));
            if invoked {
                walk(inner, arguments, switches, valued);
            }
        }
    }

    walk(&Cli::command(), arguments, &mut switches, &mut valued);

    switches.push("help".to_string());
    switches.push("version".to_string());
    (switches, valued)
}

/// What a user settled once and left alone.
fn settled(cli: &Cli) -> micro_config::Settings {
    let mut settings = micro_config::Config::load_with(&cli.config_override)
        .and_then(|config| {
            config.resolve_from_env(&micro_config::Overrides {
                model: cli.model.clone(),
                provider: cli.provider.clone(),
                theme: cli.theme.clone(),
                ..micro_config::Overrides::default()
            })
        })
        .unwrap_or_else(|error| {
            if matches!(error, micro_config::ConfigError::Override { .. }) {
                eprintln!("micro: {error}");
                std::process::exit(2);
            }
            eprintln!("note: {error}; using defaults");
            micro_config::Settings::default()
        });

    if let Some(budget) = cli.budget {
        settings.budget = budget.max(0.0);
    }
    settings
}

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut arguments = std::env::args();
        let program = arguments.next();
        if program.is_some() && arguments.next().as_deref() == Some(micro_sandbox::HELPER_ARG) {
            micro_sandbox::run_linux_helper(arguments);
        }
    }

    let arguments: Vec<String> = std::env::args().collect();
    let (switches, valued) = own_flags(&arguments);
    let (mine, given) = micro_extensions::split_unknown(
        arguments,
        &switches.iter().map(String::as_str).collect::<Vec<_>>(),
        &valued.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let cli = Cli::parse_from(mine);
    micro_config::apply_http_proxy(
        micro_config::Config::load()
            .ok()
            .and_then(|config| config.http_proxy)
            .as_deref(),
    );

    match &cli.command {
        Some(Command::Auth { action }) => {
            return match action {
                AuthAction::Login { provider, method } => {
                    subcommands::auth_login(provider, method.as_deref()).await
                }
                AuthAction::Logout { provider } => subcommands::auth_logout(provider).await,
                AuthAction::Status => subcommands::auth_status().await,
                AuthAction::Check {
                    target,
                    json,
                    credentials,
                    no_refresh,
                } => {
                    let code = subcommands::auth_check(
                        &target.target(),
                        subcommands::CheckOptions {
                            json: *json,
                            credentials: *credentials,
                            refresh: !*no_refresh,
                        },
                    )
                    .await;
                    std::process::exit(code)
                }
                AuthAction::PrintApiKey { target } => {
                    subcommands::auth_print_api_key(&target.target()).await
                }
                AuthAction::PrintBearerToken { target, min_expiry } => {
                    subcommands::auth_print_bearer_token(&target.target(), min_expiry.as_deref())
                        .await
                }
            }
        }
        Some(Command::Models { query, live, kind }) => {
            return subcommands::models(query.as_deref(), *live, kind).await
        }
        Some(Command::Llama { action }) => {
            return match action {
                LlamaAction::Connect { url, api_key } => {
                    llama::connect(url.as_deref(), api_key.as_deref()).await
                }
                LlamaAction::Status => llama::status().await,
                LlamaAction::Search { query } => llama::search(query).await,
                LlamaAction::Download { model } => llama::download(model).await,
                LlamaAction::Load {
                    model,
                    unload_others,
                } => llama::load(model, *unload_others).await,
                LlamaAction::Unload { model } => llama::unload(model).await,
            };
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
            return subcommands::bill(session, *diff).await;
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
        Some(Command::Mcp { action }) => {
            let root = runtime::workspace(&cli.cwd)?;
            return match action {
                McpAction::Add(add) => {
                    let options = mcp::AddOptions {
                        url: add.url.clone(),
                        headers: add.headers.clone(),
                        bearer_token_env_var: add.bearer_token_env_var.clone(),
                        env: add.env.clone(),
                        cwd: add.cwd.clone(),
                        description: add.description.clone(),
                        exposure: add.exposure.clone(),
                        oauth_client_id: add.oauth_client_id.clone(),
                        oauth_client_secret: add.oauth_client_secret.clone(),
                        oauth_callback_port: add.oauth_callback_port,
                        oauth_client_name: add.oauth_client_name.clone(),
                        auth_provider: add.auth_provider.clone(),
                        command: add.command.clone(),
                    };
                    mcp::add(&root, &add.name, add.local, &options).await
                }
                McpAction::Remove { name, local } => mcp::remove(&root, name, *local).await,
                McpAction::List { json } => mcp::list(&root, *json).await,
                McpAction::Login { name, timeout } => mcp::login(&root, name, *timeout).await,
                McpAction::Logout { name } => mcp::logout(&root, name).await,
            };
        }
        Some(Command::Update) => {
            return match update::update_now().await? {
                update::Outcome::Current { version } => {
                    println!("micro {version} is already the latest version.");
                    Ok(())
                }
                update::Outcome::Installed {
                    previous_version,
                    version,
                    ..
                } => {
                    println!("Updated micro {previous_version} to {version}.");
                    Ok(())
                }
                update::Outcome::Skipped { reason } => {
                    anyhow::bail!("Cannot update micro: {reason}")
                }
            };
        }
        None => {}
    }

    let root = runtime::workspace(&cli.cwd)?;
    let settings = settled(&cli);

    let arguments: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some(launcher) = update::automatic(
        &arguments,
        settings.auto_update,
        settings.update_check_interval_hours,
    )
    .await
    {
        std::process::exit(update::restart(&launcher, &arguments[1..])?);
    }

    let (asker, questions) = match cli.print || cli.rpc {
        true => (None, None),
        false => {
            let (asker, requests) = micro_tui::ui_channel();
            (Some(asker), Some(requests))
        }
    };

    let (terminal_input_asker, terminal_input_asks) = match cli.print || cli.rpc {
        true => (None, None),
        false => {
            let (asker, asks) = micro_tui::terminal_input_channel();
            (Some(asker), Some(asks))
        }
    };

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

    let opening = runtime::Opening {
        resume: match (&cli.resume, cli.continue_latest) {
            (Some(id), _) => Some(id.clone()),
            (None, true) => Some(subcommands::latest_session(&root).await?),
            (None, false) => None,
        },
        session_id: cli.session_id.clone(),
        name: cli.name.clone(),
    };

    let has_ui = !cli.print && !cli.rpc;

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

    let confined = sandbox::around(
        sandbox::policy(cli.sandbox.as_deref(), &root, trusted, &settings)?,
        &root,
    );
    let mut built = runtime::build(
        &root,
        &selection,
        &opening,
        &settings,
        trusted,
        has_ui,
        mode,
        confined.clone(),
        asker.as_ref().map(|asker| {
            std::sync::Arc::new(access::TerminalAccessApprover::new(asker.clone()))
                as std::sync::Arc<dyn micro_tools::AccessApprover>
        }),
        cli.sandbox.is_some(),
    )
    .await?;

    if let Some(asker) = &asker {
        built.commands.set_notifier(asker.clone());
    }

    let extensions = built.extensions.clone();
    if let Some(host) = extensions.as_ref() {
        let started = serde_json::json!({

            "reason": if built.resumed { "resume" } else { "startup" },
        });
        let _ = host
            .share_models(model_registry::catalog(Some(&built.models))["models"].take())
            .await;
        let _ = host.notify("session_start", started).await;
    }

    if let Some(host) = extensions.as_ref() {
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

        let (tool_snippets, prompt_guidelines) =
            extensions::tool_prompt_options(&host.tools(), &built.tool_names);

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

            all_tools: extensions::all_tools(
                &host.loaded().extensions,
                &built.tool_definitions,
                &built.tool_names,
            ),
            all_commands: extensions::all_commands(&host.loaded().extensions),
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
            models: Some(built.models.clone()),
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
    let writer = runtime::persist(built.session, built.recorder);
    let forwarder = built.forwarder;
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

        drop(rpc);
        finish_forwarder(forwarder).await;
        writer.finish().await;
        shut_down_extensions(extensions).await;
        return outcome;
    }

    let result = if cli.print {
        for warning in &built.warnings {
            eprintln!("note: {warning}");
        }
        if prompt.trim().is_empty() {
            anyhow::bail!("--print needs a prompt");
        }

        if let Some(notice) = &built.notice {
            drop(built.agent);
            finish_forwarder(forwarder).await;
            writer.finish().await;
            shut_down_extensions(extensions).await;
            anyhow::bail!("{notice}");
        }

        let prompt = match built.commands.submitted(prompt).await {
            Some(prompt) => prompt,
            None => {
                drop(built.agent);
                finish_forwarder(forwarder).await;
                writer.finish().await;
                shut_down_extensions(extensions).await;
                return Ok(());
            }
        };

        match run_command_headlessly(&mut built.commands, &prompt).await {
            Some(said) => {
                drop(built.agent);
                if said.failed {
                    writer.finish().await;
                    finish_forwarder(forwarder).await;
                    shut_down_extensions(extensions).await;
                    anyhow::bail!("{}", said.text);
                }
                println!("{}", said.text);
                Ok(())
            }
            None => headless::run(built.agent, Message::user(prompt), cli.quiet).await,
        }
    } else {
        let initial_observability =
            micro_tui::Commands::session_observability(&mut built.commands).await;
        let mut command_menu = built
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
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for skill in &built.skills {
            if micro_commands::find(&skill.name).is_none()
                && !command_menu.iter().any(|item| item.value == skill.name)
            {
                command_menu.push(micro_tui::MenuItem {
                    value: skill.name.clone(),
                    description: format!("skill — {}", skill.description),
                    raw: None,
                });
            }
        }

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

            extension_commands: command_menu,

            remote: Some(built.remote),

            commands: Some(Box::new(built.commands)),

            notice: match (built.notice, built.warnings.join("\n")) {
                (notice, said) if said.is_empty() => notice,
                (None, said) => Some(said),
                (Some(notice), said) => Some(format!("{notice}\n{said}")),
            },
            provider: built.model.provider.clone(),
            subscription: built.subscription,
            auto_compact: settings.auto_compact,
            price: Some(built.model.cost.clone()),
            session_cost: initial_observability.and_then(|observed| observed.0),
            session_usage: initial_observability.map(|observed| (observed.1, observed.2)),
            experimental: micro_config::experimental_enabled(),

            theme: cli.theme.as_deref().and_then(micro_tui::Theme::named),
            resources: built.resources,

            tui_mode: match cli.tui_mode.unwrap_or(settings.tui_mode) {
                micro_config::TuiMode::Regular => micro_tui::TuiMode::Inline,
                micro_config::TuiMode::Fullscreen => micro_tui::TuiMode::Fullscreen,
            },
        };

        micro_tui::run_with(built.agent, built.history, options)
            .await
            .map(|_| ())
    };

    finish_forwarder(forwarder).await;
    writer.finish().await;
    if !cli.print && result.is_ok() {
        let held = session.lock().await;
        if held.is_saved() {
            say_how_to_resume(held.id());
        }
    }
    shut_down_extensions(extensions).await;
    result
}

/// Leave the line that brings this conversation back.
fn say_how_to_resume(session_id: &str) {
    use std::io::IsTerminal;
    if session_id.is_empty() || !std::io::stdout().is_terminal() {
        return;
    }
    println!("To resume this session: micro --resume {session_id}");
}

async fn shut_down_extensions(extensions: Option<std::sync::Arc<micro_extensions::Host>>) {
    let Some(host) = extensions else {
        return;
    };

    host.shutdown("quit").await;
}

/// Let lifecycle notifications already sent by the agent reach observers without allowing a
/// stopped observer to hold process shutdown open indefinitely.
async fn finish_forwarder(mut forwarder: tokio::task::JoinHandle<()>) {
    if tokio::time::timeout(std::time::Duration::from_secs(2), &mut forwarder)
        .await
        .is_err()
    {
        forwarder.abort();
        let _ = forwarder.await;
    }
}

/// Run a slash command with nobody watching, and say what it printed.
use micro_tui::Commands as _;

/// What a headless slash command answered, and whether it should end the run the way an uncaught
/// error would.
struct HeadlessCommand {
    text: String,
    /// Set for a command that answered by erroring.
    failed: bool,
}

async fn run_command_headlessly(
    commands: &mut commands::CliCommands,
    line: &str,
) -> Option<HeadlessCommand> {
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

    let note = match commands.apply(outcome).await {
        micro_tui::Applied::Note { text, .. } => Some(text),
        micro_tui::Applied::Conversation { note, .. } => note,
        micro_tui::Applied::SystemPrompt { note, .. } => note,
        micro_tui::Applied::Model { note, .. } => note,
        micro_tui::Applied::RunGrantedCommand { command } => {
            Some(format!("Approved command: {command}"))
        }
        micro_tui::Applied::Nothing => None,
    };
    note.map(|text| HeadlessCommand {
        text,
        failed: false,
    })
}

#[cfg(test)]
mod flag_tests {
    use super::*;

    #[test]
    fn every_flag_micro_declares_is_known_to_be_its_own() {
        let invoked: Vec<String> = ["micro", "install", "models", "sessions", "show", "bill"]
            .iter()
            .map(|argument| argument.to_string())
            .collect();
        let (switches, valued) = own_flags(&invoked);
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

        for flag in ["local", "live", "raw"] {
            assert!(known(flag), "`--{flag}` is declared on a subcommand");
        }
        for flag in ["turn", "diff"] {
            assert!(
                valued.iter().any(|known| known == flag),
                "`--{flag}` takes a value on a subcommand"
            );
        }
    }

    /// An extension may declare a flag that only a subcommand also declares.
    #[test]
    fn a_subcommand_flag_belongs_to_micro_only_when_the_subcommand_runs() {
        let session = vec!["micro".to_string(), "--env=staging".to_string()];
        let (switches, valued) = own_flags(&session);
        assert!(!valued.iter().any(|flag| flag == "env"));
        assert!(!switches.iter().any(|flag| flag == "env"));

        let adding: Vec<String> = ["micro", "mcp", "add", "x", "--env", "A=b"]
            .iter()
            .map(|argument| argument.to_string())
            .collect();
        let (_, valued) = own_flags(&adding);
        assert!(valued.iter().any(|flag| flag == "env"));
    }
}
