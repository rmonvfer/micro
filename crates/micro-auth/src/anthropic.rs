//! Anthropic: Claude Pro/Max sign-in, its refresh, and workload identity federation.

use crate::oauth;
use crate::oauth::CallbackOptions;
use crate::oauth::CallbackServer;
use crate::oauth::Pkce;
use crate::AuthError;
use crate::OAuthCredential;
use crate::Result;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::OnceLock;

/// The client Claude Code signs in as, which is the only one a subscription answers to.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CALLBACK_PORT: u16 = 53692;
const CALLBACK_PATH: &str = "/callback";
/// Where Anthropic shows the code to copy, for a browser on another machine.
const COPY_CODE_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const SCOPES: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code \
                      user:mcp_servers user:file_upload";
/// Tokens are stored as expiring five minutes before Anthropic says, so none is used at its edge.
const EXPIRY_MARGIN_MS: i64 = 5 * 60 * 1000;

pub const FEDERATION_RULE_ID_ENV: &str = "ANTHROPIC_FEDERATION_RULE_ID";
pub const ORGANIZATION_ID_ENV: &str = "ANTHROPIC_ORGANIZATION_ID";
pub const IDENTITY_TOKEN_FILE_ENV: &str = "ANTHROPIC_IDENTITY_TOKEN_FILE";
pub const SERVICE_ACCOUNT_ID_ENV: &str = "ANTHROPIC_SERVICE_ACCOUNT_ID";
pub const WORKSPACE_ID_ENV: &str = "ANTHROPIC_WORKSPACE_ID";
/// Where the federation exchange is made.
pub const API_BASE_URL: &str = "https://api.anthropic.com";
const TOKEN_ENDPOINT: &str = "/v1/oauth/token";
const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// Required on requests that carry an OAuth bearer token.
pub const OAUTH_API_BETA: &str = "oauth-2025-04-20";
/// Routes a jwt-bearer exchange to the federation service.
const FEDERATION_BETA: &str = "oidc-federation-2026-04-01";
/// The token endpoint refuses assertions larger than this.
const MAX_ASSERTION_BYTES: usize = 16 * 1024;
/// Exchange again once a federated token has less than this left.
pub const FEDERATION_REFRESH_WINDOW_MS: i64 = 120_000;

/// The authorization URL for a browser sign-in.
pub fn authorize_url(pkce: &Pkce, redirect_uri: &str) -> String {
    oauth::url_with_query(
        AUTHORIZE_URL,
        &[
            ("code", "true"),
            ("client_id", CLIENT_ID),
            ("response_type", "code"),
            ("redirect_uri", redirect_uri),
            ("scope", SCOPES),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", &pkce.verifier),
        ],
    )
}

/// The redirect the browser login registers, which is `localhost` whatever host the callback binds.
pub fn browser_redirect_uri() -> String {
    format!("http://localhost:{CALLBACK_PORT}{CALLBACK_PATH}")
}

pub fn copy_code_redirect_uri() -> &'static str {
    COPY_CODE_REDIRECT_URI
}

/// Listen for the browser on Anthropic's registered port, or nothing when the port is taken.
pub async fn start_callback(pkce: &Pkce) -> Option<CallbackServer> {
    CallbackServer::start(CallbackOptions {
        provider_name: "Anthropic".into(),
        host: oauth::callback_host(),
        port: CALLBACK_PORT,
        path: CALLBACK_PATH.into(),
        redirect_host: Some("localhost".into()),
        state: Some(pkce.verifier.clone()),
    })
    .await
    .ok()
}

/// Trade an authorization code for tokens.
pub async fn exchange_code(
    http: &reqwest::Client,
    token_url: &str,
    code: &str,
    state: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthCredential> {
    let reply = oauth::post_json(
        http,
        token_url,
        &json!({
            "grant_type": "authorization_code",
            "client_id": CLIENT_ID,
            "code": code,
            "state": state,
            "redirect_uri": redirect_uri,
            "code_verifier": verifier,
        }),
    )
    .await?;
    credential_from_reply(&reply, "token exchange", crate::now_ms())
}

pub async fn refresh(
    http: &reqwest::Client,
    token_url: &str,
    refresh_token: &str,
) -> Result<OAuthCredential> {
    let reply = oauth::post_json(
        http,
        token_url,
        &json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh_token,
        }),
    )
    .await?;
    credential_from_reply(&reply, "token refresh", crate::now_ms())
}

