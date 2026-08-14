//! The sealed frame, and the keys it is sealed with.
//!
//! The relay routes frames without being able to read them: everything that crosses it
//! is AES-256-GCM ciphertext under a key derived from a secret only the machine and the
//! phone hold. Each direction gets its own key, so a frame captured on one leg cannot be
//! replayed onto the other.
//!
//! The shapes here are not micro's to choose — a phone already speaks this protocol —
//! so the tests check against the vectors shipped beside it rather than against
//! themselves.

use aes_gcm::aead::Aead;
use aes_gcm::aead::KeyInit;
use aes_gcm::aead::Payload;
use aes_gcm::Aes256Gcm;
use aes_gcm::Nonce;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hkdf::Hkdf;
use rand::RngCore;
use serde::Deserialize;
use serde::Serialize;
use sha2::Sha256;

/// How long a GCM nonce is, in bytes.
const NONCE_LEN: usize = 12;

/// Which leg of the pairing a key is for.
///
/// The push key is a third direction rather than a reuse of the machine's: a push
/// payload is handed to Apple, and a key that also opens session frames would put the
/// conversation behind whatever holds the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Machine to phone.
    MachineToPhone,
    /// Phone to machine.
    PhoneToMachine,
    /// The machine's push triggers.
    Push,
}

impl Direction {
    fn label(self) -> &'static str {
        match self {
            Direction::MachineToPhone => "m2p",
            Direction::PhoneToMachine => "p2m",
            Direction::Push => "push",
        }
    }
}

/// A sealed frame, as it travels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireFrame {
    pub v: u8,
    /// The nonce, base64.
    pub n: String,
    /// The ciphertext with its tag appended, base64.
    pub ct: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("frame authentication failed")]
    Authentication,
}

/// The key for one direction of one pairing.
///
/// The pairing id is mixed into the info string rather than the salt, which is empty:
/// that is what the phone does, and a key derived any other way simply never opens
/// anything it sends.
pub fn derive_key(secret: &[u8], pairing_id: &str, direction: Direction) -> [u8; 32] {
    let info = format!("parley-remote/v1/{pairing_id}/{}", direction.label());
    let hkdf = Hkdf::<Sha256>::new(None, secret);
    let mut key = [0u8; 32];
    hkdf.expand(info.as_bytes(), &mut key)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    key
}

/// Seals a payload under a nonce chosen here.
pub fn seal(key: &[u8; 32], plaintext: &str) -> WireFrame {
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    seal_with_nonce(key, plaintext, &nonce)
}

/// Seals a payload under a nonce given, which is what makes the sealing testable
/// against a fixed vector.
pub fn seal_with_nonce(key: &[u8; 32], plaintext: &str, nonce: &[u8; NONCE_LEN]) -> WireFrame {
    let cipher = Aes256Gcm::new(key.into());
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: &[],
            },
        )
        .expect("AES-GCM encryption of an in-memory payload cannot fail");
    WireFrame {
        v: 1,
        n: STANDARD.encode(nonce),
        ct: STANDARD.encode(ciphertext),
    }
}

