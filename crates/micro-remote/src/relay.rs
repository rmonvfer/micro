//! The connection to the relay, and how it is kept.

use crate::crypto::derive_key;
use crate::crypto::seal;
use crate::crypto::Direction;
use crate::crypto::WireFrame;
use crate::protocol::FrameDecoder;
use crate::protocol::FrameEncoder;
use crate::protocol::MachinePayload;
use crate::protocol::PhonePayload;
use crate::protocol::PushPayload;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::Mac;
use serde::Deserialize;
use sha2::Digest;
use std::time::Duration;
use tokio::sync::mpsc;

/// How long to wait before the first reconnect, and how long at most.
const BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// The relay closes a channel with this code when it has never heard of the pairing.
const CLOSE_PAIRING_UNKNOWN: u16 = 4404;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Machine,
    Phone,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Role::Machine => "machine",
            Role::Phone => "phone",
        }
    }
}

/// What the machine tells the caller about the connection.
#[derive(Debug, Clone, PartialEq)]
pub enum RelayEvent {
    /// The machine's own connection to the relay.
    State(ConnectionState),
    /// Whether the phone is on the other leg.
    Peer { connected: bool },
    /// Something the phone sent.
    Payload(PhonePayload),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Closed,
}

/// Everything needed to reach one phone through one relay.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub relay_url: String,
    pub pairing_id: String,
    pub secret: Vec<u8>,

    pub session_id: String,
}

impl RelayConfig {
    pub fn validate(&self) -> Result<(), String> {
        validate_relay_url(&self.relay_url)
    }

