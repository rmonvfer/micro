//! Credentials for the providers micro talks to.
//!
//! Credentials live in `auth.json`, under micro's configuration directory — one entry per
//! provider, in a file only the owner can read. Resolving a provider prefers the stored
//! credential, exchanges it for a fresh token when the provider issues short-lived ones,
//! and falls back to the conventional environment variable when nothing is stored.

pub mod copilot;
mod lockfile;

use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub const ANTHROPIC: &str = "anthropic";
pub const GOOGLE: &str = "google";
pub const GITHUB_COPILOT: &str = "github-copilot";
pub const OPENAI: &str = "openai";
pub const OPENROUTER: &str = "openrouter";
/// The ChatGPT Codex backend, reached with a ChatGPT subscription token rather than a
/// platform API key. Kept apart from `openai` because the credential is not interchangeable.
pub const OPENAI_CODEX: &str = "openai-codex";

/// One service micro can authenticate.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderEntry {
    /// The canonical id: the key its credential is stored under, and the name a UI
    /// hands back.
    pub id: String,
    /// The name to show a person.
    pub name: String,
    /// Environment variables that supply its key, in the order they are tried.
    pub env: Vec<String>,
    /// What to call the credential when asking for it.
    pub key: String,
}

/// Every provider micro can authenticate, generated alongside the model catalog so the
/// two always name the same services.
static TABLE: &str = include_str!("../data/providers.json");

pub fn provider_table() -> &'static [ProviderEntry] {
    static PARSED: std::sync::OnceLock<Vec<ProviderEntry>> = std::sync::OnceLock::new();
    PARSED.get_or_init(|| serde_json::from_str(TABLE).expect("the generated provider table parses"))
}

/// One provider by any name it answers to.
pub fn provider_entry(name: &str) -> Option<&'static ProviderEntry> {
    let id = canonical_provider(name);
    provider_table().iter().find(|entry| entry.id == id)
}

/// Every provider id, in the order a picker should show them.
pub fn providers() -> Vec<&'static str> {
    provider_table()
        .iter()
        .map(|entry| entry.id.as_str())
        .collect()
}

/// Other names a user might type, mapped onto the canonical id.
const ALIASES: &[(&str, &str)] = &[
    ("claude", ANTHROPIC),
    ("copilot", GITHUB_COPILOT),
    ("github", GITHUB_COPILOT),
    ("gemini", GOOGLE),
    ("codex", OPENAI_CODEX),
    ("chatgpt", OPENAI_CODEX),
];

/// Fold a name onto the id everything else uses. An unknown name comes back unchanged, so
/// a provider micro does not know about can still carry a credential.
pub fn canonical_provider(name: &str) -> &str {
    let trimmed = name.trim();
    for (alias, canonical) in ALIASES {
        if trimmed.eq_ignore_ascii_case(alias) {
            return canonical;
        }
    }
    for entry in provider_table() {
        if trimmed.eq_ignore_ascii_case(&entry.id) {
            return entry.id.as_str();
        }
    }
    trimmed
}

/// How a provider expects to be authenticated, which decides the login a UI presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// The user pastes a key.
    ApiKey,
    /// The user authorizes micro in a browser, through the device-code flow.
    OAuth,
}

pub fn auth_method(provider: &str) -> AuthMethod {
    match canonical_provider(provider) {
        GITHUB_COPILOT => AuthMethod::OAuth,
        _ => AuthMethod::ApiKey,
    }
}

const FILE_NAME: &str = "auth.json";
/// Refresh slightly ahead of the server's expiry so a request never races it.
const EXPIRY_SKEW_MS: i64 = 60_000;

pub type Result<T, E = AuthError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credential stored for `{provider}`; log in or set one of: {env}")]
    Missing { provider: String, env: String },

    #[error("`{provider}` credentials cannot be refreshed automatically")]
    NoRefresh { provider: String },

    #[error("no key given for `{provider}`")]
    EmptyKey { provider: String },

    #[error("cannot import from {path}: {message}")]
    Import { path: String, message: String },

    #[error("credential store {path}: {message}")]
    Storage { path: String, message: String },

    #[error("GitHub device authorization failed: {0}")]
    DeviceFlow(String),

    #[error("Copilot token exchange failed: {0}")]
    TokenExchange(String),
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// An OAuth credential.
///
/// For GitHub Copilot the refresh token is the long-lived GitHub OAuth token obtained
/// from the device flow, and the access token is the short-lived Copilot API token minted
/// from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCredential {
    pub access_token: String,
    pub refresh_token: String,
    /// Milliseconds since the Unix epoch. Zero means the token carries no expiry.
    pub expires: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Credential {
    #[serde(rename = "api_key")]
    ApiKey { key: String },
    #[serde(rename = "oauth")]
    OAuth(OAuthCredential),
}

