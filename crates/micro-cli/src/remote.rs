//! Handing this session to a phone.

use micro_remote::AvailableModel;
use micro_remote::Bridge;
use micro_remote::MachinePayload;
use micro_remote::Pairing;
use micro_remote::PushKind;
use micro_remote::PushPayload;
use micro_remote::RelayClient;
use micro_remote::RelayConfig;
use micro_remote::RelayEvent;
use micro_remote::Session as RemoteSessionAccess;
use micro_remote::SessionState;
use micro_remote::SlashCommand;
use micro_tui::remote::FromPhone;
use micro_tui::remote::ToPhone;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;

/// Where the run is copied to while a phone is watching, filled in by `/remote`.
pub type Mirror = Arc<Mutex<Option<UnboundedSender<Value>>>>;

/// The relay a pairing is made against when nothing names another.
const DEFAULT_RELAY: &str = "https://parley-relay.up.railway.app";

/// The variable that names a relay of one's own.
const RELAY_ENV: &str = "MICRO_REMOTE_RELAY_URL";

/// The commands a phone is offered.
const PHONE_COMMANDS: &[&str] = &[
    "compact", "name", "clear", "new", "cwd", "session", "skills", "thinking", "help",
];

/// What the phone is told about the session, kept current by whoever changes it.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub model: String,
    pub provider: String,
    pub thinking: String,
    pub session_name: String,
    pub cwd: String,
}

pub struct Seam {
    /// What to tell the interface to do.
    pub to_interface: UnboundedSender<FromPhone>,
    /// Whether a turn is running, kept current by the interface.
    pub running: Arc<AtomicBool>,
}

impl Seam {
    /// Builds the seam, and the half the interface keeps.
    pub fn build() -> (Seam, micro_tui::remote::Remote) {
        let (to_interface, incoming) = tokio::sync::mpsc::unbounded_channel();
        let (outgoing, from_interface) = tokio::sync::mpsc::unbounded_channel();
        let running = Arc::new(AtomicBool::new(false));

        tokio::spawn(watch_running(from_interface, Arc::clone(&running)));

        (
            Seam {
                to_interface,
                running,
            },
            micro_tui::remote::Remote { incoming, outgoing },
        )
    }
}

async fn watch_running(mut from_interface: UnboundedReceiver<ToPhone>, running: Arc<AtomicBool>) {
    while let Some(ToPhone::Running(is_running)) = from_interface.recv().await {
        running.store(is_running, Ordering::Relaxed);
    }
}

/// Whether this machine has a phone bonded to it yet.
pub fn is_paired(micro_dir: &std::path::Path) -> bool {
    micro_remote::load_pairing(&micro_remote::path_in(micro_dir)).is_some()
}

/// Bonds a phone to this machine, and says what to read out.
pub async fn pair(micro_dir: &std::path::Path, qr: bool) -> Result<Vec<String>, String> {
    let relay = std::env::var(RELAY_ENV).unwrap_or_else(|_| DEFAULT_RELAY.to_string());
    let enrolment = micro_remote::begin_enrolment(&relay).await?;
    let code = enrolment.code.clone();

    let path = micro_remote::path_in(micro_dir);
    let relay_for_task = relay.clone();
    tokio::spawn(async move {
        if let Ok(secret) = enrolment.complete().await {
            let _ = micro_remote::write_pairing(
                &path,
                &relay_for_task,
                enrolment.pairing_id(),
                &secret,
            );
        }
    });

    Ok(pairing_instructions(&code, qr))
}

/// The instructions shown while a phone completes code-based pairing.
fn pairing_instructions(code: &micro_remote::Code, qr: bool) -> Vec<String> {
    let mut lines = vec![
        format!("Pairing code:  {code}"),
        String::new(),
        format!(
            "Open Parley on your phone and type it in. The code is good for {} minutes.",
            micro_remote::CODE_LIFETIME_SECONDS / 60
        ),
    ];
    if qr {
        lines.push(String::new());
        lines.extend(micro_remote::qr_lines(code.as_str()));
    }
    lines.push(String::new());
    lines.push("Once it is paired, /remote puts a session on it — no code, no link.".into());
    lines
}