fn credential_from_reply(
    reply: &oauth::HttpReply,
    operation: &str,
    now_ms: i64,
) -> Result<OAuthCredential> {
    if !reply.is_success() {
        return Err(AuthError::OAuth(format!(
            "Anthropic {operation} failed ({}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        )));
    }
    let body = reply.json();
    let missing = |field: &str| {
        AuthError::OAuth(format!("Anthropic {operation} response is missing {field}"))
    };
    let access =
        oauth::string_field(&body, "access_token").ok_or_else(|| missing("access_token"))?;
    let refresh =
        oauth::string_field(&body, "refresh_token").ok_or_else(|| missing("refresh_token"))?;
    let expires_in =
        oauth::positive_number(&body, "expires_in").ok_or_else(|| missing("expires_in"))?;
    Ok(OAuthCredential {
        access_token: access,
        refresh_token: refresh,
        expires: now_ms + (expires_in * 1000.0) as i64 - EXPIRY_MARGIN_MS,
        client_id: None,
    })
}

/// Workload identity federation, configured entirely from the environment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Federation {
    pub rule_id: String,
    pub organization_id: String,
    pub identity_token_file: String,
    pub service_account_id: Option<String>,
    pub workspace_id: Option<String>,
}

impl Federation {
    /// The federation the environment describes, when all three required variables are set.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let value = |name: &str| get(name).filter(|value| !value.trim().is_empty());
        Some(Federation {
            rule_id: value(FEDERATION_RULE_ID_ENV)?,
            organization_id: value(ORGANIZATION_ID_ENV)?,
            identity_token_file: value(IDENTITY_TOKEN_FILE_ENV)?,
            service_account_id: value(SERVICE_ACCOUNT_ID_ENV),
            workspace_id: value(WORKSPACE_ID_ENV),
        })
    }

    /// The JSON body of the RFC 7523 jwt-bearer exchange.
    fn exchange_body(&self, assertion: &str) -> Value {
        let mut body = json!({
            "grant_type": JWT_BEARER_GRANT,
            "assertion": assertion,
            "federation_rule_id": self.rule_id,
            "organization_id": self.organization_id,
        });
        if let Some(service_account) = &self.service_account_id {
            body["service_account_id"] = json!(service_account);
        }
        if let Some(workspace) = &self.workspace_id {
            body["workspace_id"] = json!(workspace);
        }
        body
    }

    /// Read the identity token again, since the platform that issues it rotates it in place.
    fn identity_token(&self) -> Result<String> {
        let token = std::fs::read_to_string(&self.identity_token_file).map_err(|error| {
            AuthError::Federation(format!(
                "cannot read the identity token file {}: {error}",
                self.identity_token_file
            ))
        })?;
        let token = token.trim().to_string();
        if token.is_empty() {
            return Err(AuthError::Federation(format!(
                "the identity token file {} is empty",
                self.identity_token_file
            )));
        }
        if token.len() > MAX_ASSERTION_BYTES {
            return Err(AuthError::Federation(format!(
                "the identity token is {} KiB, over the 16 KiB the token endpoint accepts",
                token.len().div_ceil(1024)
            )));
        }
        Ok(token)
    }

    /// Exchange the identity token for a short-lived access token.
    pub async fn exchange(&self, http: &reqwest::Client, base_url: &str) -> Result<FederatedToken> {
        let base_url = base_url.trim_end_matches('/');
        let assertion = self.identity_token()?;
        let url = format!("{base_url}{TOKEN_ENDPOINT}");
        let reply = oauth::send(
            http.post(&url)
                .header("content-type", "application/json")
                .header(
                    "anthropic-beta",
                    format!("{OAUTH_API_BETA},{FEDERATION_BETA}"),
                )
                .json(&self.exchange_body(&assertion)),
        )
        .await
        .map_err(|error| AuthError::Federation(format!("cannot reach {url}: {error}")))?;

        if !reply.is_success() {
            let hint = match (reply.status, &self.workspace_id) {
                (401, None) => {
                    " Check that the federation rule matches the identity token; a rule scoped to \
                     several workspaces also needs ANTHROPIC_WORKSPACE_ID. Claude Console's \
                     Workload identity page lists the authentication events."
                }
                (401, Some(_)) => {
                    " Check that the federation rule matches the identity token. Claude Console's \
                     Workload identity page lists the authentication events."
                }
                _ => "",
            };
            return Err(AuthError::Federation(format!(
                "token exchange failed ({}): {}.{hint}",
                reply.status,
                oauth::refusal(&reply.body)
            )));
        }

        let body = reply.json();
        let access = oauth::string_field(&body, "access_token").ok_or_else(|| {
            AuthError::Federation("the token exchange returned no access_token".into())
        })?;
        let expires_in = oauth::positive_number(&body, "expires_in").ok_or_else(|| {
            AuthError::Federation("the token exchange returned no expires_in".into())
        })?;
        remember_federated(&access);
        Ok(FederatedToken {
            access_token: access,
            expires: crate::now_ms() + (expires_in * 1000.0) as i64,
        })
    }
}

/// An access token issued for an identity token, kept in memory only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedToken {
    pub access_token: String,
    /// Milliseconds since the Unix epoch.
    pub expires: i64,
}

