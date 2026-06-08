//! The credential a request is sent with.

use micro_auth::AuthStore;
use std::sync::Arc;

/// The key a request carries, asked for again before every request.
#[derive(Clone)]
pub enum ApiKey {
    /// A key that does not change: an API key, or one an extension declared alongside the provider
    /// it serves.
    Fixed(String),
    /// A credential the store keeps current.
    Stored {
        store: Arc<AuthStore>,
        provider: String,
        /// The token as it stood when the provider was resolved.
        resolved: String,
    },
}

impl ApiKey {
    /// The key to send now, or why there is none to send.
    ///
    /// A store that cannot produce the credential leaves in hand the last one that worked, which the
    /// provider is welcome to refuse: whether a token is still good is the provider's to say. It is
    /// only when there is nothing whatever to send that a request is not worth making, and then what
    /// the store said is the answer — a service that would not renew a token, or an account that was
    /// never signed in, is not something a provider can explain on our behalf.
    pub async fn current(&self) -> Result<String, String> {
        match self {
            ApiKey::Fixed(key) => Ok(key.clone()),
            ApiKey::Stored {
                store,
                provider,
                resolved,
            } => match store.resolve(provider).await {
                Ok(credential) => Ok(credential.token().to_string()),

                Err(_) if !resolved.trim().is_empty() => Ok(resolved.clone()),
                Err(error) => Err(format!("no credential for {provider}: {error}")),
            },
        }
    }

    /// The key as it stands, without touching the network.
    pub fn as_str(&self) -> &str {
        match self {
            ApiKey::Fixed(key) => key,
            ApiKey::Stored { resolved, .. } => resolved,
        }
    }

    /// Whether there is no credential here at all.
    pub fn is_blank(&self) -> bool {
        self.as_str().trim().is_empty()
    }
}

impl From<String> for ApiKey {
    fn from(key: String) -> Self {
        ApiKey::Fixed(key)
    }
}

impl From<&str> for ApiKey {
    fn from(key: &str) -> Self {
        ApiKey::Fixed(key.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micro_auth::Credential;
    use micro_auth::OAuthCredential;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("micro-credential-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn a_fixed_key_is_what_it_was_built_from() {
        let key = ApiKey::from("sk-test");
        assert_eq!(key.as_str(), "sk-test");
        assert_eq!(key.current().await, Ok("sk-test".to_string()));
        assert!(!key.is_blank());
        assert!(ApiKey::from("  ").is_blank());
    }

    /// The whole point of a stored key: what was read at startup is not what is sent later, because
    /// the store has the newer credential.
    #[tokio::test]
    async fn a_stored_key_follows_the_store_rather_than_the_token_it_was_built_with() {
        let store = Arc::new(AuthStore::open_at(scratch("stored").join("auth.json")).unwrap());
        store
            .set("anthropic", Credential::api_key("first"))
            .unwrap();

        let key = ApiKey::Stored {
            store: Arc::clone(&store),
            provider: "anthropic".into(),
            resolved: "first".into(),
        };
        assert_eq!(key.current().await, Ok("first".to_string()));

        store
            .set("anthropic", Credential::api_key("second"))
            .unwrap();
        assert_eq!(key.current().await, Ok("second".to_string()));

        assert_eq!(key.as_str(), "first");
    }

    /// With nothing in hand and nothing in the store, there is no request to make, and what the
    /// store said is what there is to report: an empty header is the provider's complaint, not the
    /// reason behind it.
    #[tokio::test]
    async fn a_credential_that_is_nowhere_is_reported_rather_than_sent_empty() {
        let store = Arc::new(AuthStore::open_at(scratch("nowhere").join("auth.json")).unwrap());
        let key = ApiKey::Stored {
            store,
            provider: micro_auth::GITHUB_COPILOT.into(),
            resolved: String::new(),
        };

        let error = key.current().await.expect_err("there is nothing to send");
        assert!(
            error.starts_with("no credential for github-copilot"),
            "{error}"
        );
        assert!(key.is_blank());
    }

    /// A credential the store cannot produce leaves the request carrying the last one that worked.
    #[tokio::test]
    async fn a_credential_the_store_has_lost_falls_back_to_the_one_in_hand() {
        let store = Arc::new(AuthStore::open_at(scratch("lost").join("auth.json")).unwrap());
        let key = ApiKey::Stored {
            store,
            provider: "anthropic".into(),
            resolved: "in-hand".into(),
        };
        assert_eq!(key.current().await, Ok("in-hand".to_string()));
    }

    #[tokio::test]
    async fn a_lapsed_copilot_token_is_not_sent_as_it_stands() {
        let store = Arc::new(AuthStore::open_at(scratch("lapsed").join("auth.json")).unwrap());
        store
            .set(
                micro_auth::GITHUB_COPILOT,
                Credential::OAuth(OAuthCredential {
                    access_token: "expired".into(),
                    refresh_token: String::new(),
                    expires: 1,
                }),
            )
            .unwrap();

        let key = ApiKey::Stored {
            store,
            provider: micro_auth::GITHUB_COPILOT.into(),
            resolved: "expired".into(),
        };
        assert_eq!(key.current().await, Ok("expired".to_string()));
    }
}
