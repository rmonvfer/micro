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
mod bill;
mod model;
mod outcome;
mod parse;
mod session;
mod why_miss;

pub use bill::bill;
pub use bill::Bill;
pub use bill::BillLine;
pub use bill::CompactionBill;
pub use bill::Side;
pub use bill::TurnBill;
pub use outcome::CommandOutcome;
pub use outcome::MessageKind;
pub use outcome::RemoteAction;
pub use outcome::Picker;
pub use outcome::PickerItem;
pub use outcome::PickerLayout;
pub use outcome::ThemeChoice;
pub use parse::parse;
pub use parse::suggest;
pub use parse::Input;
pub use why_miss::why_miss;

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
        name: "remote",
        argument: Some("[pair]"),
        description: "Put this session on your phone",
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
        name: "bill",
        argument: Some("[turn]"),
        description: "itemize what this session has cost, turn by turn",
    },
    Command {
        name: "why-miss",
        argument: Some("[turn]"),
        description: "say why a turn did not reuse the cached prompt",
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
    /// The models a workspace put on its shortlist, as the patterns it named them by. The
    /// model list opens on these; everything else is a keystroke away.
    pub scoped_models: &'a [String],
    /// What the conversation tree shows before anything is asked of it.
    pub tree_filter: micro_config::TreeFilter,
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
        // Bare, this puts the session on whichever phone is already paired — which is
        // what pairing once is for. `pair` is the one-off that bonds a phone in the
        // first place, and the only thing that ever shows a link.
        "remote" => CommandOutcome::RemoteControl {
            action: match argument.map(str::trim).unwrap_or_default() {
                "pair" => RemoteAction::Pair { qr: false },
                "pair qr" | "qr" => RemoteAction::Pair { qr: true },
                _ => RemoteAction::Publish,
            },
        },
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
        "bill" => bill::command(argument, context).await,
        "why-miss" => why_miss::command(argument, context).await,
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
    match argument
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
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
            "unknown reasoning effort `{argument}`: expected off, minimal, low, medium, high, xhigh or max"
        )),
    }
}

/// The levels a user can ask for, with what each one is worth saying about it.
const LEVELS: &[(&str, &str)] = &[
    ("off", "answer directly"),
    ("minimal", "the least reasoning available"),
    ("low", "a little reasoning first"),
    ("medium", "a moderate amount"),
    ("high", "a lot of reasoning"),
    ("xhigh", "extra reasoning"),
    ("max", "as much as the model will do"),
];

fn level_named(name: &str) -> Option<ThinkingLevel> {
    match name.trim().to_ascii_lowercase().as_str() {
        "off" | "none" => Some(ThinkingLevel::Off),
        "minimal" => Some(ThinkingLevel::Minimal),
        "low" => Some(ThinkingLevel::Low),
        "medium" | "med" => Some(ThinkingLevel::Medium),
        "high" => Some(ThinkingLevel::High),
        "xhigh" => Some(ThinkingLevel::XHigh),
        "max" => Some(ThinkingLevel::Max),
        _ => None,
    }
}

/// `/theme` with no argument offers the palettes; with one, switches to it.
fn theme(argument: Option<&str>) -> CommandOutcome {
    let Some(argument) = argument else {
        return CommandOutcome::Choose(
            Picker::new(
                "Theme",
                vec![
                    PickerItem::new("dark", "dark palette", "/theme dark"),
                    PickerItem::new("light", "light palette", "/theme light"),
                    PickerItem::new("auto", "follow the terminal", "/theme auto"),
                ],
            )
            // A palette's name is short, so its column is held narrow: padding three names out
            // to the width a model id needs would leave the descriptions stranded.
            .columns(12, 32),
        );
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
    let home = micro_dirs::config_dir().unwrap_or_default();
    // Listing what is on offer answers with what this run is actually offering, which is
    // the project's own only when the project has been trusted.
    let trusted = !micro_config::requires_decision(context.workspace)
        || micro_config::TrustStore::load()
            .await
            .unwrap_or_default()
            .is_trusted(context.workspace);
    let found = micro_skills::discover(
        context.workspace,
        &home,
        micro_skills::user_agents_dir(),
        trusted,
    )
    .await;

    if found.skills.is_empty() && found.diagnostics.is_empty() {
        // The user's own directory is named as it resolved rather than as a default
        // spelling, since where it lands depends on whether this install predates the
        // split between configuration and data.
        return CommandOutcome::info(format!(
            "No skills. Put a SKILL.md in .micro/skills/, {}, or ~/.agents/skills/.",
            home.join("skills").display()
        ));
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
/// `/settings` offers what can be changed, each item carrying the command that changes it.
///
/// Every row here is honoured somewhere: a setting that controlled nothing would read as a
/// feature and behave as a decoration.
/// A variant's name as it is written on the command line: `OneAtATime` is `one-at-a-time`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() && index > 0 {
            out.push('-');
        }
        out.extend(character.to_lowercase());
    }
    out
}