/// Everything the bridge is allowed to ask about the session.
struct PhoneView {
    to_interface: UnboundedSender<FromPhone>,
    running: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Snapshot>>,
    session: Arc<Mutex<micro_session::Session>>,
    models: Vec<AvailableModel>,
}

impl RemoteSessionAccess for PhoneView {
    fn submit(&mut self, text: &str, delivery: micro_remote::Delivery) -> Result<(), String> {
        let asked = match delivery {
            micro_remote::Delivery::Prompt => FromPhone::Submit(text.to_string()),
            micro_remote::Delivery::Steer => FromPhone::Steer(text.to_string()),
            micro_remote::Delivery::FollowUp => FromPhone::FollowUp(text.to_string()),
        };
        self.to_interface
            .send(asked)
            .map_err(|_| "this session has closed".to_string())
    }

    fn abort(&mut self) {
        let _ = self.to_interface.send(FromPhone::Abort);
    }

    fn is_idle(&self) -> bool {
        !self.running.load(Ordering::Relaxed)
    }

    /// The branch as the phone rebuilds a transcript from it.
    fn entries(&self) -> Vec<Value> {
        let Ok(session) = self.session.try_lock() else {
            return Vec::new();
        };
        session
            .branch()
            .iter()
            .enumerate()
            .map(|(index, message)| {
                serde_json::json!({
                    "type": "message",
                    "id": format!("entry-{index}"),
                    "message": micro_extensions::message_json(message),
                })
            })
            .collect()
    }

    fn state(&self) -> SessionState {
        let snapshot = match self.snapshot.try_lock() {
            Ok(snapshot) => snapshot.clone(),
            Err(_) => Snapshot::default(),
        };
        SessionState {
            model: snapshot.model,
            provider: snapshot.provider,
            thinking_level: snapshot.thinking,
            session_name: snapshot.session_name,
            cwd: snapshot.cwd,
            is_streaming: !self.is_idle(),
        }
    }

    fn available_models(&self) -> Vec<AvailableModel> {
        self.models.clone()
    }

    fn commands(&self) -> Vec<SlashCommand> {
        micro_commands::commands()
            .iter()
            .filter(|command| PHONE_COMMANDS.contains(&command.name))
            .map(|command| SlashCommand {
                name: command.name.to_string(),
                description: command.description.to_string(),
            })
            .collect()
    }

    fn set_model(&mut self, model_id: &str) -> Result<(), String> {
        self.submit(
            &format!("/model {model_id}"),
            micro_remote::Delivery::Prompt,
        )
    }

    fn set_thinking_level(&mut self, level: &str) -> Result<(), String> {
        self.submit(
            &format!("/thinking {level}"),
            micro_remote::Delivery::Prompt,
        )
    }
}

/// Hands this session to a phone.
pub async fn start(
    seam: &Seam,
    mirror: &Mirror,
    session: Arc<Mutex<micro_session::Session>>,
    snapshot: Arc<Mutex<Snapshot>>,
    session_id: String,
    models: Vec<AvailableModel>,
    micro_dir: &std::path::Path,
) -> Result<(), String> {
    let path = micro_remote::path_in(micro_dir);
    let pairing: Pairing =
        micro_remote::load_pairing(&path).ok_or("no phone is paired with this machine")?;
    let secret = pairing
        .secret()
        .ok_or_else(|| format!("the pairing at {} is unreadable", path.display()))?;

    let config = RelayConfig {
        relay_url: pairing.relay_url.clone(),
        pairing_id: pairing.pairing_id.clone(),
        secret: secret.clone(),
        session_id: session_id.clone(),
    };
    let push_key =
        micro_remote::derive_key(&secret, &pairing.pairing_id, micro_remote::Direction::Push);

    micro_remote::register(&config).await?;
    let (events, incoming) = tokio::sync::mpsc::unbounded_channel();
    let client = Arc::new(RelayClient::start(config, events)?);

    let (mirrored, mirrored_rx) = tokio::sync::mpsc::unbounded_channel();
    *mirror.lock().await = Some(mirrored);

    let view = PhoneView {
        to_interface: seam.to_interface.clone(),
        running: Arc::clone(&seam.running),
        snapshot: Arc::clone(&snapshot),
        session,
        models,
    };

    tokio::spawn(serve(
        client,
        incoming,
        mirrored_rx,
        view,
        Bridge::new(session_id.clone()),
        session_id,
        pairing,
        push_key,
        snapshot,
    ));

    Ok(())
}

