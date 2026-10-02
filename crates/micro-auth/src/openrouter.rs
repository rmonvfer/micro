//! OpenRouter: a PKCE sign-in that yields a permanent, user-controlled API key.

use crate::oauth;
use crate::oauth::CallbackOptions;
use crate::oauth::CallbackServer;
use crate::oauth::Pkce;
use crate::AuthError;
use crate::OAuthCredential;
use crate::Result;
use serde_json::json;

const AUTHORIZE_URL: &str = "https://openrouter.ai/auth";
pub const TOKEN_URL: &str = "https://openrouter.ai/api/v1/auth/keys";
/// The key never lapses; this is the largest expiry every JSON reader holds exactly.
pub const NEVER_EXPIRES: i64 = 9_007_199_254_740_991;

pub fn authorize_url(pkce: &Pkce, callback_url: &str) -> String {
    oauth::url_with_query(
        AUTHORIZE_URL,
        &[
            ("callback_url", callback_url),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
        ],
    )
}

/// Listen on a free port. OpenRouter sends no `state`, so a random path keeps stray requests from
/// finishing the sign-in.
pub async fn start_callback() -> std::io::Result<CallbackServer> {
    CallbackServer::start(CallbackOptions {
        provider_name: "OpenRouter".into(),
        host: oauth::callback_host(),
        port: 0,
        path: format!("/oauth/callback/{}", oauth::random_uuid()),
        redirect_host: None,
        state: None,
    })
    .await
}

/// The code in what a user pasted: a redirect URL, a query string, or the code itself.
pub fn code_from_pasted(input: &str) -> Option<String> {
    oauth::parse_authorization_input(input).code
}

/// Trade the code for an API key.
pub async fn exchange_code(
    http: &reqwest::Client,
    token_url: &str,
    code: &str,
    verifier: &str,
) -> Result<OAuthCredential> {
    let reply = oauth::post_json(
        http,
        token_url,
        &json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256",
        }),
    )
    .await?;
    if !reply.is_success() {
        return Err(AuthError::OAuth(format!(
            "OpenRouter key exchange failed (HTTP {}): {}",
            reply.status,
            oauth::refusal(&reply.body)
        )));
    }
    let key = oauth::string_field(&reply.json(), "key")
        .ok_or_else(|| AuthError::OAuth("OpenRouter's response carries no \"key\"".into()))?;
    Ok(OAuthCredential {
        access_token: key,
        refresh_token: String::new(),
        expires: NEVER_EXPIRES,
        client_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::testing::TestServer;
    use std::collections::BTreeMap;

    #[test]
    fn the_authorize_url_names_the_loopback_callback() {
        let pkce = Pkce::from_verifier("v".into());
        let url = reqwest::Url::parse(&authorize_url(
            &pkce,
            "http://127.0.0.1:5555/oauth/callback/x",
        ))
        .unwrap();
        let params: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(
            params["callback_url"],
            "http://127.0.0.1:5555/oauth/callback/x"
        );
        assert_eq!(params["code_challenge"], pkce.challenge);
        assert_eq!(params["code_challenge_method"], "S256");
    }

    #[test]
    fn a_pasted_redirect_or_code_yields_the_code() {
        assert_eq!(
            code_from_pasted("http://127.0.0.1:5555/oauth/callback/x?code=abc").as_deref(),
            Some("abc")
        );
        assert_eq!(code_from_pasted("code=abc").as_deref(), Some("abc"));
        assert_eq!(code_from_pasted("abc").as_deref(), Some("abc"));
        assert_eq!(code_from_pasted(""), None);
    }

    #[tokio::test]
    async fn the_code_becomes_a_key_that_never_expires() {
        let server = TestServer::start(vec![
            (200, r#"{"key":"sk-or-v1-abc","user_id":"u"}"#.into()),
            (403, r#"{"error":{"message":"invalid code"}}"#.into()),
        ])
        .await;
        let http = reqwest::Client::new();
        let url = server.url("/api/v1/auth/keys");

        let credential = exchange_code(&http, &url, "c", "v").await.unwrap();
        assert_eq!(credential.access_token, "sk-or-v1-abc");
        assert_eq!(credential.expires, NEVER_EXPIRES);
        let body = server.requests()[0].json();
        assert_eq!(body["code"], "c");
        assert_eq!(body["code_verifier"], "v");
        assert_eq!(body["code_challenge_method"], "S256");

        let error = exchange_code(&http, &url, "c", "v").await.unwrap_err();
        assert!(error.to_string().contains("invalid code"), "{error}");
    }
}