impl Credential {
    pub fn api_key(key: impl Into<String>) -> Self {
        Credential::ApiKey { key: key.into() }
    }

    /// The bearer value to send to the provider.
    pub fn token(&self) -> &str {
        match self {
            Credential::ApiKey { key } => key,
            Credential::OAuth(oauth) => &oauth.access_token,
        }
    }
}

/// What a UI must do next to log a provider in.
pub enum LoginFlow {
    /// Prompt for a key, then hand it to [`AuthStore::store_api_key`]. The environment
    /// variables are worth naming in the prompt, since setting one is the alternative.
    ApiKey {
        provider: String,
        env_names: Vec<String>,
    },
    /// Show the URL and code, then await [`AuthStore::complete_device_login`].
    DeviceCode(PendingDeviceLogin),
}

/// A device authorization waiting to be redeemed.
pub struct PendingDeviceLogin {
    pub provider: String,
    pub authorization: copilot::DeviceAuthorization,
}

impl PendingDeviceLogin {
    /// The page the user opens.
    pub fn verification_uri(&self) -> &str {
        &self.authorization.verification_uri
    }

    /// The code the user types into that page.
    pub fn user_code(&self) -> &str {
        &self.authorization.user_code
    }

    /// Seconds the code stays valid.
    pub fn expires_in_secs(&self) -> u64 {
        self.authorization.expires_in_secs
    }
}

/// Where a provider's credential comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSource {
    /// The credential file.
    Stored,
    /// An environment variable, which micro reads but never writes.
    Environment {
        variable: String,
    },
    Missing,
}

/// One provider's standing, for a UI to render without touching the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatus {
    pub provider: String,
    pub method: AuthMethod,
    pub source: CredentialSource,
    /// When the stored token lapses, in milliseconds since the Unix epoch. Absent for
    /// keys and for tokens that carry no expiry.
    pub expires: Option<i64>,
    /// The stored token is past its expiry and will be exchanged on the next request.
    /// This is a note, not a fault: the provider is still authenticated.
    pub needs_refresh: bool,
}

impl ProviderStatus {
    pub fn is_authenticated(&self) -> bool {
        !matches!(self.source, CredentialSource::Missing)
    }
}

/// What was read from the file, and the state of the file it was read from.
///
/// The revision is how a read notices that another process has written since, so a
/// long-lived session sees a credential stored by `micro auth login` beside it.
#[derive(Default)]
struct Cache {
    credentials: BTreeMap<String, Credential>,
    revision: Option<Revision>,
}

/// Enough of the file's state to tell one version of it from the next.
#[derive(PartialEq, Eq, Clone, Copy)]
struct Revision {
    modified: Option<std::time::SystemTime>,
    length: u64,
}

/// The credential file, kept in memory and rewritten whenever an entry changes.
pub struct AuthStore {
    path: PathBuf,
    cache: Mutex<Cache>,
    http: reqwest::Client,
}

impl AuthStore {
    /// Open the store at the default path, creating nothing until a credential is stored.
    pub fn open() -> Result<Self> {
        Self::open_at(default_path()?)
    }

