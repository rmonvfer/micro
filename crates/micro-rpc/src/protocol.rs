//! The wire format: what arrives on stdin, and what goes out on stdout.

use micro_types::ContentBlock;
use micro_types::Message;
use micro_types::ThinkingLevel;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;


#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    
    Prompt {
        #[serde(default)]
        id: Option<String>,
        message: String,
        #[serde(default)]
        images: Vec<Image>,
    },
    /// A prompt meant to reach the turn already running.
    Steer {
        #[serde(default)]
        id: Option<String>,
        message: String,
        #[serde(default)]
        images: Vec<Image>,
    },
    /// A prompt meant to wait until the turn already running is finished.
    FollowUp {
        #[serde(default)]
        id: Option<String>,
        message: String,
        #[serde(default)]
        images: Vec<Image>,
    },
    Abort {
        #[serde(default)]
        id: Option<String>,
    },
    NewSession {
        #[serde(default)]
        id: Option<String>,
    },

    
    GetState {
        #[serde(default)]
        id: Option<String>,
    },

    
    SetModel {
        #[serde(default)]
        id: Option<String>,
        provider: String,
        model_id: String,
    },
    CycleModel {
        #[serde(default)]
        id: Option<String>,
    },
    GetAvailableModels {
        #[serde(default)]
        id: Option<String>,
    },

    
    SetThinkingLevel {
        #[serde(default)]
        id: Option<String>,
        level: ThinkingLevel,
    },
    CycleThinkingLevel {
        #[serde(default)]
        id: Option<String>,
    },

    
    Compact {
        #[serde(default)]
        id: Option<String>,
    },
    SetAutoCompaction {
        #[serde(default)]
        id: Option<String>,
        enabled: bool,
    },

    
    Bash {
        #[serde(default)]
        id: Option<String>,
        command: String,
        
        #[serde(default)]
        exclude_from_context: bool,
    },
    AbortBash {
        #[serde(default)]
        id: Option<String>,
    },

    
    GetSessionStats {
        #[serde(default)]
        id: Option<String>,
    },
    SwitchSession {
        #[serde(default)]
        id: Option<String>,
        session_path: String,
    },
    /// Move the open conversation back to an earlier entry.
    NavigateTree {
        #[serde(default)]
        id: Option<String>,
        entry_id: String,
    },
    /// Copy the conversation up to an entry into a session of its own, and carry on in the copy.
    Fork {
        #[serde(default)]
        id: Option<String>,
        entry_id: String,
    },
    Clone {
        #[serde(default)]
        id: Option<String>,
    },
    GetEntries {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        since: Option<String>,
    },
    GetTree {
        #[serde(default)]
        id: Option<String>,
    },
    GetLastAssistantText {
        #[serde(default)]
        id: Option<String>,
    },
    SetSessionName {
        #[serde(default)]
        id: Option<String>,
        name: String,
    },

    
    GetMessages {
        #[serde(default)]
        id: Option<String>,
    },

    
    GetCommands {
        #[serde(default)]
        id: Option<String>,
    },
}

impl Command {
    /// The id the caller gave, to echo back on the answer.
    pub fn id(&self) -> Option<&str> {
        match self {
            Command::Prompt { id, .. }
            | Command::Steer { id, .. }
            | Command::FollowUp { id, .. }
            | Command::Abort { id }
            | Command::NewSession { id }
            | Command::GetState { id }
            | Command::SetModel { id, .. }
            | Command::CycleModel { id }
            | Command::GetAvailableModels { id }
            | Command::SetThinkingLevel { id, .. }
            | Command::CycleThinkingLevel { id }
            | Command::Compact { id }
            | Command::SetAutoCompaction { id, .. }
            | Command::Bash { id, .. }
            | Command::AbortBash { id }
            | Command::GetSessionStats { id }
            | Command::SwitchSession { id, .. }
            | Command::NavigateTree { id, .. }
            | Command::Fork { id, .. }
            | Command::Clone { id }
            | Command::GetEntries { id, .. }
            | Command::GetTree { id }
            | Command::GetLastAssistantText { id }
            | Command::SetSessionName { id, .. }
            | Command::GetMessages { id }
            | Command::GetCommands { id } => id.as_deref(),
        }
    }

