//! Handing this session to a phone.
//!
//! The interface owns the agent and the relay owns the phone, so this sits between them:
//! it holds the pairing, keeps the connection, answers what the phone asks about the
//! session, and turns the phone's prompts into lines the interface submits as though they
//! had been typed there.
//!
//! What it deliberately does not hold is the agent. Everything that changes the session
//! goes through [`micro_tui::remote`], which is the same path a keystroke takes.

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
///
/// Not every command micro knows can be run from a phone. A command that opens a picker,
/// asks for a credential or hands the terminal to an editor needs someone sitting at the
/// machine, and offering it would be offering something that appears to do nothing. What
/// is left is the set that finishes on its own and reports what it did.
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

/// Both ends of the seam to the interface, held from startup so `/remote` has
/// somewhere to plug into.
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

        // Drained from the moment the session starts rather than from the moment a phone
        // arrives: the interface reports every turn either way, and a channel nobody reads
        // is a channel that grows.
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
///
/// A code rather than a link, because a link carries the secret and is therefore too long
/// to move by hand. The two ends swap public halves under the code and arrive at the same
/// secret without it ever crossing the relay — so what has to be typed is eight
/// characters, and what the relay learns is nothing.
///
/// Returns as soon as there is a code to show. Finishing waits on somebody picking up a
/// phone, which happens on its own and writes the pairing when it lands.
pub async fn pair(micro_dir: &std::path::Path, qr: bool) -> Result<Vec<String>, String> {
    let relay = std::env::var(RELAY_ENV).unwrap_or_else(|_| DEFAULT_RELAY.to_string());
    let enrolment = micro_remote::begin_enrolment(&relay).await?;
    let code = enrolment.code.clone();

    let path = micro_remote::path_in(micro_dir);
    let relay_for_task = relay.clone();
    tokio::spawn(async move {
        // Nothing is reported back from here: the phone says whether it paired, which is
        // where somebody typing a code is already looking.
        if let Ok(secret) = enrolment.complete().await {
            let _ = micro_remote::write_pairing(
                &path,
                &relay_for_task,
                enrolment.pairing_id(),
                &secret,
            );
        }
    });

    let mut lines = vec![
        format!("Pairing code:  {code}"),
        String::new(),
        format!(
            "Open Parley on your phone and type it in. The code is good for {} minutes.",
            micro_remote::CODE_LIFETIME_SECONDS / 60
        ),
    ];
    if qr {
        // The code as a code, for a phone that is looking at this screen anyway.
        lines.push(String::new());
        lines.extend(micro_remote::qr_lines(code.as_str()));
    }
    lines.push(String::new());
    lines.push("Once it is paired, /remote puts a session on it — no code, no link.".into());
    Ok(lines)
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
    ///
    /// Entries are numbered rather than carrying the tree's own ids: what the phone keys a
    /// row on has to be stable for as long as the list is, and a reload hands it the whole
    /// list at once.
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

    /// Changing the model and the thinking level go through micro's own commands rather
    /// than around them, so a phone changing either lands exactly where the terminal would
    /// land — including in the transcript, where someone at the machine can see it.
    fn set_model(&mut self, model_id: &str) -> Result<(), String> {
        self.submit(
            &format!("/model {model_id}"),
            micro_remote::Delivery::Prompt,
        )
    }

    fn set_thinking_level(&mut self, level: &str) -> Result<(), String> {
        self.submit(&format!("/thinking {level}"), micro_remote::Delivery::Prompt)
    }
}

/// Hands this session to a phone.
///
/// Pairing happens once and is reused, so this shows a code only the first time. Everything
/// after that is the connection being made again.
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
    let pairing: Pairing = micro_remote::load_pairing(&path)
        .ok_or("no phone is paired with this machine")?;
    let secret = pairing
        .secret()
        .ok_or_else(|| format!("the pairing at {} is unreadable", path.display()))?;

    let config = RelayConfig {
        relay_url: pairing.relay_url.clone(),
        pairing_id: pairing.pairing_id.clone(),
        secret: secret.clone(),
        session_id: session_id.clone(),
    };
    let push_key = micro_remote::derive_key(
        &secret,
        &pairing.pairing_id,
        micro_remote::Direction::Push,
    );

    // Registered every time rather than only when the pairing is new. The relay stores
    // hashes of tokens derived from the secret, so writing them again writes the same
    // record — which is what puts a pairing back after a relay has lost its database,
    // without anyone being asked to pair a second time.
    micro_remote::register(&config).await?;
    let (events, incoming) = tokio::sync::mpsc::unbounded_channel();
    let client = Arc::new(RelayClient::start(config, events));

    // The run is copied to the phone from here on. Registered before the offer goes out,
    // so nothing the session does between the two is missed.
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

    // The offer goes out the moment there is anywhere to send it, and again whenever the
    // phone arrives: the relay tells a newly joined leg only about peers that connect
    // after it, so a phone already waiting would otherwise hear nothing.
    let mut announce = || {
        let snapshot = snapshot.try_lock().map(|held| held.clone()).unwrap_or_default();
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

                // A turn that has finished is worth waking a phone for; one that is still
                // going is already being watched by whoever is watching.
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
                    // Best effort: a phone on the open channel already knows, so a push
                    // that does not land is not worth interrupting the session over.
                    let _ = client.push_trigger(&push_key, &payload, Some(&session_id)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-remote-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Nothing is paired until someone pairs something.
    #[test]
    fn a_machine_starts_with_no_phone_bonded_to_it() {
        assert!(!is_paired(&scratch("unpaired")));
    }

    /// Pairing is the one-off, and it is what makes publishing possible afterwards.
    #[tokio::test]
    async fn pairing_bonds_a_phone_and_shows_it_a_link() {
        let dir = scratch("pair");
        let lines = pair(&dir, false).await.expect("a phone can be paired");

        assert!(is_paired(&dir));
        assert!(lines.iter().any(|line| line.starts_with("parley://pair?")));
        // What it says next is the point of the whole arrangement.
        assert!(lines.iter().any(|line| line.contains("no link, no code")));
    }

    /// Pairing twice keeps the phone already bonded rather than cutting it off, so
    /// running it again is a way to see the link, not a way to lose the phone.
    #[tokio::test]
    async fn pairing_again_keeps_the_phone_already_bonded() {
        let dir = scratch("pair-twice");
        let first = pair(&dir, false).await.unwrap();
        let second = pair(&dir, false).await.unwrap();

        let link = |lines: &[String]| {
            lines
                .iter()
                .find(|line| line.starts_with("parley://"))
                .cloned()
                .unwrap()
        };
        assert_eq!(link(&first), link(&second));
    }

    /// Asked for a code, it draws one — and still prints the link, because a simulator
    /// has no camera to scan with.
    #[tokio::test]
    async fn a_code_is_drawn_when_one_is_asked_for() {
        let dir = scratch("pair-qr");
        let lines = pair(&dir, true).await.unwrap();

        assert!(lines.iter().any(|line| line.contains('█')));
        assert!(lines.iter().any(|line| line.contains("parley://pair?")));
    }
}
