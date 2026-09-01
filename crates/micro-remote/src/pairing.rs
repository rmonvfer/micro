//! The pairing: what the machine and one phone share, and how the phone is told it.

use base64::engine::general_purpose::STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

use crate::relay::validate_relay_url;

/// What the file under the user's directory is called.
pub const FILE_NAME: &str = "remote-control.json";

/// The scheme the phone registers, and so the one a pairing link has to use.
const SCHEME: &str = "parley";

/// Everything needed to reach one phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pairing {
    #[serde(rename = "relayUrl")]
    pub relay_url: String,
    #[serde(rename = "pairingId")]
    pub pairing_id: String,
    /// The shared secret, base64.
    #[serde(rename = "secretB64")]
    pub secret_b64: String,
    #[serde(rename = "machineName")]
    pub machine_name: String,
}

impl Pairing {
    /// The secret as bytes, or nothing when the file has been edited into nonsense.
    pub fn secret(&self) -> Option<Vec<u8>> {
        STANDARD
            .decode(&self.secret_b64)
            .ok()
            .filter(|secret| secret.len() == 32)
    }

    /// The link that pairs a phone with this machine.
    pub fn uri(&self) -> String {
        let relay = urlencode(&self.relay_url);
        let secret = self
            .secret()
            .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
            .unwrap_or_default();
        format!(
            "{SCHEME}://pair?u={relay}&p={}&s={secret}",
            urlencode(&self.pairing_id)
        )
    }
}

/// Reads the pairing already made, if there is one.
pub fn load(path: &Path) -> Option<Pairing> {
    let raw = std::fs::read_to_string(path).ok()?;
    let pairing: Pairing = serde_json::from_str(&raw).ok()?;
    validate_relay_url(&pairing.relay_url).ok()?;
    pairing.secret()?;
    if pairing.pairing_id.len() != 32
        || !pairing
            .pairing_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    Some(pairing)
}

/// Makes a pairing and writes it down.
pub fn create(path: &Path, relay_url: &str) -> std::io::Result<Pairing> {
    require_secure_relay_url(relay_url)?;
    let mut id = [0u8; 16];
    let mut secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut id);
    rand::thread_rng().fill_bytes(&mut secret);

    let pairing = Pairing {
        relay_url: relay_url.to_string(),
        pairing_id: id.iter().map(|byte| format!("{byte:02x}")).collect(),
        secret_b64: STANDARD.encode(secret),
        machine_name: machine_name(),
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string(&pairing).expect("a pairing of our own always serializes");
    std::fs::write(path, body)?;
    restrict(path)?;
    Ok(pairing)
}

fn require_secure_relay_url(relay_url: &str) -> std::io::Result<()> {
    validate_relay_url(relay_url)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
}

/// Where the pairing lives, under whichever directory micro is keeping things in.
pub fn path_in(micro_dir: &Path) -> PathBuf {
    micro_dir.join(FILE_NAME)
}