fn settings(context: &CommandContext<'_>) -> CommandOutcome {
    let file = micro_config::default_path().unwrap_or_default();
    let saved = micro_config::Config::load_from(&file).unwrap_or_default();
    let now = saved
        .resolve(&micro_config::Overrides::default(), |_| None)
        .unwrap_or_default();

    let on_off = |value: bool| match value {
        true => "on",
        false => "off",
    };

    let items = vec![
        PickerItem::new("Thinking level", thinking_label(context), "/thinking"),
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
        PickerItem::new(
            "Auto-compact",
            on_off(now.auto_compact),
            "/set auto_compact",
        ),
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
        PickerItem::new(
            "Block images",
            on_off(now.block_images),
            "/set block_images",
        ),
        PickerItem::new(
            "Skill commands",
            on_off(now.skill_commands),
            "/set skill_commands",
        ),
        PickerItem::new(
            "Content padding",
            now.content_padding.to_string(),
            "/set content_padding",
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
            "TUI mode",
            format!("{:?}", now.tui_mode).to_lowercase(),
            "/set tui_mode",
        ),
        PickerItem::new(
            "Steering mode",
            kebab(&format!("{:?}", now.steering_mode)),
            "/set steering_mode",
        ),
        PickerItem::new(
            "Tree filter mode",
            kebab(&format!("{:?}", now.tree_filter_mode)),
            "/set tree_filter_mode",
        ),
        PickerItem::new(
            "Mermaid diagrams",
            kebab(&format!("{:?}", now.mermaid)),
            "/set mermaid",
        ),
        PickerItem::new(
            "Fullscreen exit output",
            kebab(&format!("{:?}", now.fullscreen_exit_output)),
            "/set fullscreen_exit_output",
        ),
        PickerItem::new(
            "Fullscreen scrollbar",
            kebab(&format!("{:?}", now.fullscreen_scrollbar)),
            "/set fullscreen_scrollbar",
        ),
        PickerItem::new(
            "Clear on shrink",
            on_off(now.clear_on_shrink),
            "/set clear_on_shrink",
        ),
        PickerItem::new("Sandbox", policy_name(&now), "/set sandbox"),
        PickerItem::new("Budget", spending_limit(now.budget), "/set budget"),
        PickerItem::new("Where everything is kept", kept_in(), "/debug"),
    ];
    CommandOutcome::Choose(Picker::new("Settings", items))
}

/// The directories micro reads and writes, named as one where they are one.
///
/// An install that keeps everything together should be told so in a single path; one that
/// keeps what the user wrote apart from what micro produced needs both halves named, since
/// either is somewhere a reader might be going to look.
fn kept_in() -> String {
    let config = micro_dirs::config_dir().unwrap_or_default();
    let data = micro_dirs::data_dir().unwrap_or_default();
    match config == data {
        true => config.display().to_string(),
        false => format!("{} and {}", config.display(), data.display()),
    }
}

/// The sandbox policy in force, as a name to show.
///
/// A policy can also be written as a table spelling out what it grants beyond the default,
/// which has no short name to print; it is shown by the mode it builds on, since that is
/// what a reader is looking for in a list of settings.
fn policy_name(settings: &micro_config::Settings) -> &str {
    let Some(written) = &settings.sandbox else {
        return "workspace-write";
    };
    written
        .as_str()
        .or_else(|| written.get("mode").and_then(serde_json::Value::as_str))
        .unwrap_or("workspace-write")
}

