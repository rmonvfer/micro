//! GitHub Copilot authentication.

use crate::AuthError;
use crate::OAuthCredential;
use crate::Result;
use serde_json::json;
use serde_json::Value;
use std::time::Duration;
use std::time::Instant;

/// The client the device flow signs in as.
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const SCOPE: &str = "read:user";
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const API_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// The editor Copilot is told it is serving.
pub const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
pub const EDITOR_VERSION: &str = "vscode/1.107.0";
pub const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";

/// Which Copilot integration is asking, which the service expects to be told.
pub const INTEGRATION_ID: &str = "vscode-chat";

/// GitHub's polling interval is a floor, not a target; a small margin keeps a slow clock from
/// tripping `slow_down`.
const POLL_MARGIN: Duration = Duration::from_secs(3);
const SLOW_DOWN_PENALTY_SECS: u64 = 5;

/// Used when the exchange response omits both an expiry and a refresh hint.
const DEFAULT_TOKEN_LIFETIME_MS: i64 = 25 * 60 * 1000;

/// A pending device authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuthorization {
    pub verification_uri: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

/// Ask GitHub for a device code.
pub async fn start_device_flow(http: &reqwest::Client) -> Result<DeviceAuthorization> {
    let response = http
        .post(DEVICE_CODE_URL)
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT)
        .json(&json!({ "client_id": CLIENT_ID, "scope": SCOPE }))
        .send()
        .await
        .map_err(|error| AuthError::DeviceFlow(error.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| AuthError::DeviceFlow(error.to_string()))?;

    if !status.is_success() {
        return Err(AuthError::DeviceFlow(format!(
            "GitHub returned {}: {}",
            status.as_u16(),
            refusal(&body)
        )));
    }

    let body: Value = serde_json::from_str(&body)
        .map_err(|error| AuthError::DeviceFlow(format!("unreadable response: {error}")))?;
    parse_device_authorization(&body)
}

pub async fn poll_for_token(
    http: &reqwest::Client,
    authorization: &DeviceAuthorization,
) -> Result<OAuthCredential> {
    let started = Instant::now();
    let deadline = Duration::from_secs(authorization.expires_in_secs);
    let mut interval = Duration::from_secs(authorization.interval_secs);

    loop {
        tokio::time::sleep(interval + POLL_MARGIN).await;

        if started.elapsed() > deadline {
            return Err(AuthError::DeviceFlow(
                "the device code expired before authorization completed".into(),
            ));
        }

        let response = http
            .post(ACCESS_TOKEN_URL)
            .header("accept", "application/json")
            .header("user-agent", USER_AGENT)
            .json(&json!({
                "client_id": CLIENT_ID,
                "device_code": authorization.device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            }))
            .send()
            .await
            .map_err(|error| AuthError::DeviceFlow(error.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| AuthError::DeviceFlow(error.to_string()))?;

        if !status.is_success() {
            return Err(AuthError::DeviceFlow(format!(
                "GitHub returned {}: {}",
                status.as_u16(),
                refusal(&body)
            )));
        }

        let body: Value = serde_json::from_str(&body)
            .map_err(|error| AuthError::DeviceFlow(format!("unreadable response: {error}")))?;

        match classify_poll(&body) {
            PollOutcome::Granted(github_token) => {
                return exchange_token(http, &github_token).await;
            }
            PollOutcome::Pending => continue,
            PollOutcome::SlowDown { interval_secs } => {
                interval = match interval_secs {
                    Some(seconds) => Duration::from_secs(seconds),
                    None => interval + Duration::from_secs(SLOW_DOWN_PENALTY_SECS),
                };
            }
            PollOutcome::Failed(message) => return Err(AuthError::DeviceFlow(message)),
        }
    }
}

pub fn base_url_from_token(token: &str) -> Option<String> {
    let proxy = token
        .split(';')
        .find_map(|claim| claim.trim().strip_prefix("proxy-ep="))?
        .trim();
    if proxy.is_empty() {
        return None;
    }
    let host = match proxy.strip_prefix("proxy.") {
        Some(rest) => format!("api.{rest}"),
        None => proxy.to_string(),
    };
    Some(format!("https://{host}"))
}

/// What a refusal says for itself, in a line.
///
/// A service having a bad day answers with a web page rather than with JSON, and a page is not
/// something to print at somebody: it says only that the service, not the request, is the trouble.
fn refusal(body: &str) -> String {
    let body = body.trim();

    if let Ok(value) = serde_json::from_str::<Value>(body) {
        for field in ["error_description", "message", "error"] {
            if let Some(said) = value.get(field).and_then(Value::as_str) {
                return said.to_string();
            }
        }
    }

    match body {
        "" => "nothing at all".to_string(),
        page if page.starts_with('<') => {
            "a web page rather than an answer, so the trouble is at their end".to_string()
        }
        said => said.chars().take(200).collect(),
    }
}

/// Trade a GitHub token for a Copilot API token.
pub async fn exchange_token(http: &reqwest::Client, github_token: &str) -> Result<OAuthCredential> {
    if github_token.is_empty() {
        return Err(AuthError::TokenExchange(
            "no GitHub token stored; sign in to Copilot again".into(),
        ));
    }

    let response = http
        .get(API_TOKEN_URL)
        .header("authorization", format!("token {github_token}"))
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT)
        .header("editor-version", EDITOR_VERSION)
        .header("editor-plugin-version", EDITOR_PLUGIN_VERSION)
        .header("copilot-integration-id", INTEGRATION_ID)
        .send()
        .await
        .map_err(|error| AuthError::TokenExchange(error.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| AuthError::TokenExchange(error.to_string()))?;

    if !status.is_success() {
        if status.as_u16() == 403 {
            return Err(AuthError::TokenExchange(
                "this GitHub credential is not entitled to Copilot. Sign in again with \
                 `micro auth login github-copilot`; a credential stored by an older \
                 micro was issued to an application Copilot does not answer to."
                    .to_string(),
            ));
        }
        return Err(AuthError::TokenExchange(format!(
            "GitHub returned {}: {}",
            status.as_u16(),
            refusal(&body)
        )));
    }

    let body: Value = serde_json::from_str(&body)
        .map_err(|error| AuthError::TokenExchange(format!("unreadable response: {error}")))?;
    parse_api_token(&body, github_token, crate::now_ms())
}

fn parse_device_authorization(body: &Value) -> Result<DeviceAuthorization> {
    let field = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AuthError::DeviceFlow(format!("response is missing {key}")))
    };

    Ok(DeviceAuthorization {
        verification_uri: field("verification_uri")?,
        user_code: field("user_code")?,
        device_code: field("device_code")?,
        interval_secs: number(body.get("interval")).unwrap_or(5),
        expires_in_secs: number(body.get("expires_in")).unwrap_or(900),
    })
}

