//! Signing in to remote servers with OAuth: discovery through the server's protected resource
//! metadata (RFC 9728) and the authorization server's metadata (RFC 8414), dynamic client
//! registration, and the authorization code flow with PKCE against a loopback callback.
//!
//! A connection never starts a browser on its own. It sends the stored access token and, when
//! that is refused, tries the stored refresh token; past that it reports that a sign-in is needed,
//! which `micro mcp login` or `/mcp login` performs.

mod callback;
mod discovery;
mod flow;
mod signin;
mod store;

pub use callback::Callback;
pub use callback::CallbackServer;
pub use discovery::discover;
pub use discovery::Discovered;
pub use flow::step_up_scope;
pub use signin::begin_sign_in;
pub use signin::open_browser;
pub use signin::OAuthAuthorizer;
pub use signin::PendingSignIn;
pub use signin::SignIn;
pub use store::CredentialStore;
pub use store::ServerCredentials;
pub use store::StoredState;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

/// What a server's `WWW-Authenticate` header asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Challenge {
    pub resource_metadata_url: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

impl Challenge {
    /// Read a `Bearer` (or `DPoP`) challenge. Anything else asks for nothing in particular.
    pub fn from_header(header: Option<&str>) -> Challenge {
        let Some(header) = header else {
            return Challenge::default();
        };
        let scheme = header
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if scheme != "bearer" && scheme != "dpop" {
            return Challenge::default();
        }
        Challenge {
            resource_metadata_url: challenge_field(header, "resource_metadata")
                .filter(|url| reqwest::Url::parse(url).is_ok()),
            scope: challenge_field(header, "scope"),
            error: challenge_field(header, "error"),
            error_description: challenge_field(header, "error_description"),
        }
    }

    /// Whether the server wants more scope than the current token grants.
    pub fn insufficient_scope(&self) -> bool {
        self.error.as_deref() == Some("insufficient_scope")
    }
}

/// One `name=value` or `name="value"` parameter of a challenge. An empty value says nothing, so
/// it counts as absent.
fn challenge_field(header: &str, name: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let wanted = format!("{}=", name.to_ascii_lowercase());
    let mut from = 0;
    while let Some(found) = lower[from..].find(&wanted) {
        let start = from + found;
        let boundary = start == 0
            || lower[..start]
                .chars()
                .last()
                .is_some_and(|character| character == ',' || character.is_whitespace());
        let value_start = start + wanted.len();
        if !boundary {
            from = value_start;
            continue;
        }
        let rest = &header[value_start..];
        let value = match rest.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next().unwrap_or_default(),
            None => rest
                .split(|character: char| character == ',' || character.is_whitespace())
                .next()
                .unwrap_or_default(),
        };
        return (!value.is_empty()).then(|| value.to_string());
    }
    None
}

/// What a protected resource says about itself (RFC 9728).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceMetadata {
    pub resource: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorization_servers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes_supported: Option<Vec<String>>,
}

/// What an authorization server says about itself (RFC 8414, or OpenID Connect discovery).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_challenge_methods_supported: Option<Vec<String>>,
    /// Whether authorization responses carry an `iss` parameter (RFC 9207).
    #[serde(default)]
    pub authorization_response_iss_parameter_supported: bool,
}

/// What the token endpoint granted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub token_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// The client micro is registered as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInformation {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirect_uris: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint_auth_method: Option<String>,
}

/// Servers send `null` or `""` for fields they have no value for, such as `"scope": ""`; both
/// count as absent.
fn absent(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null)) || value.and_then(Value::as_str) == Some("")
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("invalid {key}"))
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    if absent(object.get(key)) {
        return Ok(None);
    }
    required_string(object, key).map(Some)
}

fn optional_strings(object: &Map<String, Value>, key: &str) -> Result<Option<Vec<String>>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .map(Some)
            .ok_or_else(|| format!("invalid {key}")),
        Some(_) => Err(format!("invalid {key}")),
    }
}

/// A URL a document may point micro at: parseable, and not a script.
fn safe_url(value: String, key: &str) -> Result<String, String> {
    match reqwest::Url::parse(&value) {
        Ok(url) if !["javascript", "data", "vbscript"].contains(&url.scheme()) => Ok(value),
        _ => Err(format!("invalid {key}")),
    }
}

fn object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value.as_object().ok_or_else(|| format!("invalid {what}"))
}

impl ResourceMetadata {
    pub fn parse(value: &Value) -> Result<ResourceMetadata, String> {
        let input = object(value, "protected resource metadata")?;
        Ok(ResourceMetadata {
            resource: safe_url(required_string(input, "resource")?, "resource")?,
            authorization_servers: optional_strings(input, "authorization_servers")?
                .unwrap_or_default()
                .into_iter()
                .map(|url| safe_url(url, "authorization server URL"))
                .collect::<Result<_, _>>()?,
            scopes_supported: optional_strings(input, "scopes_supported")?,
        })
    }
}

