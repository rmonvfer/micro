//! Sign in with ChatGPT: a ChatGPT subscription used directly against the OpenAI API.
//!
//! Each sign-in registers a client of its own; OpenAI names the client it issued in the redirect,
//! and that client is the one every later refresh is made as.

use crate::oauth;
use crate::oauth::CallbackOptions;
use crate::oauth::CallbackParams;
use crate::oauth::CallbackServer;
use crate::oauth::Pkce;
use crate::AuthError;
use crate::OAuthCredential;
use crate::Result;

/// The placeholder every sign-in registers under; the issued id comes back in the redirect.
const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
/// The name OpenAI shows for this installation on the consent page.
const AGENT_NAME_HINT: &str = "micro";
const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const RESOURCE: &str = "https://api.openai.com/v1";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";
const DIRECT_TOKEN_SCOPE: &str = "chatgpt.tokens.use.direct";
const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// Tokens are stored as expiring three minutes early, so none starts a request about to lapse.
const EXPIRY_MARGIN_MS: i64 = 3 * 60 * 1000;

pub fn redirect_uri() -> &'static str {
    REDIRECT_URI
}

/// OpenAI identifies an installation ("agent host") by a stable `urn:uuid:` URI.
pub fn agent_host_id(device_id: &str) -> Result<String> {
    let device_id = device_id.trim();
    if !is_uuid(device_id) {
        return Err(AuthError::OAuth(
            "Sign in with ChatGPT needs a device id (a UUID) for this installation".into(),
        ));
    }
    Ok(format!("urn:uuid:{}", device_id.to_ascii_lowercase()))
}

fn is_uuid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, length)| {
            group.len() == length && group.chars().all(|c| c.is_ascii_hexdigit())
        })
}

pub fn authorize_url(pkce: &Pkce, state: &str, nonce: &str, host_id: &str) -> String {
    oauth::url_with_query(
        AUTHORIZE_URL,
        &[
            ("client_id", DYNAMIC_CLIENT_ID),
            ("agent_name_hint", AGENT_NAME_HINT),
            ("ext_agent_host_id", host_id),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT_URI),
            ("resource", RESOURCE),
            ("scope", SCOPE),
            ("state", state),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("nonce", nonce),
        ],
    )
}

pub async fn start_callback(state: &str) -> std::io::Result<CallbackServer> {
    CallbackServer::start(CallbackOptions {
        provider_name: "ChatGPT".into(),
        host: oauth::callback_host(),
        port: CALLBACK_PORT,
        path: CALLBACK_PATH.into(),
        redirect_host: Some("127.0.0.1".into()),
        state: Some(state.to_string()),
    })
    .await
}

/// What the redirect carries: the code, and the client OpenAI issued for this installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorization {
    pub code: String,
    pub client_id: String,
}

pub fn authorization_from_callback(params: &CallbackParams, state: &str) -> Result<Authorization> {
    let code = params
        .get("code")
        .filter(|code| !code.is_empty())
        .ok_or_else(|| AuthError::OAuth("missing authorization code".into()))?;
    match params.get("state") {
        None => return Err(AuthError::OAuth("missing OAuth state".into())),
        Some(sent) if sent != state => return Err(AuthError::OAuth("OAuth state mismatch".into())),
        Some(_) => {}
    }
    let client_id = params
        .get("client_id")
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            AuthError::OAuth(
                "OpenAI's registration callback did not name the client it issued".into(),
            )
        })?;
    Ok(Authorization {
        code: code.clone(),
        client_id: client_id.to_string(),
    })
}

/// The redirect URL a user pasted, which must be the one this sign-in registered.
pub fn authorization_from_pasted(input: &str, state: &str) -> Result<Authorization> {
    let url = reqwest::Url::parse(input.trim())
        .map_err(|_| AuthError::OAuth("paste the full callback URL from the browser".into()))?;
    let expected = reqwest::Url::parse(REDIRECT_URI).expect("the redirect URI is a URL");
    if url.origin() != expected.origin() || url.path() != expected.path() {
        return Err(AuthError::OAuth(format!(
            "the pasted callback URL must start with {REDIRECT_URI}"
        )));
    }
    let params: CallbackParams = url.query_pairs().into_owned().collect();
    if let Some(error) = params.get("error") {
        return Err(AuthError::OAuth(format!(
            "ChatGPT authorization failed: {error}"
        )));
    }
    authorization_from_callback(&params, state)
}

pub async fn exchange_code(
    http: &reqwest::Client,
    token_url: &str,
    authorization: &Authorization,
    verifier: &str,
) -> Result<OAuthCredential> {
    let reply = oauth::post_form(
        http,
        token_url,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &authorization.client_id),
            ("code", &authorization.code),
            ("code_verifier", verifier),
            ("redirect_uri", REDIRECT_URI),
            ("resource", RESOURCE),
        ],
    )
    .await?;
    if reply.is_success() && oauth::string_field(&reply.json(), "id_token").is_none() {
        return Err(AuthError::OAuth(
            "OpenAI's token response did not contain an ID token".into(),
        ));
    }
    credential_from_reply(&reply, &authorization.client_id, crate::now_ms())
}

pub async fn refresh(
    http: &reqwest::Client,
    token_url: &str,
    credential: &OAuthCredential,
) -> Result<OAuthCredential> {
    let client_id = credential
        .client_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            AuthError::OAuth(
                "the stored ChatGPT credential names no issued client; sign in again".into(),
            )
        })?;
    let reply = oauth::post_form(
        http,
        token_url,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", &credential.refresh_token),
            ("resource", RESOURCE),
        ],
    )
    .await?;
    credential_from_reply(&reply, client_id, crate::now_ms())
}

