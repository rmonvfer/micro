//! Bonding a phone to a machine by a code short enough to read out.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hkdf::Hkdf;
use rand::Rng;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::PublicKey;
use x25519_dalek::StaticSecret;


pub const CODE_LIFETIME_SECONDS: u64 = 300;

/// How many characters a code carries.
const CODE_LENGTH: usize = 8;

/// The alphabet a code is written in.
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A code as it is shown and as it is typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code(String);

impl Code {
    /// A new code, drawn from a source nobody can predict.
    pub fn generate() -> Code {
        let mut rng = rand::thread_rng();
        let code: String = (0..CODE_LENGTH)
            .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
            .collect();
        Code(code)
    }

    /// What a code means once someone has typed it.
    pub fn parse(typed: &str) -> Option<Code> {
        let cleaned: String = typed
            .chars()
            .filter(|character| !character.is_whitespace() && *character != '-')
            .map(|character| character.to_ascii_uppercase())
            .collect();
        if cleaned.len() != CODE_LENGTH {
            return None;
        }
        match cleaned.bytes().all(|byte| ALPHABET.contains(&byte)) {
            true => Some(Code(cleaned)),
            false => None,
        }
    }

    /// The code as the relay knows it.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The code as it is shown to be read.
    pub fn written(&self) -> String {
        let (first, second) = self.0.split_at(CODE_LENGTH / 2);
        format!("{first}-{second}")
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.written())
    }
}

/// One end's half of the exchange.
pub struct Half {
    secret: StaticSecret,
}

impl Half {
    pub fn generate() -> Half {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Half {
            secret: StaticSecret::from(bytes),
        }
    }

    /// The half that is safe to publish, base64 for travelling as JSON.
    pub fn public(&self) -> String {
        STANDARD.encode(PublicKey::from(&self.secret).as_bytes())
    }

    /// The secret both ends arrive at, from this end's private half and the other end's public one.
    pub fn shared_secret(&self, their_public: &str, pairing_id: &str) -> Option<Vec<u8>> {
        let decoded = STANDARD.decode(their_public).ok()?;
        let bytes: [u8; 32] = decoded.try_into().ok()?;
        let shared = self.secret.diffie_hellman(&PublicKey::from(bytes));

        let hkdf = Hkdf::<Sha256>::new(None, shared.as_bytes());
        let mut secret = vec![0u8; 32];
        hkdf.expand(
            format!("parley-remote/v1/pairing/{pairing_id}").as_bytes(),
            &mut secret,
        )
        .ok()?;
        Some(secret)
    }
}

/// A pairing being set up: the code to read out, and everything needed to finish once somebody has
/// typed it.
pub struct Enrolment {
    pub code: Code,
    pairing_id: String,
    half: Half,
    relay_url: String,
    http: reqwest::Client,
}

/// Publishes this machine's public half under a fresh code.
pub async fn begin(relay_url: &str) -> Result<Enrolment, String> {
    let half = Half::generate();
    let code = Code::generate();
    let mut id = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut id);
    let pairing_id: String = id.iter().map(|byte| format!("{byte:02x}")).collect();

    let http = reqwest::Client::new();
    let response = http
        .post(format!("{relay_url}/enrol/start"))
        .json(&serde_json::json!({
            "code": code.as_str(),
            "pairingId": pairing_id,
            "machinePublic": half.public(),
            "lifetimeSeconds": CODE_LIFETIME_SECONDS,
        }))
        .send()
        .await
        .map_err(|error| format!("could not reach the relay: {error}"))?;
    
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "the relay at {relay_url} does not know how to pair by code — it is running \
             a version from before this, and needs updating"
        ));
    }
    if !response.status().is_success() {
        return Err(format!("the relay refused the code: {}", response.status()));
    }

    Ok(Enrolment {
        code,
        pairing_id,
        half,
        relay_url: relay_url.to_string(),
        http,
    })
}

impl Enrolment {
    /// Waits for the code to be spent, then works out the secret both ends now share.
    pub async fn complete(&self) -> Result<Vec<u8>, String> {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(CODE_LIFETIME_SECONDS);

        while std::time::Instant::now() < deadline {
            let response = self
                .http
                .get(format!("{}/enrol/await", self.relay_url))
                .query(&[("code", self.code.as_str())])
                .send()
                .await
                .map_err(|error| format!("could not reach the relay: {error}"))?;

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err("the code has expired".into());
            }
            if response.status().is_success() && response.status() != reqwest::StatusCode::NO_CONTENT {
                let body: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|error| format!("the relay answered oddly: {error}"))?;
                let phone_public = body
                    .get("phonePublic")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("the relay answered without the phone's half")?;
                return self
                    .half
                    .shared_secret(phone_public, &self.pairing_id)
                    .ok_or_else(|| "the phone's half could not be read".to_string());
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        Err("nobody used the code in time".into())
    }

    pub fn pairing_id(&self) -> &str {
        &self.pairing_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_ends_arrive_at_the_same_secret() {
        let machine = Half::generate();
        let phone = Half::generate();

        let on_the_machine = machine.shared_secret(&phone.public(), "p1").unwrap();
        let on_the_phone = phone.shared_secret(&machine.public(), "p1").unwrap();

        assert_eq!(on_the_machine, on_the_phone);
        assert_eq!(on_the_machine.len(), 32);
    }

    /// The same two keys used for another pairing give another secret.
    #[test]
    fn the_same_keys_give_a_different_secret_for_a_different_pairing() {
        let machine = Half::generate();
        let phone = Half::generate();

        assert_ne!(
            machine.shared_secret(&phone.public(), "p1").unwrap(),
            machine.shared_secret(&phone.public(), "p2").unwrap()
        );
    }

    #[test]
    fn a_public_half_that_is_not_one_derives_nothing() {
        let machine = Half::generate();
        assert_eq!(machine.shared_secret("not base64!", "p1"), None);
        assert_eq!(machine.shared_secret(&STANDARD.encode([0u8; 8]), "p1"), None);
    }

    #[test]
    fn a_code_is_written_in_halves_and_read_back_whichever_way_it_is_typed() {
        let code = Code::generate();
        assert_eq!(code.as_str().len(), CODE_LENGTH);
        assert_eq!(code.written().len(), CODE_LENGTH + 1);

        
        assert_eq!(Code::parse(&code.written()), Some(code.clone()));
        assert_eq!(Code::parse(&code.written().to_lowercase()), Some(code.clone()));
        assert_eq!(Code::parse(&format!(" {} ", code.as_str())), Some(code));
    }

    
    #[test]
    fn a_code_that_could_not_have_been_issued_is_refused() {
        assert_eq!(Code::parse("SHORT"), None);
        assert_eq!(Code::parse("TOOLONGCODE"), None);
        
        assert_eq!(Code::parse("O0IL1UAB"), None);
        assert_eq!(Code::parse(""), None);
    }

    
    #[test]
    fn the_alphabet_holds_nothing_that_gets_misread() {
        for confusable in [b'O', b'0', b'I', b'L', b'1', b'U'] {
            assert!(
                !ALPHABET.contains(&confusable),
                "{} is easily misread",
                confusable as char
            );
        }
    }

    /// Codes have to be unpredictable; a run of them that repeats is a run anyone can sit and
    /// guess.
    #[test]
    fn codes_do_not_repeat_themselves() {
        let codes: std::collections::HashSet<String> =
            (0..500).map(|_| Code::generate().as_str().to_string()).collect();
        assert_eq!(codes.len(), 500);
    }
}
