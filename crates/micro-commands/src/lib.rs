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
        "trust" => trust(argument),
        "reload" => CommandOutcome::Reload,
        "share" => CommandOutcome::Share,
        "changelog" => CommandOutcome::info(CHANGELOG.trim()),
        "import" => match argument.map(str::trim).filter(|path| !path.is_empty()) {
            Some(path) => CommandOutcome::Import {
                path: path.to_string(),
            },
            None => CommandOutcome::error("say which file to import: /import <path>"),
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
    let found = micro_skills::discover(context.workspace, &home).await;

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
fn settings(context: &CommandContext<'_>) -> CommandOutcome {
    let home = micro_context::micro_home().unwrap_or_default();
    let mut out = String::new();
    out.push_str(&format!("home           {}\n", home.display()));
    out.push_str(&format!("workspace      {}\n", context.workspace.display()));
    out.push_str(&format!("provider       {}\n", context.provider));
    if let Some(model) = context.model {
        out.push_str(&format!("model          {}\n", model.qualified_id()));
    }
    out.push_str(&format!("credentials    {}/auth.json\n", home.display()));
    out.push_str(&format!("models         {}/models.json\n", home.display()));
    out.push_str(&format!("policy         {}/policy.json\n", home.display()));
    out.push_str(&format!("config         {}/config.json\n", home.display()));
    out.push_str(&format!("skills         {}/skills/\n", home.display()));
    out.push_str(&format!("sessions       {}/sessions/", home.display()));
    CommandOutcome::info(out)
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

/// Every key the interface listens for, in the order a reader meets them.
fn hotkeys_text() -> String {
    let rows: &[(&str, &str)] = &[
        ("enter", "send"),
        ("shift+enter / ctrl+j", "new line"),
        ("\\ then enter", "new line"),
        ("alt+enter", "queue a follow-up"),
        ("alt+up", "pull the queue back into the prompt"),
        ("escape", "interrupt"),
        ("ctrl+c", "clear, twice to leave"),
        ("ctrl+d", "leave when the prompt is empty"),
        ("ctrl+o", "expand or collapse everything"),
        ("ctrl+t", "show or hide reasoning"),
        ("shift+tab", "cycle reasoning effort"),
        ("ctrl+p / shift+ctrl+p", "next or previous model"),
        ("ctrl+l", "choose a model"),
        ("ctrl+g", "edit the prompt in $EDITOR"),
        ("ctrl+x", "copy the last answer"),
        ("ctrl+v", "attach an image"),
        ("ctrl+z", "suspend"),
        ("ctrl+y / alt+y", "yank, then cycle the kill ring"),
        ("ctrl+-", "undo"),
        ("ctrl+]", "jump to a character"),
        ("ctrl+w / alt+backspace", "delete the word before"),
        ("alt+d", "delete the word after"),
        ("ctrl+u / ctrl+k", "delete to the start or end of the line"),
        ("ctrl+a / ctrl+e", "start or end of the line"),
        ("alt+left / alt+right", "move a word"),
        ("up / down", "move, or browse sent prompts"),
        ("pageup / pagedown", "scroll the conversation"),
        ("/", "commands"),
        ("!", "run a shell command"),
    ];
    rows.iter()
        .map(|(keys, what)| format!("{keys:<24} {what}"))
        .collect::<Vec<_>>()
        .join("\n")
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
            ..harness.context()
        };
        assert!(matches!(
            dispatch("/compact", &context).await,
            Some(CommandOutcome::Compact)
        ));
    }
}