/// Opens a frame, or says it could not be opened.
///
/// Every way a frame can be wrong — a nonce that is not base64, a truncated
/// ciphertext, a tag that does not check, plaintext that is not UTF-8 — comes back as
/// the same failure. A caller cannot act differently on any of them, and saying which
/// one it was tells whoever sent it something about the key.
pub fn open(key: &[u8; 32], frame: &WireFrame) -> Result<String, CryptoError> {
    let nonce = STANDARD
        .decode(&frame.n)
        .map_err(|_| CryptoError::Authentication)?;
    if nonce.len() != NONCE_LEN {
        return Err(CryptoError::Authentication);
    }
    let ciphertext = STANDARD
        .decode(&frame.ct)
        .map_err(|_| CryptoError::Authentication)?;

    let cipher = Aes256Gcm::new(key.into());
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: &[],
            },
        )
        .map_err(|_| CryptoError::Authentication)?;
    String::from_utf8(plaintext).map_err(|_| CryptoError::Authentication)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vectors the phone and the relay are also checked against.
    const VECTORS: &str = include_str!("../vectors/crypto-test-vectors.json");

    fn vectors() -> serde_json::Value {
        serde_json::from_str(VECTORS).expect("the vectors parse")
    }

    fn secret() -> Vec<u8> {
        let hex = vectors()["secretHex"].as_str().unwrap().to_string();
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    }

    fn nonce_from(hex: &str) -> [u8; NONCE_LEN] {
        let mut nonce = [0u8; NONCE_LEN];
        for (index, byte) in nonce.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
        }
        nonce
    }

    #[test]
    fn each_direction_derives_the_key_the_phone_derives() {
        let vectors = vectors();
        let pairing_id = vectors["pairingId"].as_str().unwrap();
        for (label, direction) in [
            ("m2p", Direction::MachineToPhone),
            ("p2m", Direction::PhoneToMachine),
            ("push", Direction::Push),
        ] {
            let key = derive_key(&secret(), pairing_id, direction);
            let expected = vectors["keys"][label].as_str().unwrap();
            assert_eq!(hex(&key), expected, "{label} key");
        }
    }

    #[test]
    fn sealing_under_a_known_nonce_produces_the_known_ciphertext() {
        let vectors = vectors();
        let key = derive_key(
            &secret(),
            vectors["pairingId"].as_str().unwrap(),
            Direction::MachineToPhone,
        );
        let sealed = vectors["sealed"].clone();
        let frame = seal_with_nonce(
            &key,
            sealed["plaintext"].as_str().unwrap(),
            &nonce_from(sealed["nonceHex"].as_str().unwrap()),
        );
        assert_eq!(frame.n, sealed["n"].as_str().unwrap());
        assert_eq!(frame.ct, sealed["ct"].as_str().unwrap());
    }

    #[test]
    fn a_frame_the_phone_sealed_opens_here() {
        let vectors = vectors();
        let key = derive_key(
            &secret(),
            vectors["pairingId"].as_str().unwrap(),
            Direction::MachineToPhone,
        );
        let frame = WireFrame {
            v: 1,
            n: vectors["sealed"]["n"].as_str().unwrap().to_string(),
            ct: vectors["sealed"]["ct"].as_str().unwrap().to_string(),
        };
        assert_eq!(
            open(&key, &frame).unwrap(),
            vectors["sealed"]["plaintext"].as_str().unwrap()
        );
    }

    #[test]
    fn a_push_payload_seals_the_way_the_phone_reads_it() {
        let vectors = vectors();
        let key = derive_key(
            &secret(),
            vectors["pairingId"].as_str().unwrap(),
            Direction::Push,
        );
        let push = vectors["push"].clone();
        // Built through the real type rather than from the vector's own object: the
        // phone authenticates over the bytes of a payload written in declaration
        // order, and a `Value` would be written in sorted order instead.
        let payload = crate::protocol::PushPayload {
            kind: crate::protocol::PushKind::Offer,
            session_id: push["payload"]["sessionId"].as_str().unwrap().into(),
            session_name: push["payload"]["sessionName"].as_str().unwrap().into(),
            machine_name: push["payload"]["machineName"].as_str().unwrap().into(),
        };
        let frame = seal_with_nonce(
            &key,
            &serde_json::to_string(&payload).unwrap(),
            &nonce_from(push["nonceHex"].as_str().unwrap()),
        );
        let expected: WireFrame = serde_json::from_str(push["ct"].as_str().unwrap()).unwrap();
        assert_eq!(frame, expected);
    }

    #[test]
    fn a_round_trip_comes_back_as_itself() {
        let key = derive_key(b"a secret", "pairing", Direction::PhoneToMachine);
        let frame = seal(&key, "{\"seq\":1}");
        assert_eq!(open(&key, &frame).unwrap(), "{\"seq\":1}");
    }

    #[test]
    fn a_frame_sealed_for_the_other_direction_does_not_open() {
        let outbound = derive_key(b"a secret", "pairing", Direction::MachineToPhone);
        let inbound = derive_key(b"a secret", "pairing", Direction::PhoneToMachine);
        let frame = seal(&outbound, "hello");
        assert_eq!(open(&inbound, &frame), Err(CryptoError::Authentication));
    }

    /// Every way a frame can be wrong comes back the same way, so nothing about the
    /// key leaks back to whoever sent it.
    #[test]
    fn a_damaged_frame_fails_the_way_every_other_damaged_frame_does() {
        let key = derive_key(b"a secret", "pairing", Direction::MachineToPhone);
        let frame = seal(&key, "hello");

        let tampered = WireFrame {
            ct: STANDARD.encode(b"not the ciphertext"),
            ..frame.clone()
        };
        assert_eq!(open(&key, &tampered), Err(CryptoError::Authentication));

        let unreadable = WireFrame {
            n: "not base64!".into(),
            ..frame.clone()
        };
        assert_eq!(open(&key, &unreadable), Err(CryptoError::Authentication));

        let short_nonce = WireFrame {
            n: STANDARD.encode([0u8; 4]),
            ..frame
        };
        assert_eq!(open(&key, &short_nonce), Err(CryptoError::Authentication));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