    pub fn open_at(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let credentials = load(&path)?;
        let revision = revision_of(&path);
        Ok(AuthStore {
            path,
            cache: Mutex::new(Cache {
                credentials,
                revision,
            }),
            http: reqwest::Client::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, provider: &str) -> Option<Credential> {
        let mut cache = self.lock();
        refresh(&self.path, &mut cache);
        cache.credentials.get(canonical_provider(provider)).cloned()
    }

    pub fn set(&self, provider: &str, credential: Credential) -> Result<()> {
        let provider = canonical_provider(provider).to_string();
        self.mutate(move |credentials| {
            credentials.insert(provider, credential);
        })
    }

    pub fn remove(&self, provider: &str) -> Result<()> {
        let provider = canonical_provider(provider).to_string();
        self.mutate(move |credentials| {
            credentials.remove(&provider);
        })
    }

    /// Every provider with a stored credential, sorted.
    pub fn providers(&self) -> Vec<String> {
        let mut cache = self.lock();
        refresh(&self.path, &mut cache);
        cache.credentials.keys().cloned().collect()
    }

    /// Change the file while holding it against every other process.
    ///
    /// The file is read again inside the lock rather than trusted from startup, so a
    /// credential stored since then is carried forward instead of being written over.
    fn mutate(&self, change: impl FnOnce(&mut BTreeMap<String, Credential>)) -> Result<()> {
        let mut cache = self.lock();
        let _held = lockfile::FileLock::acquire(&self.path)
            .map_err(|error| storage_error(&self.path, error))?;

        let mut latest = load(&self.path)?;
        change(&mut latest);
        save(&self.path, &latest)?;

        cache.credentials = latest;
        cache.revision = revision_of(&self.path);
        Ok(())
    }

    /// A credential ready to send: stored if present, otherwise from the environment,
    /// refreshed first if the provider's tokens expire.
    pub async fn resolve(&self, provider: &str) -> Result<Credential> {
        let provider = canonical_provider(provider);

        if let Some(stored) = self.get(provider) {
            return self.prepare(provider, stored, true).await;
        }

        if let Some(value) = env_value(provider, |name| std::env::var(name).ok()) {
            return self
                .prepare(provider, from_env(provider, value), false)
                .await;
        }

        Err(AuthError::Missing {
            provider: provider.to_string(),
            env: env_names(provider).join(", "),
        })
    }

    /// Force a refresh of a stored OAuth credential, whatever its recorded expiry.
    pub async fn refresh(&self, provider: &str) -> Result<Credential> {
        let provider = canonical_provider(provider);
        let Some(Credential::OAuth(stored)) = self.get(provider) else {
            return Err(AuthError::NoRefresh {
                provider: provider.to_string(),
            });
        };
        let refreshed = refresh_oauth(&self.http, provider, &stored).await?;
        let credential = Credential::OAuth(refreshed);
        self.set(provider, credential.clone())?;
        Ok(credential)
    }

    /// Begin an interactive login. An API-key provider needs nothing from the network and
    /// answers immediately with the prompt to show; an OAuth provider reserves a device
    /// code the UI must display before awaiting [`AuthStore::complete_device_login`].
    pub async fn begin_login(&self, provider: &str) -> Result<LoginFlow> {
        let provider = canonical_provider(provider).to_string();
        match auth_method(&provider) {
            AuthMethod::ApiKey => Ok(LoginFlow::ApiKey {
                env_names: env_names(&provider),
                provider,
            }),
            AuthMethod::OAuth => {
                let authorization = copilot::start_device_flow(&self.http).await?;
                Ok(LoginFlow::DeviceCode(PendingDeviceLogin {
                    provider,
                    authorization,
                }))
            }
        }
    }

    /// Wait for the user to finish authorizing in the browser, then store the credential.
    /// Runs until the device code is redeemed or expires, so a UI should show progress.
    pub async fn complete_device_login(&self, pending: &PendingDeviceLogin) -> Result<Credential> {
        let credential =
            Credential::OAuth(copilot::poll_for_token(&self.http, &pending.authorization).await?);
        self.set(&pending.provider, credential.clone())?;
        Ok(credential)
    }

    /// Store a key the user pasted. Blank input is refused rather than written.
    pub fn store_api_key(&self, provider: &str, key: &str) -> Result<Credential> {
        let key = key.trim();
        if key.is_empty() {
            return Err(AuthError::EmptyKey {
                provider: canonical_provider(provider).to_string(),
            });
        }
        let credential = Credential::api_key(key);
        self.set(provider, credential.clone())?;
        Ok(credential)
    }

    /// Forget a provider's stored credential. The environment is untouched, so a provider
    /// configured that way stays usable.
    pub fn logout(&self, provider: &str) -> Result<()> {
        self.remove(provider)
    }

    /// Where every provider stands, for a UI to render. Reads the environment but never
    /// the network, so it is safe to call on every frame.
    pub fn status(&self) -> Vec<ProviderStatus> {
        let known = providers();
        let stored = self.providers();
        let extra = stored
            .iter()
            .map(String::as_str)
            .filter(|provider| !known.contains(provider));

        known
            .iter()
            .copied()
            .chain(extra)
            .map(|provider| self.status_of(provider))
            .collect()
    }

    pub fn status_of(&self, provider: &str) -> ProviderStatus {
        let provider = canonical_provider(provider).to_string();
        let method = auth_method(&provider);

        let (source, expires, needs_refresh) = match self.get(&provider) {
            Some(Credential::OAuth(oauth)) => (
                CredentialSource::Stored,
                (oauth.expires > 0).then_some(oauth.expires),
                needs_refresh(&provider, &oauth, now_ms()),
            ),
            Some(Credential::ApiKey { .. }) => (CredentialSource::Stored, None, false),
            None => match env_names(&provider)
                .into_iter()
                .find(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()))
            {
                Some(variable) => (CredentialSource::Environment { variable }, None, false),
                None => (CredentialSource::Missing, None, false),
            },
        };

        ProviderStatus {
            provider,
            method,
            source,
            expires,
            needs_refresh,
        }
    }

    /// Only credentials that came from the file are written back; an environment token is
    /// the user's to manage, so a token exchanged from one stays in memory.
    async fn prepare(
        &self,
        provider: &str,
        credential: Credential,
        persist: bool,
    ) -> Result<Credential> {
        match credential {
            Credential::ApiKey { key } => Ok(Credential::ApiKey {
                key: expand(&key, |name| std::env::var(name).ok()),
            }),
            Credential::OAuth(oauth) if needs_refresh(provider, &oauth, now_ms()) => {
                let refreshed =
                    Credential::OAuth(refresh_oauth(&self.http, provider, &oauth).await?);
                if persist {
                    self.set(provider, refreshed.clone())?;
                }
                Ok(refreshed)
            }
            other => Ok(other),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The file's current state, or nothing when there is no file yet.
fn revision_of(path: &Path) -> Option<Revision> {
    let metadata = fs::metadata(path).ok()?;
    Some(Revision {
        modified: metadata.modified().ok(),
        length: metadata.len(),
    })
}

/// Read the file again if another process has written it since it was last read.
///
/// A failed read leaves what is already held rather than emptying it: a file being
/// replaced by another process is briefly unreadable, and that is not the same as a
/// credential having been removed.
fn refresh(path: &Path, cache: &mut Cache) {
    let current = revision_of(path);
    if current == cache.revision {
        return;
    }
    if let Ok(latest) = load(path) {
        cache.credentials = latest;
        cache.revision = current;
    }
}

async fn refresh_oauth(
    http: &reqwest::Client,
    provider: &str,
    credential: &OAuthCredential,
) -> Result<OAuthCredential> {
    match provider {
        GITHUB_COPILOT => copilot::exchange_token(http, &credential.refresh_token).await,
        _ => Err(AuthError::NoRefresh {
            provider: provider.to_string(),
        }),
    }
}

/// Whether an OAuth credential must be exchanged before it can be used.
///
/// A Copilot API token is minted from the stored GitHub token and lives about half an
/// hour, so a Copilot credential with no recorded expiry has never been exchanged and is
/// not usable as it stands. Every other provider treats a zero expiry as "never expires".
fn needs_refresh(provider: &str, credential: &OAuthCredential, now: i64) -> bool {
    match provider {
        GITHUB_COPILOT => credential.expires <= now + EXPIRY_SKEW_MS,
        _ => credential.expires > 0 && credential.expires <= now + EXPIRY_SKEW_MS,
    }
}

/// A Copilot token found in the environment is a GitHub OAuth token, which buys API
/// tokens rather than being one; everything else is used as it stands.
fn from_env(provider: &str, value: String) -> Credential {
    match provider {
        GITHUB_COPILOT => Credential::OAuth(OAuthCredential {
            access_token: String::new(),
            refresh_token: value,
            expires: 0,
        }),
        _ => Credential::ApiKey { key: value },
    }
}

/// Environment variables to try for a provider, in order. A provider the table does not
/// name gets the conventional `<PROVIDER>_API_KEY`, which is what an extension declaring
/// its own provider relies on.
pub fn env_names(provider: &str) -> Vec<String> {
    match provider_entry(provider) {
        Some(entry) if !entry.env.is_empty() => entry.env.clone(),
        _ => vec![format!(
            "{}_API_KEY",
            canonical_provider(provider)
                .to_uppercase()
                .replace('-', "_")
        )],
    }
}

fn env_value(provider: &str, get: impl Fn(&str) -> Option<String>) -> Option<String> {
    env_names(provider)
        .into_iter()
        .find_map(|name| get(&name).filter(|value| !value.trim().is_empty()))
}

/// A stored key may point at the environment as `$VAR`. A name that is not set is left
/// alone so the failure surfaces at the provider rather than as a silently empty key.
fn expand(raw: &str, get: impl Fn(&str) -> Option<String>) -> String {
    match raw.strip_prefix('$') {
        Some(name) if !name.is_empty() => get(name).unwrap_or_else(|| raw.to_string()),
        _ => raw.to_string(),
    }
}

/// `auth.json`, under whichever directory holds micro's configuration.
pub fn default_path() -> Result<PathBuf> {
    let directory = micro_dirs::config_dir().ok_or_else(|| AuthError::Storage {
        path: "micro's configuration directory".into(),
        message: format!("no home directory; set {}", micro_dirs::MICRO_DIR_ENV),
    })?;
    Ok(directory.join(FILE_NAME))
}

/// Read the store. A missing file is an empty store; an entry that does not parse is
/// skipped so one damaged credential cannot lock every provider out.
fn load(path: &Path) -> Result<BTreeMap<String, Credential>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(storage_error(path, error)),
    };

    let raw: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&contents).map_err(|error| AuthError::Storage {
            path: path.display().to_string(),
            message: format!("is not valid JSON: {error}"),
        })?;

    Ok(raw
        .into_iter()
        .filter_map(|(provider, value)| {
            serde_json::from_value(value)
                .ok()
                .map(|credential| (provider, credential))
        })
        .collect())
}

/// Write the store through a temporary file created owner-only, so the credentials are
/// never briefly world-readable between creation and a follow-up permission change.
fn save(path: &Path, credentials: &BTreeMap<String, Credential>) -> Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory).map_err(|error| storage_error(path, error))?;
    restrict_directory(directory);

    let contents =
        serde_json::to_string_pretty(credentials).map_err(|error| AuthError::Storage {
            path: path.display().to_string(),
            message: format!("could not be encoded: {error}"),
        })?;

    let temporary = directory.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&temporary);

    let mut file = create_owner_only(&temporary).map_err(|error| storage_error(path, error))?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| storage_error(path, error))?;
    drop(file);

    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        storage_error(path, error)
    })
}

