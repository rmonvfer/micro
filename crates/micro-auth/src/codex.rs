//! OpenAI Codex: a ChatGPT Plus/Pro sign-in for the Codex backend, by browser or device code.

use crate::oauth;
use crate::oauth::CallbackOptions;
use crate::oauth::CallbackServer;
use crate::oauth::DevicePoll;
use crate::oauth::Pkce;
use crate::oauth::PollSchedule;
use crate::AuthError;
use crate::OAuthCredential;
use crate::Result;
use serde_json::json;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_BASE_URL: &str = "https://auth.openai.com";
const AUTHORIZE_PATH: &str = "/oauth/authorize";
const TOKEN_PATH: &str = "/oauth/token";
const DEVICE_USER_CODE_PATH: &str = "/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_PATH: &str = "/api/accounts/deviceauth/token";
const DEVICE_VERIFICATION_PATH: &str = "/codex/device";
const DEVICE_REDIRECT_PATH: &str = "/deviceauth/callback";
/// Shared with the Codex CLI; when another program holds it, the pasted redirect URL stands in.
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";
const DEVICE_CODE_TIMEOUT_SECS: u64 = 15 * 60;
/// The originator the authorization server knows this sign-in by, as the reference client sends it.
const ORIGINATOR: &str = "pi";
/// Where the account id lives inside the token, as OpenAI namespaces its claims.
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";

/// The endpoints a sign-in talks to, which tests point elsewhere.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub auth_base: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            auth_base: AUTH_BASE_URL.to_string(),
        }
    }
}

impl Endpoints {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.auth_base.trim_end_matches('/'))
    }
}

pub fn redirect_uri() -> &'static str {
    REDIRECT_URI
}

pub fn authorize_url(pkce: &Pkce, state: &str) -> String {
    oauth::url_with_query(
        &Endpoints::default().url(AUTHORIZE_PATH),
        &[
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPE),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", ORIGINATOR),
        ],
    )
}

pub async fn start_callback(state: &str) -> Option<CallbackServer> {
    CallbackServer::start(CallbackOptions {
        provider_name: "OpenAI".into(),
        host: oauth::callback_host(),
        port: CALLBACK_PORT,
        path: CALLBACK_PATH.into(),
        redirect_host: Some("localhost".into()),
        state: Some(state.to_string()),
    })
    .await
    .ok()
}

pub async fn exchange_code(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthCredential> {
    let reply = oauth::post_form(
        http,
        &endpoints.url(TOKEN_PATH),
        &[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ],
    )
    .await?;
    credential_from_reply(&reply, "exchange", crate::now_ms())
}

pub async fn refresh(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    refresh_token: &str,
) -> Result<OAuthCredential> {
    let reply = oauth::post_form(
        http,
        &endpoints.url(TOKEN_PATH),
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ],
    )
    .await?;
    credential_from_reply(&reply, "refresh", crate::now_ms())
}