/// What a session may spend, as an amount to show. Nothing at all is written as such
/// rather than as `$0.00`, which would read as a session that may spend nothing.
fn spending_limit(budget: f64) -> String {
    match budget > 0.0 {
        true => format!("${budget:.2}"),
        false => "no ceiling".to_string(),
    }
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
        // Named on its own, a setting offers what it may be rather than only saying what
        // it is: a list you cannot change from is a list of things to go and type.
        if let Some(choices) = settable(&config, name) {
            return CommandOutcome::Choose(Picker::new(name.to_string(), choices).titled());
        }
        return match describe(&config, name) {
            Some(text) => CommandOutcome::info(text),
            None => CommandOutcome::error(format!("There is no setting called `{name}`.")),
        };
    };

    if let Err(message) = assign(&mut config, name, value) {
        return CommandOutcome::error(message);
    }
    match config.save_to(&path) {
        Ok(()) if name == "tui_mode" => CommandOutcome::SetTuiMode {
            mode: config.tui_mode.unwrap_or_default(),
        },
        Ok(()) => CommandOutcome::info(format!("{name} is now {value}.")),
        Err(error) => CommandOutcome::error(format!("Could not save the settings: {error}")),
    }
}

/// One setting as it stands, and what it will take.
/// What a setting may be set to, as a list to choose from.
///
/// Every setting that has a settled few values offers them; one that takes free text — a
/// shortlist of models — has none to offer and is described instead.
fn settable(config: &micro_config::Config, name: &str) -> Option<Vec<PickerItem>> {
    let now = config
        .resolve(&micro_config::Overrides::default(), |_| None)
        .ok()?;

    let on_off = |value: bool| match value {
        true => "on",
        false => "off",
    };
    let switch = |current: bool| vec![("on", "", current), ("off", "", !current)];

    let described: Vec<(&str, &str, bool)> = match name {
        "auto_compact" => switch(now.auto_compact),
        "hide_thinking" => switch(now.hide_thinking),
        "show_images" => switch(now.show_images),
        "auto_resize_images" => switch(now.auto_resize_images),
        "block_images" => switch(now.block_images),
        "skill_commands" => switch(now.skill_commands),
        "show_hardware_cursor" => switch(now.show_hardware_cursor),
        "terminal_progress" => switch(now.terminal_progress),
        "quiet_startup" => switch(now.quiet_startup),
        "collapse_changelog" => switch(now.collapse_changelog),
        "warnings" => switch(now.warnings),
        "cache_miss_notices" => switch(now.cache_miss_notices),
        "anthropic_extra_usage" => switch(now.anthropic_extra_usage),
        "double_escape" => {
            let now = format!("{:?}", now.double_escape).to_lowercase();
            vec![
                ("tree", "show the conversation's branches", now == "tree"),
                ("fork", "branch from a message", now == "fork"),
                ("none", "do nothing", now == "none"),
            ]
        }
        "follow_up_mode" => {
            let now = format!("{:?}", now.follow_up_mode).to_lowercase();
            vec![
                ("queue", "wait for the turn to finish", now == "queue"),
                ("interrupt", "stop the turn and send", now == "interrupt"),
            ]
        }
        "default_project_trust" => {
            let now = format!("{:?}", now.default_project_trust).to_lowercase();
            vec![
                ("ask", "ask about each project", now == "ask"),
                ("always", "trust every project", now == "always"),
                ("never", "trust no project", now == "never"),
            ]
        }
        "tui_mode" => {
            let now = format!("{:?}", now.tui_mode).to_lowercase();
            vec![
                (
                    "regular",
                    "draw inline, leaving the scrollback",
                    now == "regular",
                ),
                ("fullscreen", "take the whole terminal", now == "fullscreen"),
            ]
        }
        "clear_on_shrink" => switch(now.clear_on_shrink),
        "steering_mode" => {
            let now = kebab(&format!("{:?}", now.steering_mode));
            vec![
                (
                    "one-at-a-time",
                    "the oldest, and the rest after it",
                    now == "one-at-a-time",
                ),
                ("all", "every one of them, as one message", now == "all"),
            ]
        }
        "tree_filter_mode" => {
            let now = kebab(&format!("{:?}", now.tree_filter_mode));
            vec![
                ("default", "prompts and answers", now == "default"),
                ("no-tools", "without what the tools did", now == "no-tools"),
                ("user-only", "only what was written", now == "user-only"),
                (
                    "labeled-only",
                    "only what has a name",
                    now == "labeled-only",
                ),
                ("all", "everything there is", now == "all"),
            ]
        }
        "fullscreen_exit_output" => {
            let now = kebab(&format!("{:?}", now.fullscreen_exit_output));
            vec![
                (
                    "transcript",
                    "leave the conversation on screen",
                    now == "transcript",
                ),
                (
                    "resume-hint",
                    "leave only the line that brings it back",
                    now == "resume-hint",
                ),
            ]
        }
        "fullscreen_scrollbar" => {
            let now = kebab(&format!("{:?}", now.fullscreen_scrollbar));
            vec![
                ("auto", "when there is more than fits", now == "auto"),
                ("always", "whether or not there is", now == "always"),
                ("hidden", "never", now == "hidden"),
            ]
        }
        "mermaid" => {
            let now = kebab(&format!("{:?}", now.mermaid));
            vec![
                (
                    "off",
                    "leave it as the code it was written as",
                    now == "off",
                ),
                (
                    "final",
                    "draw it once the answer is complete",
                    now == "final",
                ),
                ("streaming", "draw it as it arrives", now == "streaming"),
            ]
        }
        "transport" => vec![("sse", "how the Codex backend answers", true)],
        "budget" => vec![
            ("0", "no ceiling", now.budget <= 0.0),
            ("1", "dollars a session", now.budget == 1.0),
            ("5", "dollars a session", now.budget == 5.0),
            ("10", "dollars a session", now.budget == 10.0),
            ("25", "dollars a session", now.budget == 25.0),
            ("100", "dollars a session", now.budget == 100.0),
        ],
        "sandbox" => {
            let now = policy_name(&now).to_string();
            vec![
                ("read-only", "read anything, write nothing", now == "read-only"),
                (
                    "workspace-write",
                    "write inside this workspace only",
                    now == "workspace-write",
                ),
                ("full", "no confinement at all", now == "full"),
            ]
        }
        _ => return numbered(name, &now),
    };

    let _ = on_off;
    Some(
        described
            .into_iter()
            .map(|(value, detail, current)| {
                PickerItem::new(value, detail, format!("/set {name} {value}")).current(current)
            })
            .collect(),
    )
}

