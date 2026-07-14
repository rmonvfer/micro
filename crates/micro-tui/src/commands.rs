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
    Note { text: String, error: bool },
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
            error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Applied::Note {
            text: text.into(),
            error: true,
        }
    }
}

/// How the interface runs slash commands.
///
/// Implemented by whoever built the agent, since that is who holds the catalog, the
/// credentials, the session log and the conversation these outcomes change.
#[async_trait]
pub trait Commands: Send {
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
                error: false,
            }
        );
        assert!(matches!(
            Applied::error("nope"),
            Applied::Note { error: true, .. }
        ));
        assert_eq!(Applied::default(), Applied::Nothing);
    }
}
