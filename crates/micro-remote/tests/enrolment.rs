//! Pairing by code, against a relay that is actually running.

mod support;

use micro_remote::Code;
use micro_remote::Half;
use support::RelayFixture;

/// The phone's side of the exchange, which in the app is CryptoKit doing the same thing.
async fn claim(relay: &RelayFixture, code: &str, phone: &Half) -> Option<(String, String)> {
    let response = relay
        .http
        .post(format!("{}/enrol/claim", relay.url))
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
    let relay = RelayFixture::start().await;
    let enrolment = micro_remote::begin_enrolment_with_client(&relay.url, relay.http.clone())
        .await
        .expect("the relay takes the machine's half");

    let typed = enrolment.code.written();
    assert_eq!(typed.len(), 9, "eight characters and the dash: {typed}");

    let phone = Half::generate();
    let code = Code::parse(&typed).expect("the code reads back as itself");
    let (pairing_id, machine_public) = claim(&relay, code.as_str(), &phone)
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

#[tokio::test]
async fn the_relay_is_never_told_the_secret() {
    let relay = RelayFixture::start().await;
    let enrolment = micro_remote::begin_enrolment_with_client(&relay.url, relay.http.clone())
        .await
        .unwrap();
    let phone = Half::generate();
    let (pairing_id, machine_public) = claim(&relay, enrolment.code.as_str(), &phone)
        .await
        .unwrap();
    let secret = phone.shared_secret(&machine_public, &pairing_id).unwrap();

    let told = relay
        .http
        .get(format!("{}/enrol/await", relay.url))
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

    assert!(told.contains(&phone.public()));
}

/// A code is good once.
#[tokio::test]
async fn a_code_cannot_be_spent_twice() {
    let relay = RelayFixture::start().await;
    let enrolment = micro_remote::begin_enrolment_with_client(&relay.url, relay.http.clone())
        .await
        .unwrap();
    let first = Half::generate();
    let second = Half::generate();

    assert!(claim(&relay, enrolment.code.as_str(), &first)
        .await
        .is_some());
    assert!(claim(&relay, enrolment.code.as_str(), &second)
        .await
        .is_none());
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
