//! The requests of an authorization code grant: registering a client, sending the user to
//! authorize, and trading a code or refresh token for tokens.

use super::discovery::Discovered;
use super::ClientInformation;
use super::Tokens;
use crate::config;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore as _;
use reqwest::Url;
use serde_json::json;
use serde_json::Value;
use sha2::Digest as _;
use sha2::Sha256;

/// Why a token request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowError {
    /// The authorization server answered with an OAuth error code.
    OAuth { code: String, description: String },
    /// Anything else, already worded.
    Other(String),
}

impl std::fmt::Display for FlowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlowError::OAuth { code, description } if code == description => {
                write!(formatter, "{code}")
            }
            FlowError::OAuth { code, description } => write!(formatter, "{code}: {description}"),
            FlowError::Other(message) => write!(formatter, "{message}"),
        }
    }
}

impl From<String> for FlowError {
    fn from(message: String) -> Self {
        FlowError::Other(message)
    }
}

/// A PKCE verifier and the S256 challenge derived from it.
pub fn pkce() -> (String, String) {
    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// 32 random bytes, URL-safe.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Scopes for asking again after `insufficient_scope`: the challenged scopes plus those granted
/// so far, since a challenge may list only what is missing and a token with just that would lose
/// what the old one could do. Without challenged scopes, `None` lets the flow pick its default.
pub fn step_up_scope(granted: Option<&str>, challenged: Option<&str>) -> Option<String> {
    challenged?;
    merge_scopes(&[granted, challenged])
}

/// The scopes of every list, each once, in the order first named.
pub fn merge_scopes(lists: &[Option<&str>]) -> Option<String> {
    let mut merged: Vec<&str> = Vec::new();
    for scope in lists
        .iter()
        .flatten()
        .flat_map(|list| list.split_whitespace())
    {
        if !merged.contains(&scope) {
            merged.push(scope);
        }
    }
    (!merged.is_empty()).then(|| merged.join(" "))
}

fn secure_endpoint(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| format!("invalid OAuth endpoint {value}"))?;
    match config::secure_or_loopback(&url) {
        true => Ok(url),
        false => Err(format!(
            "the OAuth endpoint {value} must use https, except on localhost"
        )),
    }
}

fn endpoint(discovered: &Discovered, path: &str) -> Result<String, String> {
    Url::parse(&discovered.authorization_server)
        .and_then(|base| base.join(path))
        .map(|url| url.to_string())
        .map_err(|error| error.to_string())
}

/// Register micro with the authorization server (RFC 7591).
pub async fn register(
    http: &reqwest::Client,
    discovered: &Discovered,
    client_name: &str,
    redirect_uri: &str,
    scope: Option<&str>,
) -> Result<ClientInformation, String> {
    let url = match &discovered.metadata {
        Some(metadata) => metadata.registration_endpoint.clone().ok_or(
            "the authorization server does not support dynamic client registration; \
             configure oauth.clientId",
        )?,
        None => endpoint(discovered, "/register")?,
    };
    let mut body = json!({
        "client_name": client_name,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    if let Some(scope) = scope {
        body["scope"] = json!(scope);
    }
    let response = http
        .post(&url)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "client registration failed with HTTP {}: {}",
            status.as_u16(),
            text.trim()
        ));
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| "the client registration response is not JSON".to_string())?;
    let mut information = ClientInformation::parse(&value)?;
    if information.redirect_uris.is_empty() {
        information.redirect_uris = vec![redirect_uri.to_string()];
    }
    Ok(information)
}

/// Everything that goes into the URL a user authorizes at.
pub struct AuthorizationRequest<'a> {
    pub client: &'a ClientInformation,
    pub redirect_uri: &'a str,
    pub scope: Option<&'a str>,
    pub state: &'a str,
    pub code_challenge: &'a str,
    pub resource: Option<&'a str>,
}

pub fn authorization_url(
    discovered: &Discovered,
    request: &AuthorizationRequest<'_>,
) -> Result<Url, String> {
    if let Some(metadata) = &discovered.metadata {
        if !metadata.response_types_supported.is_empty()
            && !metadata
                .response_types_supported
                .iter()
                .any(|kind| kind == "code")
        {
            return Err("the authorization server does not support authorization codes".into());
        }
        if metadata
            .code_challenge_methods_supported
            .as_ref()
            .is_some_and(|methods| !methods.iter().any(|method| method == "S256"))
        {
            return Err("the authorization server does not support PKCE S256".into());
        }
    }
    let base = match &discovered.metadata {
        Some(metadata) => metadata.authorization_endpoint.clone(),
        None => endpoint(discovered, "/authorize")?,
    };
    let mut url = Url::parse(&base).map_err(|error| error.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &request.client.client_id)
            .append_pair("code_challenge", request.code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("redirect_uri", request.redirect_uri)
            .append_pair("state", request.state);
        if let Some(scope) = request.scope {
            query.append_pair("scope", scope);
            if scope
                .split_whitespace()
                .any(|scope| scope == "offline_access")
            {
                query.append_pair("prompt", "consent");
            }
        }
        if let Some(resource) = request.resource {
            query.append_pair("resource", resource);
        }
    }
    Ok(url)
}