impl FederatedToken {
    pub fn is_fresh(&self, now_ms: i64) -> bool {
        self.expires - now_ms > FEDERATION_REFRESH_WINDOW_MS
    }
}

fn federated_tokens() -> &'static Mutex<HashSet<String>> {
    static TOKENS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    TOKENS.get_or_init(Default::default)
}

fn remember_federated(token: &str) {
    federated_tokens()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(token.to_string());
}

/// Whether a token was issued by workload identity federation in this process, which is sent as an
/// OAuth bearer without Claude Code's identity.
pub fn is_federated_token(token: &str) -> bool {
    federated_tokens()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(token)
}

/// Federated tokens already exchanged, by the configuration that produced them.
#[derive(Default)]
pub(crate) struct FederationCache {
    tokens: tokio::sync::Mutex<BTreeMap<String, FederatedToken>>,
}

impl FederationCache {
    /// A token good for a while yet, exchanged again when the cached one is close to lapsing.
    pub async fn token(
        &self,
        http: &reqwest::Client,
        federation: &Federation,
        base_url: &str,
    ) -> Result<FederatedToken> {
        let key = format!("{federation:?}@{base_url}");
        let mut tokens = self.tokens.lock().await;
        if let Some(token) = tokens.get(&key) {
            if token.is_fresh(crate::now_ms()) {
                return Ok(token.clone());
            }
        }
        let token = federation.exchange(http, base_url).await?;
        tokens.insert(key, token.clone());
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::testing::TestServer;

    #[test]
    fn the_authorize_url_carries_claude_codes_client_and_the_pkce_challenge() {
        let pkce = Pkce::from_verifier("verifier".into());
        let url = reqwest::Url::parse(&authorize_url(&pkce, &browser_redirect_uri())).unwrap();
        let params: BTreeMap<String, String> = url.query_pairs().into_owned().collect();

        assert_eq!(url.host_str(), Some("claude.ai"));
        assert_eq!(params["client_id"], CLIENT_ID);
        assert_eq!(params["redirect_uri"], "http://localhost:53692/callback");
        assert_eq!(params["code_challenge"], pkce.challenge);
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["state"], "verifier");
        assert_eq!(params["code"], "true");
        assert!(params["scope"].contains("user:inference"));
    }

    #[tokio::test]
    async fn a_code_is_exchanged_for_tokens_that_expire_early() {
        let server = TestServer::start(vec![(
            200,
            r#"{"access_token":"sk-ant-oat01-a","refresh_token":"sk-ant-ort01-r","expires_in":3600}"#
                .into(),
        )])
        .await;
        let before = crate::now_ms();
        let credential = exchange_code(
            &reqwest::Client::new(),
            &server.url("/v1/oauth/token"),
            "the-code",
            "the-state",
            "the-verifier",
            COPY_CODE_REDIRECT_URI,
        )
        .await
        .unwrap();

        assert_eq!(credential.access_token, "sk-ant-oat01-a");
        assert_eq!(credential.refresh_token, "sk-ant-ort01-r");
        assert!(credential.expires >= before + 3_600_000 - EXPIRY_MARGIN_MS);
        assert!(credential.expires <= crate::now_ms() + 3_600_000 - EXPIRY_MARGIN_MS);

        let sent = &server.requests()[0];
        assert_eq!(sent.method, "POST");
        let body = sent.json();
        assert_eq!(body["grant_type"], "authorization_code");
        assert_eq!(body["code"], "the-code");
        assert_eq!(body["state"], "the-state");
        assert_eq!(body["code_verifier"], "the-verifier");
        assert_eq!(body["redirect_uri"], COPY_CODE_REDIRECT_URI);
        assert_eq!(body["client_id"], CLIENT_ID);
    }

