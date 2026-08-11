//! What a command asks the caller to do.
//!
//! A command never draws, prompts, or edits the conversation itself. It returns one of
//! these, and the caller — a TUI, a headless CLI, a test — decides how to carry it out.

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
pub enum ThemeChoice {
    Dark,
    Light,
    /// Work it out from the terminal's background, the way an unconfigured launch does.
    Auto,
}

pub enum CommandOutcome {
    /// Text to show. Nothing else changes.
    Message {
        kind: MessageKind,
        text: String,
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
    /// Repaint in this theme. Nothing about the conversation changes.
    SetTheme {
        theme: ThemeChoice,
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
    /// Offer a choice. Each item carries the line to dispatch once it is picked, so a
    /// caller can render a picker without knowing what it is choosing between.
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
        /// Set when the whole conversation is being copied rather than a point in it,
        /// which is the difference between cloning and forking.
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

    /// The text of a message outcome, for a caller that only wants to print.
    pub fn text(&self) -> Option<&str> {
        match self {
            CommandOutcome::Message { text, .. } => Some(text),
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

/// A list to choose from. `title` says what is being chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub title: String,
    pub items: Vec<PickerItem>,
    /// Something to say about what the list leaves out.
    pub hint: Option<String>,
}

impl Picker {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        Picker {
            title: title.into(),
            items,
            hint: None,
        }
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
        }
    }

    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }
}

/// Written by hand because a pending device login holds an HTTP-facing authorization
/// that is not worth printing in full.
impl fmt::Debug for CommandOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandOutcome::Message { kind, text } => formatter
                .debug_struct("Message")
                .field("kind", kind)
                .field("text", text)
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
            CommandOutcome::CopyLastAnswer => formatter.write_str("CopyLastAnswer"),
            CommandOutcome::Branch { entry_id } => formatter
                .debug_struct("Branch")
                .field("entry_id", entry_id)
                .finish(),
            CommandOutcome::Rename { title } => {
                formatter.debug_struct("Rename").field("title", title).finish()
            }
            CommandOutcome::Trust { trusted } => formatter
                .debug_struct("Trust")
                .field("trusted", trusted)
                .finish(),
            CommandOutcome::Export { path } => {
                formatter.debug_struct("Export").field("path", path).finish()
            }
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
            CommandOutcome::Import { path } => {
                formatter.debug_struct("Import").field("path", path).finish()
            }
            CommandOutcome::Share => formatter.write_str("Share"),
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