    /// The name the answer is labelled with, which is the name the command arrived under.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Prompt { .. } => "prompt",
            Command::Steer { .. } => "steer",
            Command::FollowUp { .. } => "follow_up",
            Command::Abort { .. } => "abort",
            Command::NewSession { .. } => "new_session",
            Command::GetState { .. } => "get_state",
            Command::SetModel { .. } => "set_model",
            Command::CycleModel { .. } => "cycle_model",
            Command::GetAvailableModels { .. } => "get_available_models",
            Command::SetThinkingLevel { .. } => "set_thinking_level",
            Command::CycleThinkingLevel { .. } => "cycle_thinking_level",
            Command::Compact { .. } => "compact",
            Command::SetAutoCompaction { .. } => "set_auto_compaction",
            Command::Bash { .. } => "bash",
            Command::AbortBash { .. } => "abort_bash",
            Command::GetSessionStats { .. } => "get_session_stats",
            Command::SwitchSession { .. } => "switch_session",
            Command::NavigateTree { .. } => "navigate_tree",
            Command::Fork { .. } => "fork",
            Command::Clone { .. } => "clone",
            Command::GetEntries { .. } => "get_entries",
            Command::GetTree { .. } => "get_tree",
            Command::GetLastAssistantText { .. } => "get_last_assistant_text",
            Command::SetSessionName { .. } => "set_session_name",
            Command::GetMessages { .. } => "get_messages",
            Command::GetCommands { .. } => "get_commands",
        }
    }
}

/// An image riding with a prompt, as the caller supplies it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// Base64, which is the shape every provider wants.
    pub data: String,
    pub mime_type: String,
}

impl From<Image> for ContentBlock {
    fn from(image: Image) -> Self {
        ContentBlock::Image {
            data: image.data,
            mime_type: image.mime_type,
        }
    }
}

/// What goes back for a command.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(id: Option<&str>, command: &str) -> Self {
        Response {
            kind: "response",
            id: id.map(str::to_string),
            command: command.to_string(),
            success: true,
            data: None,
            error: None,
        }
    }

    pub fn with(id: Option<&str>, command: &str, data: Value) -> Self {
        Response {
            data: Some(data),
            ..Response::ok(id, command)
        }
    }

    pub fn failed(id: Option<&str>, command: &str, error: impl Into<String>) -> Self {
        Response {
            kind: "response",
            id: id.map(str::to_string),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(error.into()),
        }
    }
}

/// What the session is doing, as `get_state` reports it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionState {
    pub model: String,
    pub provider: String,
    pub thinking_level: ThinkingLevel,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    pub auto_compaction_enabled: bool,
    pub message_count: usize,
    pub pending_message_count: usize,
}

/// A command a caller may invoke through a prompt.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    /// Where it came from: `builtin`, `skill`.
    pub source: String,
}

