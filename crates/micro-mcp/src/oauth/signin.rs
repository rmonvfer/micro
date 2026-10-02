//! Signing a user in to a server, and keeping a connection's token fresh afterwards.

use super::callback::Callback;
use super::callback::CallbackServer;
use super::discovery;
use super::discovery::Discovered;
use super::flow;
use super::Challenge;
use super::ClientInformation;
use super::ServerCredentials;
use crate::config;
use crate::config::OAuthConfig;
use crate::transport::Authorizer;
use crate::transport::TransportError;
use async_trait::async_trait;
use reqwest::Url;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

/// The name micro registers itself under, unless a server needs another.
const CLIENT_NAME: &str = "micro";

const CALLBACK_HOST: &str = "127.0.0.1";
const CALLBACK_PATH: &str = "/callback";

/// Access tokens this close to lapsing are refreshed before they are sent.
const REFRESH_SKEW_MS: i64 = 30_000;

/// How long one request of a refresh may take, so a slow server cannot hold the lock for long.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(15);

/// A sign-in waiting for the user to authorize micro in a browser.
pub struct PendingSignIn {
    /// The page the user opens.
    pub authorization_url: Url,
    callback: CallbackServer,
    state: String,
    verifier: String,
    redirect_uri: String,
    scope: Option<String>,
    resource: Option<String>,
    discovered: Discovered,
    client: ClientInformation,
    credentials: ServerCredentials,
}

/// Where a sign-in stands once it has started.
pub enum SignIn {
    /// The stored refresh token was enough; nothing for the user to do.
    Authorized,
    /// The user has to authorize micro.
    Pending(Box<PendingSignIn>),
}

/// Where the loopback callback listens, and the redirect URI it stands behind.
struct CallbackPlan {
    host: String,
    redirect_host: String,
    port: Option<u16>,
    path: String,
    /// The redirect URI exactly as configured, when its port is fixed.
    fixed: Option<String>,
}

fn callback_plan(oauth: &OAuthConfig) -> Result<CallbackPlan, String> {
    let configured = oauth
        .callback_url
        .clone()
        .unwrap_or_else(|| format!("http://{CALLBACK_HOST}{CALLBACK_PATH}"));
    let mut url = Url::parse(&configured).map_err(|error| error.to_string())?;
    let address = url
        .host_str()
        .unwrap_or(CALLBACK_HOST)
        .trim_matches(|character| character == '[' || character == ']')
        .to_string();
    let port = url.port().or(oauth.callback_port);
    let fixed = match (url.port(), port) {
        (Some(_), _) => oauth.callback_url.clone(),
        (None, Some(port)) => {
            let _ = url.set_port(Some(port));
            Some(url.to_string())
        }
        (None, None) => None,
    };
    Ok(CallbackPlan {
        host: match address.as_str() {
            "localhost" => CALLBACK_HOST.to_string(),
            other => other.to_string(),
        },
        redirect_host: address,
        port,
        path: url.path().to_string(),
        fixed,
    })
}

/// The client micro signs in as: the configured one, or the one it registered before.
fn configured_client(
    oauth: &OAuthConfig,
    redirect_uri: &str,
) -> Result<Option<ClientInformation>, String> {
    let Some(client_id) = &oauth.client_id else {
        return Ok(None);
    };
    let client_secret = oauth
        .client_secret
        .as_deref()
        .map(config::resolve_value)
        .transpose()?;
    Ok(Some(ClientInformation {
        client_id: client_id.clone(),
        client_secret,
        redirect_uris: vec![redirect_uri.to_string()],
        token_endpoint_auth_method: None,
    }))
}

