//! Kimi Code: an RFC 8628 device-code sign-in for the Kimi Code subscription.

use crate::oauth;
use crate::oauth::DeviceAuthorization;
use crate::oauth::DevicePoll;
use crate::oauth::PollSchedule;
use crate::AuthError;
use crate::OAuthCredential;
use crate::Result;
use serde_json::Value;
use std::time::Duration;

const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";
/// Variables that point the sign-in at another authorization host, in the order they are tried.
const HOST_ENV: [&str; 2] = ["KIMI_CODE_OAUTH_HOST", "KIMI_OAUTH_HOST"];
const DEVICE_CODE_TIMEOUT_SECS: u64 = 15 * 60;
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
/// How many times a refresh is retried after a rate limit or a server failure.
const REFRESH_RETRIES: u32 = 3;

/// The authorization host, from the environment when it names one.
pub fn oauth_host(get: impl Fn(&str) -> Option<String>) -> String {
    HOST_ENV
        .iter()
        .find_map(|name| get(name).filter(|value| !value.trim().is_empty()))
        .unwrap_or_else(|| DEFAULT_OAUTH_HOST.to_string())
        .trim_end_matches('/')
        .to_string()
}

pub async fn start_device_flow(http: &reqwest::Client, host: &str) -> Result<DeviceAuthorization> {
    let reply = oauth::post_form(
        http,
        &format!("{host}/api/oauth/device_authorization"),
        &[("client_id", CLIENT_ID)],
    )
    .await?;
    if !reply.is_success() {
        return Err(AuthError::DeviceFlow(format!(
            "Kimi Code device authorization failed ({}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        )));
    }
    parse_device_authorization(&reply.json())
}

fn parse_device_authorization(body: &Value) -> Result<DeviceAuthorization> {
    let invalid =
        || AuthError::DeviceFlow("Kimi Code returned an unusable device authorization".into());
    let device_code = oauth::string_field(body, "device_code").ok_or_else(invalid)?;
    let user_code = oauth::string_field(body, "user_code").ok_or_else(invalid)?;
    oauth::string_field(body, "verification_uri")
        .and_then(|uri| oauth::trusted_url(&uri, true))
        .ok_or_else(invalid)?;
    let complete = oauth::string_field(body, "verification_uri_complete")
        .and_then(|uri| oauth::trusted_url(&uri, true))
        .ok_or_else(invalid)?;
    Ok(DeviceAuthorization {
        verification_uri: complete,
        user_code,
        device_code,
        interval_secs: oauth::positive_number(body, "interval")
            .map(|seconds| seconds as u64)
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECS),
        expires_in_secs: oauth::positive_number(body, "expires_in")
            .map(|seconds| seconds as u64)
            .unwrap_or(DEVICE_CODE_TIMEOUT_SECS),
    })
}

pub async fn poll_device_flow(
    http: &reqwest::Client,
    host: &str,
    authorization: &DeviceAuthorization,
) -> Result<OAuthCredential> {
    let schedule = PollSchedule {
        interval_secs: Some(authorization.interval_secs),
        expires_in_secs: authorization.expires_in_secs,
        wait_first: true,
    };
    let url = format!("{host}/api/oauth/token");
    oauth::poll_device_code(schedule, || async {
        let reply = oauth::post_form(
            http,
            &url,
            &[
                ("client_id", CLIENT_ID),
                ("device_code", &authorization.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ],
        )
        .await?;
        Ok(classify_poll(&reply))
    })
    .await
}

fn classify_poll(reply: &oauth::HttpReply) -> DevicePoll<OAuthCredential> {
    if reply.status >= 500 {
        return DevicePoll::Failed(format!(
            "Kimi Code device token request failed ({}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        ));
    }
    let body = reply.json();
    if reply.is_success() && oauth::string_field(&body, "access_token").is_some() {
        return match credential_from_body(&body, "poll", crate::now_ms()) {
            Ok(credential) => DevicePoll::Complete(credential),
            Err(error) => DevicePoll::Failed(error.to_string()),
        };
    }
    match body.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => DevicePoll::Pending,
        Some("slow_down") => DevicePoll::SlowDown {
            interval_secs: oauth::positive_number(&body, "interval").map(|s| s as u64),
        },
        Some("expired_token") => DevicePoll::Failed(
            "Kimi Code device authorization expired; start the login again".into(),
        ),
        Some("access_denied") => DevicePoll::Failed("Kimi Code login was denied".into()),
        other => DevicePoll::Failed(format!(
            "Kimi Code device token request failed ({}){}",
            reply.status,
            other.map(|error| format!(": {error}")).unwrap_or_default()
        )),
    }
}

/// Refresh, retrying with backoff while the service is rate limiting or failing.
pub async fn refresh(
    http: &reqwest::Client,
    host: &str,
    refresh_token: &str,
) -> Result<OAuthCredential> {
    let url = format!("{host}/api/oauth/token");
    let mut last_error = None;
    for attempt in 0..=REFRESH_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
        }
        let reply = match oauth::post_form(
            http,
            &url,
            &[
                ("client_id", CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ],
        )
        .await
        {
            Ok(reply) => reply,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        let body = reply.json();
        if reply.is_success() {
            return credential_from_body(&body, "refresh", crate::now_ms());
        }
        if matches!(reply.status, 401 | 403)
            || body.get("error").and_then(Value::as_str) == Some("invalid_grant")
        {
            return Err(AuthError::OAuth(format!(
                "Kimi Code token refresh unauthorized ({}): {}; sign in again",
                reply.status,
                oauth::refusal(&reply.body)
            )));
        }
        let error = AuthError::OAuth(format!(
            "Kimi Code token refresh failed ({}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        ));
        if reply.status == 429 || reply.status >= 500 {
            last_error = Some(error);
            continue;
        }
        return Err(error);
    }
    Err(last_error.unwrap_or_else(|| AuthError::OAuth("Kimi Code token refresh failed".into())))
}

fn credential_from_body(body: &Value, operation: &str, now_ms: i64) -> Result<OAuthCredential> {
    let (Some(access), Some(refresh), Some(expires_in)) = (
        oauth::string_field(body, "access_token"),
        oauth::string_field(body, "refresh_token"),
        oauth::positive_number(body, "expires_in"),
    ) else {
        return Err(AuthError::OAuth(format!(
            "Kimi Code token {operation} response is missing fields"
        )));
    };
    Ok(OAuthCredential {
        access_token: access,
        refresh_token: refresh,
        expires: now_ms + (expires_in * 1000.0) as i64,
        client_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::testing::TestServer;
    use serde_json::json;

    #[test]
    fn the_host_can_be_overridden_from_the_environment() {
        assert_eq!(oauth_host(|_| None), DEFAULT_OAUTH_HOST);
        let get =
            |name: &str| (name == "KIMI_OAUTH_HOST").then(|| "https://auth.test/".to_string());
        assert_eq!(oauth_host(get), "https://auth.test");
    }

    #[test]
    fn the_device_authorization_needs_a_complete_web_uri() {
        let body = json!({
            "device_code": "d",
            "user_code": "U",
            "verification_uri": "https://www.kimi.com/device",
            "verification_uri_complete": "https://www.kimi.com/device?user_code=U",
        });
        let authorization = parse_device_authorization(&body).unwrap();
        assert_eq!(
            authorization.verification_uri,
            "https://www.kimi.com/device?user_code=U"
        );
        assert_eq!(authorization.interval_secs, DEFAULT_POLL_INTERVAL_SECS);
        assert_eq!(authorization.expires_in_secs, DEVICE_CODE_TIMEOUT_SECS);

        let mut missing = body.clone();
        missing["verification_uri_complete"] = json!("ftp://nope");
        assert!(parse_device_authorization(&missing).is_err());
    }

    #[tokio::test]
    async fn the_device_flow_yields_the_tokens() {
        let server = TestServer::start(vec![
            (
                200,
                json!({
                    "device_code": "dev",
                    "user_code": "U",
                    "verification_uri": "https://www.kimi.com/device",
                    "verification_uri_complete": "https://www.kimi.com/device?user_code=U",
                    "interval": 1,
                    "expires_in": 300,
                })
                .to_string(),
            ),
            (400, r#"{"error":"authorization_pending"}"#.into()),
            (
                200,
                r#"{"access_token":"ka","refresh_token":"kr","expires_in":900}"#.into(),
            ),
        ])
        .await;
        let http = reqwest::Client::new();
        let authorization = start_device_flow(&http, &server.base).await.unwrap();
        let credential = poll_device_flow(&http, &server.base, &authorization)
            .await
            .unwrap();
        assert_eq!(credential.access_token, "ka");
        assert_eq!(credential.refresh_token, "kr");
        assert_eq!(server.requests()[0].path, "/api/oauth/device_authorization");
        assert_eq!(server.requests()[1].form()["device_code"], "dev");
    }

    #[tokio::test]
    async fn a_refresh_retries_a_failing_service_and_stops_on_an_unauthorized_one() {
        let server = TestServer::start(vec![
            (503, "{}".into()),
            (
                200,
                r#"{"access_token":"ka2","refresh_token":"kr2","expires_in":900}"#.into(),
            ),
            (401, r#"{"error":"invalid_grant"}"#.into()),
        ])
        .await;
        let http = reqwest::Client::new();
        let refreshed = refresh(&http, &server.base, "kr").await.unwrap();
        assert_eq!(refreshed.access_token, "ka2");
        assert_eq!(server.requests()[0].form()["refresh_token"], "kr");

        let error = refresh(&http, &server.base, "kr2").await.unwrap_err();
        assert!(error.to_string().contains("unauthorized"), "{error}");
        assert_eq!(server.requests().len(), 3);
    }
}