/// One message, as `get_messages` reports it.
pub fn message_json(message: &Message) -> Value {
    serde_json::to_value(message).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_read_from_the_shape_the_caller_sends() {
        let command: Command =
            serde_json::from_str(r#"{"type":"prompt","message":"hello","id":"1"}"#).unwrap();
        assert_eq!(command.name(), "prompt");
        assert_eq!(command.id(), Some("1"));

        let Command::Prompt {
            message, images, ..
        } = &command
        else {
            panic!("a prompt");
        };
        assert_eq!(message, "hello");
        assert!(images.is_empty());
    }

    
    #[test]
    fn forking_and_navigating_are_separate_commands() {
        let forked: Command = serde_json::from_str(r#"{"type":"fork","entry_id":"3"}"#).unwrap();
        assert_eq!(forked.name(), "fork");
        assert!(matches!(forked, Command::Fork { .. }));

        let moved: Command =
            serde_json::from_str(r#"{"type":"navigate_tree","entry_id":"3"}"#).unwrap();
        assert_eq!(moved.name(), "navigate_tree");
        assert!(matches!(moved, Command::NavigateTree { .. }));
    }

    #[test]
    fn an_id_is_optional_and_absent_when_it_was_not_sent() {
        let command: Command = serde_json::from_str(r#"{"type":"get_state"}"#).unwrap();
        assert_eq!(command.id(), None);
        assert_eq!(command.name(), "get_state");
    }

    #[test]
    fn every_command_answers_to_the_name_it_arrived_under() {
        for (line, name) in [
            (r#"{"type":"abort"}"#, "abort"),
            (r#"{"type":"new_session"}"#, "new_session"),
            (
                r#"{"type":"set_model","provider":"openrouter","model_id":"x"}"#,
                "set_model",
            ),
            (r#"{"type":"cycle_model"}"#, "cycle_model"),
            (r#"{"type":"get_available_models"}"#, "get_available_models"),
            (
                r#"{"type":"set_thinking_level","level":"high"}"#,
                "set_thinking_level",
            ),
            (r#"{"type":"cycle_thinking_level"}"#, "cycle_thinking_level"),
            (r#"{"type":"compact"}"#, "compact"),
            (
                r#"{"type":"set_auto_compaction","enabled":false}"#,
                "set_auto_compaction",
            ),
            (r#"{"type":"bash","command":"ls"}"#, "bash"),
            (r#"{"type":"abort_bash"}"#, "abort_bash"),
            (r#"{"type":"get_session_stats"}"#, "get_session_stats"),
            (
                r#"{"type":"switch_session","session_path":"x"}"#,
                "switch_session",
            ),
            (r#"{"type":"fork","entry_id":"1"}"#, "fork"),
            (r#"{"type":"clone"}"#, "clone"),
            (r#"{"type":"get_entries"}"#, "get_entries"),
            (r#"{"type":"get_tree"}"#, "get_tree"),
            (
                r#"{"type":"get_last_assistant_text"}"#,
                "get_last_assistant_text",
            ),
            (
                r#"{"type":"set_session_name","name":"x"}"#,
                "set_session_name",
            ),
            (r#"{"type":"get_messages"}"#, "get_messages"),
            (r#"{"type":"get_commands"}"#, "get_commands"),
            (r#"{"type":"steer","message":"x"}"#, "steer"),
            (r#"{"type":"follow_up","message":"x"}"#, "follow_up"),
        ] {
            let command: Command =
                serde_json::from_str(line).unwrap_or_else(|error| panic!("{line}: {error}"));
            assert_eq!(command.name(), name);
        }
    }

    #[test]
    fn a_prompt_can_carry_images() {
        let command: Command = serde_json::from_str(
            r#"{"type":"prompt","message":"look","images":[{"data":"AAAA","mime_type":"image/png"}]}"#,
        )
        .unwrap();
        let Command::Prompt { images, .. } = command else {
            panic!("a prompt");
        };
        assert_eq!(images.len(), 1);
        let block: ContentBlock = images[0].clone().into();
        assert!(matches!(block, ContentBlock::Image { .. }));
    }

    #[test]
    fn a_response_carries_the_id_it_answers() {
        let answer = Response::with(Some("7"), "get_state", serde_json::json!({ "a": 1 }));
        let encoded = serde_json::to_value(&answer).unwrap();
        assert_eq!(encoded["type"], "response");
        assert_eq!(encoded["id"], "7");
        assert_eq!(encoded["command"], "get_state");
        assert_eq!(encoded["success"], true);
        assert_eq!(encoded["data"]["a"], 1);
        assert!(encoded.get("error").is_none());
    }

    #[test]
    fn a_failure_says_what_went_wrong_and_nothing_else() {
        let answer = Response::failed(None, "fork", "no such entry");
        let encoded = serde_json::to_value(&answer).unwrap();
        assert_eq!(encoded["success"], false);
        assert_eq!(encoded["error"], "no such entry");
        assert!(encoded.get("id").is_none());
        assert!(encoded.get("data").is_none());
    }
}
