//! The machine against a relay that is actually running.

mod support;

use futures::SinkExt;
use futures::StreamExt;
use micro_remote::Direction;
use micro_remote::FrameDecoder;
use micro_remote::FrameEncoder;
use micro_remote::MachinePayload;
use micro_remote::PhoneCommand;
use micro_remote::PhonePayload;
use micro_remote::RelayClient;
use micro_remote::RelayConfig;
use micro_remote::RelayEvent;
use micro_remote::Role;
use std::time::Duration;
use support::RelayFixture;

fn config(relay: &RelayFixture, pairing_id: &str) -> RelayConfig {
    RelayConfig {
        relay_url: relay.url.clone(),
        pairing_id: pairing_id.into(),
        secret: vec![9u8; 32],
        session_id: "s1".into(),
    }
}

/// A phone, as far as the relay is concerned: the other leg of the channel, speaking the same
/// protocol from the other side.
struct Phone {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    encoder: FrameEncoder,
    decoder: FrameDecoder,
}

impl Phone {
    async fn join(relay: &RelayFixture, config: &RelayConfig) -> Phone {
        let url = format!(
            "{}/channel/{}?role=phone&token={}",
            relay.url.replacen("https://", "wss://", 1),
            config.pairing_id,
            urlencoding(&config.auth_token(Role::Phone))
        );
        let (socket, _) = tokio_tungstenite::connect_async_tls_with_config(
            url,
            None,
            false,
            Some(relay.connector.clone()),
        )
        .await
        .expect("the relay accepts the phone's leg");
        Phone {
            socket,

            encoder: FrameEncoder::new(micro_remote::derive_key(
                &config.secret,
                &config.pairing_id,
                Direction::PhoneToMachine,
            )),
            decoder: FrameDecoder::new(micro_remote::derive_key(
                &config.secret,
                &config.pairing_id,
                Direction::MachineToPhone,
            )),
        }
    }

    async fn send(&mut self, payload: &PhonePayload) {
        let frame = self.encoder.encode(payload);
        self.socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&frame).unwrap().into(),
            ))
            .await
            .expect("the phone's frame goes out");
    }

    /// The next payload the machine sent, ignoring the relay's own chatter.
    async fn next(&mut self) -> Option<MachinePayload> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let message = tokio::time::timeout_at(deadline, self.socket.next())
                .await
                .ok()??;
            let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = message else {
                continue;
            };
            let Ok(frame) = serde_json::from_str(&text) else {
                continue;
            };
            if let Some(payload) = self.decoder.decode::<MachinePayload>(&frame) {
                return Some(payload);
            }
        }
    }
}

fn urlencoding(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[tokio::test]
async fn a_session_reaches_a_phone_through_a_running_relay() {
    let relay = RelayFixture::start().await;
    let config = config(&relay, "micro-test-offer");

    micro_remote::register_with_client(&config, &relay.http)
        .await
        .expect("the relay accepts a new pairing");
    let (events, mut incoming) = tokio::sync::mpsc::unbounded_channel();
    let client = RelayClient::start_with_transport(
        config.clone(),
        events,
        relay.http.clone(),
        Some(relay.connector.clone()),
    )
    .expect("the secure relay URL starts a client");

    let connected = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = incoming.recv().await {
            if matches!(
                event,
                RelayEvent::State(micro_remote::ConnectionState::Connected)
            ) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(connected, "the machine connects to the relay");

    let mut phone = Phone::join(&relay, &config).await;

    let peered = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = incoming.recv().await {
            if event == (RelayEvent::Peer { connected: true }) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(peered, "the machine hears that the phone joined");

    client.send(MachinePayload::SessionOffer {
        session_id: "s1".into(),
        session_name: "a session".into(),
        cwd: "/work".into(),
        machine_name: "test".into(),
    });

    match phone.next().await {
        Some(MachinePayload::SessionOffer {
            session_id,
            session_name,
            ..
        }) => {
            assert_eq!(session_id, "s1");
            assert_eq!(session_name, "a session");
        }
        other => panic!("expected the offer, got {other:?}"),
    }

    phone
        .send(&PhonePayload::Command {
            session_id: "s1".into(),
            id: "c1".into(),
            command: PhoneCommand::GetState,
        })
        .await;

    let asked = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = incoming.recv().await {
            if let RelayEvent::Payload(payload) = event {
                return Some(payload);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();

    assert_eq!(
        asked,
        Some(PhonePayload::Command {
            session_id: "s1".into(),
            id: "c1".into(),
            command: PhoneCommand::GetState,
        })
    );

    client.stop();
}

/// A token the relay has never been given the hash of does not open a channel.
#[tokio::test]
async fn a_leg_without_a_registered_pairing_is_refused() {
    let relay = RelayFixture::start().await;
    let config = config(&relay, "micro-test-unregistered");
    let url = format!(
        "{}/channel/{}?role=machine&token={}",
        relay.url.replacen("https://", "wss://", 1),
        config.pairing_id,
        urlencoding(&config.auth_token(Role::Machine))
    );

    match tokio_tungstenite::connect_async_tls_with_config(
        url,
        None,
        false,
        Some(relay.connector.clone()),
    )
    .await
    {
        Err(_) => {}
        Ok((mut socket, _)) => {
            let closed = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
            match closed {
                Ok(
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
                    | None
                    | Some(Err(_)),
                ) => {}
                other => panic!("expected the relay to decline, got {other:?}"),
            }
        }
    }
}