fn credential_from_reply(
    reply: &oauth::HttpReply,
    operation: &str,
    now_ms: i64,
) -> Result<OAuthCredential> {
    if !reply.is_success() {
        return Err(AuthError::OAuth(format!(
            "OpenAI Codex token {operation} failed ({}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        )));
    }
    let body = reply.json();
    let (Some(access), Some(refresh), Some(expires_in)) = (
        oauth::string_field(&body, "access_token"),
        oauth::string_field(&body, "refresh_token"),
        oauth::positive_number(&body, "expires_in"),
    ) else {
        return Err(AuthError::OAuth(format!(
            "OpenAI Codex token {operation} response is missing fields"
        )));
    };
    if account_id(&access).is_none() {
        return Err(AuthError::OAuth(
            "the OpenAI Codex token carries no ChatGPT account".into(),
        ));
    }
    Ok(OAuthCredential {
        access_token: access,
        refresh_token: refresh,
        expires: now_ms + (expires_in * 1000.0) as i64,
        client_id: None,
    })
}

/// The ChatGPT account a token belongs to.
pub fn account_id(token: &str) -> Option<String> {
    oauth::jwt_claims(token)?
        .get(JWT_CLAIM_PATH)?
        .get("chatgpt_account_id")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// A device authorization: the code the user types, and the id the poll is made with.
pub async fn start_device_flow(
    http: &reqwest::Client,
    endpoints: &Endpoints,
) -> Result<oauth::DeviceAuthorization> {
    let reply = oauth::post_json(
        http,
        &endpoints.url(DEVICE_USER_CODE_PATH),
        &json!({ "client_id": CLIENT_ID }),
    )
    .await?;
    if reply.status == 404 {
        return Err(AuthError::DeviceFlow(
            "OpenAI Codex device code login is not enabled for this server; use the browser \
             login instead"
                .into(),
        ));
    }
    if !reply.is_success() {
        return Err(AuthError::DeviceFlow(format!(
            "OpenAI Codex device code request failed ({}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        )));
    }
    let body = reply.json();
    let interval = body
        .get("interval")
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        })
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
    let (Some(device_auth_id), Some(user_code), Some(interval)) = (
        oauth::string_field(&body, "device_auth_id"),
        oauth::string_field(&body, "user_code"),
        interval,
    ) else {
        return Err(AuthError::DeviceFlow(
            "OpenAI Codex returned an unusable device code response".into(),
        ));
    };
    Ok(oauth::DeviceAuthorization {
        verification_uri: endpoints.url(DEVICE_VERIFICATION_PATH),
        user_code,
        device_code: device_auth_id,
        interval_secs: interval as u64,
        expires_in_secs: DEVICE_CODE_TIMEOUT_SECS,
    })
}

/// Wait for the user to approve the device code, then trade what it yields for tokens.
pub async fn poll_device_flow(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    authorization: &oauth::DeviceAuthorization,
) -> Result<OAuthCredential> {
    let schedule = PollSchedule {
        interval_secs: Some(authorization.interval_secs),
        expires_in_secs: authorization.expires_in_secs,
        wait_first: false,
    };
    let (code, verifier) = oauth::poll_device_code(schedule, || async {
        let reply = oauth::post_json(
            http,
            &endpoints.url(DEVICE_TOKEN_PATH),
            &json!({
                "device_auth_id": authorization.device_code,
                "user_code": authorization.user_code,
            }),
        )
        .await?;
        Ok(classify_device_poll(&reply))
    })
    .await?;
    exchange_code(
        http,
        endpoints,
        &code,
        &verifier,
        &endpoints.url(DEVICE_REDIRECT_PATH),
    )
    .await
}

fn classify_device_poll(reply: &oauth::HttpReply) -> DevicePoll<(String, String)> {
    let body = reply.json();
    if reply.is_success() {
        return match (
            oauth::string_field(&body, "authorization_code"),
            oauth::string_field(&body, "code_verifier"),
        ) {
            (Some(code), Some(verifier)) => DevicePoll::Complete((code, verifier)),
            _ => DevicePoll::Failed("OpenAI Codex returned an unusable device token".into()),
        };
    }
    if matches!(reply.status, 403 | 404) {
        return DevicePoll::Pending;
    }
    let code = body.get("error").and_then(|error| {
        error
            .as_str()
            .or_else(|| error.get("code").and_then(serde_json::Value::as_str))
    });
    match code {
        Some("deviceauth_authorization_pending") => DevicePoll::Pending,
        Some("slow_down") => DevicePoll::SlowDown {
            interval_secs: None,
        },
        _ => DevicePoll::Failed(format!(
            "OpenAI Codex device authorization failed ({}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::testing::TestServer;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use std::collections::BTreeMap;

    fn token_for(account: &str) -> String {
        let claims = json!({ JWT_CLAIM_PATH: { "chatgpt_account_id": account } });
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    #[test]
    fn the_authorize_url_asks_for_the_simplified_codex_flow() {
        let pkce = Pkce::from_verifier("v".into());
        let url = reqwest::Url::parse(&authorize_url(&pkce, "state-1")).unwrap();
        let params: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(url.path(), "/oauth/authorize");
        assert_eq!(params["client_id"], CLIENT_ID);
        assert_eq!(params["redirect_uri"], REDIRECT_URI);
        assert_eq!(params["state"], "state-1");
        assert_eq!(params["codex_cli_simplified_flow"], "true");
        assert_eq!(params["originator"], ORIGINATOR);
        assert_eq!(params["code_challenge"], pkce.challenge);
    }

    #[tokio::test]
    async fn a_code_is_exchanged_as_a_form_and_the_account_is_checked() {
        let access = token_for("acct_1");
        let server = TestServer::start(vec![
            (
                200,
                json!({ "access_token": access, "refresh_token": "r", "expires_in": 100 })
                    .to_string(),
            ),
            (
                200,
                json!({ "access_token": "opaque", "refresh_token": "r", "expires_in": 100 })
                    .to_string(),
            ),
        ])
        .await;
        let endpoints = Endpoints {
            auth_base: server.base.clone(),
        };
        let http = reqwest::Client::new();

        let credential = exchange_code(&http, &endpoints, "c", "v", REDIRECT_URI)
            .await
            .unwrap();
        assert_eq!(
            account_id(&credential.access_token).as_deref(),
            Some("acct_1")
        );
        let form = server.requests()[0].form();
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code_verifier"], "v");
        assert_eq!(form["client_id"], CLIENT_ID);

        let error = exchange_code(&http, &endpoints, "c", "v", REDIRECT_URI)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ChatGPT account"), "{error}");
    }

    #[tokio::test]
    async fn the_device_flow_polls_then_exchanges_against_the_device_redirect() {
        let access = token_for("acct_2");
        let server = TestServer::start(vec![
            (
                200,
                r#"{"device_auth_id":"dev-1","user_code":"ABCD-EFGH","interval":"1"}"#.into(),
            ),
            (403, "{}".into()),
            (
                400,
                r#"{"error":{"code":"deviceauth_authorization_pending"}}"#.into(),
            ),
            (
                200,
                r#"{"authorization_code":"auth-code","code_verifier":"server-verifier"}"#.into(),
            ),
            (
                200,
                json!({ "access_token": access, "refresh_token": "r", "expires_in": 100 })
                    .to_string(),
            ),
        ])
        .await;
        let endpoints = Endpoints {
            auth_base: server.base.clone(),
        };
        let http = reqwest::Client::new();

        let authorization = start_device_flow(&http, &endpoints).await.unwrap();
        assert_eq!(authorization.user_code, "ABCD-EFGH");
        assert!(authorization.verification_uri.ends_with("/codex/device"));

        let credential = poll_device_flow(&http, &endpoints, &authorization)
            .await
            .unwrap();
        assert_eq!(
            account_id(&credential.access_token).as_deref(),
            Some("acct_2")
        );

        let requests = server.requests();
        assert_eq!(requests[1].json()["device_auth_id"], "dev-1");
        let exchange = requests[4].form();
        assert_eq!(exchange["code"], "auth-code");
        assert_eq!(exchange["code_verifier"], "server-verifier");
        assert!(exchange["redirect_uri"].ends_with("/deviceauth/callback"));
    }
}
