//! Where OAuth credentials for servers live: `mcp-auth.json` beside micro's other credentials,
//! readable only by its owner, keyed by server name and URL.

use super::discovery::Discovered;
use super::ClientInformation;
use super::Tokens;
use crate::names;
use micro_auth::lockfile::FileLock;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What the file is called.
pub const FILE_NAME: &str = "mcp-auth.json";

/// One server's sign-in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredState {
    /// The client micro registered as, or was configured as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_information: Option<ClientInformation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    /// When the access token lapses, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_expire_at: Option<i64>,
    /// Where the authorization server was found, so a refresh need not look again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered: Option<Discovered>,
}

impl StoredState {
    /// Keep `tokens`, noting when they lapse.
    pub fn with_tokens(mut self, tokens: Tokens) -> StoredState {
        self.tokens_expire_at = tokens
            .expires_in
            .map(|seconds| micro_auth::now_ms() + (seconds as i64).saturating_mul(1000));
        self.tokens = Some(tokens);
        self
    }
}

/// The credential file.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    /// The file in micro's configuration directory.
    pub fn open() -> Option<CredentialStore> {
        micro_dirs::config_dir().map(|directory| CredentialStore::at(directory.join(FILE_NAME)))
    }

    pub fn at(path: impl Into<PathBuf>) -> CredentialStore {
        CredentialStore { path: path.into() }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// One server's credentials. Servers sharing a URL under different names sign in separately;
    /// the same name and URL in different files share a sign-in.
    pub fn server(&self, name: &str, url: &str) -> ServerCredentials {
        let url = reqwest::Url::parse(url)
            .map(|url| url.to_string())
            .unwrap_or_else(|_| url.to_string());
        ServerCredentials {
            store: self.clone(),
            key: format!("{}|{url}", names::namespace(name)),
        }
    }

    fn read(&self) -> BTreeMap<String, StoredState> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| {
                serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&text).ok()
            })
            .map(|raw| {
                raw.into_iter()
                    .filter_map(|(key, value)| Some((key, serde_json::from_value(value).ok()?)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Change the file while holding it against every other process.
    fn mutate<T>(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, StoredState>) -> T,
    ) -> std::io::Result<T> {
        let _held = FileLock::acquire(&self.path)?;
        let mut states = self.read();
        let outcome = change(&mut states);
        let text = serde_json::to_string_pretty(&states).map_err(std::io::Error::other)?;
        micro_auth::write_private(&self.path, format!("{text}\n").as_bytes())?;
        Ok(outcome)
    }
}

/// The credentials of one server.
#[derive(Debug, Clone)]
pub struct ServerCredentials {
    store: CredentialStore,
    key: String,
}

impl ServerCredentials {
    pub fn load(&self) -> Option<StoredState> {
        self.store.read().remove(&self.key)
    }

    pub fn save(&self, state: StoredState) -> std::io::Result<()> {
        let key = self.key.clone();
        self.store.mutate(move |states| {
            states.insert(key, state);
        })
    }

    /// Forget the server's credentials. Says whether there were any.
    pub fn remove(&self) -> std::io::Result<bool> {
        if self.load().is_none() {
            return Ok(false);
        }
        let key = self.key.clone();
        self.store
            .mutate(move |states| states.remove(&key).is_some())
    }

    /// Hold off every other process refreshing this server's tokens: many servers rotate refresh
    /// tokens, so two refreshes with the same one lose the grant.
    pub async fn refresh_lock(&self) -> std::io::Result<FileLock> {
        let digest = Sha256::digest(self.key.as_bytes());
        let hash: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let target = self
            .store
            .path
            .with_file_name(format!("mcp-auth-refresh-{hash}"));
        tokio::task::spawn_blocking(move || FileLock::acquire(&target))
            .await
            .map_err(std::io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("micro-mcp-store-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(FILE_NAME)
    }

    fn tokens(access: &str) -> Tokens {
        Tokens {
            access_token: access.into(),
            token_type: "Bearer".into(),
            expires_in: Some(60),
            scope: None,
            refresh_token: Some("refresh".into()),
        }
    }

    #[test]
    fn servers_sharing_a_url_keep_separate_accounts() {
        let store = CredentialStore::at(scratch("accounts"));
        let work = store.server("work", "https://mcp.example.com/mcp");
        let home = store.server("home", "https://mcp.example.com/mcp");

        work.save(StoredState::default().with_tokens(tokens("work-token")))
            .unwrap();
        assert!(home.load().is_none());
        assert_eq!(
            work.load().unwrap().tokens.unwrap().access_token,
            "work-token"
        );
        assert!(work.load().unwrap().tokens_expire_at.is_some());

        let same = CredentialStore::at(store.path()).server("work", "https://mcp.example.com/mcp");
        assert!(
            same.load().is_some(),
            "the same name and URL share a sign-in"
        );

        assert!(work.remove().unwrap());
        assert!(!work.remove().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_can_read_the_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let store = CredentialStore::at(scratch("private"));
        store
            .server("s", "https://x/mcp")
            .save(StoredState::default().with_tokens(tokens("t")))
            .unwrap();
        let mode = std::fs::metadata(store.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