/// What one poll of the device-code endpoint means.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PollOutcome {
    Granted(String),
    Pending,
    /// RFC 8628 §3.5: back off, either to the interval the server names or by five seconds.
    SlowDown {
        interval_secs: Option<u64>,
    },
    Failed(String),
}

fn classify_poll(body: &Value) -> PollOutcome {
    if let Some(token) = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
    {
        return PollOutcome::Granted(token.to_string());
    }

    match body.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => PollOutcome::Pending,
        Some("slow_down") => PollOutcome::SlowDown {
            interval_secs: number(body.get("interval")).filter(|seconds| *seconds > 0),
        },
        Some("expired_token") => {
            PollOutcome::Failed("the device code expired; start the login again".into())
        }
        Some("access_denied") => PollOutcome::Failed("authorization was denied".into()),
        Some(other) => PollOutcome::Failed(
            body.get("error_description")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| other.to_string()),
        ),
        None => PollOutcome::Failed("GitHub returned neither a token nor an error".into()),
    }
}

/// `expires_at` is Unix seconds.
fn parse_api_token(body: &Value, github_token: &str, now_ms: i64) -> Result<OAuthCredential> {
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| AuthError::TokenExchange("response is missing token".into()))?;

    let expires = match number(body.get("expires_at")) {
        Some(seconds) => seconds as i64 * 1000,
        None => match number(body.get("refresh_in")) {
            Some(seconds) => now_ms + seconds as i64 * 1000,
            None => now_ms + DEFAULT_TOKEN_LIFETIME_MS,
        },
    };

    Ok(OAuthCredential {
        access_token: token.to_string(),
        refresh_token: github_token.to_string(),
        expires,
    })
}

