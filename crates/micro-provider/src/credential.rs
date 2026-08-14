//! The credential a request is sent with.

use micro_auth::AuthStore;
use std::sync::Arc;

/// The key a request carries, asked for again before every request.
///
/// Most services issue a key that outlives the process, and for those this is simply the
/// string it was built from. Copilot is not one of them: its API token is minted from the
/// stored GitHub token and lives about half an hour, so a run that reads the token once
/// and keeps it is refused partway through any session that lasts longer than that.
/// Holding the store instead of the token means the store exchanges a lapsed credential
/// and the next request carries the one it just minted.
#[derive(Clone)]
pub enum ApiKey {
    /// A key that does not change: an API key, or one an extension declared alongside the
    /// provider it serves.
    Fixed(String),
    /// A credential the store keeps current.
    Stored {
        store: Arc<AuthStore>,
        provider: String,
        /// The token as it stood when the provider was resolved. This is what a caller
        /// reads when it only wants to look at the credential, and what is sent when the
        /// store cannot be consulted.
        resolved: String,
    },
}

impl ApiKey {
    /// The key to send now. A stored credential inside its expiry window is exchanged
    /// first, so this is the call that keeps a long session authenticated.
    pub async fn current(&self) -> String {
        match self {
            ApiKey::Fixed(key) => key.clone(),
            ApiKey::Stored {
                store,
                provider,
                resolved,
            } => match store.resolve(provider).await {
                Ok(credential) => credential.token().to_string(),
                // An exchange that failed is no reason to send nothing: the token in hand
                // may still be good, and the service is the one to say it is not.
                Err(_) => resolved.clone(),
            },
        }
    }

    /// The key as it stands, without touching the network. Reading it is how a caller
    /// tells one kind of credential from another, such as a subscription token from a key.
    pub fn as_str(&self) -> &str {
        match self {
            ApiKey::Fixed(key) => key,
            ApiKey::Stored { resolved, .. } => resolved,
        }
    }

    /// Whether there is no credential here at all, which every service reports as a
    /// missing authentication header rather than as a bad key.
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
        assert_eq!(key.current().await, "sk-test");
        assert!(!key.is_blank());
        assert!(ApiKey::from("  ").is_blank());
    }

    /// The whole point of a stored key: what was read at startup is not what is sent
    /// later, because the store has the newer credential.
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
        assert_eq!(key.current().await, "first");

        store
            .set("anthropic", Credential::api_key("second"))
            .unwrap();
        assert_eq!(key.current().await, "second");
        // What was resolved is still what a caller sees without asking the store.
        assert_eq!(key.as_str(), "first");
    }

    /// A credential the store cannot produce leaves the request carrying the last one
    /// that worked, so the failure is reported by the service rather than as an empty
    /// authentication header.
    #[tokio::test]
    async fn a_credential_the_store_has_lost_falls_back_to_the_one_in_hand() {
        let store = Arc::new(AuthStore::open_at(scratch("lost").join("auth.json")).unwrap());
        let key = ApiKey::Stored {
            store,
            provider: "anthropic".into(),
            resolved: "in-hand".into(),
        };
        assert_eq!(key.current().await, "in-hand");
    }

    /// A Copilot credential whose expiry has passed is exchanged before it is sent, and
    /// an exchange that cannot be made leaves the stale token rather than nothing.
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
        assert_eq!(key.current().await, "expired");
    }
}
