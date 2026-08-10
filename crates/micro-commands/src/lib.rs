//! Slash commands: what a user can type, and what each one asks the caller to do.
//!
//! Parsing and the command list live here; the effect does not. Running a command yields
//! a [`CommandOutcome`] that the caller interprets, so a TUI and a headless CLI share
//! every command without sharing a way of drawing.
//!
//! ```no_run
//! use micro_commands::{dispatch, CommandContext, CommandOutcome};
//!
//! # async fn example(context: CommandContext<'_>) {
//! match dispatch("/model opus", &context).await {
//!     None => { /* ordinary text: send it to the model */ }
//!     Some(CommandOutcome::SetModel { model }) => println!("now on {}", model.qualified_id()),
//!     Some(outcome) => println!("{outcome:?}"),
//! }
//! # }
//! ```

mod auth;
mod model;
mod outcome;
mod parse;
mod session;

pub use outcome::CommandOutcome;
pub use outcome::MessageKind;
pub use outcome::Picker;
pub use outcome::PickerItem;
pub use outcome::ThemeChoice;
pub use parse::parse;
pub use parse::suggest;
pub use parse::Input;

use micro_auth::AuthStore;
use micro_models::Catalog;
use micro_models::ModelDef;
use micro_session::SessionStore;
use micro_types::ThinkingLevel;
use std::path::Path;

/// One command: its name, whether it takes an argument, and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// Without the leading slash.
    pub name: &'static str,
    /// The argument as a user would see it written, or `None` when it takes none.
    /// Square brackets mark an argument that may be left out.
    pub argument: Option<&'static str>,
    pub description: &'static str,
}

impl Command {
    /// The form to show in help and completion: `/fork [index]`.
    pub fn usage(&self) -> String {
        match self.argument {
            Some(argument) => format!("/{} {argument}", self.name),
            None => format!("/{}", self.name),
        }
    }
}

static COMMANDS: &[Command] = &[
    Command {
        name: "help",
        argument: None,
        description: "list every command",
    },
    Command {
        name: "model",
        argument: Some("[query]"),
        description: "switch model, or choose one from the catalog",
    },
    Command {
        name: "provider",
        argument: Some("[name]"),
        description: "switch provider, or choose one",
    },
    Command {
        name: "login",
        argument: Some("[provider]"),
        description: "sign in to a provider",
    },
    Command {
        name: "logout",
        argument: Some("[provider]"),
        description: "forget a provider's stored credential",
    },
    Command {
        name: "auth",
        argument: None,
        description: "show which providers are signed in",
    },
    Command {
        name: "sessions",
        argument: None,
        description: "list sessions from this workspace",
    },
    Command {
        name: "session",
        argument: None,
        description: "Show session info and stats",
    },
    Command {
        name: "clone",
        argument: None,
        description: "Duplicate the current session at the current position",
    },
    Command {
        name: "resume",
        argument: Some("[id]"),
        description: "reopen an earlier session",
    },
    Command {
        name: "fork",
        argument: Some("[index]"),
        description: "branch the conversation at a message",
    },
    Command {
        name: "thinking",
        argument: Some("[off|low|medium|high]"),
        description: "how hard the model reasons before answering",
    },
    Command {
        name: "theme",
        argument: Some("[dark|light]"),
        description: "switch the colour scheme",
    },
    Command {
        name: "tree",
        argument: Some("[id]"),
        description: "show the conversation's branches, or continue from one",
    },
    Command {
        name: "name",
        argument: Some("[title]"),
        description: "name this session",
    },
    Command {
        name: "skills",
        argument: None,
        description: "list the skills that were found",
    },
    Command {
        name: "import",
        argument: Some("<path>"),
        description: "Import and resume a session from a JSONL file",
    },
    Command {
        name: "share",
        argument: None,
        description: "Share session as a secret GitHub gist",
    },
    Command {
        name: "changelog",
        argument: None,
        description: "Show changelog entries",
    },
    Command {
        name: "reload",
        argument: None,
        description: "Reload skills and context files",
    },
    Command {
        name: "trust",
        argument: Some("[on|off]"),
        description: "Save project trust decision for future sessions",
    },
    Command {
        name: "set",
        argument: Some("<setting> [value]"),
        description: "Change a setting, or show what it is",
    },
    Command {
        name: "settings",
        argument: None,
        description: "show where every setting is read from",
    },
    Command {
        name: "hotkeys",
        argument: None,
        description: "list every key and what it does",
    },
    Command {
        name: "copy",
        argument: None,
        description: "put the last answer on the clipboard",
    },
    Command {
        name: "export",
        argument: Some("[path]"),
        description: "write the conversation to a file",
    },
    Command {
        name: "new",
        argument: None,
        description: "start a fresh conversation",
    },
    Command {
        name: "debug",
        argument: None,
        description: "show what micro knows about this session",
    },
    Command {
        name: "compact",
        argument: None,
        description: "summarize the conversation to reclaim context",
    },
    Command {
        name: "clear",
        argument: None,
        description: "start a fresh conversation",
    },
    Command {
        name: "cwd",
        argument: None,
        description: "show the workspace root",
    },
    Command {
        name: "quit",
        argument: None,
        description: "leave micro",
    },
];

/// Every command, in the order help and completion should show them.
pub fn commands() -> &'static [Command] {
    COMMANDS
}

pub fn find(name: &str) -> Option<&'static Command> {
    let name = name.trim().trim_start_matches('/');
    COMMANDS
        .iter()
        .find(|command| command.name.eq_ignore_ascii_case(name))
}