/// Makes the file readable only by its owner.
#[cfg(unix)]
fn restrict(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// What to call this machine on the phone's screen.
fn machine_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this machine".to_string())
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

/// The pairing link as a QR code, one string per row.
pub fn qr_lines(uri: &str) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::new(uri.as_bytes()) else {
        return Vec::new();
    };
    let width = code.width();
    let modules = code.to_colors();

    let quiet = 2;
    let side = width + quiet * 2;
    let mut grid = vec![false; side * side];
    for y in 0..width {
        for x in 0..width {
            grid[(y + quiet) * side + (x + quiet)] = modules[y * width + x] == qrcode::Color::Dark;
        }
    }

    (0..side)
        .step_by(2)
        .map(|y| {
            (0..side)
                .map(|x| {
                    let top = grid[y * side + x];
                    let bottom = y + 1 < side && grid[(y + 1) * side + x];

                    match (top, bottom) {
                        (true, true) => ' ',
                        (true, false) => '▄',
                        (false, true) => '▀',
                        (false, false) => '█',
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-remote-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_pairing_written_is_a_pairing_read_back() {
        let dir = scratch("roundtrip");
        let path = path_in(&dir);
        let made = create(&path, "https://relay.example").unwrap();
        assert_eq!(load(&path), Some(made));
    }

    #[test]
    fn a_pairing_is_written_where_only_its_owner_can_read_it() {
        let dir = scratch("mode");
        let path = path_in(&dir);
        create(&path, "https://relay.example").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    /// A file left world-readable by something else is not left that way.
    #[test]
    fn rewriting_a_pairing_restores_the_mode_it_should_have() {
        let dir = scratch("rewrite");
        let path = path_in(&dir);
        create(&path, "https://relay.example").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            create(&path, "https://relay.example").unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn nothing_written_is_nothing_to_load() {
        let dir = scratch("missing");
        assert_eq!(load(&path_in(&dir)), None);
    }

    #[test]
    fn an_unreadable_pairing_is_no_pairing() {
        let dir = scratch("garbage");
        let path = path_in(&dir);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), None);

        std::fs::write(&path, r#"{"relayUrl":"http://x"}"#).unwrap();
        assert_eq!(load(&path), None);
    }

    #[test]
    fn a_pairing_secret_has_exactly_thirty_two_bytes() {
        for secret in [Vec::new(), vec![1u8; 31], vec![1u8; 33]] {
            let pairing = Pairing {
                relay_url: "https://relay.example".into(),
                pairing_id: "p1".into(),
                secret_b64: STANDARD.encode(secret),
                machine_name: "laptop".into(),
            };
            assert_eq!(pairing.secret(), None);
        }
    }

    #[test]
    fn a_saved_pairing_requires_a_generated_id_and_secret() {
        let dir = scratch("invalid-fields");
        let path = path_in(&dir);
        for (pairing_id, secret_b64) in [
            ("p1", STANDARD.encode([1u8; 32])),
            (
                "0123456789abcdef0123456789abcdeG",
                STANDARD.encode([1u8; 32]),
            ),
            (
                "0123456789abcdef0123456789abcdef",
                STANDARD.encode([1u8; 31]),
            ),
        ] {
            let body = serde_json::json!({
                "relayUrl": "https://relay.example",
                "pairingId": pairing_id,
                "secretB64": secret_b64,
                "machineName": "laptop",
            });
            std::fs::write(&path, body.to_string()).unwrap();
            assert_eq!(load(&path), None);
        }
    }

    #[test]
    fn a_pairing_is_a_new_secret_every_time() {
        let dir = scratch("unique");
        let first = create(&path_in(&dir), "https://relay.example").unwrap();
        let second = create(&path_in(&dir), "https://relay.example").unwrap();
        assert_ne!(first.pairing_id, second.pairing_id);
        assert_ne!(first.secret_b64, second.secret_b64);
        assert_eq!(first.secret().unwrap().len(), 32);
        assert_eq!(first.pairing_id.len(), 32);
    }

    /// The link is what the phone opens, so its shape is the phone's to dictate.
    #[test]
    fn the_link_carries_the_relay_the_pairing_and_the_secret() {
        let secret: Vec<u8> = (0..32).collect();
        let pairing = Pairing {
            relay_url: "https://relay.example.com".into(),
            pairing_id: "9f2c".into(),
            secret_b64: STANDARD.encode(&secret),
            machine_name: "laptop".into(),
        };
        assert_eq!(
            pairing.uri(),
            "parley://pair?u=https%3A%2F%2Frelay.example.com&p=9f2c&s=AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
        );
    }

    #[test]
    fn a_code_is_drawn_with_a_quiet_border_around_it() {
        let lines = qr_lines("parley://pair?u=https%3A%2F%2Frelay.example&p=abc&s=xyz");
        assert!(!lines.is_empty());

        let width = lines[0].chars().count();
        assert!(lines.iter().all(|line| line.chars().count() == width));

        assert!(lines[0].chars().all(|character| character == '█'));
    }

    #[test]
    fn a_uri_too_long_to_encode_draws_nothing_rather_than_panicking() {
        assert!(qr_lines(&"x".repeat(10_000)).is_empty());
    }

    #[test]
    fn a_plaintext_relay_is_not_saved_or_loaded() {
        let dir = scratch("plaintext");
        let path = path_in(&dir);
        let error = create(&path, "http://relay.example").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!path.exists());

        std::fs::write(
            &path,
            r#"{"relayUrl":"http://relay.example","pairingId":"p1","secretB64":"AQ==","machineName":"laptop"}"#,
        )
        .unwrap();
        assert_eq!(load(&path), None);
    }
}
