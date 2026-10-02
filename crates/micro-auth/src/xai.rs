//! xAI: a device-code sign-in for SuperGrok and X Premium subscribers.

use crate::oauth;
use crate::oauth::DeviceAuthorization;
use crate::oauth::DevicePoll;
use crate::oauth::PollSchedule;
use crate::AuthError;
use crate::OAuthCredential;
use crate::Result;
use serde_json::Value;

const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
pub const DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
pub const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
/// Who xAI is told sent the user.
const REFERRER: &str = "pi";
/// Tokens are stored as expiring five minutes early, so none dies mid-request.
const EXPIRY_MARGIN_MS: i64 = 5 * 60 * 1000;
const DEFAULT_TOKEN_LIFETIME_SECS: f64 = 3600.0;

/// The endpoints a sign-in talks to, which tests point elsewhere.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub device_code: String,
    pub token: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            device_code: DEVICE_CODE_URL.to_string(),
            token: TOKEN_URL.to_string(),
        }
    }
}

fn failure(action: &str, reply: &oauth::HttpReply) -> String {
    let body = reply.json();
    let detail = [
        body.get("error").and_then(Value::as_str),
        body.get("error_description").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(": ");
    match detail.is_empty() {
        true => format!("xAI {action} failed (HTTP {})", reply.status),
        false => format!("xAI {action} failed (HTTP {}): {detail}", reply.status),
    }
}

pub async fn start_device_flow(
    http: &reqwest::Client,
    endpoints: &Endpoints,
) -> Result<DeviceAuthorization> {
    let reply = oauth::post_form(
        http,
        &endpoints.device_code,
        &[
            ("client_id", CLIENT_ID),
            ("scope", SCOPE),
            ("referrer", REFERRER),
        ],
    )
    .await?;
    if !reply.is_success() {
        return Err(AuthError::DeviceFlow(failure(
            "device authorization",
            &reply,
        )));
    }
    parse_device_authorization(&reply.json())
}

fn parse_device_authorization(body: &Value) -> Result<DeviceAuthorization> {
    let invalid =
        |field: &str| AuthError::DeviceFlow(format!("invalid xAI response field: {field}"));
    let required = |field: &str| oauth::string_field(body, field).ok_or_else(|| invalid(field));
    let verification_uri = oauth::trusted_url(&required("verification_uri")?, false)
        .ok_or_else(|| AuthError::DeviceFlow("untrusted verification URI from xAI".into()))?;
    let complete =
        match oauth::string_field(body, "verification_uri_complete") {
            Some(raw) => Some(oauth::trusted_url(&raw, false).ok_or_else(|| {
                AuthError::DeviceFlow("untrusted verification URI from xAI".into())
            })?),
            None => None,
        };
    let expires_in =
        oauth::positive_number(body, "expires_in").ok_or_else(|| invalid("expires_in"))?;
    Ok(DeviceAuthorization {
        verification_uri: complete.unwrap_or(verification_uri),
        user_code: required("user_code")?,
        device_code: required("device_code")?,
        interval_secs: oauth::positive_number(body, "interval")
            .map(|seconds| seconds as u64)
            .unwrap_or(0),
        expires_in_secs: expires_in as u64,
    })
}

pub async fn poll_device_flow(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    authorization: &DeviceAuthorization,
) -> Result<OAuthCredential> {
    let schedule = PollSchedule {
        interval_secs: (authorization.interval_secs > 0).then_some(authorization.interval_secs),
        expires_in_secs: authorization.expires_in_secs,
        wait_first: true,
    };
    oauth::poll_device_code(schedule, || async {
        let reply = oauth::post_form(
            http,
            &endpoints.token,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", CLIENT_ID),
                ("device_code", &authorization.device_code),
            ],
        )
        .await?;
        if reply.is_success() {
            return Ok(
                match credential_from_body(&reply.json(), None, crate::now_ms()) {
                    Ok(credential) => DevicePoll::Complete(credential),
                    Err(error) => DevicePoll::Failed(error.to_string()),
                },
            );
        }
        let body = reply.json();
        Ok(match body.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => DevicePoll::Pending,
            Some("slow_down") => DevicePoll::SlowDown {
                interval_secs: oauth::positive_number(&body, "interval").map(|s| s as u64),
            },
            Some("access_denied" | "authorization_denied") => {
                DevicePoll::Failed("xAI device authorization was denied".into())
            }
            Some("expired_token") => DevicePoll::Failed("xAI device code expired".into()),
            _ => DevicePoll::Failed(failure("device token polling", &reply)),
        })
    })
    .await
}