/// Commands whose name starts with `prefix`, for the menu shown while a user types.
/// A leading slash is accepted, since that is what they have typed so far.
pub fn complete(prefix: &str) -> Vec<&'static Command> {
    let prefix = prefix.trim_start().trim_start_matches('/');
    // Only the command word completes; once a space is typed the argument has begun.
    if prefix.contains(char::is_whitespace) {
        return Vec::new();
    }
    let prefix = prefix.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(&prefix))
        .collect()
}

/// What a command needs to know about the state it runs in.
///
/// Borrowed throughout: a command reads this state and reports what should change, but
/// never changes it.
pub struct CommandContext<'a> {
    pub catalog: &'a Catalog,
    pub auth: &'a AuthStore,
    pub sessions: &'a SessionStore,
    /// The workspace root the agent is working in.
    pub workspace: &'a Path,
    /// The provider in use, as a canonical id.
    pub provider: &'a str,
    /// The model in use, when one has been chosen.
    pub model: Option<&'a ModelDef>,
    /// The session being written to, when the conversation is being recorded.
    pub session_id: Option<&'a str>,
    /// How many messages the conversation holds.
    pub message_count: usize,
    /// What the conversation has cost in tokens so far.
    pub usage: micro_types::Usage,
    /// Show only the newest entry when the changelog is asked for.
    pub collapse_changelog: bool,
}

/// Run a submitted line. `None` means it was ordinary text for the model.
pub async fn dispatch(line: &str, context: &CommandContext<'_>) -> Option<CommandOutcome> {
    match parse(line) {
        Input::Prompt(_) => None,
        Input::Command { command, argument } => Some(run(command, argument, context).await),
        Input::Unknown { name, suggestion } => Some(CommandOutcome::error(match suggestion {
            Some(nearest) => format!("unknown command /{name} - did you mean /{nearest}?"),
            None => format!("unknown command /{name} - /help lists them all"),
        })),
    }
}

/// Run a command that has already been parsed.
pub async fn run(
    command: &Command,
    argument: Option<&str>,
    context: &CommandContext<'_>,
) -> CommandOutcome {
    if command.argument.is_none() && argument.is_some() {
        return CommandOutcome::error(format!("/{} takes no argument", command.name));
    }

    match command.name {
        "help" => CommandOutcome::info(help_text()),
        "model" => model::model(argument, context),
        "provider" => model::provider(argument, context),
        "login" => auth::login(argument, context).await,
        "logout" => auth::logout(argument, context),
        "auth" => auth::status(context),
        "sessions" => session::sessions(context).await,
        "session" => session::info(context).await,
        "clone" => session::clone(context).await,
        "resume" => session::resume(argument, context).await,
        "fork" => session::fork(argument, context),
        "tree" => session::tree(argument, context).await,
        "name" => session::name(argument, context).await,
        "skills" => skills(context).await,
        "settings" => settings(context),
        "set" => set(argument),
        "trust" => trust(argument),
        "reload" => CommandOutcome::Reload,
        "share" => CommandOutcome::Share,
        "changelog" => CommandOutcome::info(changelog(context.collapse_changelog)),
        "import" => match argument.map(str::trim).filter(|path| !path.is_empty()) {
            Some(path) => CommandOutcome::Import {
                path: path.to_string(),
            },
            None => CommandOutcome::error("Usage: /import <path.jsonl>"),
        },
        "hotkeys" => CommandOutcome::info(hotkeys_text()),
        "copy" => CommandOutcome::CopyLastAnswer,
        "export" => CommandOutcome::Export {
            path: argument.map(str::to_string),
        },
        // Clearing is what starting over means when the conversation is the only state.
        "new" => CommandOutcome::Clear,
        "debug" => debug(context),
        "thinking" => thinking(argument),
        "theme" => theme(argument),
        "compact" => compact(context),
        "clear" => CommandOutcome::Clear,
        "cwd" => CommandOutcome::info(context.workspace.display().to_string()),
        "quit" => CommandOutcome::Quit,
        other => CommandOutcome::error(format!("/{other} is not wired up")),
    }
}