fn storage_error(path: &Path, error: std::io::Error) -> AuthError {
    AuthError::Storage {
        path: path.display().to_string(),
        message: error.to_string(),
    }
}

#[cfg(unix)]
fn create_owner_only(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_owner_only(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Credentials should not sit in a directory anyone else can list.
#[cfg(unix)]
fn restrict_directory(directory: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = fs::set_permissions(directory, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_directory(_directory: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU32;
    use std::sync::atomic::Ordering;

    /// A directory of this process's own, so tests never touch a real credential file.
    fn scratch(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "micro-auth-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn oauth(access: &str, expires: i64) -> Credential {
        Credential::OAuth(OAuthCredential {
            access_token: access.into(),
            refresh_token: "gho_github".into(),
            expires,
        })
    }

    /// Two processes writing different providers both keep their work.
    ///
    /// Each store reads the file when it opens. Without a lock and a fresh read at write
    /// time, the second one writes the map it read at startup and the first one's
    /// credential is gone with nothing to show that it ever arrived.
    #[test]
    fn a_write_carries_forward_what_another_store_wrote() {
        let path = scratch("concurrent").join("auth.json");

        let session = AuthStore::open_at(&path).unwrap();
        let login = AuthStore::open_at(&path).unwrap();

        session
            .set("anthropic", Credential::api_key("from-session"))
            .unwrap();
        login
            .set("openai", Credential::api_key("from-login"))
            .unwrap();

        let read_back = AuthStore::open_at(&path).unwrap();
        assert_eq!(
            read_back.get("anthropic"),
            Some(Credential::api_key("from-session")),
            "the first write survived the second",
        );
        assert_eq!(
            read_back.get("openai"),
            Some(Credential::api_key("from-login")),
        );
    }

    /// A store that has been open a while notices a credential stored beside it.
    #[test]
    fn a_read_sees_what_another_store_wrote() {
        let path = scratch("reload").join("auth.json");

        let session = AuthStore::open_at(&path).unwrap();
        assert_eq!(session.get("anthropic"), None);

        AuthStore::open_at(&path)
            .unwrap()
            .set("anthropic", Credential::api_key("signed-in"))
            .unwrap();

        assert_eq!(
            session.get("anthropic"),
            Some(Credential::api_key("signed-in")),
            "the long-lived store re-read rather than serving its startup snapshot",
        );
    }

    /// A credential is something the user put there, so it travels with the settings and
    /// not with what micro produced.
    #[test]
    fn credentials_sit_in_the_configuration_directory() {
        assert_eq!(
            default_path().unwrap(),
            micro_dirs::config_dir().unwrap().join(FILE_NAME)
        );
    }

    #[test]
    fn credentials_use_the_documented_on_disk_shape() {
        let encoded = serde_json::to_value(oauth("copilot-token", 42)).unwrap();
        assert_eq!(encoded["type"], "oauth");
        assert_eq!(encoded["accessToken"], "copilot-token");
        assert_eq!(encoded["refreshToken"], "gho_github");
        assert_eq!(encoded["expires"], 42);

        let encoded = serde_json::to_value(Credential::api_key("sk-test")).unwrap();
        assert_eq!(encoded["type"], "api_key");
        assert_eq!(encoded["key"], "sk-test");
    }

    #[test]
    fn unknown_credential_fields_are_ignored_and_broken_entries_skipped() {
        let directory = scratch("mixed");
        let path = directory.join("auth.json");
        fs::write(
            &path,
            r#"{
                "openrouter": { "type": "api_key", "key": "sk-or", "note": "extra" },
                "mystery": { "type": "totally-unknown" }
            }"#,
        )
        .unwrap();

        let store = AuthStore::open_at(&path).unwrap();
        assert_eq!(store.providers(), vec!["openrouter".to_string()]);
        assert_eq!(store.get(OPENROUTER).unwrap().token(), "sk-or");
    }

    #[test]
    fn a_missing_file_is_an_empty_store() {
        let store = AuthStore::open_at(scratch("absent").join("auth.json")).unwrap();
        assert!(store.providers().is_empty());
    }

    #[test]
    fn stored_credentials_survive_a_reopen() {
        let path = scratch("round-trip").join("auth.json");
        let store = AuthStore::open_at(&path).unwrap();
        store.set(OPENROUTER, Credential::api_key("sk-or")).unwrap();
        store
            .set(GITHUB_COPILOT, oauth("copilot-token", 99))
            .unwrap();
        store.remove(OPENROUTER).unwrap();

        let reopened = AuthStore::open_at(&path).unwrap();
        assert_eq!(reopened.providers(), vec![GITHUB_COPILOT.to_string()]);
        assert_eq!(
            reopened.get(GITHUB_COPILOT),
            Some(oauth("copilot-token", 99))
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_credential_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = scratch("permissions");
        let path = directory.join("auth.json");
        let store = AuthStore::open_at(&path).unwrap();
        store.set(OPENROUTER, Credential::api_key("sk-or")).unwrap();

        let file = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let parent = fs::metadata(&directory).unwrap().permissions().mode() & 0o777;
        assert_eq!(file, 0o600);
        assert_eq!(parent, 0o700);
    }

    #[test]
    fn copilot_credentials_expire_even_without_a_recorded_expiry() {
        let now = 1_000_000;
        let stale = OAuthCredential {
            access_token: "old".into(),
            refresh_token: "gho".into(),
            expires: 0,
        };
        assert!(needs_refresh(GITHUB_COPILOT, &stale, now));
        assert!(!needs_refresh(ANTHROPIC, &stale, now));
    }

    #[test]
    fn a_token_is_refreshed_once_it_is_inside_the_skew_window() {
        let now = 1_000_000;
        let expiring = |expires| OAuthCredential {
            access_token: "token".into(),
            refresh_token: "gho".into(),
            expires,
        };

        assert!(needs_refresh(GITHUB_COPILOT, &expiring(now - 1), now));
        assert!(needs_refresh(
            GITHUB_COPILOT,
            &expiring(now + EXPIRY_SKEW_MS - 1),
            now
        ));
        assert!(!needs_refresh(
            GITHUB_COPILOT,
            &expiring(now + EXPIRY_SKEW_MS + 1),
            now
        ));
        assert!(!needs_refresh(
            ANTHROPIC,
            &expiring(now + EXPIRY_SKEW_MS + 1),
            now
        ));
    }

    #[test]
    fn each_provider_has_its_conventional_environment_variables() {
        assert_eq!(env_names(OPENROUTER), vec!["OPENROUTER_API_KEY"]);
        assert_eq!(env_names(GOOGLE), vec!["GEMINI_API_KEY"]);
        assert_eq!(env_names(GITHUB_COPILOT), vec!["COPILOT_GITHUB_TOKEN"]);
        // A bearer token is tried first, since it is what points micro at a gateway and
        // is set deliberately; then a subscription token, which a signed-in plan issues;
        // then a platform key.
        assert_eq!(
            env_names(ANTHROPIC),
            vec![
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_OAUTH_TOKEN",
                "ANTHROPIC_API_KEY"
            ]
        );
        // A provider the table does not name still has a conventional variable.
        assert_eq!(env_names("z-ai"), vec!["Z_AI_API_KEY"]);
    }

    #[test]
    fn spoken_names_fold_onto_the_canonical_id() {
        assert_eq!(canonical_provider("copilot"), GITHUB_COPILOT);
        assert_eq!(canonical_provider("Copilot"), GITHUB_COPILOT);
        assert_eq!(canonical_provider("gemini"), GOOGLE);
        assert_eq!(canonical_provider("claude"), ANTHROPIC);
        assert_eq!(canonical_provider(" google "), GOOGLE);
        assert_eq!(canonical_provider("cerebras"), "cerebras");
    }

    #[test]
    fn only_copilot_logs_in_through_a_browser() {
        assert_eq!(auth_method(GITHUB_COPILOT), AuthMethod::OAuth);
        assert_eq!(auth_method("copilot"), AuthMethod::OAuth);
        for provider in [ANTHROPIC, OPENROUTER, GOOGLE, OPENAI] {
            assert_eq!(auth_method(provider), AuthMethod::ApiKey);
        }
    }

    #[test]
    fn an_alias_reaches_the_credential_stored_under_the_canonical_id() {
        let store = AuthStore::open_at(scratch("aliases").join("auth.json")).unwrap();
        store.store_api_key("Google", "gemini-key").unwrap();

        assert_eq!(store.providers(), vec![GOOGLE.to_string()]);
        assert_eq!(store.get("gemini").unwrap().token(), "gemini-key");
        assert_eq!(store.get("google").unwrap().token(), "gemini-key");
    }

    #[test]
    fn a_blank_key_is_refused_rather_than_stored() {
        let store = AuthStore::open_at(scratch("blank").join("auth.json")).unwrap();
        let error = store.store_api_key(OPENROUTER, "   ").unwrap_err();

        assert!(matches!(error, AuthError::EmptyKey { .. }), "{error}");
        assert!(store.providers().is_empty());
    }

    #[test]
    fn logging_out_forgets_only_the_stored_credential() {
        let store = AuthStore::open_at(scratch("logout").join("auth.json")).unwrap();
        store.store_api_key(OPENROUTER, "sk-or").unwrap();
        store.logout(OPENROUTER).unwrap();

        assert!(store.providers().is_empty());
        // Whatever the environment holds, nothing is stored any more.
        assert_ne!(store.status_of(OPENROUTER).source, CredentialSource::Stored);
    }

    #[test]
    fn status_covers_every_known_provider_and_reports_where_its_credential_lives() {
        let store = AuthStore::open_at(scratch("status").join("auth.json")).unwrap();
        store.store_api_key(OPENROUTER, "sk-or").unwrap();
        store.set("a-proxy", Credential::api_key("sk-c")).unwrap();

        let status = store.status();
        let ids: Vec<&str> = status.iter().map(|entry| entry.provider.as_str()).collect();
        let mut expected = providers();
        // A provider carrying a credential but absent from the table is reported after it.
        expected.push("a-proxy");
        assert_eq!(ids, expected);

        let openrouter = status
            .iter()
            .find(|entry| entry.provider == OPENROUTER)
            .expect("openrouter is reported");
        assert_eq!(openrouter.source, CredentialSource::Stored);
        assert!(openrouter.is_authenticated());
        assert!(!openrouter.needs_refresh);
        assert_eq!(openrouter.method, AuthMethod::ApiKey);
    }

    #[test]
    fn a_stored_copilot_token_reports_when_it_will_be_exchanged() {
        let store = AuthStore::open_at(scratch("copilot-status").join("auth.json")).unwrap();
        store.set(GITHUB_COPILOT, oauth("stale", 1)).unwrap();

        let status = store.status_of("copilot");
        assert_eq!(status.method, AuthMethod::OAuth);
        assert_eq!(status.source, CredentialSource::Stored);
        assert!(status.is_authenticated());
        assert!(status.needs_refresh);
        assert_eq!(status.expires, Some(1));
    }

    #[tokio::test]
    async fn an_api_key_login_asks_for_a_key_without_touching_the_network() {
        let store = AuthStore::open_at(scratch("login").join("auth.json")).unwrap();
        let LoginFlow::ApiKey {
            provider,
            env_names,
        } = store.begin_login("google").await.unwrap()
        else {
            panic!("expected an api-key login");
        };

        assert_eq!(provider, GOOGLE);
        assert_eq!(env_names, vec!["GEMINI_API_KEY"]);
    }

    #[test]
    fn the_environment_is_read_in_order_and_blanks_are_skipped() {
        let environment = HashMap::from([
            ("ANTHROPIC_OAUTH_TOKEN".to_string(), "  ".to_string()),
            (
                "ANTHROPIC_API_KEY".to_string(),
                "sk-ant-from-env".to_string(),
            ),
        ]);
        let get = |name: &str| environment.get(name).cloned();

        assert_eq!(env_value(ANTHROPIC, get), Some("sk-ant-from-env".into()));
        assert_eq!(env_value(OPENROUTER, get), None);
    }

    #[test]
    fn an_environment_copilot_token_is_treated_as_a_refresh_token() {
        let Credential::OAuth(credential) = from_env(GITHUB_COPILOT, "gho_env".into()) else {
            panic!("expected an oauth credential");
        };
        assert_eq!(credential.refresh_token, "gho_env");
        assert!(credential.access_token.is_empty());
        assert_eq!(
            from_env(OPENROUTER, "sk-or".into()),
            Credential::api_key("sk-or")
        );
    }

    #[test]
    fn a_key_written_as_a_variable_name_reads_the_environment() {
        let get = |name: &str| (name == "SET").then(|| "value".to_string());
        assert_eq!(expand("$SET", get), "value");
        assert_eq!(expand("$UNSET", get), "$UNSET");
        assert_eq!(expand("literal", get), "literal");
        assert_eq!(expand("$", get), "$");
    }

    #[tokio::test]
    async fn resolving_a_stored_key_needs_no_network() {
        let store = AuthStore::open_at(scratch("resolve").join("auth.json")).unwrap();
        store.set(OPENROUTER, Credential::api_key("sk-or")).unwrap();

        let resolved = store.resolve(OPENROUTER).await.unwrap();
        assert_eq!(resolved.token(), "sk-or");
    }

    #[tokio::test]
    async fn resolving_an_unconfigured_provider_names_its_variables() {
        let store = AuthStore::open_at(scratch("missing").join("auth.json")).unwrap();
        let error = store.resolve("nowhere").await.unwrap_err();
        assert!(error.to_string().contains("NOWHERE_API_KEY"), "{error}");
    }

    #[tokio::test]
    async fn only_copilot_credentials_can_be_refreshed() {
        let store = AuthStore::open_at(scratch("no-refresh").join("auth.json")).unwrap();
        store.set(ANTHROPIC, oauth("token", 1)).unwrap();

        let error = store.refresh(ANTHROPIC).await.unwrap_err();
        assert!(matches!(error, AuthError::NoRefresh { .. }), "{error}");
    }
}