impl ServerMetadata {
    pub fn parse(value: &Value) -> Result<ServerMetadata, String> {
        let input = object(value, "authorization server metadata")?;
        Ok(ServerMetadata {
            issuer: safe_url(required_string(input, "issuer")?, "issuer")?,
            authorization_endpoint: safe_url(
                required_string(input, "authorization_endpoint")?,
                "authorization_endpoint",
            )?,
            token_endpoint: safe_url(required_string(input, "token_endpoint")?, "token_endpoint")?,
            registration_endpoint: optional_string(input, "registration_endpoint")?
                .map(|url| safe_url(url, "registration_endpoint"))
                .transpose()?,
            response_types_supported: optional_strings(input, "response_types_supported")?
                .ok_or("invalid response_types_supported")?,
            token_endpoint_auth_methods_supported: optional_strings(
                input,
                "token_endpoint_auth_methods_supported",
            )?,
            code_challenge_methods_supported: optional_strings(
                input,
                "code_challenge_methods_supported",
            )?,
            authorization_response_iss_parameter_supported: input
                .get("authorization_response_iss_parameter_supported")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

impl Tokens {
    pub fn parse(value: &Value) -> Result<Tokens, String> {
        let input = object(value, "token response")?;
        let expires_in = match input.get("expires_in") {
            value if absent(value) => None,
            Some(Value::Number(number)) => Some(
                number
                    .as_f64()
                    .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                    .ok_or("invalid expires_in")? as u64,
            ),
            Some(Value::String(text)) => Some(
                text.trim()
                    .parse::<u64>()
                    .map_err(|_| "invalid expires_in")?,
            ),
            _ => return Err("invalid expires_in".to_string()),
        };
        Ok(Tokens {
            access_token: required_string(input, "access_token")?,
            token_type: required_string(input, "token_type")?,
            expires_in,
            scope: optional_string(input, "scope")?,
            refresh_token: optional_string(input, "refresh_token")?,
        })
    }
}

impl ClientInformation {
    pub fn parse(value: &Value) -> Result<ClientInformation, String> {
        let input = object(value, "client registration response")?;
        Ok(ClientInformation {
            client_id: required_string(input, "client_id")?,
            client_secret: optional_string(input, "client_secret")?,
            redirect_uris: optional_strings(input, "redirect_uris")?.unwrap_or_default(),
            token_endpoint_auth_method: optional_string(input, "token_endpoint_auth_method")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_challenge_names_its_metadata_and_scope() {
        let challenge = Challenge::from_header(Some(
            r#"Bearer error="insufficient_scope", scope="files:read files:write", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource""#,
        ));
        assert!(challenge.insufficient_scope());
        assert_eq!(challenge.scope.as_deref(), Some("files:read files:write"));
        assert_eq!(
            challenge.resource_metadata_url.as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource")
        );

        let empty = Challenge::from_header(Some(r#"Bearer scope="", error=invalid_token"#));
        assert_eq!(empty.scope, None, "an empty scope asks for nothing");
        assert_eq!(empty.error.as_deref(), Some("invalid_token"));
        assert_eq!(
            Challenge::from_header(Some("Basic realm=x")),
            Challenge::default()
        );
    }

    #[test]
    fn empty_and_null_token_fields_count_as_absent() {
        let tokens = Tokens::parse(&json!({
            "access_token": "at",
            "token_type": "Bearer",
            "scope": "",
            "refresh_token": null,
            "expires_in": null,
        }))
        .unwrap();
        assert_eq!(tokens.scope, None);
        assert_eq!(tokens.refresh_token, None);
        assert_eq!(tokens.expires_in, None);

        assert!(Tokens::parse(&json!({ "access_token": "", "token_type": "Bearer" })).is_err());
        assert_eq!(
            Tokens::parse(&json!({ "access_token": "a", "token_type": "b", "expires_in": "3600" }))
                .unwrap()
                .expires_in,
            Some(3600)
        );
    }

    #[test]
    fn metadata_pointing_at_scripts_is_refused() {
        let error = ServerMetadata::parse(&json!({
            "issuer": "https://auth.example.com",
            "authorization_endpoint": "javascript:alert(1)",
            "token_endpoint": "https://auth.example.com/token",
            "response_types_supported": ["code"],
        }))
        .unwrap_err();
        assert!(error.contains("authorization_endpoint"), "{error}");

        let parsed = ServerMetadata::parse(&json!({
            "issuer": "https://auth.example.com",
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token",
            "registration_endpoint": "",
            "response_types_supported": ["code"],
        }))
        .unwrap();
        assert_eq!(parsed.registration_endpoint, None);
    }
}