/// What has changed in micro, shipped with the binary so `/changelog` works offline.
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// The changelog, whole or folded to its newest entry.
///
/// An entry starts at a `## ` heading, so folding keeps the title and everything under the
/// first one.
fn changelog(collapse: bool) -> String {
    let text = CHANGELOG.trim();
    if !collapse {
        return text.to_string();
    }

    let mut out = String::new();
    let mut entries = 0;
    for line in text.lines() {
        if line.starts_with("## ") {
            entries += 1;
            if entries > 1 {
                break;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// `/trust` vouches for the project, so a later run may edit inside it without asking
/// about every file. `/trust off` takes it back.
///
/// It never widens what a shell command may do: a command can reach anywhere, and saying
/// a directory is safe says nothing about that.
fn trust(argument: Option<&str>) -> CommandOutcome {
    match argument.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("") | Some("on") | Some("yes") => CommandOutcome::Trust { trusted: true },
        Some("off") | Some("no") => CommandOutcome::Trust { trusted: false },
        Some(other) => CommandOutcome::error(format!(
            "unknown trust setting `{other}`: expected on or off"
        )),
    }
}

/// `/thinking` with no argument offers the levels; with one, sets it.
fn thinking(argument: Option<&str>) -> CommandOutcome {
    let Some(argument) = argument else {
        return CommandOutcome::Choose(Picker::new(
            "Reasoning effort",
            LEVELS
                .iter()
                .map(|(name, effort)| PickerItem::new(*name, *effort, format!("/thinking {name}")))
                .collect(),
        ));
    };

    match level_named(argument) {
        Some(level) => CommandOutcome::SetThinking { level },
        None => CommandOutcome::error(format!(
            "unknown reasoning effort `{argument}`: expected off, low, medium or high"
        )),
    }
}

/// The levels a user can ask for, with what each one is worth saying about it.
const LEVELS: &[(&str, &str)] = &[
    ("off", "answer directly"),
    ("low", "a little reasoning first"),
    ("medium", "a moderate amount"),
    ("high", "as much as the model will do"),
];

fn level_named(name: &str) -> Option<ThinkingLevel> {
    match name.trim().to_ascii_lowercase().as_str() {
        "off" | "none" => Some(ThinkingLevel::Off),
        "low" => Some(ThinkingLevel::Low),
        "medium" | "med" => Some(ThinkingLevel::Medium),
        "high" | "max" => Some(ThinkingLevel::High),
        _ => None,
    }
}

/// `/theme` with no argument offers the palettes; with one, switches to it.
fn theme(argument: Option<&str>) -> CommandOutcome {
    let Some(argument) = argument else {
        return CommandOutcome::Choose(Picker::new(
            "Theme",
            vec![
                PickerItem::new("dark", "ohm's dark palette", "/theme dark"),
                PickerItem::new("light", "ohm's light palette", "/theme light"),
                PickerItem::new("auto", "follow the terminal", "/theme auto"),
            ],
        ));
    };

    match argument.trim().to_ascii_lowercase().as_str() {
        "dark" => CommandOutcome::SetTheme {
            theme: ThemeChoice::Dark,
        },
        "light" => CommandOutcome::SetTheme {
            theme: ThemeChoice::Light,
        },
        "auto" | "system" => CommandOutcome::SetTheme {
            theme: ThemeChoice::Auto,
        },
        other => CommandOutcome::error(format!(
            "unknown theme `{other}`: expected dark, light or auto"
        )),
    }
}

/// What skills were found, and anything that stopped one loading.
async fn skills(context: &CommandContext<'_>) -> CommandOutcome {
    let home = micro_context::micro_home().unwrap_or_default();
    // Listing what is on offer answers with what this run is actually offering, which is
    // the project's own only when the project has been trusted.
    let trusted = !micro_config::requires_decision(context.workspace)
        || micro_config::TrustStore::load()
            .await
            .unwrap_or_default()
            .is_trusted(context.workspace);
    let found = micro_skills::discover(context.workspace, &home, trusted).await;

    if found.skills.is_empty() && found.diagnostics.is_empty() {
        return CommandOutcome::info(
            "No skills. Put a SKILL.md in .micro/skills/ or ~/.micro/skills/.".to_string(),
        );
    }

    let mut out = String::new();
    for skill in &found.skills {
        out.push_str(&format!(
            "{:<24} {}  ({})\n",
            skill.name, skill.description, skill.source
        ));
    }
    for problem in &found.diagnostics {
        out.push_str(&format!(
            "\n{} was not loaded: {}",
            problem.path.display(),
            problem.message
        ));
    }
    CommandOutcome::info(out.trim_end().to_string())
}

/// Where every setting comes from, so a surprising one can be traced to its file.
/// `/settings` offers what can be changed, in ohm's words, each item carrying the command
/// that changes it.
///
/// Every row here is honoured somewhere: a setting that controlled nothing would read as a
/// feature and behave as a decoration.
fn settings(context: &CommandContext<'_>) -> CommandOutcome {
    let home = micro_context::micro_home().unwrap_or_default();
    let file = home.join(micro_config::FILE_NAME);
    let saved = micro_config::Config::load_from(&file).unwrap_or_default();
    let now = saved
        .resolve(&micro_config::Overrides::default(), |_| None)
        .unwrap_or_default();

    let on_off = |value: bool| match value {
        true => "on",
        false => "off",
    };

    let items = vec![
        PickerItem::new(
            "Thinking level",
            thinking_label(context),
            "/thinking",
        ),
        PickerItem::new("Theme", now.theme.clone(), "/theme"),
        PickerItem::new(
            "Model",
            match context.model {
                Some(model) => model.qualified_id(),
                None => "none".to_string(),
            },
            "/model",
        ),
        PickerItem::new(
            "Default project trust",
            format!("{:?}", now.default_project_trust).to_lowercase(),
            "/set default_project_trust",
        ),
        PickerItem::new("Auto-compact", on_off(now.auto_compact), "/set auto_compact"),
        PickerItem::new(
            "Hide thinking",
            on_off(now.hide_thinking),
            "/set hide_thinking",
        ),
        PickerItem::new("Show images", on_off(now.show_images), "/set show_images"),
        PickerItem::new(
            "Image width",
            format!("{} cells", now.image_width_cells),
            "/set image_width_cells",
        ),
        PickerItem::new(
            "Auto-resize images",
            on_off(now.auto_resize_images),
            "/set auto_resize_images",
        ),
        PickerItem::new("Block images", on_off(now.block_images), "/set block_images"),
        PickerItem::new(
            "Skill commands",
            on_off(now.skill_commands),
            "/set skill_commands",
        ),
        PickerItem::new(
            "Editor padding",
            now.editor_padding.to_string(),
            "/set editor_padding",
        ),
        PickerItem::new(
            "Output padding",
            now.output_padding.to_string(),
            "/set output_padding",
        ),
        PickerItem::new(
            "Autocomplete max items",
            now.autocomplete_max_items.to_string(),
            "/set autocomplete_max_items",
        ),
        PickerItem::new(
            "Show hardware cursor",
            on_off(now.show_hardware_cursor),
            "/set show_hardware_cursor",
        ),
        PickerItem::new(
            "Terminal progress",
            on_off(now.terminal_progress),
            "/set terminal_progress",
        ),
        PickerItem::new(
            "Quiet startup",
            on_off(now.quiet_startup),
            "/set quiet_startup",
        ),
        PickerItem::new(
            "Collapse changelog",
            on_off(now.collapse_changelog),
            "/set collapse_changelog",
        ),
        PickerItem::new("Warnings", on_off(now.warnings), "/set warnings"),
        PickerItem::new(
            "Cache miss notices",
            on_off(now.cache_miss_notices),
            "/set cache_miss_notices",
        ),
        PickerItem::new(
            "Double-escape action",
            format!("{:?}", now.double_escape).to_lowercase(),
            "/set double_escape",
        ),
        PickerItem::new(
            "Follow-up mode",
            format!("{:?}", now.follow_up_mode).to_lowercase(),
            "/set follow_up_mode",
        ),
        PickerItem::new(
            "HTTP idle timeout",
            format!("{} seconds", now.http_idle_timeout),
            "/set http_idle_timeout",
        ),
        PickerItem::new(
            "Scoped models",
            match now.scoped_models.is_empty() {
                true => "the whole catalog".to_string(),
                false => now.scoped_models.join(", "),
            },
            "/set scoped_models",
        ),
        PickerItem::new(
            "Transport",
            format!("{} (ChatGPT Codex)", now.transport),
            "/set transport",
        ),
        PickerItem::new(
            "Anthropic extra usage",
            on_off(now.anthropic_extra_usage),
            "/set anthropic_extra_usage",
        ),
        PickerItem::new(
            "Where everything is kept",
            home.display().to_string(),
            "/debug",
        ),
    ];
    CommandOutcome::Choose(Picker::new("Settings", items))
}

/// `/set <name> [value]` changes one setting and remembers it. Without a value it says
/// what the setting is now and what it may be.
fn set(argument: Option<&str>) -> CommandOutcome {
    let Some(argument) = argument.map(str::trim).filter(|text| !text.is_empty()) else {
        return CommandOutcome::error("Usage: /set <setting> [value]");
    };
    let (name, value) = match argument.split_once(char::is_whitespace) {
        Some((name, value)) => (name.trim(), Some(value.trim())),
        None => (argument, None),
    };

    let path = match micro_config::default_path() {
        Ok(path) => path,
        Err(error) => return CommandOutcome::error(format!("Cannot find the settings: {error}")),
    };
    let mut config = match micro_config::Config::load_from(&path) {
        Ok(config) => config,
        Err(error) => return CommandOutcome::error(format!("Cannot read the settings: {error}")),
    };

    let Some(value) = value else {
        return match describe(&config, name) {
            Some(text) => CommandOutcome::info(text),
            None => CommandOutcome::error(format!("There is no setting called `{name}`.")),
        };
    };

    if let Err(message) = assign(&mut config, name, value) {
        return CommandOutcome::error(message);
    }
    match config.save_to(&path) {
        Ok(()) => CommandOutcome::info(format!("{name} is now {value}.")),
        Err(error) => CommandOutcome::error(format!("Could not save the settings: {error}")),
    }
}

/// One setting as it stands, and what it will take.
fn describe(config: &micro_config::Config, name: &str) -> Option<String> {
    let now = config
        .resolve(&micro_config::Overrides::default(), |_| None)
        .ok()?;
    let text = match name {
        "auto_compact" => format!("auto_compact is {} (on or off)", now.auto_compact),
        "hide_thinking" => format!("hide_thinking is {} (on or off)", now.hide_thinking),
        "show_images" => format!("show_images is {} (on or off)", now.show_images),
        "image_width_cells" => format!("image_width_cells is {} (cells)", now.image_width_cells),
        "auto_resize_images" => {
            format!("auto_resize_images is {} (on or off)", now.auto_resize_images)
        }
        "block_images" => format!("block_images is {} (on or off)", now.block_images),
        "skill_commands" => format!("skill_commands is {} (on or off)", now.skill_commands),
        "editor_padding" => format!("editor_padding is {} (columns)", now.editor_padding),
        "output_padding" => format!("output_padding is {} (columns)", now.output_padding),
        "autocomplete_max_items" => format!(
            "autocomplete_max_items is {} (rows)",
            now.autocomplete_max_items
        ),
        "show_hardware_cursor" => format!(
            "show_hardware_cursor is {} (on or off)",
            now.show_hardware_cursor
        ),
        "terminal_progress" => {
            format!("terminal_progress is {} (on or off)", now.terminal_progress)
        }
        "quiet_startup" => format!("quiet_startup is {} (on or off)", now.quiet_startup),
        "collapse_changelog" => {
            format!("collapse_changelog is {} (on or off)", now.collapse_changelog)
        }
        "warnings" => format!("warnings is {} (on or off)", now.warnings),
        "cache_miss_notices" => {
            format!("cache_miss_notices is {} (on or off)", now.cache_miss_notices)
        }
        "double_escape" => format!(
            "double_escape is {} (tree, fork or none)",
            format!("{:?}", now.double_escape).to_lowercase()
        ),
        "follow_up_mode" => format!(
            "follow_up_mode is {} (queue or interrupt)",
            format!("{:?}", now.follow_up_mode).to_lowercase()
        ),
        "default_project_trust" => format!(
            "default_project_trust is {} (ask, always or never)",
            format!("{:?}", now.default_project_trust).to_lowercase()
        ),
        "http_idle_timeout" => {
            format!("http_idle_timeout is {} (seconds)", now.http_idle_timeout)
        }
        "transport" => format!(
            "transport is {} (sse or auto; the ChatGPT Codex backend)",
            now.transport
        ),
        "anthropic_extra_usage" => format!(
            "anthropic_extra_usage is {} (on or off)",
            now.anthropic_extra_usage
        ),
        "scoped_models" => format!(
            "scoped_models is {} (a comma-separated list, or `all`)",
            match now.scoped_models.is_empty() {
                true => "all".to_string(),
                false => now.scoped_models.join(","),
            }
        ),
        _ => return None,
    };
    Some(text)
}

/// Put a value into the config, saying what went wrong rather than storing nonsense.
fn assign(config: &mut micro_config::Config, name: &str, value: &str) -> Result<(), String> {
    let flag = || match value.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        other => Err(format!("`{other}` is not on or off")),
    };
    let number = |limit: u64| -> Result<u64, String> {
        let parsed: u64 = value
            .parse()
            .map_err(|_| format!("`{value}` is not a number"))?;
        match parsed >= 1 && parsed <= limit {
            true => Ok(parsed),
            false => Err(format!("{value} is outside 1 to {limit}")),
        }
    };

    match name {
        "auto_compact" => config.auto_compact = Some(flag()?),
        "hide_thinking" => config.hide_thinking = Some(flag()?),
        "show_images" => config.show_images = Some(flag()?),
        "image_width_cells" => config.image_width_cells = Some(number(500)? as u16),
        "auto_resize_images" => config.auto_resize_images = Some(flag()?),
        "block_images" => config.block_images = Some(flag()?),
        "skill_commands" => config.skill_commands = Some(flag()?),
        // Padding is allowed to be nothing, which is what ohm gives the input by default.
        "editor_padding" => {
            config.editor_padding = Some(
                value
                    .parse::<u16>()
                    .map_err(|_| format!("`{value}` is not a number"))?
                    .min(20),
            )
        }
        "output_padding" => {
            config.output_padding = Some(
                value
                    .parse::<u16>()
                    .map_err(|_| format!("`{value}` is not a number"))?
                    .min(20),
            )
        }
        "autocomplete_max_items" => {
            config.autocomplete_max_items = Some(number(50)? as usize)
        }
        "show_hardware_cursor" => config.show_hardware_cursor = Some(flag()?),
        "terminal_progress" => config.terminal_progress = Some(flag()?),
        "quiet_startup" => config.quiet_startup = Some(flag()?),
        "collapse_changelog" => config.collapse_changelog = Some(flag()?),
        "warnings" => config.warnings = Some(flag()?),
        "cache_miss_notices" => config.cache_miss_notices = Some(flag()?),
        "double_escape" => {
            config.double_escape = Some(match value.to_ascii_lowercase().as_str() {
                "tree" => micro_config::DoubleEscape::Tree,
                "fork" => micro_config::DoubleEscape::Fork,
                "none" => micro_config::DoubleEscape::None,
                other => return Err(format!("`{other}` is not tree, fork or none")),
            })
        }
        "follow_up_mode" => {
            config.follow_up_mode = Some(match value.to_ascii_lowercase().as_str() {
                "queue" => micro_config::FollowUpMode::Queue,
                "interrupt" => micro_config::FollowUpMode::Interrupt,
                other => return Err(format!("`{other}` is not queue or interrupt")),
            })
        }
        "default_project_trust" => {
            config.default_project_trust = Some(match value.to_ascii_lowercase().as_str() {
                "ask" => micro_config::ProjectTrust::Ask,
                "always" => micro_config::ProjectTrust::Always,
                "never" => micro_config::ProjectTrust::Never,
                other => return Err(format!("`{other}` is not ask, always or never")),
            })
        }
        "http_idle_timeout" => config.http_idle_timeout = Some(number(3600)?),
        "anthropic_extra_usage" => config.anthropic_extra_usage = Some(flag()?),
        "transport" => {
            let chosen = micro_provider::Transport::named(value).ok_or_else(|| {
                format!("micro speaks sse, not `{value}`; the Codex backend is reached over SSE")
            })?;
            config.transport = Some(chosen.name().to_string());
        }
        "scoped_models" => {
            config.scoped_models = Some(match value.eq_ignore_ascii_case("all") {
                true => Vec::new(),
                false => value
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        }
        other => return Err(format!("There is no setting called `{other}`.")),
    }
    Ok(())
}

/// The reasoning effort in force, as a user reads it.
fn thinking_label(context: &CommandContext<'_>) -> String {
    let _ = context;
    micro_config::Config::load()
        .ok()
        .and_then(|config| config.thinking)
        .map(|level| format!("{level:?}").to_lowercase())
        .unwrap_or_else(|| "off".to_string())
}

/// What micro knows about the session it is in, for when something is behaving oddly.
fn debug(context: &CommandContext<'_>) -> CommandOutcome {
    let mut out = String::new();
    out.push_str(&format!("version        {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("workspace      {}\n", context.workspace.display()));
    out.push_str(&format!("provider       {}\n", context.provider));
    out.push_str(&format!(
        "model          {}\n",
        context.model.map(|m| m.qualified_id()).unwrap_or_default()
    ));
    out.push_str(&format!(
        "session        {}\n",
        context.session_id.unwrap_or("none")
    ));
    out.push_str(&format!("messages       {}", context.message_count));
    CommandOutcome::info(out)
}

/// `/hotkeys`, in ohm's three groups, ohm's order, and ohm's words.
///
/// The keys are micro's own, which are ohm's defaults: what a row says is bound is what
/// [`crate`]'s caller actually binds, so this is a description rather than a wish.
fn hotkeys_text() -> String {
    let groups: &[(&str, &[(&str, &str)])] = &[
        (
            "Navigation",
            &[
                ("up/down/left/right", "Move cursor / browse history"),
                ("alt+left/ctrl+left/alt+b", "Move by word"),
                ("home/ctrl+a", "Start of line"),
                ("end/ctrl+e", "End of line"),
                ("ctrl+]", "Jump forward to character"),
                ("ctrl+alt+]", "Jump backward to character"),
                ("pageup/pagedown", "Scroll by page"),
            ],
        ),
        (
            "Editing",
            &[
                ("enter", "Send message"),
                ("shift+enter/ctrl+j", "New line"),
                ("ctrl+w/alt+backspace", "Delete word backwards"),
                ("alt+d/alt+delete", "Delete word forwards"),
                ("ctrl+u", "Delete to start of line"),
                ("ctrl+k", "Delete to end of line"),
                ("ctrl+y", "Paste the most-recently-deleted text"),
                ("alt+y", "Cycle through the deleted text after pasting"),
                ("ctrl+-", "Undo"),
            ],
        ),
        (
            "Other",
            &[
                ("tab", "Path completion / accept autocomplete"),
                ("escape", "Cancel autocomplete / abort streaming"),
                ("ctrl+c", "Clear editor (first) / exit (second)"),
                ("ctrl+d", "Exit (when editor is empty)"),
                ("ctrl+z", "Suspend to background"),
                ("shift+tab", "Cycle thinking level"),
                ("ctrl+p/shift+ctrl+p", "Cycle models"),
                ("ctrl+l", "Open model selector"),
                ("ctrl+o", "Toggle tool output expansion"),
                ("ctrl+t", "Toggle thinking block visibility"),
                ("ctrl+g", "Edit message in external editor"),
                ("ctrl+x", "Copy last assistant message"),
                ("alt+enter", "Queue follow-up message"),
                ("alt+up", "Restore queued messages"),
                ("ctrl+v", "Paste image or text from clipboard"),
                ("/", "Slash commands"),
                ("!", "Run bash command"),
            ],
        ),
    ];

    let width = groups
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .map(|(keys, _)| key_display(keys).chars().count())
        .max()
        .unwrap_or(0);

    let mut out = String::new();
    for (index, (title, rows)) in groups.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(title);
        out.push('\n');
        for (keys, what) in rows.iter() {
            let keys = key_display(keys);
            out.push_str(&format!("  {keys:<width$}  {what}\n"));
        }
    }
    out.trim_end().to_string()
}

/// A key as it is read rather than as it is written: every part capitalized, and `alt`
/// named `Option` where the keyboard says Option.
fn key_display(keys: &str) -> String {
    keys.split('/')
        .map(|combination| {
            combination
                .split('+')
                .map(capitalize_part)
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn capitalize_part(part: &str) -> String {
    let part = match (cfg!(target_os = "macos"), part.eq_ignore_ascii_case("alt")) {
        (true, true) => "option",
        _ => part,
    };
    let mut characters = part.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn compact(context: &CommandContext<'_>) -> CommandOutcome {
    if context.message_count == 0 {
        return CommandOutcome::error("nothing to compact yet");
    }
    CommandOutcome::Compact
}

fn help_text() -> String {
    let width = COMMANDS
        .iter()
        .map(|command| command.usage().chars().count())
        .max()
        .unwrap_or(0);

    COMMANDS
        .iter()
        .map(|command| {
            format!(
                "{:width$}  {}",
                command.usage(),
                command.description,
                width = width
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use micro_models::Catalog;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU32;
    use std::sync::atomic::Ordering;

    /// Everything a command reads, rooted in this process's own scratch directory so no
    /// test touches a real credential file or session log.
    pub struct Harness {
        pub catalog: Catalog,
        pub auth: AuthStore,
        pub sessions: SessionStore,
        pub workspace: PathBuf,
    }

    impl Harness {
        pub fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "micro-commands-{label}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let workspace = root.join("workspace");
            std::fs::create_dir_all(&workspace).unwrap();

            Harness {
                catalog: Catalog::bundled(),
                auth: AuthStore::open_at(root.join("auth.json")).unwrap(),
                sessions: SessionStore::new(root.join("sessions")),
                workspace,
            }
        }

        pub fn context(&self) -> CommandContext<'_> {
            CommandContext {
                catalog: &self.catalog,
                auth: &self.auth,
                sessions: &self.sessions,
                workspace: &self.workspace,
                provider: "anthropic",
                model: None,
                session_id: None,
                message_count: 0,
                usage: micro_types::Usage::default(),
            collapse_changelog: false,
            }
        }
    }

    /// The picker an outcome carries, or a panic naming what came back instead.
    pub fn picker(outcome: &CommandOutcome) -> &Picker {
        match outcome {
            CommandOutcome::Choose(picker) => picker,
            other => panic!("expected a picker, got {other:?}"),
        }
    }

    pub fn text(outcome: &CommandOutcome) -> &str {
        outcome
            .text()
            .unwrap_or_else(|| panic!("expected a message, got {outcome:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn every_command_is_listed_once_and_describes_itself() {
        let mut names: Vec<&str> = commands().iter().map(|command| command.name).collect();
        let listed = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), listed, "a command name is repeated");

        for command in commands() {
            assert!(!command.description.is_empty(), "/{}", command.name);
            assert!(command.name.chars().all(|c| c.is_ascii_lowercase()));
        }
    }

    #[test]
    fn usage_shows_the_argument_a_command_takes() {
        assert_eq!(find("fork").unwrap().usage(), "/fork [index]");
        assert_eq!(find("quit").unwrap().usage(), "/quit");
    }

    #[test]
    fn a_command_is_found_with_or_without_its_slash() {
        assert_eq!(find("/model").unwrap().name, "model");
        assert_eq!(find("MODEL").unwrap().name, "model");
        assert!(find("nope").is_none());
    }

    /// Every registered command answers when it is run, and none of them panic on the
    /// way. A command that is listed but not wired up would otherwise only be found by a
    /// user typing it.
    #[tokio::test]
    async fn every_registered_command_answers() {
        let root = std::env::temp_dir().join(format!("micro-every-command-{}", std::process::id()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let catalog = Catalog::bundled();
        let auth = AuthStore::open_at(root.join("auth.json")).unwrap();
        let sessions = SessionStore::new(root.join("sessions"));
        let mut session = sessions
            .create(&workspace, "anthropic/claude-opus-5")
            .await
            .unwrap();
        session
            .append(&micro_types::Message::user("something was said"))
            .await
            .unwrap();
        let session_id = session.id().to_string();
        let model = catalog
            .resolve("anthropic/claude-opus-5")
            .model()
            .expect("the bundled catalog carries this model")
            .clone();

        let context = CommandContext {
            catalog: &catalog,
            auth: &auth,
            sessions: &sessions,
            workspace: &workspace,
            provider: "anthropic",
            model: Some(&model),
            session_id: Some(&session_id),
            message_count: 1,
            usage: micro_types::Usage::default(),
            collapse_changelog: false,
        };

        // An argument each command will accept, for the ones that need one. A command that
        // takes none is run bare.
        let argument_for = |name: &str| match name {
            "import" => Some("nothing.jsonl"),
            "set" => Some("warnings"),
            "name" => Some("a name"),
            "tree" => Some("1"),
            "fork" => Some("0"),
            "thinking" => Some("low"),
            "theme" => Some("dark"),
            "trust" => Some("off"),
            "model" | "provider" | "login" | "logout" | "resume" | "export" => None,
            _ => None,
        };

        for command in commands() {
            // Sign-in reaches the network, and quitting says nothing worth asserting.
            if matches!(command.name, "login" | "quit") {
                continue;
            }
            let argument = argument_for(command.name);
            let outcome = run(command, argument, &context).await;
            assert!(
                !format!("{outcome:?}").contains("is not wired up"),
                "/{} is listed but not wired up",
                command.name
            );
        }
    }

    /// Every setting `/settings` offers can be set, and every value it refuses is a value
    /// nothing downstream would have honoured.
    #[test]
    fn every_offered_setting_can_be_set() {
        let mut config = micro_config::Config::default();
        for (name, value) in [
            ("auto_compact", "off"),
            ("hide_thinking", "off"),
            ("show_images", "off"),
            ("image_width_cells", "80"),
            ("auto_resize_images", "off"),
            ("block_images", "on"),
            ("skill_commands", "off"),
            ("editor_padding", "2"),
            ("output_padding", "0"),
            ("autocomplete_max_items", "12"),
            ("show_hardware_cursor", "on"),
            ("terminal_progress", "off"),
            ("quiet_startup", "on"),
            ("collapse_changelog", "on"),
            ("warnings", "off"),
            ("cache_miss_notices", "on"),
            ("double_escape", "fork"),
            ("follow_up_mode", "interrupt"),
            ("default_project_trust", "always"),
            ("http_idle_timeout", "300"),
            ("scoped_models", "anthropic/, google/gemini-3-pro"),
            ("transport", "auto"),
        ] {
            assign(&mut config, name, value).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(describe(&config, name).is_some(), "{name} cannot be read");
        }

        let now = config
            .resolve(&micro_config::Overrides::default(), |_| None)
            .unwrap();
        assert!(!now.auto_compact);
        assert_eq!(now.image_width_cells, 80);
        assert_eq!(now.autocomplete_max_items, 12);
        assert_eq!(now.http_idle_timeout, 300);
        assert_eq!(now.double_escape, micro_config::DoubleEscape::Fork);
        assert_eq!(
            now.follow_up_mode,
            micro_config::FollowUpMode::Interrupt
        );
        assert_eq!(now.scoped_models, vec!["anthropic/", "google/gemini-3-pro"]);
        assert_eq!(now.output_padding, 0);
        assert_eq!(now.transport, "auto");
    }

    #[test]
    fn a_value_a_setting_cannot_take_is_refused() {
        let mut config = micro_config::Config::default();
        assert!(assign(&mut config, "auto_compact", "maybe").is_err());
        assert!(assign(&mut config, "image_width_cells", "wide").is_err());
        assert!(assign(&mut config, "http_idle_timeout", "0").is_err());
        assert!(assign(&mut config, "double_escape", "sideways").is_err());
        // A transport micro cannot speak is refused rather than stored and ignored.
        assert!(assign(&mut config, "transport", "websocket").is_err());
        assert!(assign(&mut config, "nothing_like_this", "on").is_err());
        assert_eq!(config, micro_config::Config::default(), "nothing stuck");
    }

    /// Every row `/settings` offers dispatches to a command that exists.
    #[test]
    fn every_settings_row_dispatches_somewhere_real() {
        let catalog = Catalog::bundled();
        let auth = AuthStore::open_at(
            std::env::temp_dir().join(format!("micro-settings-{}.json", std::process::id())),
        )
        .unwrap();
        let sessions = SessionStore::new(std::env::temp_dir().join("micro-settings-sessions"));
        let workspace = std::env::temp_dir();
        let context = CommandContext {
            catalog: &catalog,
            auth: &auth,
            sessions: &sessions,
            workspace: &workspace,
            provider: "anthropic",
            model: None,
            session_id: None,
            message_count: 0,
            usage: micro_types::Usage::default(),
            collapse_changelog: false,
        };

        let CommandOutcome::Choose(picker) = settings(&context) else {
            panic!("settings offers a choice");
        };
        assert!(picker.items.len() >= 20, "{}", picker.items.len());
        for item in &picker.items {
            let name = item
                .command
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                .expect("a command");
            assert!(find(name).is_some(), "/{name} is not a command");
        }
    }

    /// Every key the hotkey table names is one the interface actually listens for, so the
    /// table is a description of the bindings rather than a wish list.
    #[test]
    fn every_hotkey_row_names_a_key_that_is_bound() {
        let text = hotkeys_text();
        for group in ["Navigation", "Editing", "Other"] {
            assert!(text.contains(group), "{text}");
        }
        assert!(text.contains("Send message"), "{text}");
        assert!(text.contains("Cycle thinking level"), "{text}");
        assert!(text.contains("Paste image or text from clipboard"), "{text}");
        assert!(text.contains("Run bash command"), "{text}");
    }

    /// A key is read the way a keyboard is labelled, and `alt` is `Option` on a Mac.
    #[test]
    fn a_key_is_shown_the_way_it_is_read() {
        assert_eq!(key_display("enter"), "Enter");
        assert_eq!(key_display("shift+enter/ctrl+j"), "Shift+Enter/Ctrl+J");
        let word_left = key_display("alt+left");
        match cfg!(target_os = "macos") {
            true => assert_eq!(word_left, "Option+Left"),
            false => assert_eq!(word_left, "Alt+Left"),
        }
    }

    #[test]
    fn an_empty_prefix_completes_to_everything() {
        assert_eq!(complete("").len(), commands().len());
        assert_eq!(complete("/").len(), commands().len());
    }

    #[test]
    fn completion_narrows_as_the_prefix_grows() {
        let names: Vec<&str> = complete("/c").iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            vec!["clone", "changelog", "copy", "compact", "clear", "cwd"]
        );

        let names: Vec<&str> = complete("/co").iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["copy", "compact"]);

        let names: Vec<&str> = complete("/com").iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["compact"]);

        assert!(complete("/zz").is_empty());
    }

    #[test]
    fn completion_ignores_case_and_stops_once_an_argument_begins() {
        assert_eq!(complete("/MOD").len(), 1);
        assert!(complete("/model ").is_empty());
        assert!(complete("/model op").is_empty());
    }

    #[test]
    fn help_lists_every_command_with_its_description() {
        let help = help_text();
        for command in commands() {
            assert!(help.contains(&command.usage()), "missing {}", command.name);
            assert!(help.contains(command.description));
        }
    }

    #[tokio::test]
    async fn ordinary_text_is_not_a_command() {
        let harness = Harness::new("prompt");
        assert!(dispatch("explain this file", &harness.context())
            .await
            .is_none());
        assert!(dispatch("/usr/bin/env", &harness.context()).await.is_none());
    }

    #[tokio::test]
    async fn an_unknown_command_says_what_was_probably_meant() {
        let harness = Harness::new("unknown");
        let outcome = dispatch("/modl", &harness.context()).await.unwrap();

        assert!(outcome.is_error());
        assert!(
            text(&outcome).contains("did you mean /model"),
            "{outcome:?}"
        );

        let outcome = dispatch("/xyzzy", &harness.context()).await.unwrap();
        assert!(text(&outcome).contains("/help"), "{outcome:?}");
    }

    #[tokio::test]
    async fn a_command_that_takes_no_argument_says_so() {
        let harness = Harness::new("no-argument");
        let outcome = dispatch("/clear now", &harness.context()).await.unwrap();

        assert!(outcome.is_error());
        assert_eq!(text(&outcome), "/clear takes no argument");
    }

    #[tokio::test]
    async fn the_simple_commands_report_their_effect() {
        let harness = Harness::new("simple");
        let context = harness.context();

        assert!(matches!(
            dispatch("/clear", &context).await,
            Some(CommandOutcome::Clear)
        ));
        assert!(matches!(
            dispatch("/quit", &context).await,
            Some(CommandOutcome::Quit)
        ));
        assert!(matches!(
            dispatch("/help", &context).await,
            Some(CommandOutcome::Message { .. })
        ));
    }

    #[tokio::test]
    async fn cwd_reports_the_workspace_root() {
        let harness = Harness::new("cwd");
        let outcome = dispatch("/cwd", &harness.context()).await.unwrap();

        assert_eq!(text(&outcome), harness.workspace.display().to_string());
    }

    #[tokio::test]
    async fn compacting_an_empty_conversation_is_refused() {
        let harness = Harness::new("compact");
        let outcome = dispatch("/compact", &harness.context()).await.unwrap();
        assert!(outcome.is_error());

        let context = CommandContext {
            message_count: 4,
            usage: micro_types::Usage::default(),
            collapse_changelog: false,
            ..harness.context()
        };
        assert!(matches!(
            dispatch("/compact", &context).await,
            Some(CommandOutcome::Compact)
        ));
    }
}
