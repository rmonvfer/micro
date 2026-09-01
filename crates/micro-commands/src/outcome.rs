//! What a command asks the caller to do.

use micro_auth::PendingDeviceLogin;
use micro_models::ModelDef;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    /// Worth knowing, but nothing failed.
    Warning,
    Error,
}

/// Which palette to paint in, or a return to letting the terminal decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAction {
    /// Put this session on the paired phone.
    Publish,
    /// Bond a phone to this machine, showing it what it needs.
    Pair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    Dark,
    Light,
    /// Work it out from the terminal's background, the way an unconfigured launch does.
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionItem {
    pub label: String,
    pub detail: String,
}

pub enum CommandOutcome {
    /// Text to show.
    Message {
        kind: MessageKind,
        text: String,
    },
    /// Read-only session data shown outside the conversation.
    Inspect {
        title: String,
        text: String,
        items: Vec<InspectionItem>,
    },

    Send {
        prompt: String,
    },
    /// Use this model from the next turn on.
    SetModel {
        model: Box<ModelDef>,
    },
    /// Use this provider from the next turn on.
    SetProvider {
        provider: &'static str,
    },
    /// Reason this hard from the next turn on.
    SetThinking {
        level: micro_types::ThinkingLevel,
    },
    /// Repaint in this theme.
    SetTheme {
        theme: ThemeChoice,
    },
    /// Change how much of the terminal the interface occupies.
    SetTuiMode {
        mode: micro_config::TuiMode,
    },
    /// Put the last answer on the system clipboard.
    CopyLastAnswer,
    /// Write the conversation to a file, or to a chosen name when one is given.
    Export {
        path: Option<String>,
    },
    /// Continue the conversation from an earlier entry, keeping what came after it.
    Branch {
        entry_id: String,
    },
    /// Give the session a name of its own.
    Rename {
        title: String,
    },
    /// Remember whether this project may be edited without being asked about every file.
    Trust {
        trusted: bool,
    },
    /// Read the instruction files and skills again, and tell the model what they say now.
    Reload,
    /// Bring in a session log written elsewhere and carry on from it.
    Import {
        path: String,
    },
    /// Publish the conversation, and say where it went.
    Share,
    /// Put this session on the phone that has been paired, or pair one.
    RemoteControl {
        action: RemoteAction,
    },
    /// Configure command sandbox access in the interactive host.
    Sandbox {
        argument: Option<String>,
    },
    /// Offer a choice.
    Choose(Picker),
    /// Ask the user for a key, then hand it to [`micro_auth::AuthStore::store_api_key`].
    PromptForApiKey {
        provider: String,
        /// The variables that would supply the key instead, worth naming in the prompt.
        env_names: Vec<String>,
    },
    /// Show the verification URL and user code, then await
    /// [`micro_auth::AuthStore::complete_device_login`].
    DeviceLogin {
        pending: Box<PendingDeviceLogin>,
    },
    /// Load this session's history in place of the current conversation.
    Resume {
        session_id: String,
    },
    /// Branch the conversation, keeping messages `0..=through_index`.
    Fork {
        session_id: String,
        through_index: usize,

        whole: bool,
    },
    /// Summarize the conversation so far and continue from the summary.
    Compact,
    /// Drop the conversation and start over.
    Clear,
    Quit,
}

