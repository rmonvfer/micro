//! The seam between a slash command and the state it changes.
//!
//! A command reports what should happen; it never does it. Some of those outcomes are the
//! interface's to carry out — showing a message, opening a picker, leaving — and the rest
//! belong to whoever owns the agent, the catalog and the session log. This trait is how the
//! interface hands those on and hears what to tell the user.

use async_trait::async_trait;
use micro_auth::PendingDeviceLogin;
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

/// What running a `!` command amounted to, when whatever was listening decided instead of
/// the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashRun {
    pub output: String,
    pub failed: bool,
}

/// What asking every provider what it serves came back with.
#[derive(Debug, Default)]
pub struct Listings {
    /// Models the providers named, to be merged over what was already known.
    pub models: Vec<micro_models::ModelDef>,
    /// Providers that could not be reached, said in their own words.
    pub errors: Vec<String>,
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

    /// Ask whatever is listening before running what the user typed with `!`, and let it
    /// take over the run entirely.
    ///
    /// `None` runs the command against the shell as usual. `Some` is what running it
    /// amounted to instead — the shell is never actually asked — which is how a `user_bash`
    /// handler is honoured: an extension answering with its own result has decided what
    /// happened, not merely watched it happen.
    async fn before_bash(
        &mut self,
        command: &str,
        exclude_from_context: bool,
        cwd: &str,
    ) -> Option<BashRun> {
        let _ = (command, exclude_from_context, cwd);
        None
    }

    /// Run whatever is bound to this key, if anything is. `true` means it was handled and
    /// the key should go no further.
    async fn shortcut(&mut self, key: &str) -> bool {
        let _ = key;
        false
    }

    /// Tell whatever is listening that the reasoning effort changed.
    async fn thinking_changed(&mut self, level: micro_types::ThinkingLevel) {
        let _ = level;
    }

    /// Ask whatever is listening whether the conversation may be summarized.
    ///
    /// `false` stops it. Going ahead is the default, so a host that does not care about
    /// compaction never has to say so.
    async fn compacting(&mut self) -> bool {
        true
    }

    /// Tell whatever is listening that the conversation was summarized.
    async fn compacted(&mut self, summary: &str) {
        let _ = summary;
    }

    /// Start refreshing the model catalogs behind a list that has just opened.
    ///
    /// The list is drawn from what is already known and the refresh runs beside it, because
    /// a list that waits for the network is a list that is not there when it was asked for.
    /// `None` from a host with nothing to refresh, and the receiver answers once.
    fn begin_model_refresh(&mut self) -> Option<tokio::sync::oneshot::Receiver<Listings>> {
        None
    }

    /// Take in what a refresh found, and say how the list should read now.
    async fn apply_model_refresh(&mut self, listings: Listings) -> Option<micro_commands::Picker> {
        let _ = listings;
        None
    }

    /// Run a submitted line. `None` means it was ordinary text for the model.
    async fn dispatch(&mut self, line: &str, state: ConversationState) -> Option<CommandOutcome>;

    /// Carry out an outcome the interface cannot: swapping the model or provider, replacing
    /// the conversation, compacting it. Only outcomes the interface does not handle itself
    /// reach here.
    async fn apply(&mut self, outcome: CommandOutcome) -> Applied;

    /// Store a key the user typed at the interface's prompt.
    async fn store_api_key(&mut self, provider: String, key: String) -> Applied;

    /// Wait for a device-code sign-in the user has been shown the code for.
    ///
    /// Separate from [`Commands::apply`] because the interface has to say where to go and
    /// what to type before the waiting starts: the code is useless once the wait is over.
    async fn finish_device_login(&mut self, pending: Box<PendingDeviceLogin>) -> Applied;
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
/// When the conversation shows how far through it you are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scrollbar {
    #[default]
    Auto,
    Always,
    Hidden,
}

/// What is left on the terminal after a full-screen session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExitOutput {
    /// The conversation, so it is still there to read and copy from.
    #[default]
    Transcript,
    /// The line that brings it back, and nothing else.
    ResumeHint,
}

/// Whether a diagram written in a code block is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mermaid {
    Off,
    Final,
    #[default]
    Streaming,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preferences {
    /// Send every queued message at once when a turn ends, rather than the oldest alone.
    pub steer_all_at_once: bool,
    /// When the conversation shows how far through it you are.
    pub scrollbar: Scrollbar,
    /// Whether a diagram written in a code block is drawn.
    pub mermaid: Mermaid,
    /// What a full screen leaves behind when it goes.
    pub exit_output: ExitOutput,
    /// Whether rows the inline region gives up are cleared as it shrinks.
    pub clear_on_shrink: bool,
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
    /// Columns of breathing room on each side of the input and lower interface components.
    pub content_padding: u16,
    /// Columns and rows kept clear between the terminal's edges and the interface.
    pub interface_padding: u16,
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
            steer_all_at_once: false,
            scrollbar: Scrollbar::default(),
            mermaid: Mermaid::default(),
            exit_output: ExitOutput::default(),
            clear_on_shrink: false,
            hide_thinking: true,
            show_images: true,
            image_width_cells: 60,
            auto_resize_images: true,
            block_images: false,
            content_padding: 1,
            interface_padding: 0,
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
            content_padding: settings.content_padding,
            interface_padding: settings.interface_padding,
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
            steer_all_at_once: matches!(settings.steering_mode, micro_config::SteeringMode::All),
            scrollbar: match settings.fullscreen_scrollbar {
                micro_config::Scrollbar::Auto => Scrollbar::Auto,
                micro_config::Scrollbar::Always => Scrollbar::Always,
                micro_config::Scrollbar::Hidden => Scrollbar::Hidden,
            },
            mermaid: match settings.mermaid {
                micro_config::Mermaid::Off => Mermaid::Off,
                micro_config::Mermaid::Final => Mermaid::Final,
                micro_config::Mermaid::Streaming => Mermaid::Streaming,
            },
            exit_output: match settings.fullscreen_exit_output {
                micro_config::ExitOutput::Transcript => ExitOutput::Transcript,
                micro_config::ExitOutput::ResumeHint => ExitOutput::ResumeHint,
            },
            clear_on_shrink: settings.clear_on_shrink,
        }
    }
}
