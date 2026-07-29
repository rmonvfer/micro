//! The seam between a slash command and the state it changes.
//!
//! A command reports what should happen; it never does it. Some of those outcomes are the
//! interface's to carry out — showing a message, opening a picker, leaving — and the rest
//! belong to whoever owns the agent, the catalog and the session log. This trait is how the
//! interface hands those on and hears what to tell the user.

use async_trait::async_trait;
use micro_commands::CommandOutcome;
use micro_types::Message;

/// What the interface knows about the conversation when a command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConversationState {
    pub message_count: usize,
    /// Everything the models have been billed for in this conversation, which only the
    /// interface has seen: the session log records what was said, not what it cost.
    pub usage: micro_types::Usage,
}

/// What the host did with an outcome the interface handed it.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Applied {
    /// Nothing to report.
    #[default]
    Nothing,
    /// Say this, and leave the conversation alone.
    Note {
        text: String,
        kind: micro_commands::MessageKind,
    },
    /// The conversation is now this. The scrollback is rebuilt from it, which is what
    /// clearing, resuming, forking and compacting all look like from here.
    Conversation {
        messages: Vec<Message>,
        note: Option<String>,
    },
    /// Tell the model this instead of what it was told at launch. The conversation is
    /// left alone: only the standing instructions change.
    SystemPrompt {
        prompt: String,
        note: Option<String>,
    },
    /// Run this model from now on. The host resolves it, since it holds the catalog and the
    /// credentials; the interface applies it, since it holds the agent.
    Model {
        swap: Box<micro_agent::ModelSwap>,
        note: Option<String>,
    },
}

impl Applied {
    pub fn note(text: impl Into<String>) -> Self {
        Applied::Note {
            text: text.into(),
            kind: micro_commands::MessageKind::Info,
        }
    }

    /// Worth knowing, but nothing failed.
    pub fn warning(text: impl Into<String>) -> Self {
        Applied::Note {
            text: text.into(),
            kind: micro_commands::MessageKind::Warning,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Applied::Note {
            text: text.into(),
            kind: micro_commands::MessageKind::Error,
        }
    }

    /// Whether this reports a failure, for a caller that only cares about that.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Applied::Note {
                kind: micro_commands::MessageKind::Error,
                ..
            }
        )
    }
}

/// How the interface runs slash commands.
///
/// Implemented by whoever built the agent, since that is who holds the catalog, the
/// credentials, the session log and the conversation these outcomes change.
#[async_trait]
pub trait Commands: Send {
    /// Tell whatever is listening what the user typed, before anything is done with it.
    ///
    /// The line comes back, possibly changed: an extension may rewrite what was submitted,
    /// or swallow it by returning nothing.
    async fn submitted(&mut self, line: String) -> Option<String> {
        Some(line)
    }

    /// Tell whatever is listening what the user ran with `!`.
    async fn ran_bash(&mut self, command: &str, output: &str, failed: bool) {
        let _ = (command, output, failed);
    }

    /// Tell whatever is listening that the reasoning effort changed.
    async fn thinking_changed(&mut self, level: micro_types::ThinkingLevel) {
        let _ = level;
    }

    /// Tell whatever is listening that the conversation was summarized.
    async fn compacted(&mut self, summary: &str) {
        let _ = summary;
    }

    /// Run a submitted line. `None` means it was ordinary text for the model.
    async fn dispatch(&mut self, line: &str, state: ConversationState) -> Option<CommandOutcome>;

    /// Carry out an outcome the interface cannot: swapping the model or provider, replacing
    /// the conversation, compacting it. Only outcomes the interface does not handle itself
    /// reach here.
    async fn apply(&mut self, outcome: CommandOutcome) -> Applied;

