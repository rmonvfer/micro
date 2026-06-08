//! What a phone asks for, carried out against the session running here.

use crate::protocol::MachinePayload;
use crate::protocol::PhoneCommand;
use crate::protocol::PhonePayload;
use serde_json::json;
use serde_json::Value;

/// One model a session can switch to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableModel {
    pub id: String,
    pub name: String,
    pub provider: String,
}

/// One command the phone may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    /// Without the leading slash, which is the form both ends resolve by.
    pub name: String,
    pub description: String,
}

/// The session as the phone's controls show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub model: String,
    pub provider: String,
    pub thinking_level: String,
    pub session_name: String,
    pub cwd: String,
    pub is_streaming: bool,
}

/// What the bridge can ask of the session it is driving.
pub trait Session {
    fn submit(&mut self, text: &str, delivery: Delivery) -> Result<(), String>;
    fn abort(&mut self);
    fn is_idle(&self) -> bool;

    fn entries(&self) -> Vec<Value>;
    fn state(&self) -> SessionState;
    fn available_models(&self) -> Vec<AvailableModel>;
    /// The commands the phone is offered.
    fn commands(&self) -> Vec<SlashCommand>;
    fn set_model(&mut self, model_id: &str) -> Result<(), String>;
    fn set_thinking_level(&mut self, level: &str) -> Result<(), String>;
}

/// When submitted text should reach the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Start a turn with it.
    Prompt,
    /// Into the turn already running.
    Steer,
    /// After the turn already running.
    FollowUp,
}

/// Turns what the phone sends into what the session does, and back.
pub struct Bridge {
    session_id: String,
}

impl Bridge {
    pub fn new(session_id: impl Into<String>) -> Self {
        Bridge {
            session_id: session_id.into(),
        }
    }

    /// Wraps a session event for the phone.
    pub fn mirror(&self, event: Value) -> MachinePayload {
        MachinePayload::Event {
            session_id: self.session_id.clone(),
            event,
        }
    }

    /// Carries out one thing the phone asked for, and says what came of it.
    pub fn handle(&self, session: &mut impl Session, payload: PhonePayload) -> MachinePayload {
        let PhonePayload::Command {
            session_id,
            id,
            command,
        } = payload;

        if session_id != self.session_id {
            return self.answer(
                &id,
                command.name(),
                Err("session not active on this machine".into()),
            );
        }

        let outcome = self.run(session, &command);
        self.answer(&id, command.name(), outcome)
    }

    fn run(
        &self,
        session: &mut impl Session,
        command: &PhoneCommand,
    ) -> Result<Option<Value>, String> {
        match command {
            PhoneCommand::Prompt { text } => match session.is_idle() {
                true => session.submit(text, Delivery::Prompt).map(|()| None),
                false => Err("agent is busy — use steer or follow_up".into()),
            },
            PhoneCommand::Steer { text } => session.submit(text, Delivery::Steer).map(|()| None),
            PhoneCommand::FollowUp { text } => {
                session.submit(text, Delivery::FollowUp).map(|()| None)
            }
            PhoneCommand::Abort => {
                session.abort();
                Ok(None)
            }
            PhoneCommand::GetEntries => Ok(Some(json!({ "entries": session.entries() }))),
            PhoneCommand::GetState => {
                let state = session.state();
                Ok(Some(json!({
                    "model": state.model,
                    "provider": state.provider,
                    "thinkingLevel": state.thinking_level,
                    "sessionName": state.session_name,
                    "cwd": state.cwd,
                    "isStreaming": state.is_streaming,
                })))
            }
            PhoneCommand::GetAvailableModels => Ok(Some(json!({
                "models": session
                    .available_models()
                    .into_iter()
                    .map(|model| json!({
                        "id": model.id,
                        "name": model.name,
                        "provider": model.provider,
                    }))
                    .collect::<Vec<_>>(),
            }))),

            PhoneCommand::GetCommands => Ok(Some(json!({
                "commands": session
                    .commands()
                    .into_iter()
                    .map(|command| json!({
                        "name": command.name,
                        "description": command.description,
                    }))
                    .collect::<Vec<_>>(),
            }))),
            PhoneCommand::SetModel { model_id } => session.set_model(model_id).map(|()| None),
            PhoneCommand::SetThinkingLevel { level } => {
                session.set_thinking_level(level).map(|()| None)
            }
            PhoneCommand::Unknown => Err("this machine does not know that command".into()),
        }
    }