/// How micro proves which client it is at the token endpoint.
fn client_authentication(client: &ClientInformation, supported: &[String]) -> &'static str {
    let hinted = client.token_endpoint_auth_method.as_deref();
    for method in ["client_secret_basic", "client_secret_post", "none"] {
        if hinted == Some(method) && (supported.is_empty() || supported.iter().any(|s| s == method))
        {
            return method;
        }
    }
    let secret = client.client_secret.is_some();
    let offers = |method: &str| supported.iter().any(|offered| offered == method);
    match () {
        _ if supported.is_empty() && secret => "client_secret_basic",
        _ if supported.is_empty() => "none",
        _ if secret && offers("client_secret_basic") => "client_secret_basic",
        _ if secret && offers("client_secret_post") => "client_secret_post",
        _ if offers("none") => "none",
        _ if secret => "client_secret_post",
        _ => "none",
    }
}

async fn token_request(
    http: &reqwest::Client,
    discovered: &Discovered,
    client: &ClientInformation,
    resource: Option<&str>,
    mut params: Vec<(&str, String)>,
) -> Result<Tokens, FlowError> {
    let url = match &discovered.metadata {
        Some(metadata) => metadata.token_endpoint.clone(),
        None => endpoint(discovered, "/token")?,
    };
    let url = secure_endpoint(&url)?;
    if let Some(resource) = resource {
        params.push(("resource", resource.to_string()));
    }

    let supported = discovered
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.token_endpoint_auth_methods_supported.clone())
        .unwrap_or_default();
    let mut request = http.post(url.clone()).header("Accept", "application/json");
    match client_authentication(client, &supported) {
        "client_secret_basic" => {
            request = request.basic_auth(&client.client_id, client.client_secret.as_deref());
        }
        method => {
            params.push(("client_id", client.client_id.clone()));
            if method == "client_secret_post" {
                if let Some(secret) = &client.client_secret {
                    params.push(("client_secret", secret.clone()));
                }
            }
        }
    }

    let response = request
        .form(&params)
        .send()
        .await
        .map_err(|error| FlowError::Other(format!("cannot reach {url}: {error}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let value: Option<Value> = serde_json::from_str(&text).ok();

    // Servers report OAuth errors with any status, so the body is read before the status.
    if let Some(code) = value
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
    {
        let description = value
            .as_ref()
            .and_then(|value| value.get("error_description"))
            .and_then(Value::as_str)
            .unwrap_or(code);
        return Err(FlowError::OAuth {
            code: code.to_string(),
            description: description.to_string(),
        });
    }
    if !status.is_success() {
        return Err(FlowError::OAuth {
            code: "server_error".to_string(),
            description: format!("HTTP {}: {}", status.as_u16(), text.trim()),
        });
    }
    let value = value.ok_or_else(|| FlowError::Other("the token response is not JSON".into()))?;
    Ok(Tokens::parse(&value)?)
}

/// Trade an authorization code for tokens.
pub async fn exchange_code(
    http: &reqwest::Client,
    discovered: &Discovered,
    client: &ClientInformation,
    resource: Option<&str>,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Tokens, FlowError> {
    token_request(
        http,
        discovered,
        client,
        resource,
        vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("code_verifier", verifier.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
        ],
    )
    .await
}

/// Trade a refresh token for fresh tokens. A response without a refresh token keeps the old one.
pub async fn refresh(
    http: &reqwest::Client,
    discovered: &Discovered,
    client: &ClientInformation,
    resource: Option<&str>,
    refresh_token: &str,
) -> Result<Tokens, FlowError> {
    let mut tokens = token_request(
        http,
        discovered,
        client,
        resource,
        vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.to_string()),
        ],
    )
    .await?;
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token.to_string());
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_up_keeps_what_was_granted() {
        assert_eq!(
            step_up_scope(Some("read"), Some("write")).as_deref(),
            Some("read write")
        );
        assert_eq!(
            step_up_scope(Some("read write"), Some("write admin")).as_deref(),
            Some("read write admin")
        );
        assert_eq!(step_up_scope(Some("read"), None), None);
    }

    #[test]
    fn the_challenge_is_the_hash_of_the_verifier() {
        let (verifier, challenge) = pkce();
        assert_eq!(
            challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        );
        assert!(verifier.len() >= 43);
    }

    #[test]
    fn a_secret_goes_where_the_server_accepts_it() {
        let public = ClientInformation {
            client_id: "c".into(),
            client_secret: None,
            redirect_uris: Vec::new(),
            token_endpoint_auth_method: None,
        };
        let confidential = ClientInformation {
            client_secret: Some("s".into()),
            ..public.clone()
        };
        assert_eq!(client_authentication(&public, &[]), "none");
        assert_eq!(
            client_authentication(&confidential, &[]),
            "client_secret_basic"
        );
        assert_eq!(
            client_authentication(&confidential, &["client_secret_post".into()]),
            "client_secret_post"
        );
    }
}