/// Start signing in to the server at `url`. Uses the stored refresh token when it can; otherwise
/// registers micro if needed, starts the loopback callback, and hands back the page to open.
/// `challenge` is what the server last asked for, so a request for more scope keeps the scope
/// already granted.
pub async fn begin_sign_in(
    url: &str,
    oauth: &OAuthConfig,
    credentials: ServerCredentials,
    challenge: Option<&Challenge>,
) -> Result<SignIn, String> {
    let server = Url::parse(url).map_err(|_| format!("invalid server URL {url}"))?;
    let http = reqwest::Client::new();
    let stored = credentials.load().unwrap_or_default();
    let step_up = challenge.is_some_and(Challenge::insufficient_scope);

    let plan = callback_plan(oauth)?;
    let registered_port = stored
        .client_information
        .as_ref()
        .and_then(|client| client.redirect_uris.first())
        .and_then(|uri| Url::parse(uri).ok())
        .and_then(|uri| uri.port());
    let state = flow::random_token();
    let callback = match CallbackServer::listen(
        &plan.host,
        &plan.redirect_host,
        plan.port.or(registered_port),
        &plan.path,
        state.clone(),
    )
    .await
    {
        Ok(callback) => callback,
        Err(error) if plan.port.is_some() => {
            return Err(format!("cannot listen for the sign-in callback: {error}"))
        }
        Err(_) => CallbackServer::listen(
            &plan.host,
            &plan.redirect_host,
            None,
            &plan.path,
            state.clone(),
        )
        .await
        .map_err(|error| format!("cannot listen for the sign-in callback: {error}"))?,
    };
    let redirect_uri = plan
        .fixed
        .clone()
        .unwrap_or_else(|| callback.redirect_uri().to_string());

    let mut stored = stored;
    let registered_here = stored
        .client_information
        .as_ref()
        .is_some_and(|client| client.redirect_uris.contains(&redirect_uri));
    if oauth.client_id.is_none() && !registered_here {
        // A registered client cannot use another redirect URI, and its tokens belong to it.
        stored.client_information = None;
        stored.tokens = None;
        stored.tokens_expire_at = None;
    }

    let discovered = discovery::discover(
        &http,
        &server,
        challenge.and_then(|challenge| challenge.resource_metadata_url.as_deref()),
        oauth.auth_server_metadata_url.as_deref(),
    )
    .await?;
    let resource = discovery::select_resource(&server, discovered.resource.as_ref())?;

    let challenged = match step_up {
        true => flow::step_up_scope(
            stored
                .tokens
                .as_ref()
                .and_then(|tokens| tokens.scope.as_deref()),
            challenge.and_then(|challenge| challenge.scope.as_deref()),
        ),
        false => challenge.and_then(|challenge| challenge.scope.clone()),
    };
    let advertised = discovered
        .resource
        .as_ref()
        .and_then(|resource| resource.scopes_supported.as_ref())
        .map(|scopes| scopes.join(" "))
        .filter(|scopes| !scopes.is_empty());
    let scope = flow::merge_scopes(&[oauth.scope.as_deref(), challenged.as_deref()]).or(advertised);

    let client = match configured_client(oauth, &redirect_uri)? {
        Some(client) => client,
        None => match stored.client_information.clone() {
            Some(client) => client,
            None => {
                flow::register(
                    &http,
                    &discovered,
                    oauth.client_name.as_deref().unwrap_or(CLIENT_NAME),
                    &redirect_uri,
                    scope.as_deref(),
                )
                .await?
            }
        },
    };
    stored.client_information = Some(client.clone());
    stored.discovered = Some(discovered.clone());

    // A refresh keeps the granted scope, so a server asking for more needs the browser.
    let refresh_token = stored
        .tokens
        .as_ref()
        .and_then(|tokens| tokens.refresh_token.clone());
    if let (false, Some(refresh_token)) = (step_up, refresh_token) {
        if let Ok(mut tokens) = flow::refresh(
            &http,
            &discovered,
            &client,
            resource.as_deref(),
            &refresh_token,
        )
        .await
        {
            if tokens.scope.is_none() {
                tokens.scope = stored.tokens.as_ref().and_then(|old| old.scope.clone());
            }
            credentials
                .save(stored.with_tokens(tokens))
                .map_err(|error| error.to_string())?;
            return Ok(SignIn::Authorized);
        }
        stored.tokens = None;
        stored.tokens_expire_at = None;
    }

    let (verifier, code_challenge) = flow::pkce();
    let authorization_url = flow::authorization_url(
        &discovered,
        &flow::AuthorizationRequest {
            client: &client,
            redirect_uri: &redirect_uri,
            scope: scope.as_deref(),
            state: &state,
            code_challenge: &code_challenge,
            resource: resource.as_deref(),
        },
    )?;
    credentials
        .save(stored)
        .map_err(|error| error.to_string())?;

    Ok(SignIn::Pending(Box::new(PendingSignIn {
        authorization_url,
        callback,
        state,
        verifier,
        redirect_uri,
        scope,
        resource,
        discovered,
        client,
        credentials,
    })))
}

