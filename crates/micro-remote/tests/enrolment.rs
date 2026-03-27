//! Pairing by code, against a relay that is actually running.
//!
//! What matters here is the property the whole arrangement exists for: the two ends come
//! away holding the same secret, and the relay never saw it. Both halves of that are
//! checked — the agreement, and what the relay was actually told.
//!
//! Start one to run this:
//!
//! ```text
//! cd locally && DB_PATH=:memory: PORT=8090 bun run relay/src/main.ts
//! ```
//!
//! With nothing listening these skip rather than fail.

use micro_remote::Code;
use micro_remote::Half;
use std::time::Duration;

const RELAY: &str = "http://localhost:8090";

async fn relay_is_up() -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(500),
            reqwest::Client::new().post(format!("{RELAY}/pairings")).send(),
        )
        .await,
        Ok(Ok(_))
    )
}

/// The phone's side of the exchange, which in the app is CryptoKit doing the same thing.
async fn claim(code: &str, phone: &Half) -> Option<(String, String)> {
    let response = reqwest::Client::new()
        .post(format!("{RELAY}/enrol/claim"))
        .json(&serde_json::json!({ "code": code, "phonePublic": phone.public() }))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    Some((
        body["pairingId"].as_str()?.to_string(),
        body["machinePublic"].as_str()?.to_string(),
    ))
}

/// The whole point: a code short enough to type leaves both ends holding the same secret.
#[tokio::test]
async fn a_code_leaves_both_ends_holding_the_same_secret() {
    if !relay_is_up().await {
        eprintln!("no relay on {RELAY}; skipping");
        return;
    }

    let enrolment = micro_remote::begin_enrolment(RELAY)
        .await
        .expect("the relay takes the machine's half");

    // What a person carries across the room.
    let typed = enrolment.code.written();
    assert_eq!(typed.len(), 9, "eight characters and the dash: {typed}");

    let phone = Half::generate();
    let code = Code::parse(&typed).expect("the code reads back as itself");
    let (pairing_id, machine_public) = claim(code.as_str(), &phone)
        .await
        .expect("the relay hands over the machine's half");

    let on_the_phone = phone
        .shared_secret(&machine_public, &pairing_id)
        .expect("the phone derives the secret");
    let on_the_machine = enrolment
        .complete()
        .await
        .expect("the machine derives the secret");

    assert_eq!(on_the_machine, on_the_phone);
    assert_eq!(on_the_machine.len(), 32);
    assert_eq!(pairing_id, enrolment.pairing_id());
}

/// The relay is handed public halves and nothing else. If the secret ever appeared in
/// what it stores, everything the sealed frames are for would be undone.
#[tokio::test]
async fn the_relay_is_never_told_the_secret() {
    if !relay_is_up().await {
        eprintln!("no relay on {RELAY}; skipping");
        return;
    }

    let enrolment = micro_remote::begin_enrolment(RELAY).await.unwrap();
    let phone = Half::generate();
    let (pairing_id, machine_public) = claim(enrolment.code.as_str(), &phone).await.unwrap();
    let secret = phone.shared_secret(&machine_public, &pairing_id).unwrap();

    // Everything the relay will give anyone who asks, for this code.
    let told = reqwest::Client::new()
        .get(format!("{RELAY}/enrol/await"))
        .query(&[("code", enrolment.code.as_str())])
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let encoded = base64_of(&secret);
    assert!(
        !told.contains(&encoded),
        "the relay is holding the secret: {told}"
    );
    assert!(!told.contains(&hex_of(&secret)));
    // What it does hold is the phone's public half, which is public by construction.
    assert!(told.contains(&phone.public()));
}

/// A code is good once. A second phone racing the same code is refused rather than
/// quietly replacing the first, which would leave two phones each believing they paired.
#[tokio::test]
async fn a_code_cannot_be_spent_twice() {
    if !relay_is_up().await {
        eprintln!("no relay on {RELAY}; skipping");
        return;
    }

    let enrolment = micro_remote::begin_enrolment(RELAY).await.unwrap();
    let first = Half::generate();
    let second = Half::generate();

    assert!(claim(enrolment.code.as_str(), &first).await.is_some());
    assert!(claim(enrolment.code.as_str(), &second).await.is_none());
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