/// A setting counted in columns, rows, cells or seconds, offered over the range it takes.
fn numbered(name: &str, now: &micro_config::Settings) -> Option<Vec<PickerItem>> {
    let (range, unit, current): (Vec<u64>, &str, u64) = match name {
        "content_padding" => ((0..=3).collect(), "columns", now.content_padding as u64),
        "interface_padding" => (
            (0..=3).collect(),
            "columns and rows",
            now.interface_padding as u64,
        ),
        "autocomplete_max_items" => (
            (3..=20).collect(),
            "rows",
            now.autocomplete_max_items as u64,
        ),
        "image_width_cells" => (
            (10..=100).step_by(10).collect(),
            "cells",
            now.image_width_cells as u64,
        ),
        "http_idle_timeout" => (
            vec![30, 60, 120, 300, 600, 1200, 3600],
            "seconds",
            now.http_idle_timeout,
        ),
        _ => return None,
    };
    Some(
        range
            .into_iter()
            .map(|value| {
                PickerItem::new(value.to_string(), unit, format!("/set {name} {value}"))
                    .current(value == current)
            })
            .collect(),
    )
}

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
            format!(
                "auto_resize_images is {} (on or off)",
                now.auto_resize_images
            )
        }
        "block_images" => format!("block_images is {} (on or off)", now.block_images),
        "skill_commands" => format!("skill_commands is {} (on or off)", now.skill_commands),
        "content_padding" => format!("content_padding is {} (columns)", now.content_padding),
        "interface_padding" => format!(
            "interface_padding is {} (columns and rows around the interface)",
            now.interface_padding
        ),
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
            format!(
                "collapse_changelog is {} (on or off)",
                now.collapse_changelog
            )
        }
        "warnings" => format!("warnings is {} (on or off)", now.warnings),
        "cache_miss_notices" => {
            format!(
                "cache_miss_notices is {} (on or off)",
                now.cache_miss_notices
            )
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
        "sandbox" => format!(
            "sandbox is {} (read-only, workspace-write or full)",
            policy_name(&now)
        ),
        "budget" => format!(
            "budget is {} (US dollars one session may spend; 0 is no ceiling)",
            spending_limit(now.budget)
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
        "content_padding" => {
            config.content_padding = Some(
                value
                    .parse::<u16>()
                    .map_err(|_| format!("`{value}` is not a number"))?
                    .min(20),
            )
        }
        "interface_padding" => {
            config.interface_padding = Some(
                value
                    .parse::<u16>()
                    .map_err(|_| format!("`{value}` is not a number"))?
                    .min(20),
            )
        }
        "autocomplete_max_items" => config.autocomplete_max_items = Some(number(50)? as usize),
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
        "sandbox" => {
            let policy = value.to_ascii_lowercase();
            if !["read-only", "workspace-write", "full"].contains(&policy.as_str()) {
                return Err(format!(
                    "`{value}` is not read-only, workspace-write or full"
                ));
            }
            config.sandbox = Some(policy.into())
        }
        "budget" => {
            // Written with the currency sign as often as without it, and the sign carries
            // no information this has to keep.
            let amount: f64 = value
                .trim_start_matches('$')
                .parse()
                .map_err(|_| format!("`{value}` is not an amount"))?;
            if !amount.is_finite() || amount < 0.0 {
                return Err(format!("`{value}` is not an amount a session could spend"));
            }
            config.budget = Some(amount)
        }
        "steering_mode" => {
            config.steering_mode = Some(match value.to_ascii_lowercase().as_str() {
                "one-at-a-time" => micro_config::SteeringMode::OneAtATime,
                "all" => micro_config::SteeringMode::All,
                other => return Err(format!("`{other}` is not one-at-a-time or all")),
            })
        }
        "tree_filter_mode" => {
            config.tree_filter_mode = Some(match value.to_ascii_lowercase().as_str() {
                "default" => micro_config::TreeFilter::Default,
                "no-tools" => micro_config::TreeFilter::NoTools,
                "user-only" => micro_config::TreeFilter::UserOnly,
                "labeled-only" => micro_config::TreeFilter::LabeledOnly,
                "all" => micro_config::TreeFilter::All,
                other => {
                    return Err(format!(
                        "`{other}` is not default, no-tools, user-only, labeled-only or all"
                    ))
                }
            })
        }
        "fullscreen_exit_output" => {
            config.fullscreen_exit_output = Some(match value.to_ascii_lowercase().as_str() {
                "transcript" => micro_config::ExitOutput::Transcript,
                "resume-hint" => micro_config::ExitOutput::ResumeHint,
                other => return Err(format!("`{other}` is not transcript or resume-hint")),
            })
        }
        "fullscreen_scrollbar" => {
            config.fullscreen_scrollbar = Some(match value.to_ascii_lowercase().as_str() {
                "auto" => micro_config::Scrollbar::Auto,
                "always" => micro_config::Scrollbar::Always,
                "hidden" => micro_config::Scrollbar::Hidden,
                other => return Err(format!("`{other}` is not auto, always or hidden")),
            })
        }
        "clear_on_shrink" => config.clear_on_shrink = Some(flag()?),
        "mermaid" => {
            config.mermaid = Some(match value.to_ascii_lowercase().as_str() {
                "off" => micro_config::Mermaid::Off,
                "final" => micro_config::Mermaid::Final,
                "streaming" => micro_config::Mermaid::Streaming,
                other => return Err(format!("`{other}` is not off, final or streaming")),
            })
        }
        "tui_mode" => {
            config.tui_mode = Some(match value.to_ascii_lowercase().as_str() {
                "regular" => micro_config::TuiMode::Regular,
                "fullscreen" => micro_config::TuiMode::Fullscreen,
                other => return Err(format!("`{other}` is not regular or fullscreen")),
            })
        }
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

/// `/hotkeys`, grouped as navigation, editing, and everything else.
///
/// What a row says is bound is what [`crate`]'s caller actually binds, so this is a
/// description rather than a wish.
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
                scoped_models: &[],
                tree_filter: Default::default(),
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
            // Lower case throughout, with a dash where a name is two words, which is what
            // the parser accepts as a name and what completion offers.
            assert!(command
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-'));
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
            scoped_models: &[],
            tree_filter: Default::default(),
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
            ("content_padding", "2"),
            ("interface_padding", "2"),
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
        assert_eq!(now.follow_up_mode, micro_config::FollowUpMode::Interrupt);
        assert_eq!(now.scoped_models, vec!["anthropic/", "google/gemini-3-pro"]);
        assert_eq!(now.content_padding, 2);
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
            scoped_models: &[],
            tree_filter: Default::default(),
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
        assert!(
            text.contains("Paste image or text from clipboard"),
            "{text}"
        );
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
            scoped_models: &[],
            tree_filter: Default::default(),
            ..harness.context()
        };
        assert!(matches!(
            dispatch("/compact", &context).await,
            Some(CommandOutcome::Compact)
        ));
    }
}
