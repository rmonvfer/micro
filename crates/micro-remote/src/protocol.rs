//! What the two ends say to each other, inside the sealed frames.

use crate::crypto::open;
use crate::crypto::seal;
use crate::crypto::WireFrame;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

/// What the machine tells the phone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MachinePayload {
    /// A session is on the air, and this is what it is.
    SessionOffer {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "sessionName")]
        session_name: String,
        cwd: String,
        #[serde(rename = "machineName")]
        machine_name: String,
    },
    /// The session is no longer reachable here.
    SessionOffline {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    /// Something the session did, forwarded as the agent reported it.
    Event {
        #[serde(rename = "sessionId")]
        session_id: String,
        event: Value,
    },
    /// The answer to one command the phone sent.
    Response {
        #[serde(rename = "sessionId")]
        session_id: String,
        id: String,
        command: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// What the phone asks of the machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhonePayload {
    Command {
        #[serde(rename = "sessionId")]
        session_id: String,
        id: String,
        command: PhoneCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhoneCommand {
    Prompt {
        text: String,
    },
    Steer {
        text: String,
    },
    FollowUp {
        text: String,
    },
    Abort,
    GetState,
    GetEntries,
    GetAvailableModels,
    GetCommands,
    SetModel {
        #[serde(rename = "modelId")]
        model_id: String,
    },
    SetThinkingLevel {
        level: String,
    },
    /// Anything a newer phone asks for that this machine has no answer to.
    #[serde(other)]
    Unknown,
}

impl PhoneCommand {
    /// What to call this command when answering it.
    pub fn name(&self) -> &'static str {
        match self {
            PhoneCommand::Prompt { .. } => "prompt",
            PhoneCommand::Steer { .. } => "steer",
            PhoneCommand::FollowUp { .. } => "follow_up",
            PhoneCommand::Abort => "abort",
            PhoneCommand::GetState => "get_state",
            PhoneCommand::GetEntries => "get_entries",
            PhoneCommand::GetAvailableModels => "get_available_models",
            PhoneCommand::GetCommands => "get_commands",
            PhoneCommand::SetModel { .. } => "set_model",
            PhoneCommand::SetThinkingLevel { .. } => "set_thinking_level",
            PhoneCommand::Unknown => "unknown",
        }
    }
}

/// What a push notification carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushPayload {
    pub kind: PushKind,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "sessionName")]
    pub session_name: String,
    #[serde(rename = "machineName")]
    pub machine_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushKind {
    /// A session has been offered to the phone.
    Offer,
    /// A turn has finished and the session is waiting.
    Settled,
}

/// A payload with its place in the sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Envelope<T> {
    seq: u64,
    payload: T,
}

/// Seals outgoing payloads, numbering each one.
pub struct FrameEncoder {
    key: [u8; 32],
    seq: u64,
}

impl FrameEncoder {
    pub fn new(key: [u8; 32]) -> Self {
        FrameEncoder { key, seq: 0 }
    }

    pub fn encode<T: Serialize>(&mut self, payload: &T) -> WireFrame {
        self.seq += 1;
        let envelope = Envelope {
            seq: self.seq,
            payload,
        };
        let plaintext =
            serde_json::to_string(&envelope).expect("a payload of our own always serializes");
        seal(&self.key, &plaintext)
    }
}

/// Opens incoming frames, refusing any that does not move the sequence forward.
pub struct FrameDecoder {
    key: [u8; 32],
    last_seq: u64,
    seen_nonces: HashSet<String>,
}

impl FrameDecoder {
    pub fn new(key: [u8; 32]) -> Self {
        FrameDecoder {
            key,
            last_seq: 0,
            seen_nonces: HashSet::new(),
        }
    }

    /// The payload inside a frame, or nothing when there is no reason to trust it.
    pub fn decode<T: for<'de> Deserialize<'de>>(&mut self, frame: &WireFrame) -> Option<T> {
        if self.seen_nonces.contains(&frame.n) {
            return None;
        }
        let plaintext = open(&self.key, frame).ok()?;
        let envelope: Envelope<T> = serde_json::from_str(&plaintext).ok()?;
        if envelope.seq <= self.last_seq {
            return None;
        }
        self.seen_nonces.insert(frame.n.clone());
        self.last_seq = envelope.seq;
        Some(envelope.payload)
    }

