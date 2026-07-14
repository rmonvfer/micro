//! Driving an [`Agent`] to completion and reading back what it emitted.

use std::ops::Deref;

use micro_agent::Agent;
use micro_types::AgentEvent;
use micro_types::AssistantMessage;
use micro_types::Message;
use micro_types::StreamEvent;

/// Run one exchange to completion and collect both what it returned and what it emitted.
pub async fn run_agent(agent: &mut Agent, prompt: Message) -> (Vec<Message>, EventLog) {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let messages = agent.run(prompt, &sender).await;
    drop(sender);

    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    (messages, EventLog(events))
}

/// Everything an agent run emitted, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct EventLog(Vec<AgentEvent>);

impl EventLog {
    pub fn new(events: Vec<AgentEvent>) -> Self {
        EventLog(events)
    }

    pub fn events(&self) -> &[AgentEvent] {
        &self.0
    }

    /// The variant of each event, which is how a test asserts on ordering without
    /// spelling out every payload.
    pub fn names(&self) -> Vec<&'static str> {
        self.0.iter().map(name_of).collect()
    }

    pub fn count(&self, name: &str) -> usize {
        self.names().iter().filter(|found| **found == name).count()
    }

    /// The index of the first event with this name, for asserting relative order.
    pub fn position(&self, name: &str) -> Option<usize> {
        self.names().iter().position(|found| *found == name)
    }

    pub fn deltas(&self) -> Vec<&StreamEvent> {
        self.0
            .iter()
            .filter_map(|event| match event {
                AgentEvent::MessageDelta { event } => Some(event),
                _ => None,
            })
            .collect()
    }

    /// The assistant text as it streamed, assembled from the text deltas.
    pub fn streamed_text(&self) -> String {
        self.deltas()
            .into_iter()
            .filter_map(|event| match event {
                StreamEvent::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn message_starts(&self) -> Vec<&Message> {
        self.messages_of(|event| match event {
            AgentEvent::MessageStart { message } => Some(message),
            _ => None,
        })
    }

    pub fn message_ends(&self) -> Vec<&Message> {
        self.messages_of(|event| match event {
            AgentEvent::MessageEnd { message } => Some(message),
            _ => None,
        })
    }

    /// Assistant messages reported complete, which excludes the placeholder an assistant
    /// `MessageStart` carries.
    pub fn assistant_message_ends(&self) -> Vec<&AssistantMessage> {
        self.message_ends()
            .into_iter()
            .filter_map(|message| match message {
                Message::Assistant(assistant) => Some(assistant),
                _ => None,
            })
            .collect()
    }

    /// Every tool the loop started: `(id, name, arguments)`.
    pub fn tool_starts(&self) -> Vec<(&str, &str, &serde_json::Value)> {
        self.0
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolStart {
                    id,
                    name,
                    arguments,
                } => Some((id.as_str(), name.as_str(), arguments)),
                _ => None,
            })
            .collect()
    }

    /// Every tool the loop finished: `(id, name, output, is_error)`.
    pub fn tool_ends(&self) -> Vec<(&str, &str, &str, bool)> {
        self.0
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolEnd {
                    id,
                    name,
                    output,
                    is_error,
                } => Some((id.as_str(), name.as_str(), output.as_str(), *is_error)),
                _ => None,
            })
            .collect()
    }

    /// Every retry the loop announced: `(attempt, delay_ms)`.
    pub fn retries(&self) -> Vec<(u32, u64)> {
        self.0
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Retry {
                    attempt, delay_ms, ..
                } => Some((*attempt, *delay_ms)),
                _ => None,
            })
            .collect()
    }

    /// The messages carried by the terminal `AgentEnd`.
    pub fn final_messages(&self) -> Option<&[Message]> {
        self.0.iter().find_map(|event| match event {
            AgentEvent::AgentEnd { messages } => Some(messages.as_slice()),
            _ => None,
        })
    }

    fn messages_of<'a>(
        &'a self,
        pick: impl Fn(&'a AgentEvent) -> Option<&'a Message>,
    ) -> Vec<&'a Message> {
        self.0.iter().filter_map(pick).collect()
    }
}

impl Deref for EventLog {
    type Target = [AgentEvent];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IntoIterator for EventLog {
    type Item = AgentEvent;
    type IntoIter = std::vec::IntoIter<AgentEvent>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

fn name_of(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::AgentStart => "AgentStart",
        AgentEvent::TurnStart => "TurnStart",
        AgentEvent::MessageStart { .. } => "MessageStart",
        AgentEvent::MessageDelta { .. } => "MessageDelta",
        AgentEvent::MessageEnd { .. } => "MessageEnd",
        AgentEvent::ToolStart { .. } => "ToolStart",
        AgentEvent::ToolEnd { .. } => "ToolEnd",
        AgentEvent::Retry { .. } => "Retry",
        AgentEvent::AgentEnd { .. } => "AgentEnd",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_types::StopReason;
    use micro_types::Usage;

    fn assistant(text: &str) -> Message {
        Message::Assistant(AssistantMessage {
            content: vec![micro_types::ContentBlock::text(text)],
            provider: "fake".into(),
            model: "test-model".into(),
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error: None,
            timestamp: 0,
        })
    }

    fn log() -> EventLog {
        EventLog::new(vec![
            AgentEvent::AgentStart,
            AgentEvent::TurnStart,
            AgentEvent::MessageStart {
                message: assistant(""),
            },
            AgentEvent::MessageDelta {
                event: StreamEvent::TextDelta {
                    index: 0,
                    delta: "he".into(),
                },
            },
            AgentEvent::MessageDelta {
                event: StreamEvent::TextDelta {
                    index: 0,
                    delta: "llo".into(),
                },
            },
            AgentEvent::MessageEnd {
                message: assistant("hello"),
            },
            AgentEvent::AgentEnd {
                messages: vec![assistant("hello")],
            },
        ])
    }

    #[test]
    fn names_report_the_event_order() {
        assert_eq!(
            log().names(),
            vec![
                "AgentStart",
                "TurnStart",
                "MessageStart",
                "MessageDelta",
                "MessageDelta",
                "MessageEnd",
                "AgentEnd",
            ]
        );
    }

    #[test]
    fn counting_and_positions_locate_events() {
        let log = log();
        assert_eq!(log.count("MessageDelta"), 2);
        assert_eq!(log.count("ToolStart"), 0);
        assert!(log.position("AgentStart") < log.position("AgentEnd"));
        assert_eq!(log.position("Retry"), None);
    }

    #[test]
    fn streamed_text_reassembles_the_deltas() {
        assert_eq!(log().streamed_text(), "hello");
    }

    #[test]
    fn terminal_messages_are_read_from_agent_end() {
        let log = log();
        assert_eq!(log.final_messages().unwrap().len(), 1);
        assert_eq!(log.assistant_message_ends().len(), 1);
        assert_eq!(log.assistant_message_ends()[0].text(), "hello");
    }
}
