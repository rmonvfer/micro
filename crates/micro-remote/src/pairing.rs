//! The pairing: what the machine and one phone share, and how the phone is told it.
//!
//! Pairing happens once. The secret is written to disk under the user's own directory
//! and reused for every session afterwards, so a phone that has been paired stays
//! paired across restarts — and a machine that has never been paired is the only one
//! that ever shows a code.

use base64::engine::general_purpose::STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

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
    /// The shared secret, base64. Every key either end uses is derived from it.
    #[serde(rename = "secretB64")]
    pub secret_b64: String,
    #[serde(rename = "machineName")]
    pub machine_name: String,
}

impl Pairing {
    /// The secret as bytes, or nothing when the file has been edited into nonsense.
    pub fn secret(&self) -> Option<Vec<u8>> {
        STANDARD.decode(&self.secret_b64).ok()
    }

    /// The link that pairs a phone with this machine.
    ///
    /// The secret travels in the link, so the link is the secret: it is shown once, on
    /// the machine's own screen, and is worth no less care than the session it opens.
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
///
/// A file that cannot be read or does not parse is treated as no pairing at all: the
/// next attempt makes a new one, which is recoverable, where refusing to start is not.
pub fn load(path: &Path) -> Option<Pairing> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Makes a pairing and writes it down.
///
/// The file holds a secret, so it is written readable only by its owner. The mode is
/// set again after writing because a mode given at creation does nothing when the file
/// is already there, and a pairing rewritten into a world-readable file would be a
/// secret published quietly.
pub fn create(path: &Path, relay_url: &str) -> std::io::Result<Pairing> {
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

/// Writes a pairing the two ends have already agreed on.
///
/// Separate from [`create`], which invents a secret: here the secret was arrived at by
/// both ends independently, and this only records it.
pub fn write(
    path: &Path,
    relay_url: &str,
    pairing_id: &str,
    secret: &[u8],
) -> std::io::Result<Pairing> {
    let pairing = Pairing {
        relay_url: relay_url.to_string(),
        pairing_id: pairing_id.to_string(),
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
///
/// The name is a convenience rather than an identity — it is shown beside a session so
/// someone with two machines paired can tell them apart — so a system that will not say
/// gets a placeholder rather than an error.
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
///
/// Two rows of the code are drawn per line of text using half-block characters, which
/// is what keeps a code that a phone can actually focus on inside a terminal window.
pub fn qr_lines(uri: &str) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::new(uri.as_bytes()) else {
        return Vec::new();
    };
    let width = code.width();
    let modules = code.to_colors();

    // A quiet border is part of the code rather than decoration: a scanner needs it to
    // find the edges at all. The grid is built with the border already in it, so the
    // drawing below never has to reason about which cells are inside the code.
    let quiet = 2;
    let side = width + quiet * 2;
    let mut grid = vec![false; side * side];
    for y in 0..width {
        for x in 0..width {
            grid[(y + quiet) * side + (x + quiet)] = modules[y * width + x] == qrcode::Color::Dark;
        }
    }

    // Two rows of the code share one line of text, so a code stays roughly square in a
    // terminal whose cells are twice as tall as they are wide.
    (0..side)
        .step_by(2)
        .map(|y| {
            (0..side)
                .map(|x| {
                    let top = grid[y * side + x];
                    let bottom = y + 1 < side && grid[(y + 1) * side + x];
                    // Drawn light-on-dark: a dark module is where the block is absent,
                    // which is what makes the terminal's own background the quiet zone.
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
        let made = create(&path, "http://localhost:8090").unwrap();
        assert_eq!(load(&path), Some(made));
    }

    #[test]
    fn a_pairing_is_written_where_only_its_owner_can_read_it() {
        let dir = scratch("mode");
        let path = path_in(&dir);
        create(&path, "http://localhost:8090").unwrap();

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
        create(&path, "http://localhost:8090").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            create(&path, "http://localhost:8090").unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn nothing_written_is_nothing_to_load() {
        let dir = scratch("missing");
        assert_eq!(load(&path_in(&dir)), None);
    }

    /// A file edited into nonsense is no pairing rather than a reason not to start.
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
    fn a_pairing_is_a_new_secret_every_time() {
        let dir = scratch("unique");
        let first = create(&path_in(&dir), "http://localhost:8090").unwrap();
        let second = create(&path_in(&dir), "http://localhost:8090").unwrap();
        assert_ne!(first.pairing_id, second.pairing_id);
        assert_ne!(first.secret_b64, second.secret_b64);
        assert_eq!(first.secret().unwrap().len(), 32);
        assert_eq!(first.pairing_id.len(), 32);
    }

    /// The link is what the phone opens, so its shape is the phone's to dictate.
    #[test]
    fn the_link_carries_the_relay_the_pairing_and_the_secret() {
        let pairing = Pairing {
            relay_url: "http://localhost:8090".into(),
            pairing_id: "abc123".into(),
            secret_b64: STANDARD.encode([1u8; 32]),
            machine_name: "laptop".into(),
        };
        let uri = pairing.uri();
        assert!(uri.starts_with("parley://pair?"));
        assert!(uri.contains("u=http%3A%2F%2Flocalhost%3A8090"));
        assert!(uri.contains("p=abc123"));
        // The secret travels url-safe and unpadded, which is the spelling the phone
        // decodes; standard base64's `+`, `/` and `=` would not survive a query string.
        assert!(uri.contains(&format!("s={}", URL_SAFE_NO_PAD.encode([1u8; 32]))));
    }

    #[test]
    fn a_code_is_drawn_with_a_quiet_border_around_it() {
        let lines = qr_lines("parley://pair?u=http%3A%2F%2Flocalhost%3A8090&p=abc&s=xyz");
        assert!(!lines.is_empty());
        // Every row is the same width, or the code is not square on screen.
        let width = lines[0].chars().count();
        assert!(lines.iter().all(|line| line.chars().count() == width));
        // The first row is entirely quiet zone.
        assert!(lines[0].chars().all(|character| character == '█'));
    }

    #[test]
    fn a_uri_too_long_to_encode_draws_nothing_rather_than_panicking() {
        assert!(qr_lines(&"x".repeat(10_000)).is_empty());
    }
}