    /// Store a key the user typed at the interface's prompt.
    async fn store_api_key(&mut self, provider: String, key: String) -> Applied;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_applied_result_carries_what_to_say() {
        assert_eq!(
            Applied::note("switched"),
            Applied::Note {
                text: "switched".into(),
                kind: micro_commands::MessageKind::Info,
            }
        );
        assert!(matches!(
            Applied::error("nope"),
            Applied::Note {
                kind: micro_commands::MessageKind::Error,
                ..
            }
        ));
        assert!(matches!(
            Applied::warning("mind this"),
            Applied::Note {
                kind: micro_commands::MessageKind::Warning,
                ..
            }
        ));
        assert!(Applied::error("nope").is_error());
        assert!(!Applied::warning("mind this").is_error());
        assert_eq!(Applied::default(), Applied::Nothing);
    }
}

/// What a second escape does when the prompt is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DoubleEscape {
    /// Show the conversation's branches, to go back to one.
    #[default]
    Tree,
    /// Branch from an earlier message.
    Fork,
    /// Nothing at all.
    None,
}

/// The preferences the interface honours, as it needs them.
///
/// Held here rather than reaching for [`micro_config`] so this crate stays a drawing
/// crate: a caller with its own idea of where settings come from can still fill this in.
#[derive(Debug, Clone, PartialEq)]
pub struct Preferences {
    /// Keep the model's reasoning folded away until it is asked for.
    pub hide_thinking: bool,
    /// Draw images in the terminal, where the terminal can.
    pub show_images: bool,
    /// The widest an image may be drawn, in cells.
    pub image_width_cells: u16,
    /// Shrink an image that would be wider than the room it has.
    pub auto_resize_images: bool,
    /// Refuse to attach images at all.
    pub block_images: bool,
    /// Columns of breathing room on each side of the input.
    pub editor_padding: u16,
    /// Columns of breathing room on each side of the conversation.
    pub output_padding: u16,
    /// How many completions the command menu offers at once.
    pub autocomplete_max_items: usize,
    /// Let the terminal draw its own cursor.
    pub show_hardware_cursor: bool,
    /// Report progress to the terminal while a turn runs.
    pub terminal_progress: bool,
    /// Open without the introduction.
    pub quiet_startup: bool,
    /// Show warnings at all.
    pub warnings: bool,
    /// Say when a request paid to write a cache it could have read.
    pub cache_miss_notices: bool,
    /// What a second escape does when the prompt is empty.
    pub double_escape: DoubleEscape,
    /// Send a prompt written mid-answer straight away, interrupting what is running.
    pub follow_up_interrupts: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            hide_thinking: true,
            show_images: true,
            image_width_cells: 60,
            auto_resize_images: true,
            block_images: false,
            editor_padding: 0,
            output_padding: 1,
            autocomplete_max_items: crate::menu::MAX_VISIBLE,
            show_hardware_cursor: false,
            terminal_progress: true,
            quiet_startup: false,
            warnings: true,
            cache_miss_notices: false,
            double_escape: DoubleEscape::Tree,
            follow_up_interrupts: false,
        }
    }
}

impl From<&micro_config::Settings> for Preferences {
    fn from(settings: &micro_config::Settings) -> Self {
        Preferences {
            hide_thinking: settings.hide_thinking,
            show_images: settings.show_images,
            image_width_cells: settings.image_width_cells,
            auto_resize_images: settings.auto_resize_images,
            block_images: settings.block_images,
            editor_padding: settings.editor_padding,
            output_padding: settings.output_padding,
            autocomplete_max_items: settings.autocomplete_max_items,
            show_hardware_cursor: settings.show_hardware_cursor,
            terminal_progress: settings.terminal_progress,
            quiet_startup: settings.quiet_startup,
            warnings: settings.warnings,
            cache_miss_notices: settings.cache_miss_notices,
            double_escape: match settings.double_escape {
                micro_config::DoubleEscape::Tree => DoubleEscape::Tree,
                micro_config::DoubleEscape::Fork => DoubleEscape::Fork,
                micro_config::DoubleEscape::None => DoubleEscape::None,
            },
            follow_up_interrupts: matches!(
                settings.follow_up_mode,
                micro_config::FollowUpMode::Interrupt
            ),
        }
    }
}