impl PendingSignIn {
    /// Wait for the browser to come back, then trade the code for tokens.
    pub async fn finish(mut self, within: Duration) -> Result<(), String> {
        let callback = self.wait_for_browser(within).await?;
        self.complete(callback).await
    }

    /// Wait for the browser to come back with the authorization response.
    pub async fn wait_for_browser(&mut self, within: Duration) -> Result<Callback, String> {
        self.callback.wait(within).await
    }

    /// Read the URL the browser was sent back to, for when it could not reach this machine, such
    /// as over SSH.
    pub fn parse_redirect(&self, pasted: &str) -> Result<Callback, String> {
        let url = Url::parse(pasted.trim())
            .map_err(|_| "expected the full redirect URL from the browser's address bar")?;
        let parameter = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        if let Some(error) = parameter("error") {
            return Err(parameter("error_description").unwrap_or(error));
        }
        if parameter("state").as_deref() != Some(self.state.as_str()) {
            return Err("the redirect URL belongs to a different sign-in".to_string());
        }
        let code = parameter("code").ok_or("the redirect URL has no authorization code")?;
        Ok(Callback {
            code,
            iss: parameter("iss"),
        })
    }

    /// Trade the authorization response for tokens and keep them.
    pub async fn complete(self, callback: Callback) -> Result<(), String> {
        // RFC 9207: a code from another authorization server is never sent to this one.
        if let Some(metadata) = &self.discovered.metadata {
            if (callback.iss.is_some() || metadata.authorization_response_iss_parameter_supported)
                && callback.iss.as_deref() != Some(metadata.issuer.as_str())
            {
                return Err(format!(
                    "the authorization response came from issuer {}, expected {}",
                    callback.iss.as_deref().unwrap_or("(none)"),
                    metadata.issuer
                ));
            }
        }

        let http = reqwest::Client::new();
        let exchanged = flow::exchange_code(
            &http,
            &self.discovered,
            &self.client,
            self.resource.as_deref(),
            &callback.code,
            &self.verifier,
            &self.redirect_uri,
        )
        .await;
        let mut stored = self.credentials.load().unwrap_or_default();
        let mut tokens = match exchanged {
            Ok(tokens) => tokens,
            Err(flow::FlowError::OAuth { code, description })
                if code == "invalid_client" || code == "unauthorized_client" =>
            {
                stored.client_information = None;
                let _ = self.credentials.save(stored);
                return Err(format!(
                    "the authorization server no longer knows micro's client ({description}); \
                     sign in again"
                ));
            }
            Err(error) => return Err(error.to_string()),
        };
        // A response without `scope` grants what was asked for (RFC 6749 §5.1), recorded so a
        // later request for more keeps it.
        if tokens.scope.is_none() {
            tokens.scope = self.scope.clone();
        }
        stored.client_information = Some(self.client.clone());
        stored.discovered = Some(self.discovered.clone());
        self.credentials
            .save(stored.with_tokens(tokens))
            .map_err(|error| error.to_string())
    }
}

/// Gives a connection the stored access token, refreshing it when it is about to lapse or the
/// server refuses it. When only the user can help, says a sign-in is needed.
pub struct OAuthAuthorizer {
    server: Url,
    oauth: OAuthConfig,
    credentials: ServerCredentials,
    http: reqwest::Client,
    refreshing: tokio::sync::Mutex<()>,
    challenge: Arc<Mutex<Option<Challenge>>>,
}

impl OAuthAuthorizer {
    pub fn new(server: Url, oauth: OAuthConfig, credentials: ServerCredentials) -> OAuthAuthorizer {
        OAuthAuthorizer {
            server,
            oauth,
            credentials,
            http: reqwest::Client::builder()
                .timeout(REFRESH_TIMEOUT)
                .build()
                .unwrap_or_default(),
            refreshing: tokio::sync::Mutex::new(()),
            challenge: Arc::default(),
        }
    }

    /// Where the server's last challenge is kept, for the sign-in that answers it.
    pub fn challenges(&self) -> Arc<Mutex<Option<Challenge>>> {
        Arc::clone(&self.challenge)
    }