pub async fn refresh(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    refresh_token: &str,
) -> Result<OAuthCredential> {
    let reply = oauth::post_form(
        http,
        &endpoints.token,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ],
    )
    .await?;
    if !reply.is_success() {
        return Err(AuthError::OAuth(failure("token refresh", &reply)));
    }
    credential_from_body(&reply.json(), Some(refresh_token), crate::now_ms())
}

/// xAI may leave out the refresh token when it did not rotate it.
fn credential_from_body(
    body: &Value,
    previous_refresh: Option<&str>,
    now_ms: i64,
) -> Result<OAuthCredential> {
    let invalid = |field: &str| AuthError::OAuth(format!("invalid xAI response field: {field}"));
    let access =
        oauth::string_field(body, "access_token").ok_or_else(|| invalid("access_token"))?;
    let refresh = match (body.get("refresh_token"), previous_refresh) {
        (None, Some(previous)) => previous.to_string(),
        _ => oauth::string_field(body, "refresh_token").ok_or_else(|| invalid("refresh_token"))?,
    };
    let expires_in = match body.get("expires_in") {
        None => DEFAULT_TOKEN_LIFETIME_SECS,
        Some(_) => {
            oauth::positive_number(body, "expires_in").ok_or_else(|| invalid("expires_in"))?
        }
    };
    Ok(OAuthCredential {
        access_token: access,
        refresh_token: refresh,
        expires: now_ms + (expires_in * 1000.0) as i64 - EXPIRY_MARGIN_MS,
        client_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::testing::TestServer;
    use serde_json::json;

    #[test]
    fn the_complete_verification_uri_is_preferred_and_must_be_https() {
        let body = json!({
            "device_code": "d",
            "user_code": "U",
            "verification_uri": "https://accounts.x.ai/device",
            "verification_uri_complete": "https://accounts.x.ai/device?code=U",
            "expires_in": 600,
            "interval": 0,
        });
        let authorization = parse_device_authorization(&body).unwrap();
        assert_eq!(
            authorization.verification_uri,
            "https://accounts.x.ai/device?code=U"
        );
        assert_eq!(authorization.interval_secs, 0);

        let unsafe_uri = json!({
            "device_code": "d",
            "user_code": "U",
            "verification_uri": "javascript:alert(1)",
            "expires_in": 600,
        });
        assert!(parse_device_authorization(&unsafe_uri).is_err());
    }

    #[test]
    fn an_unrotated_refresh_token_is_kept() {
        let credential =
            credential_from_body(&json!({ "access_token": "a" }), Some("kept"), 0).unwrap();
        assert_eq!(credential.refresh_token, "kept");
        assert_eq!(
            credential.expires,
            (DEFAULT_TOKEN_LIFETIME_SECS * 1000.0) as i64 - EXPIRY_MARGIN_MS
        );
        assert!(credential_from_body(&json!({ "access_token": "a" }), None, 0).is_err());
    }

    #[tokio::test]
    async fn the_device_flow_waits_before_polling_and_stores_the_grant() {
        let server = TestServer::start(vec![
            (
                200,
                json!({
                    "device_code": "dev",
                    "user_code": "ABCD",
                    "verification_uri": "https://accounts.x.ai/device",
                    "expires_in": 600,
                    "interval": 1,
                })
                .to_string(),
            ),
            (400, r#"{"error":"authorization_pending"}"#.into()),
            (
                200,
                r#"{"access_token":"xa","refresh_token":"xr","expires_in":3600}"#.into(),
            ),
        ])
        .await;
        let endpoints = Endpoints {
            device_code: server.url("/device"),
            token: server.url("/token"),
        };
        let http = reqwest::Client::new();
        let authorization = start_device_flow(&http, &endpoints).await.unwrap();
        let credential = poll_device_flow(&http, &endpoints, &authorization)
            .await
            .unwrap();
        assert_eq!(credential.access_token, "xa");

        let requests = server.requests();
        let start = requests[0].form();
        assert_eq!(start["client_id"], CLIENT_ID);
        assert_eq!(start["scope"], SCOPE);
        let poll = requests[1].form();
        assert_eq!(poll["device_code"], "dev");
        assert_eq!(
            poll["grant_type"],
            "urn:ietf:params:oauth:grant-type:device_code"
        );
    }

    #[tokio::test]
    async fn a_refused_refresh_says_why() {
        let server = TestServer::start(vec![(
            400,
            r#"{"error":"invalid_grant","error_description":"expired"}"#.into(),
        )])
        .await;
        let endpoints = Endpoints {
            device_code: server.url("/device"),
            token: server.url("/token"),
        };
        let error = refresh(&reqwest::Client::new(), &endpoints, "r")
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("invalid_grant: expired"),
            "{error}"
        );
    }
}