impl CommandOutcome {
    pub fn info(text: impl Into<String>) -> Self {
        CommandOutcome::Message {
            kind: MessageKind::Info,
            text: text.into(),
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        CommandOutcome::Message {
            kind: MessageKind::Warning,
            text: text.into(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        CommandOutcome::Message {
            kind: MessageKind::Error,
            text: text.into(),
        }
    }

    pub fn inspect(title: impl Into<String>, text: impl Into<String>) -> Self {
        CommandOutcome::Inspect {
            title: title.into(),
            text: text.into(),
            items: Vec::new(),
        }
    }

    pub fn inspect_items(
        title: impl Into<String>,
        text: impl Into<String>,
        items: Vec<InspectionItem>,
    ) -> Self {
        CommandOutcome::Inspect {
            title: title.into(),
            text: text.into(),
            items,
        }
    }

    /// The text of a message outcome, for a caller that only wants to print.
    pub fn text(&self) -> Option<&str> {
        match self {
            CommandOutcome::Message { text, .. } | CommandOutcome::Inspect { text, .. } => {
                Some(text)
            }
            _ => None,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(
            self,
            CommandOutcome::Message {
                kind: MessageKind::Error,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub title: String,
    pub items: Vec<PickerItem>,
    /// Something to say about what the list leaves out.
    pub hint: Option<String>,
    /// Whether the list has a query line to narrow it.
    pub searchable: bool,
    /// Whether the list names itself and says which keys work.
    pub titled: bool,
    /// How a row is put together.
    pub layout: PickerLayout,
    /// How narrow and how wide the label's column may be.
    pub column: (usize, usize),
    /// The same choices cut down to what the workspace put on its shortlist, when it has one.
    pub scoped: Vec<PickerItem>,

    pub refreshes: bool,
}

pub const DEFAULT_COLUMN: usize = 32;

/// How a picker's rows are laid out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PickerLayout {
    /// The label in a column of its own, the detail lined up after it.
    #[default]
    Columns,
    /// The label, then the detail as a badge one space after it, and the chosen row's note beneath
    /// the list.
    Badges,
}

impl Picker {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        Picker {
            title: title.into(),
            items,
            hint: None,
            searchable: false,
            titled: false,
            layout: PickerLayout::default(),
            column: (DEFAULT_COLUMN, DEFAULT_COLUMN),
            scoped: Vec::new(),
            refreshes: false,
        }
    }

    /// The same list, with a line to narrow it by.
    pub fn searchable(mut self) -> Self {
        self.searchable = true;
        self
    }

    /// The same list, naming itself and saying which keys work.
    pub fn titled(mut self) -> Self {
        self.titled = true;
        self
    }

    /// The same list, with the label's column held between these widths.
    pub fn columns(mut self, min: usize, max: usize) -> Self {
        self.column = (min.min(max).max(1), max.max(min).max(1));
        self
    }

    /// The same list, kept up to date by asking the providers while it is open.
    pub fn refreshing(mut self) -> Self {
        self.refreshes = true;
        self
    }

    /// The same list, opening on a workspace's shortlist with the whole of it a key away.
    pub fn scoping(mut self, scoped: Vec<PickerItem>) -> Self {
        self.scoped = scoped;
        self
    }

    /// The same list, laid out the given way.
    pub fn laid_out(mut self, layout: PickerLayout) -> Self {
        self.layout = layout;
        self
    }

    /// The same list, with a line saying what it is not showing.
    pub fn saying(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// The line to dispatch for the item at `index`.
    pub fn command_at(&self, index: usize) -> Option<&str> {
        self.items.get(index).map(|item| item.command.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    /// The primary line: an id, a name.
    pub label: String,
    /// A secondary line: context size, price, when it was last touched.
    pub detail: String,
    /// The command line to dispatch when this item is chosen.
    pub command: String,
    /// This item is what is in use now, so a caller can mark it.
    pub current: bool,
    /// What a query is matched against, when that is more than what is on the row.
    pub search: Option<String>,
    /// A line shown under the list while this row is the chosen one, for what does not fit on the
    /// row itself.
    pub note: Option<String>,
}

impl PickerItem {
    pub fn new(
        label: impl Into<String>,
        detail: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        PickerItem {
            label: label.into(),
            detail: detail.into(),
            command: command.into(),
            current: false,
            search: None,
            note: None,
        }
    }

    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    /// What a query is matched against, in place of the row's own text.
    pub fn found_by(mut self, search: impl Into<String>) -> Self {
        self.search = Some(search.into());
        self
    }

    /// What is said under the list while this row is chosen.
    pub fn noting(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// Written by hand because a pending device login holds an HTTP-facing authorization that is not
/// worth printing in full.
impl fmt::Debug for CommandOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandOutcome::Message { kind, text } => formatter
                .debug_struct("Message")
                .field("kind", kind)
                .field("text", text)
                .finish(),
            CommandOutcome::Inspect { title, text, items } => formatter
                .debug_struct("Inspect")
                .field("title", title)
                .field("text", text)
                .field("items", items)
                .finish(),
            CommandOutcome::SetModel { model } => formatter
                .debug_struct("SetModel")
                .field("model", &model.qualified_id())
                .finish(),
            CommandOutcome::SetProvider { provider } => formatter
                .debug_struct("SetProvider")
                .field("provider", provider)
                .finish(),
            CommandOutcome::SetThinking { level } => formatter
                .debug_struct("SetThinking")
                .field("level", level)
                .finish(),
            CommandOutcome::SetTheme { theme } => formatter
                .debug_struct("SetTheme")
                .field("theme", theme)
                .finish(),
            CommandOutcome::SetTuiMode { mode } => formatter
                .debug_struct("SetTuiMode")
                .field("mode", mode)
                .finish(),
            CommandOutcome::CopyLastAnswer => formatter.write_str("CopyLastAnswer"),
            CommandOutcome::Branch { entry_id } => formatter
                .debug_struct("Branch")
                .field("entry_id", entry_id)
                .finish(),
            CommandOutcome::Rename { title } => formatter
                .debug_struct("Rename")
                .field("title", title)
                .finish(),
            CommandOutcome::Trust { trusted } => formatter
                .debug_struct("Trust")
                .field("trusted", trusted)
                .finish(),
            CommandOutcome::Export { path } => formatter
                .debug_struct("Export")
                .field("path", path)
                .finish(),
            CommandOutcome::Sandbox { argument } => formatter
                .debug_struct("Sandbox")
                .field("argument", argument)
                .finish(),
            CommandOutcome::Choose(picker) => {
                formatter.debug_tuple("Choose").field(picker).finish()
            }
            CommandOutcome::PromptForApiKey {
                provider,
                env_names,
            } => formatter
                .debug_struct("PromptForApiKey")
                .field("provider", provider)
                .field("env_names", env_names)
                .finish(),
            CommandOutcome::DeviceLogin { pending } => formatter
                .debug_struct("DeviceLogin")
                .field("provider", &pending.provider)
                .field("verification_uri", &pending.verification_uri())
                .finish(),
            CommandOutcome::Resume { session_id } => formatter
                .debug_struct("Resume")
                .field("session_id", session_id)
                .finish(),
            CommandOutcome::Fork {
                session_id,
                through_index,
                whole,
            } => formatter
                .debug_struct("Fork")
                .field("session_id", session_id)
                .field("through_index", through_index)
                .field("whole", whole)
                .finish(),
            CommandOutcome::Reload => formatter.write_str("Reload"),
            CommandOutcome::Import { path } => formatter
                .debug_struct("Import")
                .field("path", path)
                .finish(),
            CommandOutcome::Share => formatter.write_str("Share"),
            CommandOutcome::RemoteControl { action } => {
                write!(formatter, "RemoteControl {{ action: {action:?} }}")
            }
            CommandOutcome::Send { .. } => formatter.write_str("Send"),
            CommandOutcome::Compact => formatter.write_str("Compact"),
            CommandOutcome::Clear => formatter.write_str("Clear"),
            CommandOutcome::Quit => formatter.write_str("Quit"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_carries_its_text_and_severity() {
        let info = CommandOutcome::info("all good");
        assert_eq!(info.text(), Some("all good"));
        assert!(!info.is_error());

        let failure = CommandOutcome::error("nope");
        assert!(failure.is_error());
        assert!(!CommandOutcome::Quit.is_error());
        assert_eq!(CommandOutcome::Quit.text(), None);
    }

    #[test]
    fn a_picker_hands_back_the_line_to_dispatch() {
        let picker = Picker::new(
            "Select a model",
            vec![
                PickerItem::new(
                    "anthropic/claude-opus-5",
                    "200k context",
                    "/model anthropic/claude-opus-5",
                ),
                PickerItem::new(
                    "gemini/gemini-2.5-pro",
                    "1M context",
                    "/model gemini/gemini-2.5-pro",
                )
                .current(true),
            ],
        );

        assert_eq!(picker.command_at(0), Some("/model anthropic/claude-opus-5"));
        assert!(picker.items[1].current);
        assert_eq!(picker.command_at(9), None);
    }
}