    fn answer(
        &self,
        id: &str,
        command: &str,
        outcome: Result<Option<Value>, String>,
    ) -> MachinePayload {
        let (success, data, error) = match outcome {
            Ok(data) => (true, data, None),
            Err(reason) => (false, None, Some(reason)),
        };
        MachinePayload::Response {
            session_id: self.session_id.clone(),
            id: id.to_string(),
            command: command.to_string(),
            success,
            data,
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session that records what it was asked to do.
    #[derive(Default)]
    struct Fake {
        idle: bool,
        submitted: Vec<(String, Delivery)>,
        aborted: bool,
        model: Option<String>,
        thinking: Option<String>,
        refuse: Option<String>,
    }

    impl Fake {
        fn idle() -> Self {
            Fake {
                idle: true,
                ..Fake::default()
            }
        }
    }

    impl Session for Fake {
        fn submit(&mut self, text: &str, delivery: Delivery) -> Result<(), String> {
            if let Some(reason) = &self.refuse {
                return Err(reason.clone());
            }
            self.submitted.push((text.to_string(), delivery));
            Ok(())
        }

        fn abort(&mut self) {
            self.aborted = true;
        }

        fn is_idle(&self) -> bool {
            self.idle
        }

        fn entries(&self) -> Vec<Value> {
            vec![json!({ "type": "message", "id": "e1" })]
        }

        fn state(&self) -> SessionState {
            SessionState {
                model: "claude-sonnet-5".into(),
                provider: "anthropic".into(),
                thinking_level: "medium".into(),
                session_name: "a session".into(),
                cwd: "/work".into(),
                is_streaming: !self.idle,
            }
        }

        fn available_models(&self) -> Vec<AvailableModel> {
            vec![AvailableModel {
                id: "claude-sonnet-5".into(),
                name: "Sonnet 5".into(),
                provider: "anthropic".into(),
            }]
        }

        fn commands(&self) -> Vec<SlashCommand> {
            vec![SlashCommand {
                name: "compact".into(),
                description: "Compact this session's context".into(),
            }]
        }

        fn set_model(&mut self, model_id: &str) -> Result<(), String> {
            if let Some(reason) = &self.refuse {
                return Err(reason.clone());
            }
            self.model = Some(model_id.to_string());
            Ok(())
        }

        fn set_thinking_level(&mut self, level: &str) -> Result<(), String> {
            if let Some(reason) = &self.refuse {
                return Err(reason.clone());
            }
            self.thinking = Some(level.to_string());
            Ok(())
        }
    }

    fn command(command: PhoneCommand) -> PhonePayload {
        PhonePayload::Command {
            session_id: "s1".into(),
            id: "c1".into(),
            command,
        }
    }

    fn response(payload: &MachinePayload) -> (bool, Option<Value>, Option<String>) {
        match payload {
            MachinePayload::Response {
                success,
                data,
                error,
                ..
            } => (*success, data.clone(), error.clone()),
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn a_prompt_while_idle_is_submitted() {
        let mut session = Fake::idle();
        let bridge = Bridge::new("s1");

        let answer = bridge.handle(
            &mut session,
            command(PhoneCommand::Prompt {
                text: "what changed?".into(),
            }),
        );

        assert!(response(&answer).0);
        assert_eq!(
            session.submitted,
            vec![("what changed?".to_string(), Delivery::Prompt)]
        );
    }

    #[test]
    fn a_prompt_while_busy_is_refused_with_the_alternatives() {
        let mut session = Fake::default();
        let bridge = Bridge::new("s1");

        let answer = bridge.handle(
            &mut session,
            command(PhoneCommand::Prompt {
                text: "what changed?".into(),
            }),
        );

        let (success, _, error) = response(&answer);
        assert!(!success);
        assert_eq!(error.unwrap(), "agent is busy — use steer or follow_up");
        assert!(session.submitted.is_empty());
    }

    /// Both ways of reaching a running turn work while it runs, which is the whole point of them.
    #[test]
    fn a_steer_and_a_follow_up_reach_a_running_turn() {
        let mut session = Fake::default();
        let bridge = Bridge::new("s1");

        bridge.handle(
            &mut session,
            command(PhoneCommand::Steer {
                text: "go left".into(),
            }),
        );
        bridge.handle(
            &mut session,
            command(PhoneCommand::FollowUp {
                text: "then right".into(),
            }),
        );

        assert_eq!(
            session.submitted,
            vec![
                ("go left".to_string(), Delivery::Steer),
                ("then right".to_string(), Delivery::FollowUp),
            ]
        );
    }

    #[test]
    fn an_abort_stops_the_turn() {
        let mut session = Fake::default();
        Bridge::new("s1").handle(&mut session, command(PhoneCommand::Abort));
        assert!(session.aborted);
    }

    #[test]
    fn the_state_is_answered_in_the_shape_the_phone_reads() {
        let mut session = Fake::idle();
        let answer = Bridge::new("s1").handle(&mut session, command(PhoneCommand::GetState));

        let (success, data, _) = response(&answer);
        assert!(success);
        let data = data.unwrap();
        assert_eq!(data["model"], "claude-sonnet-5");
        assert_eq!(data["thinkingLevel"], "medium");
        assert_eq!(data["sessionName"], "a session");
        assert_eq!(data["isStreaming"], false);
    }

    #[test]
    fn the_entries_models_and_commands_are_answered_with_what_the_session_has() {
        let mut session = Fake::idle();
        let bridge = Bridge::new("s1");

        let entries = bridge.handle(&mut session, command(PhoneCommand::GetEntries));
        assert_eq!(response(&entries).1.unwrap()["entries"][0]["id"], "e1");

        let models = bridge.handle(&mut session, command(PhoneCommand::GetAvailableModels));
        let models = response(&models).1.unwrap();
        assert_eq!(models["models"][0]["id"], "claude-sonnet-5");
        assert_eq!(models["models"][0]["provider"], "anthropic");

        let commands = bridge.handle(&mut session, command(PhoneCommand::GetCommands));
        assert_eq!(
            response(&commands).1.unwrap()["commands"][0]["name"],
            "compact"
        );
    }

    #[test]
    fn the_model_and_the_thinking_level_can_be_changed() {
        let mut session = Fake::idle();
        let bridge = Bridge::new("s1");

        bridge.handle(
            &mut session,
            command(PhoneCommand::SetModel {
                model_id: "gpt-5".into(),
            }),
        );
        bridge.handle(
            &mut session,
            command(PhoneCommand::SetThinkingLevel {
                level: "high".into(),
            }),
        );

        assert_eq!(session.model.as_deref(), Some("gpt-5"));
        assert_eq!(session.thinking.as_deref(), Some("high"));
    }

    #[test]
    fn a_session_that_refuses_is_answered_with_its_reason() {
        let mut session = Fake {
            idle: true,
            refuse: Some("unknown model \"gpt-9\"".into()),
            ..Fake::default()
        };

        let answer = Bridge::new("s1").handle(
            &mut session,
            command(PhoneCommand::SetModel {
                model_id: "gpt-9".into(),
            }),
        );

        let (success, _, error) = response(&answer);
        assert!(!success);
        assert_eq!(error.unwrap(), "unknown model \"gpt-9\"");
    }

    #[test]
    fn a_command_for_another_session_is_answered_rather_than_run() {
        let mut session = Fake::idle();
        let answer = Bridge::new("s1").handle(
            &mut session,
            PhonePayload::Command {
                session_id: "another".into(),
                id: "c1".into(),
                command: PhoneCommand::Prompt {
                    text: "what changed?".into(),
                },
            },
        );

        let (success, _, error) = response(&answer);
        assert!(!success);
        assert_eq!(error.unwrap(), "session not active on this machine");
        assert!(session.submitted.is_empty());
    }

    #[test]
    fn a_command_this_machine_does_not_know_is_answered_with_a_reason() {
        let mut session = Fake::idle();
        let answer = Bridge::new("s1").handle(&mut session, command(PhoneCommand::Unknown));

        let (success, _, error) = response(&answer);
        assert!(!success);
        assert!(error.unwrap().contains("does not know"));
    }

    /// Every answer carries back the id it was asked under, which is how a phone with several
    /// requests in flight tells the answers apart.
    #[test]
    fn an_answer_carries_the_id_and_the_command_it_answers() {
        let mut session = Fake::idle();
        let answer = Bridge::new("s1").handle(&mut session, command(PhoneCommand::Abort));

        match answer {
            MachinePayload::Response {
                session_id,
                id,
                command,
                ..
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(id, "c1");
                assert_eq!(command, "abort");
            }
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn a_mirrored_event_is_wrapped_for_the_session_it_came_from() {
        let payload = Bridge::new("s1").mirror(json!({ "type": "agent_start" }));
        assert_eq!(
            payload,
            MachinePayload::Event {
                session_id: "s1".into(),
                event: json!({ "type": "agent_start" }),
            }
        );
    }
}