/// GitHub sends these counts as numbers, but some proxies re-encode them as strings.
fn number(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_code_response_carries_what_the_user_needs() {
        let body = json!({
            "device_code": "device-1",
            "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 899,
            "interval": 5,
        });

        assert_eq!(
            parse_device_authorization(&body).unwrap(),
            DeviceAuthorization {
                verification_uri: "https://github.com/login/device".into(),
                user_code: "ABCD-1234".into(),
                device_code: "device-1".into(),
                interval_secs: 5,
                expires_in_secs: 899,
            }
        );
    }

    #[test]
    fn a_device_code_response_without_a_code_is_an_error() {
        let body = json!({ "verification_uri": "https://github.com/login/device" });
        assert!(parse_device_authorization(&body).is_err());
    }

    #[test]
    fn polling_intervals_default_when_github_omits_them() {
        let body = json!({
            "device_code": "device-1",
            "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device",
        });
        let authorization = parse_device_authorization(&body).unwrap();

        assert_eq!(authorization.interval_secs, 5);
        assert_eq!(authorization.expires_in_secs, 900);
    }

    #[test]
    fn a_granted_poll_yields_the_github_token() {
        let body = json!({ "access_token": "gho_token", "token_type": "bearer" });
        assert_eq!(
            classify_poll(&body),
            PollOutcome::Granted("gho_token".into())
        );
    }

    #[test]
    fn a_pending_poll_keeps_waiting() {
        let body = json!({ "error": "authorization_pending" });
        assert_eq!(classify_poll(&body), PollOutcome::Pending);
    }

    #[test]
    fn slow_down_carries_the_servers_interval_when_it_names_one() {
        assert_eq!(
            classify_poll(&json!({ "error": "slow_down", "interval": 10 })),
            PollOutcome::SlowDown {
                interval_secs: Some(10)
            }
        );
        assert_eq!(
            classify_poll(&json!({ "error": "slow_down" })),
            PollOutcome::SlowDown {
                interval_secs: None
            }
        );
    }

    #[test]
    fn terminal_poll_errors_explain_themselves() {
        assert!(matches!(
            classify_poll(&json!({ "error": "expired_token" })),
            PollOutcome::Failed(_)
        ));
        assert_eq!(
            classify_poll(&json!({ "error": "unsupported", "error_description": "no such grant" })),
            PollOutcome::Failed("no such grant".into())
        );
        assert!(matches!(classify_poll(&json!({})), PollOutcome::Failed(_)));
    }

    #[test]
    fn an_exchanged_token_keeps_the_github_token_for_the_next_refresh() {
        let body = json!({ "token": "tid=abc;exp=1700000000", "expires_at": 1_700_000_000u64 });
        let credential = parse_api_token(&body, "gho_token", 1_699_999_000_000).unwrap();

        assert_eq!(credential.access_token, "tid=abc;exp=1700000000");
        assert_eq!(credential.refresh_token, "gho_token");
        assert_eq!(credential.expires, 1_700_000_000_000);
    }

    #[test]
    fn refresh_in_stands_in_for_a_missing_expiry() {
        let now = 1_000_000;
        let credential =
            parse_api_token(&json!({ "token": "t", "refresh_in": 1500 }), "gho", now).unwrap();
        assert_eq!(credential.expires, now + 1_500_000);

        let credential = parse_api_token(&json!({ "token": "t" }), "gho", now).unwrap();
        assert_eq!(credential.expires, now + DEFAULT_TOKEN_LIFETIME_MS);
    }

    #[test]
    fn an_exchange_response_without_a_token_is_an_error() {
        assert!(parse_api_token(&json!({ "expires_at": 1 }), "gho", 0).is_err());
    }

    #[test]
    fn counts_are_read_whether_numeric_or_quoted() {
        assert_eq!(number(Some(&json!(7))), Some(7));
        assert_eq!(number(Some(&json!("7"))), Some(7));
        assert_eq!(number(Some(&json!("soon"))), None);
        assert_eq!(number(None), None);
    }

    #[tokio::test]
    async fn exchanging_without_a_github_token_fails_before_any_request() {
        let error = exchange_token(&reqwest::Client::new(), "")
            .await
            .unwrap_err();
        assert!(matches!(error, AuthError::TokenExchange(_)), "{error}");
    }
}

#[cfg(test)]
mod endpoints {
    use super::*;

    /// A Business or Enterprise account is served somewhere other than the default host, and the
    /// token is what says where.
    #[test]
    fn the_token_says_where_the_account_is_served() {
        let token = "tid=abc;exp=1234;proxy-ep=proxy.business.githubcopilot.com;other=1";
        assert_eq!(
            base_url_from_token(token).as_deref(),
            Some("https://api.business.githubcopilot.com")
        );
    }

    #[test]
    fn a_host_that_is_not_prefixed_is_used_as_it_is() {
        let token = "tid=abc;proxy-ep=copilot-api.example.com";
        assert_eq!(
            base_url_from_token(token).as_deref(),
            Some("https://copilot-api.example.com")
        );
    }

    /// A token saying nothing about it leaves the caller to use the default.
    #[test]
    fn a_token_without_the_claim_says_nothing() {
        assert_eq!(base_url_from_token("tid=abc;exp=1234"), None);
        assert_eq!(base_url_from_token(""), None);
        assert_eq!(base_url_from_token("proxy-ep="), None);
    }
}