fn credential_from_reply(
    reply: &oauth::HttpReply,
    client_id: &str,
    now_ms: i64,
) -> Result<OAuthCredential> {
    if !reply.is_success() {
        return Err(AuthError::OAuth(format!(
            "OpenAI token request failed ({}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        )));
    }
    let body = reply.json();
    let invalid =
        |field: &str| AuthError::OAuth(format!("OpenAI's token response has an invalid {field}"));
    let access =
        oauth::string_field(&body, "access_token").ok_or_else(|| invalid("access_token"))?;
    let refresh =
        oauth::string_field(&body, "refresh_token").ok_or_else(|| invalid("refresh_token"))?;
    let scope = oauth::string_field(&body, "scope").ok_or_else(|| invalid("scope"))?;
    let expires_in =
        oauth::positive_number(&body, "expires_in").ok_or_else(|| invalid("expires_in"))?;
    if !scope
        .split_whitespace()
        .any(|granted| granted == DIRECT_TOKEN_SCOPE)
    {
        return Err(AuthError::OAuth(format!(
            "the ChatGPT grant did not include {DIRECT_TOKEN_SCOPE}"
        )));
    }
    Ok(OAuthCredential {
        access_token: access,
        refresh_token: refresh,
        expires: now_ms + (expires_in * 1000.0) as i64 - EXPIRY_MARGIN_MS,
        client_id: Some(client_id.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::testing::TestServer;
    use serde_json::json;
    use std::collections::BTreeMap;

    const DEVICE: &str = "0F8FAD5B-D9CB-469F-A165-70867728950E";

    #[test]
    fn the_host_id_is_the_device_uuid_as_a_urn() {
        assert_eq!(
            agent_host_id(DEVICE).unwrap(),
            "urn:uuid:0f8fad5b-d9cb-469f-a165-70867728950e"
        );
        assert!(agent_host_id("not-a-uuid").is_err());
        assert!(agent_host_id("").is_err());
    }

    #[test]
    fn the_authorize_url_registers_a_dynamic_client_for_this_host() {
        let pkce = Pkce::from_verifier("v".into());
        let host = agent_host_id(DEVICE).unwrap();
        let url = reqwest::Url::parse(&authorize_url(&pkce, "s", "n", &host)).unwrap();
        let params: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(params["client_id"], DYNAMIC_CLIENT_ID);
        assert_eq!(params["ext_agent_host_id"], host);
        assert_eq!(params["redirect_uri"], REDIRECT_URI);
        assert_eq!(params["resource"], RESOURCE);
        assert!(params["scope"].contains(DIRECT_TOKEN_SCOPE));
        assert_eq!(params["nonce"], "n");
    }

    #[test]
    fn a_pasted_redirect_must_match_the_registered_one_and_the_state() {
        let good = "http://127.0.0.1:1455/auth/callback?code=c&state=s&client_id=app_1";
        assert_eq!(
            authorization_from_pasted(good, "s").unwrap(),
            Authorization {
                code: "c".into(),
                client_id: "app_1".into()
            }
        );
        assert!(authorization_from_pasted(good, "other").is_err());
        assert!(authorization_from_pasted(
            "http://localhost:9999/auth/callback?code=c&state=s&client_id=a",
            "s"
        )
        .is_err());
        assert!(authorization_from_pasted("just-a-code", "s").is_err());
        assert!(authorization_from_pasted(
            "http://127.0.0.1:1455/auth/callback?code=c&state=s",
            "s"
        )
        .is_err());
    }

    #[tokio::test]
    async fn the_issued_client_is_kept_for_refresh() {
        let granted = json!({
            "access_token": "at",
            "refresh_token": "rt",
            "id_token": "it",
            "scope": "openid chatgpt.tokens.use.direct",
            "expires_in": 3600,
        });
        let server = TestServer::start(vec![
            (200, granted.to_string()),
            (
                200,
                json!({ "access_token": "at2", "refresh_token": "rt2", "scope": "chatgpt.tokens.use.direct", "expires_in": 3600 })
                    .to_string(),
            ),
        ])
        .await;
        let http = reqwest::Client::new();
        let url = server.url("/token");
        let authorization = Authorization {
            code: "c".into(),
            client_id: "app_issued".into(),
        };

        let credential = exchange_code(&http, &url, &authorization, "v")
            .await
            .unwrap();
        assert_eq!(credential.client_id.as_deref(), Some("app_issued"));
        let form = server.requests()[0].form();
        assert_eq!(form["client_id"], "app_issued");
        assert_eq!(form["resource"], RESOURCE);

        let refreshed = refresh(&http, &url, &credential).await.unwrap();
        assert_eq!(refreshed.access_token, "at2");
        assert_eq!(server.requests()[1].form()["client_id"], "app_issued");
    }

    #[tokio::test]
    async fn a_grant_without_the_direct_scope_is_refused() {
        let server = TestServer::start(vec![(
            200,
            json!({ "access_token": "at", "refresh_token": "rt", "id_token": "it", "scope": "openid", "expires_in": 60 })
                .to_string(),
        )])
        .await;
        let error = exchange_code(
            &reqwest::Client::new(),
            &server.url("/token"),
            &Authorization {
                code: "c".into(),
                client_id: "a".into(),
            },
            "v",
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains(DIRECT_TOKEN_SCOPE), "{error}");
    }

    #[tokio::test]
    async fn a_credential_without_a_client_cannot_refresh() {
        let credential = OAuthCredential {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires: 0,
            client_id: None,
        };
        let error = refresh(&reqwest::Client::new(), "http://127.0.0.1:1/", &credential)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("sign in again"), "{error}");
    }
}