/// Everything the phone says and everything it is told, for as long as the session lasts.
#[allow(clippy::too_many_arguments)]
async fn serve(
    client: Arc<RelayClient>,
    mut incoming: UnboundedReceiver<RelayEvent>,
    mut mirrored: UnboundedReceiver<Value>,
    mut view: PhoneView,
    bridge: Bridge,
    session_id: String,
    pairing: Pairing,
    push_key: [u8; 32],
    snapshot: Arc<Mutex<Snapshot>>,
) {
    let offer = |name: String, cwd: String| MachinePayload::SessionOffer {
        session_id: session_id.clone(),
        session_name: name,
        cwd,
        machine_name: pairing.machine_name.clone(),
    };

    let announce = || {
        let snapshot = snapshot
            .try_lock()
            .map(|held| held.clone())
            .unwrap_or_default();
        let name = match snapshot.session_name.is_empty() {
            true => session_id.clone(),
            false => snapshot.session_name.clone(),
        };
        offer(name, snapshot.cwd.clone())
    };

    loop {
        tokio::select! {
            event = incoming.recv() => {
                let Some(event) = event else { return };
                match event {
                    RelayEvent::State(micro_remote::ConnectionState::Connected) => {
                        client.send(announce());
                    }
                    RelayEvent::Peer { connected: true } => {
                        client.send(announce());
                    }
                    RelayEvent::Peer { connected: false } => {}
                    RelayEvent::State(_) => {}
                    RelayEvent::Payload(payload) => {
                        client.send(bridge.handle(&mut view, payload));
                    }
                }
            }
            event = mirrored.recv() => {
                let Some(event) = event else { return };
                let settled = event.get("type").and_then(Value::as_str) == Some("agent_settled");
                client.send(bridge.mirror(event));


                if settled {
                    let snapshot = snapshot.lock().await.clone();
                    let payload = PushPayload {
                        kind: PushKind::Settled,
                        session_id: session_id.clone(),
                        session_name: match snapshot.session_name.is_empty() {
                            true => session_id.clone(),
                            false => snapshot.session_name,
                        },
                        machine_name: pairing.machine_name.clone(),
                    };

                    let _ = client.push_trigger(&push_key, &payload, Some(&session_id)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("micro-remote-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Nothing is paired until someone pairs something.
    #[test]
    fn a_machine_starts_with_no_phone_bonded_to_it() {
        assert!(!is_paired(&scratch("unpaired")));
    }

    #[tokio::test]
    async fn the_seam_delivers_phone_actions_and_tracks_when_a_turn_stops() {
        let (seam, mut remote) = Seam::build();
        seam.to_interface
            .send(FromPhone::Submit("hello".to_string()))
            .expect("the interface is listening");
        assert_eq!(
            timeout(Duration::from_secs(1), remote.incoming.recv())
                .await
                .expect("the action arrived"),
            Some(FromPhone::Submit("hello".to_string()))
        );

        remote.report_running(true);
        timeout(Duration::from_secs(1), async {
            while !seam.running.load(Ordering::Relaxed) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the running state arrived");
        remote.report_running(false);
        timeout(Duration::from_secs(1), async {
            while seam.running.load(Ordering::Relaxed) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the stopped state arrived");
    }

    #[test]
    fn pairing_instructions_name_the_code_and_explain_how_to_finish() {
        let code = micro_remote::Code::parse("ABCD-EFGH").expect("valid code");
        let lines = pairing_instructions(&code, false);

        assert!(lines.iter().any(|line| line == "Pairing code:  ABCD-EFGH"));
        assert!(lines.iter().any(|line| line.contains("Open Parley")));
        assert!(lines.iter().any(|line| line.contains("no code, no link")));
    }

    #[test]
    fn pairing_instructions_include_a_qr_code_when_requested() {
        let code = micro_remote::Code::parse("ABCD-EFGH").expect("valid code");
        let lines = pairing_instructions(&code, true);

        assert!(lines.iter().any(|line| line.contains('█')));
    }
}