    /// The token that proves this end may use its leg of the channel.
    pub fn auth_token(&self, role: Role) -> String {
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&self.secret)
            .expect("HMAC accepts a key of any length");
        mac.update(format!("parley-remote/auth/{}/{}", self.pairing_id, role.label()).as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    /// What the relay stores in place of the token.
    fn verifier(token: &str) -> String {
        let digest = sha2::Sha256::digest(token.as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn channel_url(&self) -> Result<String, String> {
        self.validate()?;
        let base = websocket_base(&self.relay_url);
        Ok(format!(
            "{base}/channel/{}?role=machine&session={}&token={}",
            self.pairing_id,
            urlencode(&self.session_id),
            urlencode(&self.auth_token(Role::Machine))
        ))
    }
}

/// A frame the relay wrote itself, telling one leg about the other.
#[derive(Debug, Deserialize)]
struct PeerFrame {
    relay: String,
    role: String,
    connected: bool,
}

/// What the caller keeps hold of: a way to send, and a way to stop.
pub struct RelayClient {
    config: RelayConfig,
    outgoing: mpsc::UnboundedSender<MachinePayload>,
    stop: mpsc::UnboundedSender<()>,
    http: reqwest::Client,
}

impl RelayClient {
    /// Opens the connection and starts keeping it.
    pub fn start(
        config: RelayConfig,
        events: mpsc::UnboundedSender<RelayEvent>,
    ) -> Result<Self, String> {
        Self::start_with_transport(config, events, reqwest::Client::new(), None)
    }

    #[doc(hidden)]
    pub fn start_with_transport(
        config: RelayConfig,
        events: mpsc::UnboundedSender<RelayEvent>,
        http: reqwest::Client,
        connector: Option<tokio_tungstenite::Connector>,
    ) -> Result<Self, String> {
        config.validate()?;
        let (outgoing, outgoing_rx) = mpsc::unbounded_channel();
        let (stop, stop_rx) = mpsc::unbounded_channel();

        tokio::spawn(run(
            config.clone(),
            events,
            outgoing_rx,
            stop_rx,
            http.clone(),
            connector,
        ));

        Ok(RelayClient {
            config,
            outgoing,
            stop,
            http,
        })
    }

    /// Sends a payload, if there is anywhere to send it.
    pub fn send(&self, payload: MachinePayload) {
        let _ = self.outgoing.send(payload);
    }

    pub fn stop(&self) {
        let _ = self.stop.send(());
    }

    /// Tells the relay a pairing exists, handing it the hashes of both legs' tokens.
    pub async fn register_pairing(&self) -> Result<(), String> {
        register_pairing(&self.http, &self.config).await
    }

    /// Asks the relay to wake the phone.
    pub async fn push_trigger(
        &self,
        push_key: &[u8; 32],
        payload: &PushPayload,
        collapse_id: Option<&str>,
    ) -> Result<(), String> {
        self.config.validate()?;
        let body = serde_json::to_string(payload).expect("a payload of our own serializes");
        let frame = seal(push_key, &body);
        let request = serde_json::json!({
            "pairingId": self.config.pairing_id,
            "token": self.config.auth_token(Role::Machine),
            "ct": serde_json::to_string(&frame).expect("a frame of our own serializes"),
            "collapseId": collapse_id,
        });

        let response = self
            .http
            .post(format!("{}/push", self.config.relay_url))
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("push failed: {error}"))?;
        match response.status().is_success() {
            true => Ok(()),
            false => Err(format!("push failed: {}", response.status())),
        }
    }
}

/// Tells the relay about a pairing before anything tries to use it.
pub async fn register(config: &RelayConfig) -> Result<(), String> {
    register_pairing(&reqwest::Client::new(), config).await
}

#[doc(hidden)]
pub async fn register_with_client(
    config: &RelayConfig,
    http: &reqwest::Client,
) -> Result<(), String> {
    register_pairing(http, config).await
}

async fn register_pairing(http: &reqwest::Client, config: &RelayConfig) -> Result<(), String> {
    config.validate()?;
    let body = serde_json::json!({
        "pairingId": config.pairing_id,
        "machineVerifier": RelayConfig::verifier(&config.auth_token(Role::Machine)),
        "phoneVerifier": RelayConfig::verifier(&config.auth_token(Role::Phone)),
    });
    let response = http
        .post(format!("{}/pairings", config.relay_url))
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("registering the pairing failed: {error}"))?;
    match response.status().is_success() {
        true => Ok(()),
        false => Err(format!(
            "registering the pairing failed: {}",
            response.status()
        )),
    }
}

/// The connection, for as long as it is wanted.
async fn run(
    config: RelayConfig,
    events: mpsc::UnboundedSender<RelayEvent>,
    mut outgoing: mpsc::UnboundedReceiver<MachinePayload>,
    mut stop: mpsc::UnboundedReceiver<()>,
    http: reqwest::Client,
    connector: Option<tokio_tungstenite::Connector>,
) {
    let mut backoff = BACKOFF_INITIAL;
    let mut decoder = FrameDecoder::new(derive_key(
        &config.secret,
        &config.pairing_id,
        Direction::PhoneToMachine,
    ));

    loop {
        if events
            .send(RelayEvent::State(ConnectionState::Connecting))
            .is_err()
        {
            return;
        }

        let attempt = tokio::select! {
            biased;
            _ = stop.recv() => break,
            attempt = tokio_tungstenite::connect_async_tls_with_config(
                config.channel_url().expect("a relay URL validated before the connection loop"),
                None,
                false,
                connector.clone(),
            ) => attempt,
        };

        let close_code = match attempt {
            Ok((socket, _)) => {
                backoff = BACKOFF_INITIAL;
                serve(
                    socket,
                    &config,
                    &events,
                    &mut outgoing,
                    &mut stop,
                    &mut decoder,
                )
                .await
            }

            Err(_) => Outcome::Retry(None),
        };

        let reregister = match close_code {
            Outcome::Stopped => break,
            Outcome::Retry(code) => code == Some(CLOSE_PAIRING_UNKNOWN),
        };

        let waited = tokio::select! {
            biased;
            _ = stop.recv() => false,
            _ = tokio::time::sleep(backoff) => true,
        };
        if !waited {
            break;
        }
        backoff = (backoff * 2).min(BACKOFF_CAP);

        if reregister {
            let _ = register_pairing(&http, &config).await;
        }
    }

    let _ = events.send(RelayEvent::State(ConnectionState::Closed));
}

/// Why a connection ended.
enum Outcome {
    /// The caller asked for it.
    Stopped,
    /// Something else ended it, with the close code if there was one.
    Retry(Option<u16>),
}

/// One connection, from open to close.
async fn serve(
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    config: &RelayConfig,
    events: &mpsc::UnboundedSender<RelayEvent>,
    outgoing: &mut mpsc::UnboundedReceiver<MachinePayload>,
    stop: &mut mpsc::UnboundedReceiver<()>,
    decoder: &mut FrameDecoder,
) -> Outcome {
    use futures::SinkExt;
    use futures::StreamExt;

    let (mut writer, mut reader) = socket.split();

    let mut encoder = FrameEncoder::new(derive_key(
        &config.secret,
        &config.pairing_id,
        Direction::MachineToPhone,
    ));
    decoder.reset();
    if events
        .send(RelayEvent::State(ConnectionState::Connected))
        .is_err()
    {
        return Outcome::Stopped;
    }

    loop {
        tokio::select! {
            biased;
            _ = stop.recv() => {
                let _ = writer.close().await;
                return Outcome::Stopped;
            }
            payload = outgoing.recv() => {
                let Some(payload) = payload else { return Outcome::Stopped };
                let frame = encoder.encode(&payload);
                let body = serde_json::to_string(&frame).expect("a frame of our own serializes");
                if writer.send(tokio_tungstenite::tungstenite::Message::Text(body.into())).await.is_err() {
                    return Outcome::Retry(None);
                }
            }
            message = reader.next() => {
                match message {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        handle(&text, events, decoder);
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(frame))) => {
                        return Outcome::Retry(frame.map(|frame| u16::from(frame.code)));
                    }

                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return Outcome::Retry(None),
                }
            }
        }
    }
}

