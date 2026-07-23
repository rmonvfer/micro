//! Turning what the agent reports into what an extension listens for.
//!
//! ohm names its events from the session's point of view — a turn starts, a message ends,
//! a tool finished — and an extension subscribes by that name. micro's agent reports the
//! same moments under its own names, so the two are mapped here, in one place, rather than
//! at every point that emits one.
//!
//! An event micro has no counterpart for is not invented: it simply never fires, and an
//! extension listening for it waits forever rather than being lied to.

use micro_types::AgentEvent;
use serde_json::json;
use serde_json::Value;

/// What an extension calls this event, or nothing when it is not one they can hear.
pub fn name_of(event: &AgentEvent) -> Option<&'static str> {
    match event {
        AgentEvent::AgentStart => Some("agent_start"),
        AgentEvent::AgentEnd { .. } => Some("agent_end"),
        AgentEvent::TurnStart => Some("turn_start"),
        AgentEvent::MessageStart { .. } => Some("message_start"),
        AgentEvent::MessageDelta { .. } => Some("message_update"),
        AgentEvent::MessageEnd { .. } => Some("message_end"),
        AgentEvent::ToolStart { .. } => Some("tool_execution_start"),
        AgentEvent::ToolEnd { .. } => Some("tool_execution_end"),
        // A retry is micro's own business: the request is repeated, and nothing about the
        // conversation changed.
        AgentEvent::Retry { .. } => None,
    }
}

/// What the event carries, in the shape ohm's handlers are written against.
pub fn payload_of(event: &AgentEvent) -> Value {
    match event {
        AgentEvent::AgentStart | AgentEvent::TurnStart => json!({}),
        AgentEvent::AgentEnd { messages } => json!({ "messages": messages }),
        AgentEvent::MessageStart { message } => json!({ "message": message }),
        AgentEvent::MessageDelta { event } => json!({ "delta": event }),
        AgentEvent::MessageEnd { message } => json!({ "message": message }),
        AgentEvent::ToolStart {
            id,
            name,
            arguments,
        } => json!({ "toolCallId": id, "toolName": name, "args": arguments }),
        AgentEvent::ToolEnd {
            id,
            name,
            output,
            is_error,
        } => json!({
            "toolCallId": id,
            "toolName": name,
            "result": output,
            "isError": is_error,
        }),
        AgentEvent::Retry { .. } => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_moments_an_extension_listens_for_are_named_as_ohm_names_them() {
        assert_eq!(name_of(&AgentEvent::AgentStart), Some("agent_start"));
        assert_eq!(name_of(&AgentEvent::TurnStart), Some("turn_start"));
        assert_eq!(
            name_of(&AgentEvent::ToolStart {
                id: "call_1".into(),
                name: "read".into(),
                arguments: json!({}),
            }),
            Some("tool_execution_start")
        );
        assert_eq!(
            name_of(&AgentEvent::AgentEnd {
                messages: Vec::new()
            }),
            Some("agent_end")
        );
    }

    /// A retry is not a moment in the conversation, so nothing is told about it.
    #[test]
    fn a_retry_is_not_an_event_an_extension_hears() {
        assert_eq!(
            name_of(&AgentEvent::Retry {
                attempt: 1,
                max_attempts: 3,
                delay_ms: 100,
            }),
            None
        );
    }

    #[test]
    fn a_tool_carries_what_it_was_called_with_and_what_it_returned() {
        let started = payload_of(&AgentEvent::ToolStart {
            id: "call_1".into(),
            name: "read".into(),
            arguments: json!({ "path": "src/main.rs" }),
        });
        assert_eq!(started["toolCallId"], "call_1");
        assert_eq!(started["toolName"], "read");
        assert_eq!(started["args"]["path"], "src/main.rs");

        let ended = payload_of(&AgentEvent::ToolEnd {
            id: "call_1".into(),
            name: "read".into(),
            output: "fn main() {}".into(),
            is_error: false,
        });
        assert_eq!(ended["result"], "fn main() {}");
        assert_eq!(ended["isError"], false);
    }

    #[test]
    fn a_message_carries_the_message() {
        let payload = payload_of(&AgentEvent::MessageStart {
            message: micro_types::Message::user("hello"),
        });
        assert_eq!(payload["message"]["role"], "user");
    }
}