    /// Restarts sequence validation for a reconnected peer while retaining replay history.
    pub fn reset(&mut self) {
        self.last_seq = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::derive_key;
    use crate::crypto::Direction;

    fn key() -> [u8; 32] {
        derive_key(b"a secret", "pairing", Direction::MachineToPhone)
    }

    #[test]
    fn a_payload_survives_the_round_trip() {
        let mut encoder = FrameEncoder::new(key());
        let mut decoder = FrameDecoder::new(key());
        let payload = MachinePayload::SessionOffer {
            session_id: "s1".into(),
            session_name: "a session".into(),
            cwd: "/work".into(),
            machine_name: "laptop".into(),
        };

        let frame = encoder.encode(&payload);
        let decoded: MachinePayload = decoder.decode(&frame).unwrap();
        assert_eq!(decoded, payload);
    }

    /// A frame sent twice is acted on once.
    #[test]
    fn a_replayed_frame_is_dropped() {
        let mut encoder = FrameEncoder::new(key());
        let mut decoder = FrameDecoder::new(key());
        let frame = encoder.encode(&MachinePayload::SessionOffline {
            session_id: "s1".into(),
        });

        assert!(decoder.decode::<MachinePayload>(&frame).is_some());
        assert!(decoder.decode::<MachinePayload>(&frame).is_none());
    }

    /// A reconnected peer may restart its sequence, but a frame captured before reconnect remains
    /// invalid.
    #[test]
    fn a_reset_decoder_accepts_a_new_sequence_but_rejects_an_old_frame() {
        let mut encoder = FrameEncoder::new(key());
        let mut decoder = FrameDecoder::new(key());
        let captured = encoder.encode(&MachinePayload::SessionOffline {
            session_id: "s1".into(),
        });
        assert!(decoder.decode::<MachinePayload>(&captured).is_some());

        let mut reconnected = FrameEncoder::new(key());
        let frame = reconnected.encode(&MachinePayload::SessionOffline {
            session_id: "s1".into(),
        });
        assert!(decoder.decode::<MachinePayload>(&frame).is_none());

        decoder.reset();
        assert!(decoder.decode::<MachinePayload>(&frame).is_some());
        assert!(decoder.decode::<MachinePayload>(&captured).is_none());
    }

    #[test]
    fn a_command_the_phone_sends_parses_into_its_variant() {
        let payload: PhonePayload = serde_json::from_str(
            r#"{"type":"command","sessionId":"s1","id":"c1","command":{"type":"steer","text":"go left"}}"#,
        )
        .unwrap();
        let PhonePayload::Command { command, id, .. } = payload;
        assert_eq!(id, "c1");
        assert_eq!(
            command,
            PhoneCommand::Steer {
                text: "go left".into()
            }
        );
    }

    #[test]
    fn a_command_this_machine_does_not_know_still_parses() {
        let payload: PhonePayload = serde_json::from_str(
            r#"{"type":"command","sessionId":"s1","id":"c1","command":{"type":"do_a_barrel_roll"}}"#,
        )
        .unwrap();
        let PhonePayload::Command { command, .. } = payload;
        assert_eq!(command, PhoneCommand::Unknown);
    }

    /// The wire spells these in camelCase; a payload written any other way is one the phone quietly
    /// ignores.
    #[test]
    fn payloads_are_written_the_way_the_phone_reads_them() {
        let json = serde_json::to_value(MachinePayload::Response {
            session_id: "s1".into(),
            id: "c1".into(),
            command: "prompt".into(),
            success: true,
            data: None,
            error: None,
        })
        .unwrap();

        assert_eq!(json["type"], "response");
        assert_eq!(json["sessionId"], "s1");

        assert!(json.get("data").is_none());
        assert!(json.get("error").is_none());
    }
}