/// One message off the socket: either the relay talking about the channel, or the phone talking
/// through it.
fn handle(text: &str, events: &mpsc::UnboundedSender<RelayEvent>, decoder: &mut FrameDecoder) {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };

    if parsed.get("relay").and_then(serde_json::Value::as_str) == Some("peer") {
        let Ok(peer) = serde_json::from_value::<PeerFrame>(parsed) else {
            return;
        };
        if peer.relay != "peer" || peer.role != "phone" {
            return;
        }

        if peer.connected {
            decoder.reset();
        }
        let _ = events.send(RelayEvent::Peer {
            connected: peer.connected,
        });
        return;
    }

    let Ok(frame) = serde_json::from_str::<WireFrame>(text) else {
        return;
    };
    let Some(payload) = decoder.decode::<PhonePayload>(&frame) else {
        return;
    };
    let _ = events.send(RelayEvent::Payload(payload));
}

/// The relay's websocket address, from the address its HTTP side is on.
fn websocket_base(relay_url: &str) -> String {
    format!(
        "wss://{}",
        relay_url
            .strip_prefix("https://")
            .expect("relay URLs are validated before conversion")
    )
}

/// Accepts only an HTTPS origin because relay requests carry pairing credentials.
pub(crate) fn validate_relay_url(relay_url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(relay_url)
        .map_err(|error| format!("the remote relay URL is invalid: {error}"))?;
    if parsed.scheme() != "https" {
        return Err("the remote relay URL must use HTTPS (and WSS for its channel)".into());
    }
    if parsed.host_str().is_none() {
        return Err("the remote relay URL must include a host".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("the remote relay URL must not contain credentials".into());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("the remote relay URL must not contain a query or fragment".into());
    }
    Ok(())
}

/// Percent-encodes the characters that would otherwise end a query value.
fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> RelayConfig {
        RelayConfig {
            relay_url: "https://relay.example".into(),
            pairing_id: "vector-pairing".into(),
            secret: vec![7u8; 32],
            session_id: "s1".into(),
        }
    }

    /// The two legs must not share a token, or either end could take the other's place.
    #[test]
    fn each_leg_has_its_own_token() {
        let config = config();
        assert_ne!(
            config.auth_token(Role::Machine),
            config.auth_token(Role::Phone)
        );
    }

    /// The token is derived.
    #[test]
    fn a_token_is_the_same_every_time_it_is_derived() {
        assert_eq!(
            config().auth_token(Role::Machine),
            config().auth_token(Role::Machine)
        );
    }

    #[test]
    fn the_verifier_is_not_the_token() {
        let token = config().auth_token(Role::Machine);
        let verifier = RelayConfig::verifier(&token);
        assert_ne!(verifier, token);
        assert_eq!(verifier.len(), 64);
        assert!(verifier
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn the_websocket_address_follows_the_relays_own_scheme() {
        assert_eq!(
            websocket_base("https://relay.example"),
            "wss://relay.example"
        );
    }

    #[test]
    fn plaintext_relay_urls_are_rejected() {
        for relay_url in ["http://relay.example", "ws://relay.example"] {
            let mut config = config();
            config.relay_url = relay_url.into();
            assert!(config.validate().is_err(), "accepted {relay_url}");
            assert!(config.channel_url().is_err(), "built a URL for {relay_url}");
        }
    }

    #[test]
    fn the_channel_address_carries_the_role_and_the_token() {
        let url = config().channel_url().unwrap();
        assert!(url.starts_with("wss://relay.example/channel/vector-pairing?"));
        assert!(url.contains("role=machine"));

        assert!(url.contains("token="));
    }

    #[test]
    fn a_peer_frame_is_reported_as_the_phone_arriving() {
        let (events, mut received) = mpsc::unbounded_channel();
        let mut decoder = FrameDecoder::new([0u8; 32]);

        handle(
            r#"{"relay":"peer","role":"phone","connected":true}"#,
            &events,
            &mut decoder,
        );
        assert_eq!(
            received.try_recv().unwrap(),
            RelayEvent::Peer { connected: true }
        );
    }

    /// The machine hearing about its own leg changes nothing.
    #[test]
    fn a_peer_frame_about_this_end_is_ignored() {
        let (events, mut received) = mpsc::unbounded_channel();
        let mut decoder = FrameDecoder::new([0u8; 32]);

        handle(
            r#"{"relay":"peer","role":"machine","connected":true}"#,
            &events,
            &mut decoder,
        );
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn a_message_that_is_neither_changes_nothing() {
        let (events, mut received) = mpsc::unbounded_channel();
        let mut decoder = FrameDecoder::new([0u8; 32]);

        for text in ["not json", "{}", "[]", r#"{"v":1,"n":"x","ct":"y"}"#] {
            handle(text, &events, &mut decoder);
        }
        assert!(received.try_recv().is_err());
    }

    /// A sealed frame from the phone comes back out as what the phone put in.
    #[test]
    fn a_sealed_frame_is_reported_as_its_payload() {
        let config = config();
        let key = derive_key(
            &config.secret,
            &config.pairing_id,
            Direction::PhoneToMachine,
        );
        let mut encoder = FrameEncoder::new(key);
        let frame = encoder.encode(&PhonePayload::Command {
            session_id: "s1".into(),
            id: "c1".into(),
            command: crate::protocol::PhoneCommand::Abort,
        });

        let (events, mut received) = mpsc::unbounded_channel();
        let mut decoder = FrameDecoder::new(key);
        handle(
            &serde_json::to_string(&frame).unwrap(),
            &events,
            &mut decoder,
        );

        assert_eq!(
            received.try_recv().unwrap(),
            RelayEvent::Payload(PhonePayload::Command {
                session_id: "s1".into(),
                id: "c1".into(),
                command: crate::protocol::PhoneCommand::Abort,
            })
        );
    }

    #[test]
    fn a_frame_captured_before_reconnect_is_not_reported_again() {
        let config = config();
        let key = derive_key(
            &config.secret,
            &config.pairing_id,
            Direction::PhoneToMachine,
        );
        let mut encoder = FrameEncoder::new(key);
        let frame = serde_json::to_string(&encoder.encode(&PhonePayload::Command {
            session_id: "s1".into(),
            id: "c1".into(),
            command: crate::protocol::PhoneCommand::Abort,
        }))
        .unwrap();
        let (events, mut received) = mpsc::unbounded_channel();
        let mut decoder = FrameDecoder::new(key);

        handle(&frame, &events, &mut decoder);
        assert!(matches!(received.try_recv(), Ok(RelayEvent::Payload(_))));
        handle(
            r#"{"relay":"peer","role":"phone","connected":true}"#,
            &events,
            &mut decoder,
        );
        assert_eq!(
            received.try_recv().unwrap(),
            RelayEvent::Peer { connected: true }
        );

        handle(&frame, &events, &mut decoder);
        assert!(received.try_recv().is_err());
    }
}