    #[tokio::test]
    async fn a_refresh_sends_the_refresh_token_and_reports_refusals() {
        let server = TestServer::start(vec![
            (
                200,
                r#"{"access_token":"new","refresh_token":"rotated","expires_in":60}"#.into(),
            ),
            (
                400,
                r#"{"error":"invalid_grant","error_description":"revoked"}"#.into(),
            ),
        ])
        .await;
        let http = reqwest::Client::new();
        let url = server.url("/v1/oauth/token");

        let refreshed = refresh(&http, &url, "old-refresh").await.unwrap();
        assert_eq!(refreshed.refresh_token, "rotated");
        assert_eq!(server.requests()[0].json()["refresh_token"], "old-refresh");

        let error = refresh(&http, &url, "rotated").await.unwrap_err();
        assert!(error.to_string().contains("revoked"), "{error}");
    }

    fn federation(file: &std::path::Path) -> Federation {
        Federation {
            rule_id: "fdrl_1".into(),
            organization_id: "org_1".into(),
            identity_token_file: file.display().to_string(),
            service_account_id: Some("svac_1".into()),
            workspace_id: None,
        }
    }

    #[test]
    fn federation_needs_all_three_variables() {
        let environment = BTreeMap::from([
            (FEDERATION_RULE_ID_ENV, "fdrl_1"),
            (ORGANIZATION_ID_ENV, "org_1"),
            (IDENTITY_TOKEN_FILE_ENV, "/var/run/token"),
            (WORKSPACE_ID_ENV, "wrkspc_1"),
        ]);
        let get = |name: &str| environment.get(name).map(|value| value.to_string());
        let found = Federation::from_env(get).unwrap();
        assert_eq!(found.workspace_id.as_deref(), Some("wrkspc_1"));
        assert_eq!(found.service_account_id, None);

        let partial = |name: &str| (name != ORGANIZATION_ID_ENV).then(|| get(name)).flatten();
        assert_eq!(Federation::from_env(partial), None);
    }

    #[tokio::test]
    async fn the_identity_token_is_exchanged_with_the_federation_beta() {
        let directory =
            std::env::temp_dir().join(format!("micro-federation-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("token");
        std::fs::write(&file, "  header.payload.signature\n").unwrap();

        let server = TestServer::start(vec![(
            200,
            r#"{"access_token":"federated-access","expires_in":600,"token_type":"Bearer"}"#.into(),
        )])
        .await;
        let token = federation(&file)
            .exchange(&reqwest::Client::new(), &server.base)
            .await
            .unwrap();

        assert_eq!(token.access_token, "federated-access");
        assert!(is_federated_token("federated-access"));
        assert!(token.is_fresh(crate::now_ms()));

        let sent = &server.requests()[0];
        assert_eq!(sent.path, "/v1/oauth/token");
        assert_eq!(
            sent.header("anthropic-beta"),
            Some("oauth-2025-04-20,oidc-federation-2026-04-01")
        );
        let body = sent.json();
        assert_eq!(body["grant_type"], JWT_BEARER_GRANT);
        assert_eq!(body["assertion"], "header.payload.signature");
        assert_eq!(body["federation_rule_id"], "fdrl_1");
        assert_eq!(body["organization_id"], "org_1");
        assert_eq!(body["service_account_id"], "svac_1");
        assert!(body.get("workspace_id").is_none());
    }

    #[tokio::test]
    async fn a_refused_exchange_explains_the_workspace() {
        let directory =
            std::env::temp_dir().join(format!("micro-federation-401-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("token");
        std::fs::write(&file, "jwt").unwrap();

        let server = TestServer::start(vec![(401, r#"{"error":"invalid_grant"}"#.into())]).await;
        let error = federation(&file)
            .exchange(&reqwest::Client::new(), &server.base)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("ANTHROPIC_WORKSPACE_ID"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn an_empty_identity_token_is_refused_before_any_request() {
        let directory =
            std::env::temp_dir().join(format!("micro-federation-empty-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("token");
        std::fs::write(&file, "\n").unwrap();

        let error = federation(&file)
            .exchange(&reqwest::Client::new(), "https://unreachable.invalid")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("empty"), "{error}");
    }
}