    /// Replace `stale`, the access token that lapsed or was refused.
    async fn refresh(&self, stale: Option<&str>) -> Result<(), String> {
        let _in_process = self.refreshing.lock().await;
        let _across_processes = self.credentials.refresh_lock().await.ok();

        let Some(stored) = self.credentials.load() else {
            return Err("not signed in".to_string());
        };
        let current = stored
            .tokens
            .as_ref()
            .map(|tokens| tokens.access_token.as_str());
        if current.is_some() && current != stale {
            // Another request or process already replaced it.
            return Ok(());
        }
        let refresh_token = stored
            .tokens
            .as_ref()
            .and_then(|tokens| tokens.refresh_token.clone())
            .ok_or("no refresh token")?;
        let discovered = match (&stored.discovered, &self.oauth.auth_server_metadata_url) {
            (Some(discovered), None) => discovered.clone(),
            _ => {
                discovery::discover(
                    &self.http,
                    &self.server,
                    None,
                    self.oauth.auth_server_metadata_url.as_deref(),
                )
                .await?
            }
        };
        let client = match stored.client_information.clone() {
            Some(client) => client,
            None => configured_client(&self.oauth, "")?.ok_or("no registered client")?,
        };
        let resource = discovery::select_resource(&self.server, discovered.resource.as_ref())?;
        let mut tokens = flow::refresh(
            &self.http,
            &discovered,
            &client,
            resource.as_deref(),
            &refresh_token,
        )
        .await
        .map_err(|error| error.to_string())?;
        if tokens.scope.is_none() {
            tokens.scope = stored.tokens.as_ref().and_then(|old| old.scope.clone());
        }
        let mut stored = stored;
        stored.discovered = Some(discovered);
        self.credentials
            .save(stored.with_tokens(tokens))
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl Authorizer for OAuthAuthorizer {
    async fn token(&self) -> Result<Option<String>, TransportError> {
        let stored = self.credentials.load();
        let token = stored
            .as_ref()
            .and_then(|stored| stored.tokens.as_ref())
            .map(|tokens| tokens.access_token.clone());
        let lapsing = stored
            .as_ref()
            .and_then(|stored| stored.tokens_expire_at)
            .is_some_and(|expires| expires - REFRESH_SKEW_MS <= micro_auth::now_ms());
        if !lapsing {
            return Ok(token);
        }
        // A failed refresh sends the old token; the server's answer decides what happens next.
        let _ = self.refresh(token.as_deref()).await;
        Ok(self
            .credentials
            .load()
            .and_then(|stored| stored.tokens)
            .map(|tokens| tokens.access_token))
    }

    async fn unauthorized(
        &self,
        challenge: &Challenge,
        rejected: Option<&str>,
    ) -> Result<(), TransportError> {
        *self
            .challenge
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(challenge.clone());
        if challenge.insufficient_scope() || rejected.is_none() {
            return Err(TransportError::AuthRequired(challenge.clone()));
        }
        self.refresh(rejected)
            .await
            .map_err(|_| TransportError::AuthRequired(challenge.clone()))
    }
}

/// Open `url` in the user's browser, if this machine has one to open.
pub fn open_browser(url: &str) {
    let mut command = match std::env::consts::OS {
        "macos" => std::process::Command::new("open"),
        "windows" => {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "start", ""]);
            command
        }
        _ => std::process::Command::new("xdg-open"),
    };
    let _ = command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configured_callback_url_is_sent_exactly_as_written() {
        let plan = callback_plan(&OAuthConfig {
            callback_url: Some("http://localhost:8080/oauth/callback".into()),
            ..OAuthConfig::default()
        })
        .unwrap();
        assert_eq!(plan.host, "127.0.0.1");
        assert_eq!(plan.redirect_host, "localhost");
        assert_eq!(plan.port, Some(8080));
        assert_eq!(plan.path, "/oauth/callback");
        assert_eq!(
            plan.fixed.as_deref(),
            Some("http://localhost:8080/oauth/callback")
        );

        let port_only = callback_plan(&OAuthConfig {
            callback_port: Some(8765),
            ..OAuthConfig::default()
        })
        .unwrap();
        assert_eq!(
            port_only.fixed.as_deref(),
            Some("http://127.0.0.1:8765/callback")
        );

        let free = callback_plan(&OAuthConfig::default()).unwrap();
        assert_eq!(free.port, None);
        assert_eq!(free.fixed, None);
    }
}
